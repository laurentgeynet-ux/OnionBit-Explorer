// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Apply de resynchro DOM **hors verrou** (étape 55h).
//!
//! Le mur mesuré en mission 15 (`storm.html` : `setInterval`
//! mutateur à 10 ms, ~600 nœuds) : un seul `apply_dom_sync`
//! = **157 s sous verrou** (`incr.remap_relayout` 150 s) —
//! l'API mourait entre deux battements. La coalescence
//! `sync_floor` (55g) borne la cadence, jamais l'atomicité.
//!
//! Mécanisme — mêmes phases, deux contextes :
//!
//! 1. **File** : les points de synchro (`dispatch_event`,
//!    pompe JS, livraison réseau) produisent le HTML
//!    sérialisé puis `queue_apply` — une entrée par onglet
//!    (la sérialisation complète subsume la précédente).
//! 2. **Détachement** : `try_detach_paint` sort la
//!    [`PaintPayload`] de l'onglet (`mem::take` — les champs
//!    restent présents, vides : toute la session dégrade
//!    déjà sur `dom: None` comme pour une page jamais
//!    peinte, étape 56b) + un [`ApplyCtx`] cloné à coût
//!    quasi nul (Arc + copie).
//! 3. **Drain** : [`Session::drain_applies`] applique sous
//!    `&mut self` (hôtes synchrones : FFI, CLI, tests) ;
//!    [`drain_applies_shared`] applique en `spawn_blocking`
//!    puis réinsère sous verrou court (daemon, API) — le
//!    verrou n'est tenu qu'au take/commit.
//! 4. **Garde epoch** : une navigation supplantant la page
//!    invalide l'apply en vol (la payload est jetée — le
//!    nouveau `nav_epoch` a sa propre peinture) ; un
//!    `close_tab` jette l'onglet et l'apply.
//!
//! Borne connue : le coût **unitaire** de
//! `relayout_subtree` (~150 s terrain vs ~14 s profil) est
//! côté PIPE — ici on libère le verrou, pas la durée ; la
//! fenêtre d'indisponibilité de l'onglet reste le temps de
//! l'apply (dégradation documentée : `/frame` → 204 heal,
//! inputs sans arbre inertes).

use super::*;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use explorer_css::value::Color;
use explorer_css::{ComputedStyle, Origin, StyleEngine, StyleEnv, StyleMap};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;
use explorer_js::CanvasOpsTable;
use explorer_layout::font::FontSource;
use explorer_layout::{
    build_box_tree_sized, layout_tree, relayout_subtree, scrollable_overflow_bottom, BlockCtx,
    LayoutBox, Rect, Relayout,
};
use explorer_render::canvas::CanvasTable;
use explorer_render::{
    collect_data_images, decode_image, DecodedImage, DisplayItem, DisplayList, Frame, PaintImages,
    WebFontSource,
};

/// Complément de feuille UA pour `<video>`/`<audio>` sans
/// décodeur (étape 58, ADR-0023) — posé à l'origine
/// `UserAgent`, donc l'auteur l'emporte toujours. La boîte
/// elle-même est un **remplacé natif** depuis R-34 (taille
/// intrinsèque = affiche décodée, sinon gabarit standard,
/// `object-fit` honoré — tout ça dans le domaine pur) ;
/// cette feuille ne porte plus que le « look produit »
/// d'un moteur sans codec : fond noir de la vidéo, chrome
/// gris de la barre audio.
const MEDIA_UA_CSS: &str = "
video { background-color: #000 }
audio[controls] {
    background-color: #f1f3f4;
    border: 1px solid #767676;
}
";

/// Fontes de la session partagées avec un apply détaché :
/// le `WebFontSource` vit sous `Mutex` (enregistrements
/// `@font-face` pendant le fetch vs `font_data` pendant le
/// raster détaché) — chaque appel ne retient le verrou
/// qu'une recherche+clone de bytes.
pub(crate) struct SharedFonts(pub Arc<std::sync::Mutex<WebFontSource>>);

impl FontSource for SharedFonts {
    fn font_data(&self, req: &explorer_layout::font::FontRequest) -> Option<Vec<u8>> {
        self.0.lock().ok()?.font_data(req)
    }

    /// Octets + index de face (`.ttc`, R-69) — sans cette
    /// délégation le défaut du trait reservait `font_data` à
    /// l'index 0 : les faces de collection restaient
    /// inaccessibles en session.
    fn font_face(&self, req: &explorer_layout::font::FontRequest) -> Option<(Arc<Vec<u8>>, u32)> {
        self.0.lock().ok()?.font_face(req)
    }

    /// Repli par glyphe (R-69) — sans cette délégation le
    /// défaut du trait rendait `None` : le fallback mesuré
    /// PIPE était dormant en session réelle.
    fn fallback_for(
        &self,
        c: char,
        req: &explorer_layout::font::FontRequest,
    ) -> Option<(Arc<Vec<u8>>, u32)> {
        self.0.lock().ok()?.fallback_for(c, req)
    }

    /// Révision du registre `@font-face` — le `GlyphCache`
    /// persistant de la payload la lit par passe (R-36) :
    /// une face inscrite entre deux repaints vide le cache
    /// (une famille « inconnue » figée resterait repliée).
    fn revision(&self) -> u64 {
        self.0.lock().map_or(0, |w| w.revision())
    }
}

/// `BlockCtx` de la passe, amorcé sur le `FontCache`
/// persistant de la payload (R-65 — transplantation : le
/// cache voyage avec [`PaintPayload`] comme le `GlyphCache`
/// raster R-36) — symétrique de [`restitute_font_cache`].
pub(crate) fn layout_ctx<'a>(ctx: &'a ApplyCtx, p: &mut PaintPayload) -> BlockCtx<'a> {
    BlockCtx {
        fonts: Some(&ctx.fonts as &dyn FontSource),
        viewport_w: ctx.viewport.0,
        viewport_h: ctx.viewport.1,
        ..BlockCtx::default()
    }
    .with_font_cache(std::mem::take(&mut p.layout_font_cache))
}

/// Restitue le cache en fin de passe — un oubli n'est pas
/// un bug (le `BlockCtx` l'emporterait en le détruisant,
/// la passe suivante repartirait froide), juste du shaping
/// re-facturé.
pub(crate) fn restitute_font_cache(p: &mut PaintPayload, block: &BlockCtx) {
    p.layout_font_cache = block.take_font_cache();
}

/// Font-size calculée de `documentElement` en px — base des
/// `rem` (CSS Values §5.2). La cascade stocke `font-size` en
/// valeur absolue (R-17, `"{px}px"`) : parse direct, même
/// forme que `StyleEngine::root_font_size` (cascade.rs).
/// `None` sur résidu non résolu (`calc()`…) → le ctx retombe
/// sur l'initiale, comportement `layout_tree` identique.
fn root_font_size_px(new_dom: &Dom, styles: &StyleMap) -> Option<f32> {
    new_dom
        .document_element()
        .and_then(|el| styles.get(el))
        .and_then(|st| st.get("font-size"))
        .and_then(|v| v.trim().trim_end_matches("px").trim().parse::<f32>().ok())
        .filter(|fs| *fs > 0.0)
}

/// Contexte d'apply cloné sous verrou court — tout ce que
/// les phases lisent hors de l'onglet. `Send` : la payload
/// + ce contexte migrent vers `spawn_blocking`.
pub(crate) struct ApplyCtx {
    /// Fontes composite (locale + `@font-face` enregistrées).
    pub fonts: SharedFonts,
    /// Viewport de mise en page (`vw`, `vh`) en px CSS.
    pub viewport: (f32, f32),
    /// Échelle de rasterisation (DPI).
    pub scale: f32,
    /// Cap de pixels des bitmaps embarqués (images, svg).
    pub image_max_px: u32,
    /// Schéma de couleurs réglé — propagé à `StyleEnv`
    /// (étape 97).
    pub color_scheme: explorer_css::ColorScheme,
    /// Borne d'ops du journal de mutations (ADR-0025 —
    /// `SessionConfig::dom_journal_max_ops`, cloné pour le
    /// worker détaché qui n'a pas la session).
    pub dom_journal_max_ops: usize,
    /// « scripting flag » du re-parse (CAP-155) : le HTML
    /// sérialisé du miroir est relu avec `<noscript>` en
    /// rawtext si le JS tourne — sinon le fallback
    /// réapparaîtrait à chaque apply (divergence miroir/DOM).
    pub scripting: bool,
}

/// Lance l'apply du job — voie journal (rejeu en place,
/// ADR-0025) ou re-parse ; `Err(reject)` = journal refusé,
/// l'appelant re-enfile en mode baseline (`freshen_apply`
/// tirera `sync_dom_keyed`).
pub(crate) fn run_apply_job(
    payload: &mut PaintPayload,
    ctx: &ApplyCtx,
    job: &PendingApply,
    old_title: Option<&str>,
) -> Result<ApplyOutcome, JournalReject> {
    match &job.journal {
        Some(j) => apply_journal(payload, ctx, job, j, old_title, ctx.dom_journal_max_ops),
        None => Ok(apply_payload(payload, ctx, job, old_title)),
    }
}

/// État de peinture d'un onglet sorti pour un apply
/// (étape 55h) — exactement les champs que les phases
/// lisent/mutent ; la comptabilité de l'onglet (`hist`,
/// `js`, `focused`, `pending_nav`…) reste sur `Tab` et
/// répond normalement pendant la fenêtre.
///
/// `Default` = payload **scratch** — le rendu borné d'un
/// sous-contexte `<iframe>` en consomme une par passe
/// (étape 129 : `dom` reste `None`, l'arbre est passé
/// par référence).
#[derive(Default)]
pub(crate) struct PaintPayload {
    /// DOM parsé commis.
    pub dom: Option<Dom>,
    /// Arbre de boîtes posé.
    pub tree: Option<LayoutBox>,
    /// Epoch de `tree` — bumpée à chaque **remplacement**
    /// (`relayout_subtree` mute en place → epoch stable ⇒
    /// fast-path pointeur du build incrémental, R-37).
    pub tree_epoch: u64,
    /// Display list de la dernière peinture (diff dirty).
    pub list: Option<DisplayList>,
    /// Frame rasterisée courante.
    pub frame: Option<Frame>,
    /// Cache glyphes cross-passe du raster (R-36) — `Send`,
    /// voyage avec la payload détachée.
    pub font_cache: explorer_render::GlyphCache,
    /// Cache fonte du layout persisté entre applys (R-65,
    /// `Send` depuis R-67) — résolutions/métriques/runs
    /// shapés réutilisés d'une passe à l'autre ; transplanté
    /// dans le `BlockCtx` de chaque passe (`layout_ctx`).
    pub layout_font_cache: explorer_layout::FontCache,
    /// PNG paresseux (cache de `frame_png`).
    pub png: Option<Vec<u8>>,
    /// Popup `<select>` ouvert.
    pub popup: Option<SelectPopup>,
    /// Recherche en cours (rectangles de surlignage).
    pub find: Option<FindState>,
    /// Snapshot du champ focalisé (chemin `(tag, index)`) —
    /// copié à la sortie pour le caret ; jamais ré-écrit sur
    /// `Tab` (le focus y reste autoritaire : il peut changer
    /// pendant la fenêtre d'apply).
    pub focused: Option<Vec<(String, u32)>>,
    /// Bitmaps indexés par `NodeId`.
    pub images: PaintImages,
    /// Sous-contextes `<iframe>` de la page (étape 128/129)
    /// — le composite rasterise chaque `Dom` enfant dans sa
    /// surface et blitte le bitmap en `images.replaced` à
    /// la clé `NodeId` de sa boîte. Voyage avec la payload
    /// comme `images` (rendu détaché sans `Tab`).
    pub frames: Vec<SubFrame>,
    /// `NodeId` des boîtes `<iframe>` peintes par le
    /// composite — purgé puis rempli à chaque passe
    /// (`images.replaced` survivant d'un re-parse).
    pub frame_nodes: Vec<explorer_dom::NodeId>,
    /// Surfaces de sous-contextes déjà rasterisées, clées
    /// `(url résolue, w px, h px)` — réutilisées d'une
    /// passe à l'autre sans jamais croiser deux contextes
    /// (un cache par `NodeId` aliasserait au re-parse).
    /// Borné par [`composite::MAX_FRAME_BITMAPS`].
    pub frame_bitmaps: composite::SubFrameBitmaps,
    /// URL de la page — copie de `Tab.url` pour la
    /// résolution des `src` dans le composite (lecture
    /// seule : `put_paint` ne la réécrit jamais sur `Tab`,
    /// qui fait foi).
    pub base_url: Option<Url>,
    /// Politiques CSP du document (CAP-145) — miroir de
    /// `Tab::csp` pour la passe de peinture détachée :
    /// `styles_payload` re-filtre les `<style>` inline à
    /// chaque cascade (un relayout ne doit pas réintroduire
    /// une feuille bloquée à la navigation). Lecture seule,
    /// comme `base_url`.
    pub csp: Vec<explorer_net::csp::CspPolicy>,
    /// Feuilles `<link>` — une entrée par feuille, parsées
    /// isolément à la cascade (étape 104, cf. R-68).
    pub linked_sheets: Vec<String>,
    /// Styles bruts de la dernière cascade (transitions).
    pub prev_styles: HashMap<usize, ComputedStyle>,
    /// Transitions en vol.
    pub transitions: Vec<ActiveTransition>,
    /// Animations `@keyframes` enregistrées.
    pub animations: Vec<ActiveAnimation>,
    /// Hauteur du document posé.
    pub doc_h: f32,
    /// Scroll peint dans `frame` (le `scroll_y` de `Tab`
    /// fait foi hors de la payload — un scroll pendant la
    /// fenêtre n'est pas écrasé à la réinsertion).
    pub scroll_y: f32,
    /// Drapeau peinture spéculative (56c).
    pub spec_paint: bool,
    /// Régions invalides accumulées (diff dirty).
    pub dirty: Vec<Rect>,
    /// Watermark des régions déjà rapportées.
    pub emitted: usize,
    /// Tampon des fonds `url(http…)` fetchés pendant la vague
    /// (étape 98) — posés sur la payload entre commit et
    /// relayout : `bg_aliases` relie le texte `url(...)` brut
    /// à l'URL résolue, `bg_bodies` les corps par résolue ;
    /// `relayout_dom_payload` les consomme après cascade.
    /// `bg_aliases` est **persistant** (les URL résolues ne
    /// changent pas au re-parse) ; `bg_bodies` est drainé une
    /// fois vers `bg_decoded`.
    pub bg_aliases: HashMap<String, String>,
    pub bg_bodies: HashMap<String, Vec<u8>>,
    /// Alias `mask-image: url(http…)` fetchés (étape 121) :
    /// texte `url(...)` brut → URL résolue ; les corps sont
    /// partagés avec `bg_bodies`, le décodage avec
    /// `bg_decoded`. Persistant : `masks_url` meurt avec le
    /// RAZ `PaintImages` de chaque apply — les masques sont
    /// ré-enregistrés depuis cette table à chaque cascade.
    pub mask_aliases: HashMap<String, String>,
    /// Caches décodés persistants (étape 108) — les tables
    /// `images.*` sont indexées `NodeId` et RAZ à chaque apply
    /// (re-parse → ids régénérés) ; les bitmaps vivent ici
    /// sous une clé stable. `img_decoded` : valeur d'attribut
    /// `src`/`poster` → image ; `bg_decoded` : URL résolue →
    /// image (`None` = décodage échoué, jamais retenté). Sans
    /// eux tout `<img>`/fond fetché disparaît à la première
    /// synchro JS post-fetch (trigger : logo du portail
    /// Wikipedia, vol 22/10).
    pub img_decoded: HashMap<String, Arc<DecodedImage>>,
    pub bg_decoded: HashMap<String, Option<Arc<DecodedImage>>>,
    /// Corps compressés retenus (étape 133) — clé URL
    /// résolue, magasin partagé `<img>`/fonds/masques ;
    /// source du re-décodage bfcache (`snap.images` gelée
    /// sans bitmaps). Borné `MAX_IMG_BODY_STORE`.
    pub img_bodies: HashMap<String, Vec<u8>>,
    /// Table d'identité `miroir → explorer-dom` (ADR-0025,
    /// étape 111) : reconstruite à chaque baseline cléée
    /// (`apply_payload`), étendue par les `Insert` du journal,
    /// purgée par les `Remove`. Vide = pas de baseline —
    /// tout journal tombe en `NoBaseline` → re-parse. Vit et
    /// meurt avec `dom` (même granule de remplacement).
    pub keymap: HashMap<u64, explorer_dom::NodeId>,
}

/// Titre courant de l'onglet — lu au début d'apply pour
/// détecter le changement (le champ reste sur `Tab` ; la
/// comparaison s'appuie sur ce snapshot).
#[derive(Default)]
pub(crate) struct PendingApply {
    /// Onglet cible.
    pub tab: TabId,
    /// `nav_epoch` capturé à l'enfilement — une navigation
    /// ultérieure invalide l'entrée.
    pub epoch: u64,
    /// HTML sérialisé du miroir.
    pub html: String,
    /// Chemin du sous-arbre sale rapporté par le miroir
    /// (R-12) — `Some` = tentative incrémentale.
    pub dirty: Option<Vec<(String, u32)>>,
    /// Op-lists `<canvas>` du miroir capturées à
    /// l'enfilement (le moteur JS reste sur `Tab` — non
    /// accessible depuis la payload détachée).
    pub canvas_ops: CanvasOpsTable,
    /// `true` = apply complet (relayout) ; `false` =
    /// re-parse + titre seulement (sous-ressource en vol).
    pub relayout: bool,
    /// Journal de mutations à rejouer en place (ADR-0025,
    /// étape 111) — `Some` = voie différentielle, le `html`
    /// n'est alors rempli qu'au repli (rejet → re-parse).
    pub journal: Option<explorer_js::DomJournal>,
    /// `NodeId` miroir en pré-ordre accompagnant `html`
    /// (baseline cléée [`explorer_js::DomSyncKeyed`]) — vide
    /// pour les moteurs sans journal ou les parsings hors
    /// synchro (navigation initiale : pas de baseline).
    pub ids: Vec<u64>,
    /// Tags parallèles à `ids` (`"#text\x00{fnv64}"` pour
    /// les textes) — l'alignement de la table se fait **par
    /// tag**, les nœuds synthétisés par le parse moteur
    /// (`<tbody>`…) étant ignorés plutôt que décalés.
    pub tags: Vec<String>,
}

/// Produit d'un point de synchro DOM (ADR-0025, étape 111)
/// — la voie différentielle est tentée d'abord ; la
/// sérialisation **cléée** reste baseline + repli.
pub(crate) enum ApplySync {
    /// Journal de mutations rejouable en place (`NodeId`
    /// stables — plus de re-parse).
    Journal(explorer_js::DomJournal),
    /// Baseline cléée : sérialisation + ids miroir.
    Keyed(explorer_js::DomSyncKeyed),
}

/// Ce qu'un apply rapporte au commit (effets de bord
/// émis sous verrou, jamais depuis le worker).
#[derive(Default)]
pub(crate) struct ApplyOutcome {
    /// Le `<title>` a changé — nouvelle valeur à écrire
    /// dans `entry.title` puis émettre `TitleChanged`.
    #[allow(clippy::option_option)] // trois états réels : inchangé / `Some` / `None`
    pub title: Option<Option<String>>,
    /// Une frame a été repeinte (`emit_invalidated` +
    /// `sync_floor` à armer).
    pub repainted: bool,
    /// Le relayout a emprunté le chemin incrémental R-12
    /// (compteur `incremental_syncs`).
    pub incremental: bool,
    /// Durée totale de l'apply — base du `sync_floor`.
    pub cost: std::time::Duration,
}

impl Tab {
    /// Sort l'état de peinture pour un apply (les champs
    /// `mem::take` restent vides sur `Tab` — la session
    /// dégrade naturellement : `/frame` → 204 lazy-heal,
    /// hit-test mort, `advance_animations` inerte).
    pub(crate) fn take_paint(&mut self) -> PaintPayload {
        PaintPayload {
            dom: self.dom.take(),
            tree: self.tree.take(),
            tree_epoch: self.tree_epoch,
            list: self.list.take(),
            frame: self.frame.take(),
            font_cache: std::mem::take(&mut self.font_cache),
            layout_font_cache: std::mem::take(&mut self.layout_font_cache),
            png: self.png.take(),
            popup: self.popup.take(),
            find: self.find.take(),
            focused: self.focused.clone(),
            images: std::mem::take(&mut self.images),
            frames: std::mem::take(&mut self.frames),
            frame_nodes: std::mem::take(&mut self.frame_nodes),
            frame_bitmaps: std::mem::take(&mut self.frame_bitmaps),
            // `<base href>` résolu (étape 135) ou URL du
            // document — base de toutes les résolutions
            // relatives de la passe de peinture détachée.
            base_url: self.base_url.clone().or_else(|| self.url.clone()),
            // Miroir `Tab::csp` (CAP-145) — cloné, jamais
            // ré-écrit par `put_paint` (Tab fait foi).
            csp: self.csp.clone(),
            linked_sheets: std::mem::take(&mut self.linked_sheets),
            prev_styles: std::mem::take(&mut self.prev_styles),
            transitions: std::mem::take(&mut self.transitions),
            animations: std::mem::take(&mut self.animations),
            doc_h: self.doc_h,
            scroll_y: self.scroll_y,
            spec_paint: std::mem::take(&mut self.spec_paint),
            dirty: std::mem::take(&mut self.dirty),
            emitted: self.emitted,
            bg_aliases: std::mem::take(&mut self.bg_aliases),
            bg_bodies: std::mem::take(&mut self.bg_bodies),
            mask_aliases: std::mem::take(&mut self.mask_aliases),
            img_decoded: std::mem::take(&mut self.img_decoded),
            bg_decoded: std::mem::take(&mut self.bg_decoded),
            img_bodies: std::mem::take(&mut self.img_bodies),
            keymap: std::mem::take(&mut self.dom_keymap),
        }
    }

    /// Réinsère la payload — appelé au commit (epoch
    /// vérifiée par l'appelant). `scroll_y` n'est pas
    /// restauré : le champ sur `Tab` a pu avancer pendant
    /// la fenêtre (scroll input) et fait foi.
    pub(crate) fn put_paint(&mut self, p: PaintPayload) {
        self.dom = p.dom;
        self.tree = p.tree;
        self.tree_epoch = p.tree_epoch;
        self.list = p.list;
        self.frame = p.frame;
        self.font_cache = p.font_cache;
        self.layout_font_cache = p.layout_font_cache;
        self.png = p.png;
        self.popup = p.popup;
        self.find = p.find;
        self.images = p.images;
        self.frames = p.frames;
        self.frame_nodes = p.frame_nodes;
        self.frame_bitmaps = p.frame_bitmaps;
        self.linked_sheets = p.linked_sheets;
        self.prev_styles = p.prev_styles;
        self.transitions = p.transitions;
        self.animations = p.animations;
        self.doc_h = p.doc_h;
        self.spec_paint = p.spec_paint;
        self.dirty = p.dirty;
        self.emitted = p.emitted;
        // Tampon étape 98 non consommé (relayout sauté) :
        // restitué — un apply ultérieur le re-prendra.
        self.bg_aliases = p.bg_aliases;
        self.bg_bodies = p.bg_bodies;
        self.mask_aliases = p.mask_aliases;
        self.img_decoded = p.img_decoded;
        self.bg_decoded = p.bg_decoded;
        self.img_bodies = p.img_bodies;
        self.dom_keymap = p.keymap;
        // Issue #19 — la table de géométrie publiée au JS se
        // re-remplit à chaque état commis (nav, apply,
        // relayout, reclaim) : `getBoundingClientRect` &
        // co ne mesurent jamais un arbre en mutation.
        self.rebuild_geometry();
    }
}

/// Payload détachée sous garde anti-perte (issue #23) :
/// entre `take_*` et `commit_*` la payload vit hors de
/// `Tab` — une tâche annulée en vol (client HTTP de la
/// navigation tombé en timeout, drain interrompu) la
/// perdait à vie et laissait `paint_detached` collé :
/// sondes 404 permanentes, hit-test et scroll morts,
/// `doc_h` replié. À sa destruction — où qu'elle soit
/// (tâche async droppée, sortie de worker jamais
/// récupérée) — la garde parque la payload dans
/// [`Session::orphaned_paints`] ; le prochain passage
/// sous verrou la réinsère via
/// [`Session::reclaim_orphaned_paint`] (epoch-gardée).
pub(crate) struct DetachedPaint {
    /// Slot de parking partagé avec la session — joignable
    /// sans le verrou tokio (`Drop` ne peut pas `.await`).
    slot: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<TabId, (u64, PaintPayload)>>>,
    /// Onglet propriétaire.
    tab: TabId,
    /// Epoch de navigation au moment du détachement.
    epoch: u64,
    /// La payload — `None` une fois extraite pour commit.
    p: Option<PaintPayload>,
}

impl DetachedPaint {
    /// Arme la garde sur `p` pour `tab`/`epoch`.
    pub(crate) fn new(
        slot: std::sync::Arc<
            std::sync::Mutex<std::collections::HashMap<TabId, (u64, PaintPayload)>>,
        >,
        tab: TabId,
        epoch: u64,
        p: PaintPayload,
    ) -> Self {
        Self {
            slot,
            tab,
            epoch,
            p: Some(p),
        }
    }

    /// Accès lecture à la payload gardée — `None` une
    /// fois extraite (chemin impossible hors commit).
    pub(crate) fn payload(&self) -> Option<&PaintPayload> {
        self.p.as_ref()
    }

    /// Accès mutable à la payload pour le worker de rendu —
    /// la garde reste armée : si le résultat du worker est
    /// jeté (tâche droppée), `Drop` parque quand même.
    pub(crate) fn payload_mut(&mut self) -> &mut PaintPayload {
        self.p.get_or_insert_with(PaintPayload::default)
    }

    /// Extrait la payload pour le commit — désarme la
    /// garde (rien à parquer, la propriété a été cédée).
    pub(crate) fn into_payload(mut self) -> PaintPayload {
        self.p.take().unwrap_or_default()
    }
}

impl Drop for DetachedPaint {
    fn drop(&mut self) {
        if let Some(p) = self.p.take() {
            let _ = self
                .slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(self.tab, (self.epoch, p));
        }
    }
}

/// Sortie de [`Session::take_nav_paint`] (étape 90).
pub(crate) enum NavPaint {
    /// Payload gardée + contexte prêts pour le worker de
    /// rendu (issue #23 : la garde parque l'orpheline si le
    /// détenteur meurt avant le commit).
    Ready(Box<DetachedPaint>, ApplyCtx, CanvasOpsTable),
    /// Un apply (ou un rendu de nav) détaché tient la
    /// payload — réessayer après son commit : le détenteur
    /// la restitue toujours, même supplanté (epoch jetée).
    Busy,
    /// Onglet fermé ou navigation supplantée — abandon.
    Superseded,
}

/// Corps de l'apply — exécutable sous `&mut Session` ou en
/// `spawn_blocking` sur payload détachée (mêmes phases).
/// Retourne l'[`ApplyOutcome`] à committer sous verrou.
pub(crate) fn apply_payload(
    p: &mut PaintPayload,
    ctx: &ApplyCtx,
    job: &PendingApply,
    old_title: Option<&str>,
) -> ApplyOutcome {
    let t0 = Instant::now();
    let mut out = ApplyOutcome::default();
    let new_dom = TreeBuilder::parse_scripting(&job.html, ctx.scripting);
    let new_title = extract_title(&new_dom);
    // Relayout incrémental (étape 55f) avant tout
    // remplacement — il a besoin de l'ancien DOM et de
    // l'ancien arbre, tous deux dans la payload.
    let incremental = job.relayout
        && job.dirty.as_deref().is_some_and(|path| {
            let (Some(t_old), Some(t_new)) = (
                p.dom.as_ref().and_then(|d| dom_path_node(d, path)),
                dom_path_node(&new_dom, path),
            ) else {
                return false;
            };
            try_incr_payload(p, ctx, None, &new_dom, t_old, t_new, &job.canvas_ops)
        });
    out.incremental = incremental;
    // Les op-lists `<canvas>` survivent au re-parse : le
    // miroir les conserve (bitmap cumulative) — la table est
    // construite sur `new_dom` depuis les ops capturées à
    // l'enfilement (chemins → `NodeId` du DOM commis).
    let canvases = canvas_table_from_ops(&job.canvas_ops, &new_dom);
    // ADR-0025 : baseline cléée — la table `miroir → NodeId`
    // suit le DOM frais (vide hors synchro cléée → le journal
    // suivant tombera en `NoBaseline` → re-parse, jamais à
    // faux).
    build_keymap(&mut p.keymap, &new_dom, &job.ids, &job.tags);
    p.dom = Some(new_dom);
    p.images = PaintImages::default();
    p.images.canvases = canvases;
    // `<img>`/affiches fetchés : re-match par `src`/`poster`
    // d'attribut — les `NodeId` viennent d'être régénérés au
    // re-parse et la table a été RAZ juste au-dessus (étape
    // 108 ; avant ce cache, tout `<img>` http(s) mourait à la
    // première synchro JS post-fetch).
    if let Some(dom) = p.dom.as_ref() {
        net::reinsert_imgs_by_src(
            dom,
            &mut p.images,
            &p.img_decoded,
            ctx.viewport.0,
            ctx.scale,
        );
    }
    // Un re-parse de synchro rend obsolètes les options
    // figées du popup `<select>` — il se referme (étape 54) ;
    // les `NodeId` du find meurent de même → effacé (57bis).
    p.popup = None;
    p.find = None;
    if old_title != new_title.as_deref() {
        out.title = Some(new_title);
    }
    if job.relayout {
        if !incremental {
            if let Some(dom) = p.dom.take() {
                if relayout_dom_payload(p, ctx, &dom, &job.canvas_ops).is_err() {
                    p.dom = Some(dom);
                    out.cost = t0.elapsed();
                    return out;
                }
                p.dom = Some(dom);
            }
        }
        out.repainted = true;
    }
    out.cost = t0.elapsed();
    out
}

// ---------------------------------------------------------------------
// Journal de mutations (ADR-0025, étape 111)
// ---------------------------------------------------------------------

/// Pourquoi un journal a été refusé — toute entrée renvoie au
/// re-parse (garde-fou auto-réparant de l'ADR : la
/// sérialisation complète subsume toujours l'état vrai).
#[derive(Debug)]
pub(crate) enum JournalReject {
    /// Pas de baseline cléée (premier apply, moteur sans ids)
    /// — la table d'identité n'existe pas.
    NoBaseline,
    /// Clé miroir absente de la table (nœud jamais vu ou
    /// désaccord d'appariement — conservateur).
    UnmappedKey,
    /// `explorer-dom` a refusé l'opération (hiérarchie,
    /// handle mort).
    Dom,
    /// Empreinte post-rejeu ≠ miroir — divergence structurelle.
    Fingerprint,
    /// Plus d'ops que `SessionConfig::dom_journal_max_ops` —
    /// le re-parse coûte moins cher.
    TooManyOps,
    /// Pas de DOM posé (page jamais peinte).
    NoDom,
}

/// Nœuds du DOM moteur **dans l'ordre de sérialisation
/// miroir** (pré-ordre ; les enfants d'un `<template>` sont
/// descendus via `template_contents` — le miroir les émet
/// inline par `serial_children`). Apparié position par
/// position à `DomSyncKeyed::ids` pour monter la table
/// `miroir → NodeId`. Itératif — profondeur hostile.
/// `true` = nœud présent dans le miroir JS (élément ou texte —
/// le miroir n'a ni doctype ni commentaire : les sauter ici est
/// ce qui garde le zip `engine_preorder` ↔ `ids` aligné sur les
/// pages réelles, toutes porteuses de `<!-- -->`/`<!doctype>`).
fn serializable_kind(dom: &Dom, id: explorer_dom::NodeId) -> bool {
    use explorer_dom::NodeData;
    matches!(dom.data(id), Ok(NodeData::Element(_) | NodeData::Text(_)))
}

fn engine_preorder(dom: &Dom) -> Vec<explorer_dom::NodeId> {
    let mut out = Vec::new();
    let Ok(mut stack) = dom
        .children(dom.document())
        .map(|c| c.into_iter().rev().collect::<Vec<_>>())
    else {
        return out;
    };
    while let Some(id) = stack.pop() {
        if !serializable_kind(dom, id) {
            continue;
        }
        out.push(id);
        // Même ordre que `serial_children` miroir : contenu de
        // `<template>` à la place des enfants ordinaires.
        let kids = match dom.template_contents(id) {
            Ok(Some(frag)) => dom.children(frag).unwrap_or_default(),
            _ => dom.children(id).unwrap_or_default(),
        };
        for &c in kids.iter().rev() {
            stack.push(c);
        }
    }
    out
}

/// Variante sous-arbre de [`engine_preorder`] — collecte
/// dans `out` (pré-ordre, `<template>` → `template_contents`).
fn engine_preorder_subtree(
    dom: &Dom,
    root: explorer_dom::NodeId,
    out: &mut Vec<explorer_dom::NodeId>,
) {
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !serializable_kind(dom, id) {
            continue;
        }
        out.push(id);
        let kids = match dom.template_contents(id) {
            Ok(Some(frag)) => dom.children(frag).unwrap_or_default(),
            _ => dom.children(id).unwrap_or_default(),
        };
        for &c in kids.iter().rev() {
            stack.push(c);
        }
    }
}

/// Empreinte structurelle du DOM commis — mêmes règles que
/// `Dom::structural_fingerprint` miroir : tags d'éléments en
/// pré-ordre sérialisation (contenu `<template>` compris,
/// texte exclu), **casse normalisée** — le moteur garde la
/// casse SVG (`clipPath`, `feGaussianBlur`) là où le miroir
/// sérialise en minuscules. Restreinte aux nœuds **mappés**
/// (`keymap` — univers miroir) : les éléments synthétisés
/// par le parse moteur (`<tbody>` sous un `<table>`…) n'ont
/// pas de correspondant miroir et ne doivent pas diverger
/// l'empreinte.
fn engine_fingerprint(dom: &Dom, keymap: &HashMap<u64, explorer_dom::NodeId>) -> u64 {
    let mapped: std::collections::HashSet<explorer_dom::NodeId> =
        keymap.values().copied().collect();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for id in engine_preorder(dom) {
        if !mapped.contains(&id) {
            continue;
        }
        if let Ok(tag) = dom.tag_name(id) {
            for b in tag.bytes().map(|b| b.to_ascii_lowercase()) {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
            h ^= 0xff;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// (Re)monte la table `miroir → NodeId` après une baseline
/// cléée — alignement **par tag** `engine_preorder` ↔
/// `(ids, tags)` : le parse moteur peut synthétiser des
/// éléments absents du miroir (`<tbody>` sous un `<table>`
/// aux `<tr>` directs) ou perdre des textes — le zip
/// positionnel décalerait alors **tout** l'appariement.
/// Monotone : les nœuds moteur sans correspondant miroir
/// sont sautés ; les clés miroir restantes retombent en
/// `UnmappedKey` au rejeu (repli, jamais faux positif).
/// Textes miroir (`"#text\x00{fnv64}"`) : mappés uniquement
/// par **adjacence stricte à hash de contenu égal** — sauter
/// un texte moteur hors-place risquerait un `SetText` sur le
/// mauvais nœud, invisible à l'empreinte (texte exclu) ;
/// un texte miroir sans concordant reste non mappé →
/// `UnmappedKey` → repli sûr.
/// Sans `tags` (moteur ancien) : repli zip positionnel.
fn build_keymap(
    keymap: &mut HashMap<u64, explorer_dom::NodeId>,
    dom: &Dom,
    ids: &[u64],
    tags: &[String],
) {
    keymap.clear();
    let engine = engine_preorder(dom);
    if tags.len() == ids.len() && !ids.is_empty() {
        let mut j = 0usize;
        'outer: for (i, &m) in ids.iter().enumerate() {
            if let Some(hex) = tags[i].strip_prefix("#text\x00") {
                // Texte : concordance exacte du contenu à
                // la position courante, sinon non mappé.
                let Some(e) = engine.get(j) else { break };
                let Ok(h) = u64::from_str_radix(hex, 16) else {
                    break;
                };
                if let Ok(explorer_dom::NodeData::Text(t)) = dom.data(*e) {
                    if fnv64(t.as_str()) == h {
                        keymap.insert(m, *e);
                        j += 1;
                    }
                }
                continue;
            }
            while j < engine.len() {
                let e = engine[j];
                j += 1;
                if dom
                    .tag_name(e)
                    .is_ok_and(|t| t.eq_ignore_ascii_case(&tags[i]))
                {
                    keymap.insert(m, e);
                    continue 'outer;
                }
            }
            break; // plus de moteur : le reste reste non mappé
        }
        return;
    }
    for (e, m) in engine.into_iter().zip(ids.iter().copied()) {
        keymap.insert(m, e);
    }
}

/// FNV-1a 64 — même fonction que `fnv` miroir (marqueurs
/// `#text` des séries cléées).
fn fnv64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in s.as_bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Greffe un sous-arbre `src_id` (arène `src`, fragment
/// fraîchement parsé) sous `dst_parent` à `index` — copie
/// nœud à nœud (les arènes ne se partagent pas leurs ids).
/// Retourne la racine greffée.
fn graft_subtree(
    dom: &mut Dom,
    src: &Dom,
    src_id: explorer_dom::NodeId,
    dst_parent: explorer_dom::NodeId,
    index: usize,
) -> Result<explorer_dom::NodeId, JournalReject> {
    use explorer_dom::NodeData;
    let new = match src.data(src_id).map_err(|_| JournalReject::Dom)? {
        NodeData::Element(_) => dom
            .create_element(src.tag_name(src_id).map_err(|_| JournalReject::Dom)?)
            .map_err(|_| JournalReject::Dom)?,
        NodeData::Text(t) => dom
            .create_text(t.as_str())
            .map_err(|_| JournalReject::Dom)?,
        NodeData::Comment(c) => dom
            .create_comment(c.as_str())
            .map_err(|_| JournalReject::Dom)?,
        _ => return Err(JournalReject::Dom),
    };
    if let Ok(attrs) = src.attributes(src_id) {
        for a in attrs {
            dom.set_attribute(new, &a.name, &a.value)
                .map_err(|_| JournalReject::Dom)?;
        }
    }
    // Position : `index` dans les enfants **sérialisables**
    // courants (espace miroir — les commentaires n'y existent
    // pas : indexer dessus décalerait l'insertion devant un
    // `<!-- -->` → empreinte divergente). Les ops antérieures
    // ont déjà rejoué — invariant séquentiel.
    let kids = dom.children(dst_parent).map_err(|_| JournalReject::Dom)?;
    let refn = kids
        .iter()
        .copied()
        .filter(|&n| serializable_kind(dom, n))
        .nth(index);
    dom.insert_before(dst_parent, new, refn)
        .map_err(|_| JournalReject::Dom)?;
    // Contenu `<template>` : arène à part des deux côtés.
    if let Ok(Some(tfrag)) = src.template_contents(src_id) {
        let tnew = dom.create_fragment().map_err(|_| JournalReject::Dom)?;
        dom.set_template_contents(new, tnew)
            .map_err(|_| JournalReject::Dom)?;
        for (i, &c) in src.children(tfrag).unwrap_or_default().iter().enumerate() {
            graft_subtree(dom, src, c, tnew, i)?;
        }
    }
    for (i, &c) in src.children(src_id).unwrap_or_default().iter().enumerate() {
        graft_subtree(dom, src, c, new, i)?;
    }
    Ok(new)
}

/// `NodeId` moteur d'une clé miroir — `Err` = non mappée.
fn key_node(p: &PaintPayload, key: u64) -> Result<explorer_dom::NodeId, JournalReject> {
    if p.keymap.is_empty() {
        return Err(JournalReject::NoBaseline);
    }
    p.keymap
        .get(&key)
        .copied()
        .ok_or(JournalReject::UnmappedKey)
}

/// Étape 137b — replay `attachShadow` programmatique : le
/// fragment est créé côté `explorer-dom` et attaché à l'hôte
/// mappé ; la clé miroir de la racine entre en `keymap` pour
/// que les `Insert` suivants dans l'ombre résolvent (API
/// `Dom::attach_shadow_root` PIPE livrée R-96 — consommation
/// pure).
fn journal_attach_shadow(
    p: &mut PaintPayload,
    host: u64,
    root: u64,
    open: bool,
) -> Result<explorer_dom::NodeId, JournalReject> {
    let h = key_node(p, host)?;
    let dom = p.dom.as_mut().ok_or(JournalReject::NoDom)?;
    let r = dom.create_fragment().map_err(|_| JournalReject::Dom)?;
    dom.attach_shadow_root(h, r, open);
    p.keymap.insert(root, r);
    Ok(h)
}

/// Applique un journal de mutations **en place** sur le DOM
/// commis (ADR-0025 — les `NodeId` survivent : `images`,
/// `find`, `popup`, `focused` restent valides sans
/// re-key). `Err` = repli re-parse par l'appelant.
fn apply_journal(
    p: &mut PaintPayload,
    ctx: &ApplyCtx,
    job: &PendingApply,
    journal: &explorer_js::DomJournal,
    old_title: Option<&str>,
    max_ops: usize,
) -> Result<ApplyOutcome, JournalReject> {
    let t0 = Instant::now();
    let mut out = ApplyOutcome::default();
    if journal.ops.len() > max_ops {
        return Err(JournalReject::TooManyOps);
    }
    if p.dom.is_none() {
        return Err(JournalReject::NoDom);
    }
    if p.keymap.is_empty() && !journal.ops.is_empty() {
        return Err(JournalReject::NoBaseline);
    }
    // Étape 137a — cibles réelles des ops pour le relayout
    // incrémental ciblé (le journal remplace `job.dirty`).
    let mut touched = Vec::new();
    for op in &journal.ops {
        match op {
            explorer_js::DomMutation::Remove { key } => {
                let id = key_node(p, *key)?;
                // Purger les descendants de la table — les
                // clés miroir meurent avec leur sous-arbre.
                let mut gone = std::collections::HashSet::new();
                let mut stack = vec![id];
                let dom = p.dom.as_mut().ok_or(JournalReject::NoDom)?;
                while let Some(n) = stack.pop() {
                    gone.insert(n);
                    stack.extend(dom.children(n).unwrap_or_default());
                    if let Ok(Some(f)) = dom.template_contents(n) {
                        stack.extend(dom.children(f).unwrap_or_default());
                    }
                }
                touched.push(dom.parent(id).ok().flatten().unwrap_or(id));
                dom.remove(id).map_err(|_| JournalReject::Dom)?;
                p.keymap.retain(|_, v| !gone.contains(v));
            }
            explorer_js::DomMutation::SetAttr { key, name, value } => {
                let id = key_node(p, *key)?;
                let dom = p.dom.as_mut().ok_or(JournalReject::NoDom)?;
                match value {
                    Some(v) => dom.set_attribute(id, name, v),
                    None => dom.remove_attribute(id, name).map(|_| ()),
                }
                .map_err(|_| JournalReject::Dom)?;
                touched.push(id);
            }
            explorer_js::DomMutation::SetText { key, text } => {
                let id = key_node(p, *key)?;
                let dom = p.dom.as_mut().ok_or(JournalReject::NoDom)?;
                touched.push(dom.parent(id).ok().flatten().unwrap_or(id));
                dom.set_text(id, text).map_err(|_| JournalReject::Dom)?;
            }
            // Étape 137b — `attachShadow` programmatique : le
            // fragment est créé côté `explorer-dom` et attaché à
            // l'hôte mappé ; la clé miroir de la racine entre en
            // `keymap` pour que les `Insert` suivants dans
            // l'ombre résolvent (API `Dom::attach_shadow_root`
            // PIPE livrée R-96 — consommation pure).
            explorer_js::DomMutation::AttachShadow { host, root, open } => {
                touched.push(journal_attach_shadow(p, *host, *root, *open)?);
            }
            explorer_js::DomMutation::Insert {
                parent,
                index,
                html,
                ids,
            } => {
                let par = key_node(p, *parent)?;
                // Pas de parse de fragment contextuel dans
                // `explorer-html` : le fragment est parsé en
                // document minimal et ses enfants de `<body>`
                // greffés (le classifieur miroir a déjà écarté
                // les contenus à parsing contextuel).
                let frag = TreeBuilder::parse_scripting(html, ctx.scripting);
                let Some(root_el) = frag.document_element() else {
                    return Err(JournalReject::Dom);
                };
                let mut to_move = Vec::new();
                // `<body>` (ou `<html>` à défaut) = porte-greffe.
                let Ok(body) = frag.children(root_el).map(|c| {
                    c.iter()
                        .find(|&&n| frag.tag_name(n).is_ok_and(|t| t == "body"))
                        .copied()
                        .unwrap_or(root_el)
                }) else {
                    return Err(JournalReject::Dom);
                };
                to_move.extend(frag.children(body).unwrap_or_default());
                let dom = p.dom.as_mut().ok_or(JournalReject::NoDom)?;
                let base = *index as usize;
                let mut created_all = Vec::new();
                for (i, &src_root) in to_move.iter().enumerate() {
                    let new_root = graft_subtree(dom, &frag, src_root, par, base + i)?;
                    engine_preorder_subtree(dom, new_root, &mut created_all);
                }
                // Appariement `créés (pré-ordre)` ↔ `ids` — zip
                // tronqué : le compte exact n'est pas garanti
                // (round-trip), les orphelins retomberont en
                // `UnmappedKey` → repli.
                for (e, m) in created_all.into_iter().zip(ids.iter().copied()) {
                    p.keymap.insert(m, e);
                }
                touched.push(par);
            }
        }
    }
    // Garde de cohérence ADR-0025 : l'empreinte structurelle
    // du DOM rejoué doit égaler celle du miroir post-journal.
    let dom = p.dom.as_ref().ok_or(JournalReject::NoDom)?;
    if engine_fingerprint(dom, &p.keymap) != journal.fingerprint {
        return Err(JournalReject::Fingerprint);
    }
    // Les op-lists `<canvas>` sont chemins→NodeId : rejouées
    // sur le DOM muté (un canvas inséré par journal peint
    // dès cette frame — les ids stables gardent valides les
    // entrées des canvas inchangés).
    p.images.canvases = canvas_table_from_ops(&job.canvas_ops, dom);
    let new_title = extract_title(dom);
    if old_title != new_title.as_deref() {
        out.title = Some(new_title);
    }
    if job.relayout {
        journal_relayout(p, ctx, job, &touched, &mut out)?;
    }
    out.cost = t0.elapsed();
    Ok(out)
}

/// Relayout incrémental ciblé par `NodeId` (137a — voie
/// journal et avance animée) : `relayout_subtree` sur la
/// cible, repli reflow complet si la voie ciblée refuse.
pub(crate) fn relayout_subtree_payload(
    p: &mut PaintPayload,
    ctx: &ApplyCtx,
    dom: &Dom,
    target: explorer_dom::NodeId,
    canvas_ops: &CanvasOpsTable,
) -> Result<(), CoreError> {
    if try_incr_payload(p, ctx, Some(dom), dom, target, target, canvas_ops) {
        Ok(())
    } else {
        relayout_dom_payload(p, ctx, dom, canvas_ops)
    }
}

/// Relayout post-rejeu journal (137a) : incrémental ciblé
/// sur l'ancêtre commun des ops — `relayout_subtree` au
/// lieu du reflow complet, repli `relayout_dom_payload` si
/// la voie ciblée refuse.
fn journal_relayout(
    p: &mut PaintPayload,
    ctx: &ApplyCtx,
    job: &PendingApply,
    touched: &[explorer_dom::NodeId],
    out: &mut ApplyOutcome,
) -> Result<(), JournalReject> {
    let dom = p.dom.take().ok_or(JournalReject::NoDom)?;
    let incremental = journal_cover(&dom, touched)
        .is_some_and(|t| try_incr_payload(p, ctx, Some(&dom), &dom, t, t, &job.canvas_ops));
    if !incremental && relayout_dom_payload(p, ctx, &dom, &job.canvas_ops).is_err() {
        p.dom = Some(dom);
        return Err(JournalReject::Dom);
    }
    out.incremental = incremental;
    p.dom = Some(dom);
    out.repainted = true;
    Ok(())
}

/// Ancêtre commun le plus profond des cibles d'ops journal
/// (137a — la zone à re-poser). `None` = vide (pas de cible
/// → le repli `relayout_dom_payload` tourne comme avant).
pub(crate) fn journal_cover(
    dom: &Dom,
    touched: &[explorer_dom::NodeId],
) -> Option<explorer_dom::NodeId> {
    let mut seen = std::collections::HashSet::new();
    let mut it = touched.iter().copied().filter(|n| seen.insert(*n));
    let first = it.next()?;
    let mut chain = Vec::new();
    let mut cur = first;
    loop {
        chain.push(cur);
        match dom.parent(cur).ok().flatten() {
            Some(par) => cur = par,
            None => break,
        }
    }
    // `chain` = nœud → racine ; `allowed` = le plus petit
    // indice atteint — l'ancêtre le plus profond commun.
    let mut allowed = chain.len();
    for n in it {
        let mut cur = n;
        loop {
            if let Some(pos) = chain.iter().position(|&c| c == cur) {
                allowed = allowed.min(pos);
                break;
            }
            cur = dom.parent(cur).ok().flatten()?;
        }
    }
    chain.get(allowed).copied()
}

/// `try_sync_incremental` sur payload — mêmes gardes que
/// la version session : cible résolue dans les deux DOM,
/// remap des ids conservés, repli `false` → reflow global.
fn try_incr_payload(
    p: &mut PaintPayload,
    ctx: &ApplyCtx,
    old_dom_opt: Option<&Dom>,
    new_dom: &Dom,
    target_old: explorer_dom::NodeId,
    target_new: explorer_dom::NodeId,
    canvas_ops: &CanvasOpsTable,
) -> bool {
    // Phase 1 — cible résolue par l'appelant (chemin DOM ou
    // `NodeId` d'op journal — 137a : les ops rejouées portent
    // déjà leur cible réelle). `old_dom_opt` = DOM détaché du
    // journal in-place (même DOM ancien/nouveau) ; `None` =
    // chemin re-parse (l'ancien vit dans `p.dom`). L'emprunt
    // de `p.dom` n'est pris qu'en Phase 2 — disjoint de
    // `p.tree`, jamais à travers un `&mut p`.
    if (p.dom.is_none() && old_dom_opt.is_none()) || p.tree.is_none() {
        return false;
    }
    let Some(root_el) = new_dom.document_element() else {
        return false;
    };
    // Tables `images.*` indexées par `NodeId` de l'ANCIEN DOM —
    // re-key par `src`/styles AVANT la mesure intrinsèque
    // (étape 108) : `<img>` fetchés, `data:` et fonds
    // `url(http…)` seraient sinon perdus dans le relayout
    // incrémental (même trou que le RAZ `apply_payload`,
    // en silencieux — les entrées obsolètes survivent
    // indexées sur des ids morts).
    composite::purge_frame_bitmaps(p);
    p.images.replaced.clear();
    p.images.backgrounds.clear();
    p.images.masks_url.clear();
    net::reinsert_imgs_by_src(
        new_dom,
        &mut p.images,
        &p.img_decoded,
        ctx.viewport.0,
        ctx.scale,
    );
    let styles = styles_payload(p, new_dom, ctx.viewport, ctx.color_scheme, ctx.scripting);
    let data_imgs = collect_data_images(new_dom, &styles, ctx.image_max_px);
    p.images.replaced.extend(data_imgs.replaced);
    p.images.backgrounds.extend(data_imgs.backgrounds);
    // `masks_url` n'est pas indexée `NodeId` : les masques
    // `data:` sont ré-étendus à chaque apply comme les
    // fetchés sont ré-enregistrés via `mask_aliases` — sans
    // ce merge la table (RAZ `PaintImages::default()`)
    // rendait tout `mask-image` aveugle en session.
    p.images.masks_url.extend(data_imgs.masks_url);
    insert_fetched_images(p, &styles, ctx.image_max_px);
    let mut sizes = image_sizes(new_dom, &p.images.replaced);
    sizes.extend(explorer_render::svg::svg_sizes(new_dom, root_el));
    // `<iframe>` (étape 129) : gabarit remplacé — un cadre
    // inséré par mutation JS est atomique comme au reflow
    // complet, sans quoi son repli RAWTEXT rendrait.
    for n in composite::iframe_nodes(new_dom) {
        sizes
            .entry(n)
            .or_insert_with(|| composite::iframe_intrinsic(new_dom, n));
    }
    let svgs = explorer_render::svg::collect_svg(new_dom, root_el, ctx.image_max_px);
    let block = layout_ctx(ctx, p);
    // Reliquat R-88 : `relayout_subtree` ne pose pas
    // `root_font_size`, contrairement à `layout_tree` —
    // tout `rem` du sous-arbre relayouté retombait sur
    // l'initiale à la première synchro incrémentale.
    // Posée ici depuis la cascade fraîche, même base.
    block
        .root_font_size
        .set(root_font_size_px(new_dom, &styles));
    // Phase 2 — remap + relayout local (le remap précède :
    // `bx.node == target` compare des ids du nouveau DOM).
    // Le cache fonte est restitué sur tous les chemins —
    // succès comme repli : les entrées déjà chaudes restent
    // utiles à la passe suivante.
    let doc_h = {
        // Emprunts disjoints : `p.dom` (immuable, pour le
        // remap) cohabite avec `p.tree` (mutable) — jamais un
        // `&mut p` complet tant que `old_dom` vit.
        let res = match (old_dom_opt.or(p.dom.as_ref()), p.tree.as_mut()) {
            (Some(old_dom), Some(tree)) => {
                if remap_box_nodes(old_dom, new_dom, tree, target_old, target_new)
                    && relayout_subtree(tree, new_dom, &styles, &sizes, target_new, &block)
                        == Relayout::Done
                {
                    Some(tree.rect.height.max(scrollable_overflow_bottom(tree)))
                } else {
                    None
                }
            }
            _ => None,
        };
        restitute_font_cache(p, &block);
        match res {
            Some(h) => h,
            None => return false,
        }
    };
    p.doc_h = doc_h;
    p.images.svgs = svgs;
    p.images.canvases = canvas_table_from_ops(canvas_ops, new_dom);
    // Sous-contextes `<iframe>` (étape 129) : bitmaps
    // re-blittés sur les boîtes du nouvel arbre — les
    // surfaces `frame_bitmaps` clées `(url,w,h)` rendent
    // la passe gratuite si les cadres n'ont pas bougé.
    composite::composite_frames(p, ctx, new_dom);
    repaint_payload(p, ctx, None).is_ok()
}

/// `StyleEnv` dérivé du viewport session (f32 → px entiers,
/// saturé à `[0, u32::MAX]` — un viewport invalide ne peut
/// pas produire un env négatif ou absurdement grand), du
/// schéma de couleurs réglé (étape 97 — `prefers-color-scheme`
/// des media queries et défauts UA sombres, R-59) et du
/// « scripting flag » du contexte (R-93 — MQ `(scripting)` +
/// `<noscript>` masqué UA : sans filage, le fallback rawtext
/// peignait en clair sous un moteur installé — issue #22).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
// px CSS ≪ 2^32 en pratique — `as` sature déjà.
pub(crate) fn viewport_env(
    viewport: (f32, f32),
    color_scheme: explorer_css::ColorScheme,
    scripting: bool,
) -> StyleEnv {
    StyleEnv {
        width_px: viewport.0.max(0.0).round() as u32,
        height_px: viewport.1.max(0.0).round() as u32,
        color_scheme,
        scripting,
        ..StyleEnv::default()
    }
}

/// Cascade + transitions/animations sur payload — la passe
/// de styles partagée par reflow complet et relayout
/// incrémental. Écrit `prev_styles`/`transitions`/
/// `animations` de la payload.
pub(crate) fn styles_payload(
    p: &mut PaintPayload,
    dom: &Dom,
    viewport: (f32, f32),
    color_scheme: explorer_css::ColorScheme,
    scripting: bool,
) -> StyleMap {
    let mut eng = StyleEngine::new();
    eng.add_css(MEDIA_UA_CSS, Origin::UserAgent);
    // Une passe de parse par feuille (étape 104) : l'ordre de
    // cascade est l'ordre d'insertion, donc inchangé — mais un
    // défaut de fermeture (accolade, commentaire) ne peut plus
    // absorber la feuille suivante, et `@import`/`@charset`
    // en tête de feuille redeviennent valides (cf. R-68).
    // R-96 — chaque `<style>` se charge dans son arbre :
    // `scope = Some(racine)` → feuille confinée shadow
    // (`add_scoped_css`), `None` → feuille document.
    for (scope, css) in collect_style_sheets(dom, &p.csp) {
        match scope {
            Some(root) => eng.add_scoped_css(&css, Origin::Author, root),
            None => eng.add_css(&css, Origin::Author),
        }
    }
    for css in &p.linked_sheets {
        eng.add_css(css, Origin::Author);
    }
    // Media queries évaluées contre le viewport RÉEL de la
    // session (demande PIPE→NAV : `max-width:1007` doit être
    // vrai à 800px, pas lue sur le 1024×768 par défaut) et
    // le schéma de couleurs réglé (étape 97).
    let env = viewport_env(viewport, color_scheme, scripting);
    let mut styles = eng.compute_tree(dom, dom.document(), &env);
    let ord_of = element_ordinals(dom);
    let raw: HashMap<usize, ComputedStyle> = styles
        .iter()
        .filter_map(|(n, c)| ord_of.get(&n).map(|o| (*o, c.clone())))
        .collect();
    let now = Instant::now();
    update_transitions(&p.prev_styles, &mut p.transitions, &styles, &ord_of, now);
    sync_animations(&mut p.animations, &styles, &ord_of, &eng, now);
    apply_anim_overrides(
        &p.transitions,
        &mut p.animations,
        &eng,
        &mut styles,
        &ord_of,
        now,
    );
    p.prev_styles = raw;
    styles
}

/// `relayout_dom` sur payload — style → boîtes → layout →
/// display list → raster. `canvas_ops` = op-lists du miroir
/// (capturées sous verrou — la payload n'a pas `entry.js`).
pub(crate) fn relayout_dom_payload(
    p: &mut PaintPayload,
    ctx: &ApplyCtx,
    dom: &Dom,
    canvas_ops: &CanvasOpsTable,
) -> Result<(), CoreError> {
    let t0 = Instant::now();
    // Bitmaps de sous-contexte `<iframe>` de la passe
    // précédente : purgés AVANT tout insert sous les
    // `NodeId` du DOM courant — un re-parse réattribue les
    // cases, retirer plus tard pourrait emporter un
    // bitmap frais (étape 129).
    composite::purge_frame_bitmaps(p);
    let styles = styles_payload(p, dom, ctx.viewport, ctx.color_scheme, ctx.scripting);
    tracing::info!(
        ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX),
        "apply: cascade"
    );
    let Some(root_el) = dom.document_element() else {
        return Err(CoreError::EmptyDocument);
    };
    let data_imgs = collect_data_images(dom, &styles, ctx.image_max_px);
    p.images.replaced.extend(data_imgs.replaced);
    p.images.backgrounds.extend(data_imgs.backgrounds);
    // Même merge `masks_url` que la voie incrémentale (voir
    // plus haut) : masques `data:` sinon invisibles en
    // session depuis R-52.
    p.images.masks_url.extend(data_imgs.masks_url);
    insert_fetched_images(p, &styles, ctx.image_max_px);
    let mut sizes = image_sizes(dom, &p.images.replaced);
    sizes.extend(explorer_render::svg::svg_sizes(dom, root_el));
    // `<iframe>` (étape 129) : gabarit remplacé (300×150
    // / attributs px) — la boîte devient feuille atomique
    // (`intrinsic.is_some()` → pas d'enfants boxifiés : le
    // texte de repli RAWTEXT ne rend plus) et accueillera
    // le bitmap du sous-contexte au composite.
    for n in composite::iframe_nodes(dom) {
        sizes
            .entry(n)
            .or_insert_with(|| composite::iframe_intrinsic(dom, n));
    }
    let svgs = explorer_render::svg::collect_svg(dom, root_el, ctx.image_max_px);
    let Some(mut bx) = build_box_tree_sized(dom, root_el, &styles, &sizes) else {
        return Err(CoreError::NothingToPaint);
    };
    tracing::info!(
        ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX),
        "apply: boxtree"
    );
    let vw = ctx.viewport.0;
    let block = layout_ctx(ctx, p);
    layout_tree(&mut bx, vw, &block);
    tracing::info!(
        ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX),
        "apply: layout"
    );
    restitute_font_cache(p, &block);
    // Hauteur défilante ≠ hauteur de boîte : le contenu qui
    // déborde un ancêtre à hauteur spécifiée (`height:100%` de
    // vector-2022) reste scrollable — R-82.
    p.doc_h = bx.rect.height.max(scrollable_overflow_bottom(&bx));
    p.tree = Some(bx);
    p.tree_epoch = p.tree_epoch.wrapping_add(1);
    p.images.svgs = svgs;
    // `<canvas>` 2D (JS-C28) : op-lists cumulées du miroir,
    // re-résolues par chemin d'élément dans `dom`.
    p.images.canvases = canvas_table_from_ops(canvas_ops, dom);
    // Sous-contextes `<iframe>` (étape 129) : chaque
    // `SubFrame` fetchée est rasterisée dans sa surface
    // (viewport = content box de sa boîte) et blittée en
    // `images.replaced` — `paint_replaced` émet l'`Image`
    // à la peinture qui suit.
    composite::composite_frames(p, ctx, dom);
    repaint_payload(p, ctx, Some(dom))?;
    tracing::info!(
        ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX),
        "apply: repaint"
    );
    Ok(())
}

/// `repaint` sur payload — display list + raster + diff
/// dirty. Les overlays (find, popup `<select>`) suivent la
/// payload — le find est effacé au re-parse (`NodeId`s
/// morts) mais peut revivre via `find_step` sous verrou.
/// `dom_hint` : le `dom` du job quand `p.dom` est sorti pour
/// la durée du pipeline (`relayout` le prend pour disjoindre
/// les emprunts) — le caret résout le focus dessus.
pub(crate) fn repaint_payload(
    p: &mut PaintPayload,
    ctx: &ApplyCtx,
    dom_hint: Option<&Dom>,
) -> Result<(), CoreError> {
    let Some(bx) = p.tree.as_ref() else {
        return Err(CoreError::EmptyDocument);
    };
    let (vw, vh) = ctx.viewport;
    // R-37 : build incrémental — les sous-arbres inchangés
    // (même signature `box_sig`, même taille, Δ uniforme)
    // sont copiés depuis la liste précédente au lieu d'être
    // ré-émis ; `prev=None` ou largeur changée → plein.
    let mut list = explorer_render::build_display_list_incremental(
        bx,
        vw,
        vh,
        (0.0, p.scroll_y),
        &p.images,
        p.list.as_ref(),
        p.tree_epoch,
    );
    if let Some(f) = &p.find {
        append_find_highlights(&mut list, f);
    }
    if let Some(pop) = &p.popup {
        append_select_popup(&mut list, pop, vw, p.doc_h, &ctx.fonts);
    }
    // Caret du champ focalisé (M-H1 #1) — dernier overlay :
    // il doit rester visible au-dessus du popup `<select>`.
    let cdom = dom_hint.or(p.dom.as_ref());
    if let Some((rect, color)) = caret::caret_rect(p, ctx, cdom) {
        list.items.push(DisplayItem::FillRect { rect, color });
    }
    let frame_cap = MAX_FRAME_DIM / ctx.scale.max(f32::EPSILON);
    if list.height > frame_cap {
        return raster_window(p, ctx, list, vw, vh);
    }
    // Le diff se calcule sur les listes **non cullées** :
    // la cullée stockée perdait ses `spans` (indices
    // décalés → purgés), ce qui tuait le build incrémental
    // à chaque frame dès qu'un item sortait de la vue.
    // Le diff non cullé est d'ailleurs plus précis — plus
    // de faux changements à la frontière de cull.
    let regions = p.list.as_ref().map_or_else(
        || {
            vec![Rect {
                x: 0.0,
                y: 0.0,
                width: list.width,
                height: list.height,
            }]
        },
        |prev| {
            if list.items.len() > DIRTY_DIFF_MAX_ITEMS || prev.items.len() > DIRTY_DIFF_MAX_ITEMS {
                vec![Rect {
                    x: 0.0,
                    y: 0.0,
                    width: list.width,
                    height: list.height,
                }]
            } else {
                list.changed_rects(prev)
            }
        },
    );
    // R-37 : les régions de diff pilotent maintenant le
    // raster — repeinte bornée par-dessus la frame
    // précédente (équivalence plein cadre assertée par
    // explorer-render/tests/dirty_raster.rs). Repli plein
    // cadre : pas de frame précédente, cible différente
    // (dims/scale), borne de fenêtres dépassée, ou diff
    // qui couvre déjà toute la page.
    let full_cover = regions.iter().any(|r| {
        r.x <= 0.0 && r.y <= 0.0 && r.x + r.width >= list.width && r.y + r.height >= list.height
    });
    let frame = p
        .frame
        .take()
        .filter(|_| !full_cover)
        .and_then(|prev| {
            explorer_render::rasterize_dirty_cached(
                &list,
                &ctx.fonts as &dyn FontSource,
                ctx.scale,
                prev,
                &regions,
                &mut p.font_cache,
            )
        })
        .map_or_else(
            || {
                // Le cull ne sert que le plein cadre (sauter
                // les items hors vue) — `rasterize_dirty`
                // est déjà borné par ses fenêtres de damage.
                // La cullée est une copie jetable : `list`
                // garde ses `spans` pour le prochain build.
                let mut culled = list.clone();
                cull_display_list(&mut culled);
                explorer_render::rasterize_cached(
                    &culled,
                    &ctx.fonts as &dyn FontSource,
                    ctx.scale,
                    &mut p.font_cache,
                )
            },
            Some,
        )
        .ok_or(CoreError::FrameTooBig)?;
    p.dirty.extend(regions);
    if p.dirty.len() > MAX_DIRTY_RECTS {
        p.emitted = 0;
        p.dirty = vec![Rect {
            x: 0.0,
            y: 0.0,
            width: list.width,
            height: list.height,
        }];
    }
    p.list = Some(list);
    p.frame = Some(frame);
    p.png = None;
    p.spec_paint = false;
    Ok(())
}

/// Fenêtre raster (consommation R-62) : document plus haut
/// que le cap → la frame ne couvre que la tranche viewport.
/// Les items sont déjà ancrés `-scroll_y` au build, la
/// fenêtre liste est donc `[0, vh]` — le cap passe du
/// document à la fenêtre : github/MDN/stallman (docs >16 384
/// px) produisent une frame `vw×vh` au lieu d'un pixmap de
/// ~52 Mio re-rasterisé à chaque scroll. `doc_h` reste réel
/// pour le clamp de scroll ; `doc_h_clamped` marque le mode.
/// Pas de diff dirty ici : le scroll re-translate tous les
/// items (`full_cover` quasi systématique) et la fenêtre est
/// petite — la tranche pleine coûte moins que la machinerie.
fn raster_window(
    p: &mut PaintPayload,
    ctx: &ApplyCtx,
    list: DisplayList,
    vw: f32,
    vh: f32,
) -> Result<(), CoreError> {
    let frame = explorer_render::rasterize_window_cached(
        &list,
        &ctx.fonts as &dyn FontSource,
        ctx.scale,
        0.0,
        vh,
        &mut p.font_cache,
    )
    .ok_or(CoreError::FrameTooBig)?;
    p.emitted = 0;
    p.dirty = vec![Rect {
        x: 0.0,
        y: 0.0,
        width: vw,
        height: vh,
    }];
    p.list = Some(list);
    p.frame = Some(frame);
    p.png = None;
    p.spec_paint = false;
    Ok(())
}

/// Surlignage find en overlay : jaune par occurrence,
/// orange sur la courante — un match qui enjambe plusieurs
/// fragments peint chacun de ses sous-rects (R-35).
fn append_find_highlights(list: &mut DisplayList, f: &FindState) {
    let base = Color {
        r: 255,
        g: 235,
        b: 59,
        a: 110,
    };
    let cur = Color {
        r: 255,
        g: 152,
        b: 0,
        a: 160,
    };
    for (i, occ) in f.rects.iter().enumerate() {
        let color = if i == f.current { cur } else { base };
        for &rect in occ {
            list.items.push(DisplayItem::FillRect { rect, color });
        }
    }
}

/// Table `<canvas>` depuis les ops capturées — variante de
/// [`canvas_table`] sans accès moteur (la payload n'a pas
/// `entry.js` ; les ops voyagent dans [`PendingApply`]).
fn canvas_table_from_ops(ops: &CanvasOpsTable, dom: &Dom) -> CanvasTable {
    let mut out = CanvasTable::new();
    for (path, recs) in ops {
        if recs.is_empty() {
            continue;
        }
        let Some(n) = dom_path_node(dom, path) else {
            continue;
        };
        out.insert(n, recs.iter().map(to_canvas_op).collect());
    }
    out
}

impl Session {
    /// Contexte d'apply cloné — Arc + copies, sous verrou
    /// court (étape 55h).
    pub(crate) fn apply_ctx(&self) -> ApplyCtx {
        ApplyCtx {
            fonts: SharedFonts(Arc::clone(&self.webfonts)),
            viewport: self.viewport,
            scale: self.scale,
            image_max_px: self.image_max_px,
            color_scheme: self.color_scheme,
            dom_journal_max_ops: self.dom_journal_max_ops,
            scripting: self.js_factory.is_some(),
        }
    }

    /// Point de synchro DOM unifié (ADR-0025) : **journal
    /// d'abord**, baseline cléée sinon — `None` sans
    /// nouveauté. Les moteurs sans journal répondent `None`
    /// à `drain_dom_mutations` (défaut du trait) et livrent
    /// `ids` vide — comportement historique conservé.
    pub(crate) fn sync_for_apply(eng: &mut dyn JsEngine) -> Option<ApplySync> {
        if let Some(j) = eng.drain_dom_mutations() {
            return Some(ApplySync::Journal(j));
        }
        eng.sync_dom_keyed().map(ApplySync::Keyed)
    }

    /// Enfile le produit de [`Self::sync_for_apply`].
    pub(crate) fn queue_apply_sync(&mut self, tab: TabId, s: ApplySync) {
        match s {
            ApplySync::Journal(j) => self.queue_apply_journal(tab, j),
            ApplySync::Keyed(k) => self.queue_apply_keyed(tab, k),
        }
    }

    /// Variante cléée de [`Self::queue_apply`] (ADR-0025) —
    /// `keyed.ids` arme la table d'identité au re-parse.
    pub(crate) fn queue_apply_keyed(&mut self, tab: TabId, keyed: explorer_js::DomSyncKeyed) {
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        let canvas_ops = entry
            .js
            .as_deref_mut()
            .map_or_else(CanvasOpsTable::new, explorer_js::JsEngine::canvas_ops);
        let epoch = entry.nav_epoch;
        self.pending_applies.retain(|p| p.tab != tab);
        self.pending_applies.push_back(PendingApply {
            tab,
            epoch,
            html: keyed.html,
            dirty: keyed.dirty,
            canvas_ops,
            relayout: true,
            journal: None,
            ids: keyed.ids,
            tags: keyed.tags,
        });
    }

    /// Enfile un **journal** de mutations (ADR-0025) — le
    /// DOM commis est muté en place au drain (`NodeId`
    /// stables), le re-parse n'intervient qu'au rejet.
    /// `html` reste vide : le repli est re-alimenté par
    /// `freshen_apply`/`sync_dom_keyed`.
    pub(crate) fn queue_apply_journal(&mut self, tab: TabId, journal: explorer_js::DomJournal) {
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        let canvas_ops = entry
            .js
            .as_deref_mut()
            .map_or_else(CanvasOpsTable::new, explorer_js::JsEngine::canvas_ops);
        let epoch = entry.nav_epoch;
        self.pending_applies.retain(|p| p.tab != tab);
        self.pending_applies.push_back(PendingApply {
            tab,
            epoch,
            html: String::new(),
            dirty: None,
            canvas_ops,
            relayout: true,
            journal: Some(journal),
            ids: Vec::new(),
            tags: Vec::new(),
        });
    }

    /// Commit d'un apply abouti : titre (`TitleChanged`),
    /// `sync_floor` proportionnel au coût (55g) et
    /// `FrameInvalidated` — sous `&mut self`, jamais depuis
    /// le worker.
    pub(crate) fn commit_apply(&mut self, tab: TabId, out: &ApplyOutcome) {
        if let Some(t) = &out.title {
            if let Some(entry) = self.tabs.get_mut(&tab) {
                entry.title.clone_from(t);
            }
            if let Some(t) = t {
                let _ = self.events.send(CoreEvent::TitleChanged {
                    tab,
                    title: t.clone(),
                });
            }
        }
        if out.repainted {
            if out.incremental {
                self.incremental_syncs += 1;
            }
            let floor = out.cost.clamp(self.paint_interval, self.resync_gap_max);
            if let Some(entry) = self.tabs.get_mut(&tab) {
                entry.sync_floor = Some(Instant::now() + floor);
            }
            // L'apply a repeint → DOM + arbre en phase :
            // les surfaces média insérées/retirées/déplacées
            // par la page JS sont diffées ici (étape 71b).
            self.refresh_media(tab);
            self.emit_invalidated(tab);
        }
    }

    /// Rafraîchit une entrée de file depuis le miroir : si
    /// celui-ci a remuté **après** l'enfilement (séquence
    /// mutation → synchro → contre-mutation, ex. checkbox
    /// basculée puis restaurée par `preventDefault`), la
    /// sérialisation capturée à l'enfilement est périmée —
    /// la fraîche la subsume (document complet). `None` du
    /// miroir = rien de neuf depuis → la sérialisation
    /// enfilée est déjà la plus récente. Moteur absent
    /// (pompe hors verrou en vol) → idem, repli conservateur.
    fn freshen_apply(&mut self, tab: TabId, job: &mut PendingApply) {
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        let Some(eng) = entry.js.as_deref_mut() else {
            return;
        };
        // ADR-0025 : un journal s'enchaîne — les ops
        // produites après l'enfilement se soudent en
        // queue (jamais de « périmé » : la séquence est
        // l'état, pas un snapshot). L'empreinte la plus
        // récente fait foi.
        if let Some(j) = &mut job.journal {
            if let Some(tail) = eng.drain_dom_mutations() {
                j.ops.extend(tail.ops);
                j.fingerprint = tail.fingerprint;
            }
        } else {
            // `html` vide = rejet de journal re-enfilé —
            // le dirty a été consommé au drain : forcer la
            // re-sérialisation. Sinon rafraîchissement
            // ordinaire, conditionné par le bit sale.
            let k = if job.html.is_empty() {
                eng.resync_dom_keyed()
            } else {
                eng.sync_dom_keyed()
            };
            if let Some(k) = k {
                job.html = k.html;
                job.dirty = k.dirty;
                job.ids = k.ids;
                job.tags = k.tags;
                job.canvas_ops = eng.canvas_ops();
            }
        }
    }

    /// Sort le prochain apply en file et détache la payload
    /// de l'onglet — `None` si la file est vide ou si la
    /// peinture de l'onglet tête est déjà en vol (l'apply en
    /// cours gagne ; l'entrée est repoussée pour le prochain
    /// drain — sa sérialisation sera de toute façon régénérée
    /// au prochain point de synchro si la page remute).
    /// Les entrées dont l'epoch a été supplantée (navigation
    /// plus récente) sont jetées ici.
    pub(crate) fn take_next_apply(
        &mut self,
    ) -> Option<(PendingApply, DetachedPaint, ApplyCtx, Option<String>)> {
        loop {
            let mut job = self.pending_applies.pop_front()?;
            self.reclaim_orphaned_paint(job.tab);
            let ctx = self.apply_ctx();
            {
                let Some(entry) = self.tabs.get_mut(&job.tab) else {
                    continue; // onglet fermé entre-temps
                };
                if entry.paint_detached {
                    self.pending_applies.push_front(job);
                    return None;
                }
                if entry.nav_epoch != job.epoch {
                    continue; // navigation supplantée → caduque
                }
            }
            self.freshen_apply(job.tab, &mut job);
            let entry = self.tabs.get_mut(&job.tab)?;
            entry.paint_detached = true;
            let p = entry.take_paint();
            let g = DetachedPaint::new(
                std::sync::Arc::clone(&self.orphaned_paints),
                job.tab,
                job.epoch,
                p,
            );
            return Some((job, g, ctx, entry.title.clone()));
        }
    }

    /// Réinsère la payload et commit les effets — garde
    /// epoch : si l'onglet a été re-navigué pendant l'apply,
    /// la payload est jetée (le drapeau est réarmé pour la
    /// peinture de la nouvelle page).
    pub(crate) fn commit_apply_detached(
        &mut self,
        tab: TabId,
        epoch: u64,
        payload: PaintPayload,
        out: &ApplyOutcome,
    ) {
        self.reclaim_orphaned_paint(tab);
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return; // onglet fermé pendant l'apply
        };
        if !entry.paint_detached {
            // Drapeau déjà libéré : la payload est parquée
            // plutôt que jetée (issue #23 — la prochaine
            // réclamation la réinsère si encore vivante).
            self.park_orphaned_paint(tab, epoch, payload);
            return;
        }
        entry.paint_detached = false;
        if entry.nav_epoch == epoch {
            // Le scroll a pu avancer pendant l'apply (il vit
            // sur `Tab`, pas dans la payload) — une passe de
            // repaint replace la frame si besoin.
            let scrolled = (entry.scroll_y - payload.scroll_y).abs() > f32::EPSILON;
            entry.put_paint(payload);
            if scrolled && self.repaint(tab).is_ok() && !out.repainted {
                // L'apply n'a pas repeint (échec) mais le
                // repositionnement, si : son invalidation.
                self.emit_invalidated(tab);
            }
            self.commit_apply(tab, out);
            // Étape 91 : les entrées différées pendant
            // l'apply rejouent maintenant que `dom`/`tree`
            // sont de retour (focus/édition restaurés).
            self.drain_deferred_inputs(tab);
        }
        // Epoch changée : la page a été supplantée — la
        // payload est abandonnée (sa propre nav repeint).
    }

    /// Détache la payload de l'onglet pour le rendu de
    /// navigation hors verrou (étape 90) — même mécanique
    /// que l'apply (55h) : `paint_detached` posé, les
    /// applies en file attendent la réinsertion. Le
    /// `canvas_ops` du moteur est capturé ici (le moteur
    /// reste sur `Tab`).
    pub(crate) fn take_nav_paint(&mut self, tab: TabId, epoch: u64) -> NavPaint {
        self.reclaim_orphaned_paint(tab);
        let ctx = self.apply_ctx();
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return NavPaint::Superseded;
        };
        if entry.paint_detached {
            return NavPaint::Busy;
        }
        if entry.nav_epoch != epoch {
            return NavPaint::Superseded;
        }
        let canvas_ops = entry
            .js
            .as_deref_mut()
            .map_or_else(CanvasOpsTable::new, explorer_js::JsEngine::canvas_ops);
        entry.paint_detached = true;
        let p = entry.take_paint();
        NavPaint::Ready(
            Box::new(DetachedPaint::new(
                std::sync::Arc::clone(&self.orphaned_paints),
                tab,
                epoch,
                p,
            )),
            ctx,
            canvas_ops,
        )
    }

    /// Réinsère la payload de rendu de navigation — garde
    /// epoch identique à `commit_apply_detached` : le flag
    /// est toujours libéré (le worker est le seul détenteur,
    /// une nav plus récente ne peut pas l'avoir repris —
    /// `Busy` l'en empêche) ; la payload n'est réinsérée que
    /// si l'epoch vit encore. `true` = commise sur l'epoch
    /// vivante (l'appelant enchaîne `sub_commit_tail`).
    pub(crate) fn commit_nav_paint(
        &mut self,
        tab: TabId,
        epoch: u64,
        payload: PaintPayload,
    ) -> bool {
        self.reclaim_orphaned_paint(tab);
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return false;
        };
        if !entry.paint_detached {
            // Drapeau déjà libéré (rearm/réclamation) : la
            // payload est parquée plutôt que jetée — la
            // prochaine réclamation la réinsèrera si elle
            // est encore d'actualité (issue #23).
            self.park_orphaned_paint(tab, epoch, payload);
            return false;
        }
        entry.paint_detached = false;
        if entry.nav_epoch != epoch {
            return false; // nav supplantée : la payload est jetée
        }
        let scrolled = (entry.scroll_y - payload.scroll_y).abs() > f32::EPSILON;
        entry.put_paint(payload);
        if scrolled && self.repaint(tab).is_ok() {
            self.emit_invalidated(tab);
        }
        self.drain_deferred_inputs(tab);
        true
    }

    /// Journal rejeté en apply détaché (ADR-0025) : restitue
    /// la payload intacte (epoch gardée — une navigation
    /// supplantée la jette) et re-enfile le job en mode
    /// parse ; `freshen_apply` tirera `sync_dom_keyed` au
    /// prochain drain — baseline cléée, table remontée.
    pub(crate) fn journal_apply_rejected(
        &mut self,
        tab: TabId,
        epoch: u64,
        payload: PaintPayload,
        mut job: PendingApply,
        why: &JournalReject,
    ) {
        self.journal_fallbacks += 1;
        tracing::warn!(tab, ?why, "journal DOM rejeté → repli re-parse (ADR-0025)");
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.paint_detached = false;
            if entry.nav_epoch == epoch {
                entry.put_paint(payload);
            }
        }
        job.journal = None;
        job.html.clear();
        job.ids.clear();
        job.tags.clear();
        self.pending_applies.push_front(job);
    }

    /// Réarme `paint_detached` après une panique du worker de
    /// rendu de nav — la payload est perdue avec lui : l'état
    /// vide dégrade en lazy-heal (comme après apply, 55h).
    /// Les entrées différées sont jetées aussi — le `dom`
    /// sur lequel elles visaient n'existe plus.
    pub(crate) fn rearm_nav_paint(&mut self, tab: TabId) {
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.paint_detached = false;
            entry.deferred_inputs.clear();
        }
    }

    /// Parque une payload détachée dont le commit n'a pu
    /// aboutir (issue #23) — même slot que la garde
    /// [`DetachedPaint`] : elle sera reprise par
    /// [`Self::reclaim_orphaned_paint`] à la prochaine
    /// opportunité sous verrou.
    pub(crate) fn park_orphaned_paint(&mut self, tab: TabId, epoch: u64, p: PaintPayload) {
        let _ = self
            .orphaned_paints
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(tab, (epoch, p));
    }

    /// Réinsère une payload détachée devenue orpheline
    /// (issue #23) : une tâche de rendu annulée en vol —
    /// timeout/déconnexion du client HTTP de navigation
    /// pendant le `spawn_blocking`, drain interrompu —
    /// jetait la payload avec `paint_detached` collé :
    /// `/dom`/`/layout`/`/mem` 404 permanents, hit-test et
    /// scroll morts. Ici, à la première opportunité sous
    /// verrou (sonde, prochaine peinture, input), l'onglet
    /// la reprend si son epoch vit encore — sinon elle est
    /// jetée (la nav plus récente porte la sienne).
    /// `paint_detached` est toujours libéré : un orphelin
    /// en attente signifie que le détenteur est mort.
    pub fn reclaim_orphaned_paint(&mut self, tab: TabId) {
        let parked = self
            .orphaned_paints
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&tab);
        let Some((epoch, p)) = parked else {
            return;
        };
        let Some(e) = self.tabs.get_mut(&tab) else {
            return; // onglet fermé entre-temps — jetée
        };
        e.paint_detached = false;
        if e.nav_epoch == epoch {
            e.put_paint(p);
        }
    }

    /// Seam de test/diagnostic (issue #23) : simule la mort
    /// d'un futur détenant la payload détachée — détache
    /// comme `nav_commit_shared` puis détruit la garde, ce
    /// qui parque l'orpheline exactement comme un drop de
    /// tâche en vol. La réclamation suivante doit la
    /// réinsérer. Hors tests, ne pas appeler.
    #[doc(hidden)]
    pub fn debug_drop_detached_paint(&mut self, tab: TabId) {
        let epoch = self.tabs.get(&tab).map_or(0, |e| e.nav_epoch);
        if let NavPaint::Ready(g, _ctx, _ops) = self.take_nav_paint(tab, epoch) {
            drop(g);
        }
    }

    /// Apply synchrone d'une entrée de file — take/put sous
    /// `&mut self` (hôtes `&mut` : compat `handle_input`,
    /// pompe synchrone, FFI/CLI/tests).
    fn run_apply(&mut self, job: &mut PendingApply) {
        let tab = job.tab;
        self.reclaim_orphaned_paint(tab);
        let ctx = self.apply_ctx();
        {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return;
            };
            if entry.paint_detached || entry.nav_epoch != job.epoch {
                // Peinture déjà en vol (l'apply gagne) ou
                // epoch supplantée (la nav courante a sa
                // peinture).
                return;
            }
        }
        let t_apply = Instant::now();
        tracing::info!(tab, journal = job.journal.is_some(), "apply: début");
        self.freshen_apply(tab, job);
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        let old_title = entry.title.clone();
        let epoch = job.epoch;
        let mut p = entry.take_paint();
        // ADR-0025 : rejeu du journal en place — `NodeId`
        // stables, zéro re-parse. Tout rejet bascule sur la
        // sérialisation complète (garde-fou auto-réparant :
        // le job est re-enfilé en mode parse, `freshen_apply`
        // tirera la baseline cléée la plus fraîche).
        match run_apply_job(&mut p, &ctx, job, old_title.as_deref()) {
            Ok(out) => {
                if job.journal.is_some() {
                    self.journal_applies += 1;
                }
                if let Some(entry) = self.tabs.get_mut(&tab) {
                    entry.put_paint(p);
                }
                self.commit_apply(tab, &out);
                self.drain_deferred_inputs(tab);
                tracing::info!(
                    tab,
                    ms = u64::try_from(t_apply.elapsed().as_millis()).unwrap_or(u64::MAX),
                    "apply: fin"
                );
            }
            Err(why) => {
                let job = std::mem::take(job);
                self.journal_apply_rejected(tab, epoch, p, job, &why);
            }
        }
    }

    /// Vide la file d'applies sous `&mut self` — drain de
    /// compat pour les hôtes synchrones (même sémantique que
    /// l'apply immédiat d'avant 55h, à coalescence près).
    pub fn drain_applies(&mut self) {
        while let Some(mut job) = self.pending_applies.pop_front() {
            self.run_apply(&mut job);
        }
    }

    /// Nombre d'applies en file (observabilité/tests).
    #[must_use]
    pub fn pending_apply_count(&self) -> usize {
        self.pending_applies.len()
    }
}

/// File des applies en attente — sur `Session` (une entrée
/// par onglet maximum, coalescée à `queue_apply`).
pub(crate) type ApplyQueue = VecDeque<PendingApply>;

/// Fonds `url(http…)` et masques `mask-image` fetchés
/// pendant la vague (étapes 98 et 121 — demande PIPE R-86) :
/// les corps posés sur `p` par `stage_backgrounds`/l'apply
/// détachée sont insérés dans `images.backgrounds` et
/// `images.masks_url`. Pour les fonds, le `NodeId`
/// propriétaire est retrouvé par la valeur
/// `background-image` calculée, via `bg_aliases` (texte
/// `url(...)` brut → URL résolue ; la feuille d'origine ne
/// survit pas à la cascade). Pour les masques, la clé est le
/// texte brut lui-même — `PaintImages::register_mask` indexe
/// `masks_url` comme le fait `mask_url(style)` côté render,
/// ce qui couvre les pseudo-boîtes dépourvues de `NodeId`.
/// Fond/masque `data:` ou non fetché → pas d'alias → ignoré,
/// jamais d'échec.
///
/// **Idempotent** (étape 108) : les corps `bg_bodies` sont
/// drainés une fois vers le cache persistant `bg_decoded`
/// (URL résolue → image, partagé entre les deux rôles),
/// `bg_aliases`/`mask_aliases` restent — chaque apply
/// ré-insère sur les `NodeId` régénérés au re-parse et
/// ré-enregistre `masks_url`, RAZ avec `PaintImages::default()`.
/// Symétrique de `reinsert_imgs_by_src` ; avant ce cache,
/// les fonds fetchés mouraient au second apply.
fn insert_fetched_images(p: &mut PaintPayload, styles: &StyleMap, max_px: u32) {
    if p.bg_aliases.is_empty() && p.mask_aliases.is_empty() {
        return;
    }
    // Drain unique : corps → cache décodé **+ magasin retenu
    // `img_bodies`** (étape 133 — source du re-décodage au
    // retour bfcache). Les applys suivants trouvent
    // `bg_bodies` vide et ne coûtent que la boucle de
    // correspondance styles → cache.
    for (resolved, body) in std::mem::take(&mut p.bg_bodies) {
        net::retain_img_body(&mut p.img_bodies, &resolved, body.clone());
        p.bg_decoded
            .entry(resolved)
            .or_insert_with(|| decode_image(&body, max_px).map(Arc::new));
    }
    for (node, st) in styles.iter() {
        let Some(raw) = st.get("background-image").and_then(css_url_arg) else {
            continue;
        };
        let Some(resolved) = p.bg_aliases.get(raw.as_str()) else {
            continue;
        };
        if let Some(Some(img)) = p.bg_decoded.get(resolved.as_str()) {
            p.images.backgrounds.insert(node, img.clone());
        }
    }
    // Masques : la table `masks_url` est cléée par le texte
    // `url(...)` brut (la forme relue par `mask_url(style)`
    // au paint) — inutile de re-scanner `styles` : alias →
    // cache décodé → enregistrement direct. Couvre de fait
    // les pseudo-éléments, qui n'ont pas de `NodeId`.
    for (raw, resolved) in &p.mask_aliases {
        if let Some(Some(img)) = p.bg_decoded.get(resolved.as_str()) {
            let img = img.clone();
            p.images.register_mask(raw, img);
        }
    }
    if !p.mask_aliases.is_empty() {
        tracing::trace!(
            declared = p.mask_aliases.len(),
            registered = p.images.masks_url.len(),
            "mask: masques enregistrés"
        );
    }
}

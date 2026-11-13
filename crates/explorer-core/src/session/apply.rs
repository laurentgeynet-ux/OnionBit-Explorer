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
use explorer_css::{
    parse_stylesheet, ComputedStyle, Origin, StyleEngine, StyleEnv, StyleMap, Stylesheet,
};
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
    /// Cap d'octets des bitmaps décodés retenus (§B-59 —
    /// `SessionConfig::image_max_bytes`).
    pub image_max_bytes: usize,
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
    /// Noms `customElements` du miroir — clonés de
    /// `Tab.defined_names` au `take_paint` pour le bake des
    /// `Dom` reconstruits pendant la passe détachée
    /// (`:defined`, selectors-4 §13). Jamais ré-écrit par
    /// `put_paint` : `Tab.defined_names` fait foi (le drain
    /// a pu livrer plus frais pendant la fenêtre).
    pub defined_names: Vec<String>,
    /// `document.adoptedStyleSheets` → textes CSS par feuille
    /// (CAP-195 tranche 2) — cascadées en feuilles auteur
    /// document **après** `linked_sheets` (ordre spec).
    pub adopted_sheets: Vec<String>,
    /// `<style>` inline expansés `@import` (étape 218) :
    /// texte d'origine → texte expansé — substitué par
    /// contenu dans `styles_payload`.
    pub inline_css_expanded: HashMap<String, String>,
    /// Feuilles déjà **parsées** clées par texte CSS
    /// (étape 227) — voyage avec la payload ; miroir de
    /// `Tab::sheet_cache`, borné à `types::MAX_SHEET_CACHE`.
    pub sheet_cache: HashMap<String, Stylesheet>,
    /// Map de styles persistée (étape 228) — base du restyle
    /// scopé sur la voie journal in-place ; `None` =
    /// invalide (DOM neuf installé). Miroir de `Tab::styles`.
    pub styles: Option<StyleMap>,
    /// Moteur de cascade persisté (étape 228) — réutilisé
    /// tant que `style_sig` tient (feuilles identiques,
    /// DOM indifférent).
    pub style_eng: Option<StyleEngine>,
    /// Signature des feuilles de `style_eng`.
    pub style_sig: Option<u64>,
    /// `:has(` présent dans les feuilles de `style_eng` —
    /// interdit le restyle scopé (portée non bornable).
    pub eng_has_relational: bool,
    /// `StyleEnv` de la passe qui a produit `styles`.
    pub styles_env: Option<StyleEnv>,
    /// Styles bruts de la dernière cascade (transitions).
    pub prev_styles: HashMap<usize, ComputedStyle>,
    /// Transitions en vol.
    pub transitions: Vec<ActiveTransition>,
    /// Animations `@keyframes` enregistrées.
    pub animations: Vec<ActiveAnimation>,
    /// Naissances `animationstart` de la passe (étape 141b)
    /// — `(NodeId, nom keyframes)` collectées par
    /// [`sync_animations`], vidées dans
    /// `Tab::pending_anim_births` au `put_paint` : la
    /// session les traduit en événements `animationstart`
    /// sur le miroir JS (découverte lazy des registries CE,
    /// github `x(e)` → `m.get(animationName)`).
    pub anim_births: Vec<(explorer_dom::NodeId, String)>,
    /// Hauteur du document posé.
    pub doc_h: f32,
    /// Scroll peint dans `frame` (le `scroll_y` de `Tab`
    /// fait foi hors de la payload — un scroll pendant la
    /// fenêtre n'est pas écrasé à la réinsertion).
    pub scroll_y: f32,
    /// Drapeau peinture spéculative (56c).
    pub spec_paint: bool,
    /// `frame` est une **tranche viewport** (raster fenêtré
    /// Étape 180, P4 — `raster_window` l'a posé) : les
    /// consommateurs de « frame = document » (`frame_png_full`,
    /// `doc_h_clamped`) lisent ce flag — jamais dérivé d'une
    /// constante, la borne qui arme le mode est dynamique.
    pub windowed: bool,
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
    /// Watermark du prochain `Dom::collect` (Étape 208,
    /// AR-3) — voyage avec la payload comme `keymap` ;
    /// `Tab::dom_gc_mark` fait foi hors de la passe.
    pub dom_gc_mark: usize,
    /// Chemins de cycle §B-26 déjà journalisés pour cet
    /// onglet — la borne saute des sur-émissions à *chaque*
    /// apply (vol 67 : 994 WARN identiques `[0,0,0,0]` sur
    /// meteofrance noyaient le journal) : un WARN par chemin
    /// suffit, les suivants passent en `debug`.
    pub cycle_warned: std::collections::HashSet<Box<[u16]>>,
    /// Dette de restyle : chemins moteur de sous-arbres dont
    /// les attributs ont changé lors d'une synchro **sans
    /// relayout** (`apply_dom_sync` `relayout=false` — DOM
    /// re-parsé et titre seulement pendant que les
    /// sous-ressources arrivent). Un sélecteur d'attribut
    /// (`[inert]`, classe, `+`/`~`, `:nth-*`) resterait figé
    /// dans `bx.style` jusqu'au prochain relayout couvrant —
    /// vol santemag (§B-17) : le drawer `inert` restait
    /// ciblable car aucun apply couvrant n'était passé
    /// depuis sa pose. Chaque entrée = `element_path` du
    /// **parent** du nœud changé (couvre self et descendants
    /// et frères) ; fusionnée au `dirty` du prochain apply
    /// relayout, purgée au commit.
    pub style_dirty: Vec<Vec<(String, u32)>>,
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
            // Le registre voyage avec la passe : les `Dom`
            // reconstruits par la synchro le re-bakent
            // (`apply_journal`), `put_paint` re-bake aussi
            // depuis `Tab.defined_names` (plus frais possible).
            defined_names: self.defined_names.clone(),
            adopted_sheets: self.adopted_sheets.clone(),
            inline_css_expanded: std::mem::take(&mut self.inline_css_expanded),
            sheet_cache: std::mem::take(&mut self.sheet_cache),
            styles: self.styles.take(),
            style_eng: self.style_eng.take(),
            style_sig: self.style_sig,
            eng_has_relational: self.eng_has_relational,
            styles_env: self.styles_env.take(),
            prev_styles: std::mem::take(&mut self.prev_styles),
            transitions: std::mem::take(&mut self.transitions),
            animations: std::mem::take(&mut self.animations),
            // `pending_anim_births` reste sur `Tab` — ce ne sont
            // pas des données de la passe, mais une file
            // d'événements JS en attente de livraison.
            anim_births: Vec::new(),
            doc_h: self.doc_h,
            scroll_y: self.scroll_y,
            spec_paint: std::mem::take(&mut self.spec_paint),
            windowed: self.windowed,
            dirty: std::mem::take(&mut self.dirty),
            emitted: self.emitted,
            bg_aliases: std::mem::take(&mut self.bg_aliases),
            bg_bodies: std::mem::take(&mut self.bg_bodies),
            mask_aliases: std::mem::take(&mut self.mask_aliases),
            img_decoded: std::mem::take(&mut self.img_decoded),
            bg_decoded: std::mem::take(&mut self.bg_decoded),
            img_bodies: std::mem::take(&mut self.img_bodies),
            keymap: std::mem::take(&mut self.dom_keymap),
            dom_gc_mark: self.dom_gc_mark,
            cycle_warned: std::mem::take(&mut self.cycle_warned),
            style_dirty: std::mem::take(&mut self.style_dirty),
        }
    }

    /// Réinsère la payload — appelé au commit (epoch
    /// vérifiée par l'appelant). `scroll_y` n'est pas
    /// restauré : le champ sur `Tab` a pu avancer pendant
    /// la fenêtre (scroll input) et fait foi.
    pub(crate) fn put_paint(&mut self, p: PaintPayload) {
        self.dom = p.dom;
        // Registre `customElements` (CAP-195 tranche 2 —
        // `:defined`) : la copie vivant dans `p.dom` date du
        // `take_paint` ; `drain_script_net` a pu livrer des
        // noms plus frais pendant la fenêtre détachée.
        if let Some(d) = self.dom.as_mut() {
            d.set_defined_names(self.defined_names.iter().cloned());
        }
        self.tree = p.tree;
        self.tree_epoch = p.tree_epoch;
        self.list = p.list;
        self.frame = p.frame;
        // Génération frame (étape 188 — atomicité du pull
        // top-slice : `e` dans `FrameMeta`/`?slice=top`).
        self.frame_epoch = self.frame_epoch.wrapping_add(1);
        // Scroll **peint** de cette passe (même étape) :
        // `scroll_y` a pu avancer pendant la fenêtre
        // détachée — la méta frame/top décale de l'ancrage
        // réel de la frame, pas de la position demandée.
        self.painted_scroll_y = p.scroll_y;
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
        self.adopted_sheets = p.adopted_sheets;
        self.inline_css_expanded = p.inline_css_expanded;
        self.sheet_cache = p.sheet_cache;
        self.styles = p.styles;
        self.style_eng = p.style_eng;
        self.style_sig = p.style_sig;
        self.eng_has_relational = p.eng_has_relational;
        self.styles_env = p.styles_env;
        self.prev_styles = p.prev_styles;
        self.transitions = p.transitions;
        self.animations = p.animations;
        if self.pending_anim_births.len() < types::MAX_ANIM_BIRTHS {
            self.pending_anim_births.extend(p.anim_births);
            while self.pending_anim_births.len() > types::MAX_ANIM_BIRTHS {
                self.pending_anim_births.pop_back();
            }
        }
        self.doc_h = p.doc_h;
        self.spec_paint = p.spec_paint;
        self.windowed = p.windowed;
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
        self.dom_gc_mark = p.dom_gc_mark;
        self.cycle_warned = p.cycle_warned;
        self.style_dirty = p.style_dirty;
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

    /// Epoch de navigation capturée au détachement — le
    /// commit la confronte à `Tab::nav_epoch` (garde
    /// anti-réinsertion d'une passe supplantée).
    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
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
    let mut new_dom = TreeBuilder::parse_scripting(&job.html, ctx.scripting);
    // JS-C291 — resplice les nœuds transportés en marqueurs
    // `<template data-obreloc*>` : le re-parse spec foster-parente
    // tout enfant hors modèle de contenu (`document.head.
    // appendChild(iconset)` légal côté DOM → `<body>` côté
    // parseur). Sans ce resplice, l'ordre diverge → `UnmappedKey`
    // + `Fingerprint` → rejet journal en boucle (mur YouTube :
    // iconsets Polymer stampés dans `head`).
    relocate_obreloc(&mut new_dom);
    // B-50 (bordeaux) : un apply mort avant « apply: cascade »
    // n'avait aucun jalon — le parse de la sérialisation miroir
    // est maintenant borné temporellement dans le journal.
    tracing::info!(
        ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX),
        "apply: parse"
    );
    // Registre `customElements` (`:defined`) — transporté
    // par la payload, le Dom frais repart nu à chaque
    // re-parse.
    new_dom.set_defined_names(p.defined_names.iter().cloned());
    let new_title = extract_title(&new_dom);
    // §B-17 : la baseline cléée sert aussi de **diff
    // d'attributs** — les mutations d'attributs que le chemin
    // `dirty` du miroir a perdues (synchro `relayout=false`,
    // apply en attente évincé, marquage sauté sur nœud
    // détaché) figeraient sinon `bx.style` des sous-arbres
    // jamais reconstruits (vol santemag : le drawer `inert`
    // restait ciblable alors que l'attribut était dans le
    // DOM commis). Snapshot de la vieille table avant la RAZ.
    let old_keymap = std::mem::take(&mut p.keymap);
    build_keymap(&mut p.keymap, &new_dom, &job.ids, &job.tags);
    tracing::debug!(
        ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX),
        "apply: keymap"
    );
    // Chemins des **parents** des nœuds dont les attributs
    // ont changé (le parent couvre self + descendants +
    // frères — `+`/`~`/`:nth-*`) + dette accumulée des
    // synchros sans relayout.
    let mut extra = attrs_dirty_paths(p.dom.as_ref(), &old_keymap, &new_dom, &p.keymap);
    extra.append(&mut p.style_dirty);
    let eff_dirty = merged_dirty(job.dirty.clone(), &extra);
    // Relayout incrémental (étape 55f) avant tout
    // remplacement — il a besoin de l'ancien DOM et de
    // l'ancien arbre, tous deux dans la payload.
    let incremental = job.relayout
        && eff_dirty.as_deref().is_some_and(|path| {
            let (Some(t_old), Some(t_new)) = (
                p.dom.as_ref().and_then(|d| dom_path_node(d, path)),
                dom_path_node(&new_dom, path),
            ) else {
                return false;
            };
            try_incr_payload(
                p,
                ctx,
                job.tab,
                None,
                &new_dom,
                t_old,
                t_new,
                &job.canvas_ops,
            )
        });
    out.incremental = incremental;
    tracing::debug!(
        ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX),
        incremental,
        "apply: incr"
    );
    // Les op-lists `<canvas>` survivent au re-parse : le
    // miroir les conserve (bitmap cumulative) — la table est
    // construite sur `new_dom` depuis les ops capturées à
    // l'enfilement (chemins → `NodeId` du DOM commis).
    let canvases = canvas_table_from_ops(&job.canvas_ops, &new_dom);
    // ADR-0025 : baseline cléée — la table `miroir → NodeId`
    // suit le DOM frais (vide hors synchro cléée → le journal
    // suivant tombera en `NoBaseline` → re-parse, jamais à
    // faux) — reconstruite plus haut, avant le diff §B-17.
    // Étape 228 : DOM neuf — la map de styles persistée
    // (clée `NodeId` de l'ancien document) est invalidée,
    // sinon un index+génération coincident servirait un
    // style du document mort. `style_eng` survit : sa
    // validité est la signature des feuilles, pas le DOM.
    p.styles = None;
    p.dom = Some(new_dom);
    p.images = PaintImages::default();
    p.images.canvases = canvases;
    // `<img>`/affiches fetchés : re-match par `src`/`poster`
    // d'attribut — les `NodeId` viennent d'être régénérés au
    // re-parse et la table a été RAZ juste au-dessus (étape
    // 108 ; avant ce cache, tout `<img>` http(s) mourait à la
    // première synchro JS post-fetch).
    if let Some(dom) = p.dom.as_ref() {
        let env = viewport_env(ctx.viewport, ctx.color_scheme, ctx.scripting, ctx.scale);
        net::reinsert_imgs_by_src(dom, &mut p.images, &p.img_decoded, &env);
    }
    // Un re-parse de synchro rend obsolètes les options
    // figées du popup `<select>` — il se referme (étape 54) ;
    // les `NodeId` du find meurent de même → effacé (57bis).
    p.popup = None;
    p.find = None;
    tracing::debug!(
        ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX),
        "apply: imgs"
    );
    if old_title != new_title.as_deref() {
        out.title = Some(new_title);
    }
    if job.relayout {
        if !incremental {
            if let Some(dom) = p.dom.take() {
                if relayout_dom_payload(p, ctx, &dom, &job.canvas_ops).is_err() {
                    p.dom = Some(dom);
                    // Rien n'a été stylé : la dette repart
                    // entière (diff de ce tour inclus).
                    p.style_dirty = extra;
                    out.cost = t0.elapsed();
                    return out;
                }
                p.dom = Some(dom);
            }
        }
        out.repainted = true;
    } else {
        // Synchro sans relayout (sous-ressources en vol) :
        // ni le sous-arbre `job.dirty` ni les changements
        // d'attributs détectés ne sont stylés — tout part en
        // dette pour le prochain apply relayout.
        p.style_dirty = extra;
        if let Some(d) = &job.dirty {
            p.style_dirty.push(d.clone());
        }
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

/// `id` a-t-il un `<template>` repère dont le fragment
/// `content` **est** la racine shadow `root` (DSD) ? La
/// descente canonique `template_contents` couvre alors déjà
/// l'ombre — ne pas la réémettre une seconde fois.
fn shadow_has_repere(dom: &Dom, root: explorer_dom::NodeId, kids: &[explorer_dom::NodeId]) -> bool {
    kids.iter()
        .any(|&c| dom.template_contents(c).ok().flatten() == Some(root))
}

/// Empile l'ombre **programmatique** de `id` (journal
/// `AttachShadow` — le fragment n'a pas de `<template>`
/// repère en light DOM). Le miroir l'émet en
/// `<template shadowrootmode>` EN TÊTE des enfants : poussée
/// ici après les enfants light, la racine poppe avant eux —
/// ordre sérialisé `[shadow][light]` préservé.
fn push_orphan_shadow(
    dom: &Dom,
    id: explorer_dom::NodeId,
    kids: &[explorer_dom::NodeId],
    stack: &mut Vec<explorer_dom::NodeId>,
) {
    if let Some(root) = dom.shadow_root(id, true) {
        if !shadow_has_repere(dom, root, kids) {
            stack.push(root);
        }
    }
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
        if !(serializable_kind(dom, id) || dom.is_shadow_root(id)) {
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
        push_orphan_shadow(dom, id, &kids, &mut stack);
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
        if !(serializable_kind(dom, id) || dom.is_shadow_root(id)) {
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
        push_orphan_shadow(dom, id, &kids, &mut stack);
    }
}

/// Tag d'appariement d'une position moteur — une racine
/// shadow **programmatique** s'apparie au `template`
/// synthétique que le miroir émet pour elle (`<template
/// shadowrootmode>` : le fragment n'a pas de balise propre).
/// Le cas DSD reste couvert par le `<template>` repère
/// ordinaire (son fragment `content` est la racine — pas
/// `is_shadow_root` sur l'élément).
fn engine_tag_matches(dom: &Dom, id: explorer_dom::NodeId, tag: &str) -> bool {
    if dom.is_shadow_root(id) {
        return tag.eq_ignore_ascii_case("template");
    }
    dom.tag_name(id).is_ok_and(|t| t.eq_ignore_ascii_case(tag))
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
/// Hash par nœud de l'empreinte structurelle (JS-C315) —
/// FNV-1a sur les octets de tag + terminateur `0xff`,
/// graine identique au miroir (`dom.rs`). Combiné en
/// **somme** par `engine_fingerprint`/`structural_fingerprint`
/// → multiset : l'ordre pré-ordre n'influence plus la
/// valeur (la resplice `data-obreloc*` est invisible) mais
/// le compte et les tags comptent toujours (nœud
/// manquant/en trop/tag différent détectés).
fn fp_tag(tag: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in tag.bytes().map(|b| b.to_ascii_lowercase()) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h ^= 0xff;
    h.wrapping_mul(0x0000_0100_0000_01b3)
}

fn engine_fingerprint(dom: &Dom, keymap: &HashMap<u64, explorer_dom::NodeId>) -> u64 {
    let mapped: std::collections::HashSet<explorer_dom::NodeId> =
        keymap.values().copied().collect();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for id in engine_preorder(dom) {
        if !mapped.contains(&id) {
            continue;
        }
        // Fragment racine shadow (programmatique) : le miroir
        // compte le `template` synthétique émis à sa place.
        let tag = if dom.is_shadow_root(id) {
            Some("template")
        } else {
            dom.tag_name(id).ok()
        };
        let Some(tag) = tag else { continue };
        h = h.wrapping_add(fp_tag(tag));
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
        align_keyed(keymap, dom, &engine, ids, tags);
        return;
    }
    for (e, m) in engine.into_iter().zip(ids.iter().copied()) {
        keymap.insert(m, e);
    }
}

/// §B-17 — nœuds **conservés** au re-parse dont l'ensemble
/// d'attributs a changé, détectés sur les clés miroir
/// présentes des deux côtés de la baseline (ADR-0025). Un
/// `setAttribute` peut retourner un sélecteur d'attribut ou
/// de classe (`[inert]`, `[hidden]`, `.open`, `:nth-*`,
/// `+`/`~`) **hors du sous-arbre `dirty`** que le miroir
/// rapporte : sans ce diff, les boîtes du sous-arbre gardent
/// le style d'avant mutation indéfiniment (vol santemag —
/// le drawer `inert` restait ciblable alors que l'attribut
/// vivait dans le DOM commis). Rend l'`element_path` du
/// **parent** de chaque nœud changé : le sous-arbre parent
/// couvre le nœud, ses descendants et ses frères.
fn attrs_dirty_paths(
    old_dom: Option<&Dom>,
    old_keymap: &HashMap<u64, explorer_dom::NodeId>,
    new_dom: &Dom,
    new_keymap: &HashMap<u64, explorer_dom::NodeId>,
) -> Vec<Vec<(String, u32)>> {
    let Some(old_dom) = old_dom else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (mid, &new_n) in new_keymap {
        let Some(&old_n) = old_keymap.get(mid) else {
            continue;
        };
        if attrs_eq(old_dom.attributes(old_n), new_dom.attributes(new_n)) {
            continue;
        }
        // Parent — couvre aussi les frères (`+`/`~`, `:nth-*`)
        // dont la correspondance dépend de l'attribut muté.
        let target = new_dom.parent(new_n).ok().flatten().unwrap_or(new_n);
        if let Some(path) = element_path(new_dom, target) {
            out.push(path);
        }
    }
    out
}

/// Égalité d'**ensembles** d'attributs — l'ordre de
/// sérialisation n'est pas garanti entre miroir et re-parse.
/// Double inclusion (≈O(n²) par nœud, n = nb d'attributs ≤
/// ~10 : plus léger que deux tris alloués, appelé sur chaque
/// nœud de la baseline). `Err` d'un côté = non comparable →
/// égal (pas de fausse dette sur nœud invalide).
fn attrs_eq(
    a: Result<&[explorer_dom::Attribute], explorer_dom::DomError>,
    b: Result<&[explorer_dom::Attribute], explorer_dom::DomError>,
) -> bool {
    let (Ok(a), Ok(b)) = (a, b) else {
        return true;
    };
    let has = |x: &explorer_dom::Attribute, s: &[explorer_dom::Attribute]| {
        s.iter().any(|y| x.name == y.name && x.value == y.value)
    };
    a.len() == b.len() && a.iter().all(|x| has(x, b)) && b.iter().all(|x| has(x, a))
}

/// Cible de relayout effective = `dirty` du miroir ∪ dette
/// d'attributs : le **plus long préfixe commun** de tous les
/// chemins est le plus profond ancêtre dont le sous-arbre
/// couvre chaque mutation (`element_path` ordonne racine →
/// nœud — le préfixe commun est l'ancêtre partagé).
/// `Some(vec![])` = racine (≈ relayout complet), `None` =
/// rien de sale.
fn merged_dirty(
    dirty: Option<Vec<(String, u32)>>,
    extra: &[Vec<(String, u32)>],
) -> Option<Vec<(String, u32)>> {
    let mut it = dirty.into_iter().chain(extra.iter().cloned());
    let mut acc = it.next()?;
    for p in it {
        let n = acc.iter().zip(p.iter()).take_while(|(a, b)| a == b).count();
        acc.truncate(n);
    }
    Some(acc)
}

/// Appariement monotone **par tag** d'une séquence moteur
/// `engine` (pré-ordre, kinds sérialisables) aux clés miroir
/// `(ids, tags)` — partagé entre la baseline cléée
/// ([`build_keymap`]) et le rejeu des `Insert` (JS-C129 : un
/// nœud moteur synthétisé au re-parse ou non émis par le
/// miroir ne décale plus les clés suivantes — l'ancien zip
/// positionnel tronqué laissait les orphelins retomber en
/// `UnmappedKey` → re-parse). Textes : adjacence stricte +
/// hash de contenu égal, sinon non mappés. Les clés miroir
/// sans correspondant restent hors table — `UnmappedKey` au
/// rejeu (repli sûr, jamais faux positif).
fn align_keyed(
    keymap: &mut HashMap<u64, explorer_dom::NodeId>,
    dom: &Dom,
    engine: &[explorer_dom::NodeId],
    ids: &[u64],
    tags: &[String],
) {
    let mut j = 0usize;
    let mut unmapped: Vec<String> = Vec::new();
    'outer: for (i, &m) in ids.iter().enumerate() {
        if let Some(hex) = tags[i].strip_prefix("#text\x00") {
            // Texte : concordance exacte du contenu à
            // la position courante, sinon non mappé.
            let Some(e) = engine.get(j) else { break };
            let Ok(h) = u64::from_str_radix(hex, 16) else {
                break;
            };
            if let Ok(explorer_dom::NodeData::Text(t)) = dom.data(*e) {
                if fnv64(dom.text_at(*t)) == h {
                    keymap.insert(m, *e);
                    j += 1;
                }
            } else {
                unmapped.push(format!("#{i}:#text@{e:?}"));
            }
            continue;
        }
        let mut hit = false;
        while j < engine.len() {
            let e = engine[j];
            j += 1;
            if engine_tag_matches(dom, e, &tags[i]) {
                keymap.insert(m, e);
                hit = true;
                break;
            }
        }
        if hit {
            continue 'outer;
        }
        unmapped.push(format!("#{i}:{}(key={m})", tags[i]));
        // Fenêtres miroir + moteur autour du premier non-mappé —
        // localise le sous-arbre divergent (diagnostic JS-C289) :
        // miroir attend `tags[i]` ; le moteur montre ce qui reste
        // à partir du curseur `j`.
        let lo = i.saturating_sub(6);
        let hi = (i + 4).min(tags.len());
        let ewin: Vec<String> = engine[j.saturating_sub(6)..engine.len().min(j + 4)]
            .iter()
            .map(|&e| {
                if let Ok(t) = dom.tag_name(e) {
                    return t.to_string();
                }
                match dom.data(e) {
                    Ok(explorer_dom::NodeData::Text(_)) => "#text".to_string(),
                    _ => "?".to_string(),
                }
            })
            .collect();
        tracing::warn!(
            mirror_i = i,
            mirror_n = tags.len(),
            mirror = %tags[lo..hi].join(","),
            engine_j = j,
            engine = %ewin.join(","),
            "fenêtres miroir/moteur autour de la clé non mappée"
        );
        break; // plus de moteur : le reste reste non mappé
    }
    // Diagnostic JS-C289 — les clés miroir restées sans
    // correspondant (curseur épuisé ou tag introuvable)
    // deviennent `UnmappedKey` au rejeu : les nommer évite
    // de deviner la divergence (borne : 8 premières).
    if !unmapped.is_empty() {
        tracing::warn!(
            unmapped = %unmapped.iter().take(8).cloned().collect::<Vec<_>>().join(","),
            engine_n = engine.len(),
            "keymap : clés miroir sans correspondant"
        );
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
            .create_text(src.text_at(*t))
            .map_err(|_| JournalReject::Dom)?,
        NodeData::Comment(c) => dom
            .create_comment(src.text_at(*c))
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
    let refn = dom
        .children_iter(dst_parent)
        .map_err(|_| JournalReject::Dom)?
        .filter(|&n| serializable_kind(dom, n))
        .nth(index);
    dom.insert_before(dst_parent, new, refn)
        .map_err(|_| JournalReject::Dom)?;
    // Contenu `<template>` : arène à part des deux côtés.
    if let Ok(Some(tfrag)) = src.template_contents(src_id) {
        let tnew = dom.create_fragment().map_err(|_| JournalReject::Dom)?;
        dom.set_template_contents(new, tnew)
            .map_err(|_| JournalReject::Dom)?;
        for (i, c) in src.children_iter(tfrag).into_iter().flatten().enumerate() {
            explorer_common::deep(|| graft_subtree(dom, src, c, tnew, i))?;
        }
    }
    for (i, c) in src.children_iter(src_id).into_iter().flatten().enumerate() {
        explorer_common::deep(|| graft_subtree(dom, src, c, new, i))?;
    }
    Ok(new)
}

/// JS-C291 — resplice les sous-arbres transportés en marqueurs
/// `<template data-obreloc*>` par la sérialisation miroir : le
/// DOM autorise des parentages que le parseur spec renormalise
/// (enfants non-head sous `head`, `td` hors `tr`, éléments sous
/// `script`/`html`…). Chaque marqueur est dépouillé de ses
/// `template_contents` — resplicés sous la cible encodée par
/// l'attribut — puis supprimé. Les marqueurs n'ont ni clé ni
/// tag : `ids`/`tags`/`engine_preorder` restent alignés.
fn relocate_obreloc(dom: &mut Dom) {
    use explorer_dom::NodeData;
    // Borne : chaque itération supprime un marqueur — au plus
    // le nombre initial de marqueurs (+ marge), jamais de boucle.
    let mut budget = engine_preorder(dom).len().saturating_add(8);
    while budget > 0 {
        budget -= 1;
        // Premier marqueur en pré-ordre — les marqueurs
        // emboîtés (enveloppes `data-obreloc-x` incluses dans
        // le contenu resplicé) sont traités au passage suivant.
        let mut found = None;
        for n in engine_preorder(dom) {
            if !dom.tag_name(n).is_ok_and(|t| t == "template") {
                continue;
            }
            for a in [
                "data-obreloc",
                "data-obreloc-prev",
                "data-obreloc-html",
                "data-obreloc-x",
            ] {
                if let Ok(Some(v)) = dom.attribute_str(n, a) {
                    found = Some((n, a, v));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        let Some((marker, attr, val)) = found else {
            break;
        };
        // Cible + point d'insertion selon le marqueur.
        let (target, refr) = match attr {
            "data-obreloc-prev" => {
                // Frère élément précédent — le parent rawtext ne
                // peut pas porter le marqueur inline.
                let p = dom.parent(marker).ok().flatten();
                let mut prev = None;
                if let Some(p) = p {
                    let ks = dom.children(p).unwrap_or_default();
                    if let Some(mi) = ks.iter().position(|&k| k == marker) {
                        prev = ks[..mi]
                            .iter()
                            .rev()
                            .copied()
                            .find(|&k| matches!(dom.data(k), Ok(NodeData::Element(_))));
                    }
                }
                (prev, None)
            }
            "data-obreloc-html" => {
                // Cible = `documentElement` à l'index porté
                // (le marqueur a atterri sous head/body).
                let de = dom.document_element();
                let i = val.parse::<usize>().unwrap_or(usize::MAX);
                let refr = de
                    .and_then(|d| dom.children_iter(d).ok())
                    .and_then(|mut ks| ks.nth(i));
                (de, refr)
            }
            _ => {
                // `data-obreloc` / `data-obreloc-x` : le marqueur
                // est à la position exacte — splice avant lui.
                (dom.parent(marker).ok().flatten(), Some(marker))
            }
        };
        let Ok(Some(frag)) = dom.template_contents(marker) else {
            let _ = dom.remove(marker);
            continue;
        };
        let mut kids = dom.children(frag).unwrap_or_default();
        if attr == "data-obreloc-x" {
            // Déballe les enveloppes de contexte `"table>tbody>tr"`
            // — la cible reçoit les enfants du dernier niveau.
            for w in val.split('>') {
                let Some(inner) = kids
                    .iter()
                    .copied()
                    .find(|&k| dom.tag_name(k).is_ok_and(|t| t == w))
                else {
                    kids = Vec::new();
                    break;
                };
                kids = dom.children(inner).unwrap_or_default();
            }
        }
        if let Some(t) = target {
            for &c in &kids {
                let _ = dom.insert_before(t, c, refr);
            }
        }
        let _ = dom.remove(marker);
        let _ = dom.remove(frag);
    }
}

/// Seeds de vivacité pour `Dom::collect` (Étape 208, AR-3)
/// — recalculés à chaque passe : la `keymap` (pont
/// miroir → réel : tout nœud que le journal peut encore
/// adresser), les cibles du relayout à venir (`touched`),
/// les nœuds `<iframe>` peints (`frame_nodes`), les
/// naissances d'animation de la passe.
/// Absents **par design** : boxtree/display list/`images.*`
/// (leurs `NodeId` meurent déjà au re-parse — les lecteurs
/// tolèrent `StaleHandle`, cf. `input.rs`, et `images.*`
/// est purgée par [`PaintImages::retain_alive`]) ;
/// `focused`/`popup`/`find` (chemins `(tag, index)`, pas
/// des `NodeId`) ; `prev_styles`/`transitions`/`animations`
/// (ordinaux `usize`) ; `pending_anim_births` côté `Tab`
/// (le drain saute déjà les ids morts via `element_path`) ;
/// `frame_ctxs[].node` (tolère la mort — « nettoyé,
/// jamais déréférencé aveuglément », `types.rs`).
/// **Exhaustivité requise** : tout nouveau champ
/// `PaintPayload` porteur de `NodeId` persistants doit
/// entrer ici — un oubli = `StaleHandle` latent, visible
/// seulement à l'usage post-balayage.
fn dom_live_seeds(p: &PaintPayload, touched: &[explorer_dom::NodeId]) -> Vec<explorer_dom::NodeId> {
    let mut seeds = Vec::with_capacity(
        p.keymap.len() + touched.len() + p.frame_nodes.len() + p.anim_births.len(),
    );
    seeds.extend(p.keymap.values().copied());
    seeds.extend(touched.iter().copied());
    seeds.extend(p.frame_nodes.iter().copied());
    seeds.extend(p.anim_births.iter().map(|&(n, _)| n));
    seeds
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

/// Résumé borné d'une op de journal — le `html` d'un
/// `Insert` peut peser des Ko : on logue nature + clés +
/// tailles, jamais le contenu (diagnostic JS-C289).
/// Le breadcrumb §B-111 `journal: op` vit sous `dlv_trace`
/// (partagé `session::` — JS-C409) : la dernière ligne avant
/// un overflow nomme l'op fautive.
fn journal_op_desc(op: &explorer_js::DomMutation) -> String {
    use explorer_js::DomMutation::{AttachShadow, Insert, Remove, SetAttr, SetText};
    match op {
        Remove { key } => format!("Remove(key={key})"),
        SetAttr { key, name, value } => {
            format!(
                "SetAttr(key={key},{name}={:?})",
                value.as_deref().unwrap_or("<none>")
            )
        }
        SetText { key, text } => {
            format!(
                "SetText(key={key},len={},«{}»)",
                text.len(),
                &text[..text.len().min(24)]
            )
        }
        AttachShadow { host, root, open } => {
            format!("AttachShadow(host={host},root={root},open={open})")
        }
        Insert {
            parent,
            index,
            html,
            ids,
            tags,
        } => format!(
            "Insert(parent={parent},index={index},html={}o,ids={},tags={},head=«{}»)",
            html.len(),
            ids.len(),
            tags.len(),
            &html[..html.len().min(64)]
        ),
    }
}

/// Rejoue **une** op de journal sur le DOM commis — extrait
/// de la boucle pour nommer l'op fautive au rejet
/// (diagnostic JS-C289). Alimente `touched` des cibles
/// réelles (relayout incrémental, Étape 137a).
fn apply_journal_op(
    p: &mut PaintPayload,
    ctx: &ApplyCtx,
    op: &explorer_js::DomMutation,
    touched: &mut Vec<explorer_dom::NodeId>,
) -> Result<(), JournalReject> {
    match op {
        explorer_js::DomMutation::Remove { key } => {
            // JS-C292 — un nœud créé puis retiré **entre deux
            // drains** n'entre dans aucune sérialisation (le
            // `html`/`ids` d'un `Insert` sont capturés au drain,
            // état courant) : son `Remove` frappe une clé
            // inconnue — no-op sémantique, le moteur ne l'a
            // jamais eu. L'empreinte finale reste l'arbitre
            // d'un vrai désalignement (mur youtube.com :
            // `Remove(key=1687)` orphelin après JS-C291).
            let Ok(id) = key_node(p, *key) else {
                tracing::debug!(
                    key,
                    "journal DOM : Remove de clé inconnue toléré (nœud créé+retiré dans la fenêtre)"
                );
                return Ok(());
            };
            // Purger les descendants de la table — les
            // clés miroir meurent avec leur sous-arbre.
            let mut gone = std::collections::HashSet::new();
            let mut stack = vec![id];
            let dom = p.dom.as_mut().ok_or(JournalReject::NoDom)?;
            while let Some(n) = stack.pop() {
                gone.insert(n);
                if let Ok(ch) = dom.children_iter(n) {
                    stack.extend(ch);
                }
                if let Ok(Some(f)) = dom.template_contents(n) {
                    if let Ok(ch) = dom.children_iter(f) {
                        stack.extend(ch);
                    }
                }
                if let Some(r) = dom.shadow_root(n, true) {
                    if let Ok(ch) = dom.children_iter(r) {
                        stack.extend(ch);
                    }
                    gone.insert(r);
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
            // §B-17 — le **parent** : `+`/`~`/`:nth-*` des
            // frères dépendent aussi de l'attribut muté
            // (même règle que `SetText`/`Remove` ; un
            // `[inert]` posé ici doit exclure le
            // sous-arbre, pas seulement la boîte cible).
            touched.push(dom.parent(id).ok().flatten().unwrap_or(id));
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
            tags,
        } => {
            touched.push(journal_insert(p, ctx, *parent, *index, html, ids, tags)?);
        }
    }
    Ok(())
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

/// Rejeu `Insert` : fragment sérialisé re-parsé en
/// mini-document puis greffé sous le parent mappé ; les clés
/// miroir du sous-arbre entrent en `keymap` par **appariement
/// de tags** (JS-C129 — un nœud moteur synthétisé au
/// re-parse ou non émis par le miroir ne décale plus les
/// clés suivantes ; l'ancien zip tronqué les laissait
/// dériver → `UnmappedKey` en cascade → repli re-parse).
/// Retourne le parent (cible du relayout incrémental).
fn journal_insert(
    p: &mut PaintPayload,
    ctx: &ApplyCtx,
    parent: u64,
    index: u32,
    html: &str,
    ids: &[u64],
    tags: &[String],
) -> Result<explorer_dom::NodeId, JournalReject> {
    let par = key_node(p, parent)?;
    // Pas de parse de fragment contextuel dans
    // `explorer-html` : le fragment est parsé en
    // document minimal et ses enfants de `<body>`
    // greffés (le classifieur miroir a déjà écarté
    // les contenus à parsing contextuel).
    let frag = TreeBuilder::parse_scripting(html, ctx.scripting);
    let Some(root_el) = frag.document_element() else {
        return Err(JournalReject::Dom);
    };
    // `<body>` (ou `<html>` à défaut) = porte-greffe.
    let Ok(body) = frag.children(root_el).map(|c| {
        c.iter()
            .find(|&&n| frag.tag_name(n).is_ok_and(|t| t == "body"))
            .copied()
            .unwrap_or(root_el)
    }) else {
        return Err(JournalReject::Dom);
    };
    let to_move = frag.children(body).unwrap_or_default();
    let dom = p.dom.as_mut().ok_or(JournalReject::NoDom)?;
    let base = index as usize;
    let mut new_roots = Vec::with_capacity(to_move.len());
    for (i, &src_root) in to_move.iter().enumerate() {
        new_roots.push(graft_subtree(dom, &frag, src_root, par, base + i)?);
    }
    // JS-C291 — un insert peut véhiculer des marqueurs
    // `data-obreloc*` (structure restreinte dans le sous-arbre
    // inséré) : resplice **avant** le pré-ordre d'alignement.
    relocate_obreloc(dom);
    let mut created_all = Vec::new();
    for &r in &new_roots {
        engine_preorder_subtree(dom, r, &mut created_all);
    }
    if tags.len() == ids.len() && !ids.is_empty() {
        align_keyed(&mut p.keymap, dom, &created_all, ids, tags);
    } else {
        // Repli positionnel — moteur plus ancien que la
        // couture (journal sans tags).
        for (e, m) in created_all.into_iter().zip(ids.iter().copied()) {
            p.keymap.insert(m, e);
        }
    }
    Ok(par)
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
    dlv_info!(
        ops = journal.ops.len(),
        fp = journal.fingerprint,
        "journal: apply"
    );
    let mut touched = Vec::new();
    for (op_i, op) in journal.ops.iter().enumerate() {
        // §B-111 — breadcrumb opt-in : la dernière ligne avant
        // la mort nomme l'op fautive (gate `dlv_trace` interne).
        dlv_info!(op_i, "journal: op {}", journal_op_desc(op));
        if let Err(e) = apply_journal_op(p, ctx, op, &mut touched) {
            // Diagnostic JS-C289 — nomme l'op et la clé
            // fautive (le `html` d'un Insert peut peser des
            // Ko : résumé borné).
            tracing::warn!(
                op_i,
                op = %journal_op_desc(op),
                "journal DOM : op rejetée ({e:?})"
            );
            return Err(e);
        }
    }
    // Garde de cohérence ADR-0025 : l'empreinte structurelle
    // du DOM rejoué doit égaler celle du miroir post-journal.
    // JS-C315 — l'empreinte est **insensible à l'ordre**
    // (multiset additif) : la resplice `data-obreloc*`
    // (JS-C291) déplace les templates sans ajouter ni
    // retirer de nœud — invisible désormais, alors qu'elle
    // armait `Fingerprint` → re-parse à chaque pompe
    // (bissect nav, vol 606). Les vraies divergences
    // (nœud manquant/en trop/tag différent) rejettent
    // toujours. JS-C314 — mais uniquement quand des ops
    // ont été rejouées : sur journal vide (drain propre
    // JS-C293) rien n'a changé → rien à arbitrer ;
    // rejeter `Fingerprint` armerait la baseline keyed
    // prématurément puis re-parserait à chaque pompe
    // (la garde a été perdue dans la réécriture C315 —
    // restaurée).
    dlv_info!("journal: ops done");
    let dom = p.dom.as_ref().ok_or(JournalReject::NoDom)?;
    if !journal.ops.is_empty() && engine_fingerprint(dom, &p.keymap) != journal.fingerprint {
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
    // Étape 208 (AR-3, chantier ARÈNE) — la voie journal
    // détache sans libérer : `live`/`slots` étaient
    // monotone-croissants jusqu'à `BudgetExceeded` (audit
    // arène, constat 1). Passe amortie par watermark ×2 :
    // `seeds` = vérité de vivacité recalculée (keymap =
    // pont miroir→réel, cibles du relayout, nœuds iframe) —
    // un seed oublié échoue typé (`StaleHandle`), jamais
    // muet. Boxtree/display list exclus par design : leurs
    // lecteurs tolèrent déjà les ids morts (re-parse les
    // tue de même, `input.rs`). Les bitmaps `images.*`
    // clés `NodeId` persistent sur la voie journal — leurs
    // entrées mortes sont **purgées** (`retain_alive`) :
    // les semer retiendrait les nœuds retirés à vie.
    if p.dom
        .as_ref()
        .is_some_and(|d| d.live_count() >= p.dom_gc_mark)
    {
        let seeds = dom_live_seeds(p, &touched);
        if let Some(dom) = p.dom.as_mut() {
            let freed = dom.collect(&seeds);
            p.images.retain_alive(dom);
            p.dom_gc_mark = dom.live_count().saturating_mul(2).max(1);
            if freed > 0 {
                tracing::debug!(
                    freed,
                    live = dom.live_count(),
                    mark = p.dom_gc_mark,
                    "apply: dom collect"
                );
            }
        }
    }
    // JS-C293 — journal vide (drain propre) : l'empreinte
    // ci-dessus reste l'arbitre, mais rien n'a bougé → pas de
    // relayout (un relayout complet par pompe sinon).
    if job.relayout && !journal.ops.is_empty() {
        journal_relayout(p, ctx, job, &touched, &mut out)?;
    } else {
        // §B-17 — même trou que la voie re-parse : les
        // sous-arbres touchés ne seront pas restylés, leurs
        // chemins partent en dette pour le prochain apply
        // couvrant (journal ou cléé — les deux drainent).
        let mut paths = Vec::with_capacity(touched.len());
        if let Some(dom) = p.dom.as_ref() {
            for &n in &touched {
                if let Some(path) = element_path(dom, n) {
                    paths.push(path);
                }
            }
        }
        p.style_dirty.extend(paths);
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
    tab: TabId,
    dom: &Dom,
    target: explorer_dom::NodeId,
    canvas_ops: &CanvasOpsTable,
) -> Result<(), CoreError> {
    if try_incr_payload(p, ctx, tab, Some(dom), dom, target, target, canvas_ops) {
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
    // §B-17 — la dette de restyle des synchros `relayout=
    // false` (voie re-parse) fusionne à la couverture
    // journal : un chemin non résolu replie sur la racine
    // (couverture conservatrice, jamais de dette perdue).
    let debt = std::mem::take(&mut p.style_dirty);
    let mut touched_all;
    let touched: &[explorer_dom::NodeId] = if debt.is_empty() {
        touched
    } else {
        touched_all = touched.to_vec();
        for path in &debt {
            touched_all.push(dom_path_node(&dom, path).unwrap_or_else(|| dom.document()));
        }
        &touched_all
    };
    dlv_info!(touched = touched.len(), "journal: relayout");
    let incremental = journal_cover(&dom, touched).is_some_and(|t| {
        try_incr_payload(p, ctx, job.tab, Some(&dom), &dom, t, t, &job.canvas_ops)
    });
    if !incremental && relayout_dom_payload(p, ctx, &dom, &job.canvas_ops).is_err() {
        p.dom = Some(dom);
        p.style_dirty = debt;
        return Err(JournalReject::Dom);
    }
    dlv_info!(incremental, "journal: relayout done");
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
#[allow(clippy::too_many_arguments)] // cible/canvas : même paquetage que `relayout_dom_payload`.
fn try_incr_payload(
    p: &mut PaintPayload,
    ctx: &ApplyCtx,
    tab: TabId,
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
    let mut t = Instant::now();
    if (p.dom.is_none() && old_dom_opt.is_none()) || p.tree.is_none() {
        return false;
    }
    let Some(root_el) = new_dom.document_element() else {
        return false;
    };
    dlv_info!(target = ?target_new, "journal: incr");
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
    p.images.background_layers.clear();
    p.images.masks_url.clear();
    net::reinsert_imgs_by_src(
        new_dom,
        &mut p.images,
        &p.img_decoded,
        &viewport_env(ctx.viewport, ctx.color_scheme, ctx.scripting, ctx.scale),
    );
    t = sync_phase(tab, "incr.rekey", t);
    // Étape 228 — restyle scopé à `parent(target)` (voie
    // journal in-place seule — `old_dom_opt` présent = même
    // objet `Dom` ancien/nouveau ; `None` = re-parse, ids
    // nouveaux → re-cascade complète). Le parent couvre les
    // combinators `~`/`+`/`:nth-*` sur les frères, `:empty`
    // et les descendants de la cible ; `:has(` et racine
    // morte replient en interne. `eng` inutilisé en aval :
    // la passe 2 `@container` est bornée à
    // `relayout_dom_payload` (demande PIPE journalisée — la
    // voie incrémentale garde les règles inertes) → il est
    // re-persisté tout de suite pour la passe suivante.
    let scope_root = old_dom_opt.and_then(|_| new_dom.parent(target_new).ok().flatten());
    let (styles, eng) = styles_payload(
        p,
        new_dom,
        ctx.viewport,
        ctx.color_scheme,
        ctx.scripting,
        ctx.scale,
        scope_root,
    );
    p.style_eng = Some(eng);
    t = sync_phase(tab, "incr.style", t);
    let data_imgs = collect_data_images(new_dom, &styles, ctx.image_max_px);
    p.images.replaced.extend(data_imgs.replaced);
    p.images.backgrounds.extend(data_imgs.backgrounds);
    // R-203 — couches `data:` indexées comme les http (même
    // contrat `emit_bg_layers` : couche puis repli).
    for (n, v) in data_imgs.background_layers {
        let e = p.images.background_layers.entry(n).or_default();
        for (idx, img) in v {
            e.retain(|(i, _)| *i != idx);
            e.push((idx, img));
        }
    }
    // `masks_url` n'est pas indexée `NodeId` : les masques
    // `data:` sont ré-étendus à chaque apply comme les
    // fetchés sont ré-enregistrés via `mask_aliases` — sans
    // ce merge la table (RAZ `PaintImages::default()`)
    // rendait tout `mask-image` aveugle en session.
    p.images.masks_url.extend(data_imgs.masks_url);
    insert_fetched_images(p, &styles, ctx.image_max_px, ctx.image_max_bytes);
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
    // R-133 : les sources XML suivent les bitmaps — la
    // display list re-rasterise à la taille utilisée quand
    // la boîte dépasse le naturel (§B-32/§B-36).
    let (svgs, svg_sources) =
        explorer_render::svg::collect_svg_styled(new_dom, root_el, &styles, ctx.image_max_px);
    let block = layout_ctx(ctx, p);
    // Reliquat R-88 : `relayout_subtree` ne pose pas
    // `root_font_size`, contrairement à `layout_tree` —
    // tout `rem` du sous-arbre relayouté retombait sur
    // l'initiale à la première synchro incrémentale.
    // Posée ici depuis la cascade fraîche, même base.
    block
        .root_font_size
        .set(root_font_size_px(new_dom, &styles));
    t = sync_phase(tab, "incr.assets", t);
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
        t = sync_phase(tab, "incr.relayout", t);
        if let Some(h) = res {
            h
        } else {
            // La map fusionnée reste valide pour la
            // passe suivante — la re-persister avant
            // le repli reflow complet.
            p.styles = Some(styles);
            return false;
        }
    };
    // Map fusionnée persistée (étape 228) — base du
    // prochain restyle scopé sur cette voie journal.
    p.styles = Some(styles);
    p.doc_h = doc_h;
    p.images.svgs = svgs;
    p.images.svg_sources = svg_sources;
    prune_paint_images(p, ctx.image_max_bytes);
    p.images.canvases = canvas_table_from_ops(canvas_ops, new_dom);
    // Sous-contextes `<iframe>` (étape 129) : bitmaps
    // re-blittés sur les boîtes du nouvel arbre — les
    // surfaces `frame_bitmaps` clées `(url,w,h)` rendent
    // la passe gratuite si les cadres n'ont pas bougé.
    composite::composite_frames(p, ctx, new_dom);
    let ok = repaint_payload(p, ctx, None).is_ok();
    sync_phase(tab, "incr.repaint", t);
    ok
}

/// `StyleEnv` dérivé du viewport session (f32 → px entiers,
/// saturé à `[0, u32::MAX]` — un viewport invalide ne peut
/// pas produire un env négatif ou absurdement grand), du
/// schéma de couleurs réglé (étape 97 — `prefers-color-scheme`
/// des media queries et défauts UA sombres, R-59), du
/// « scripting flag » du contexte (R-93 — MQ `(scripting)` +
/// `<noscript>` masqué UA : sans filage, le fallback rawtext
/// peignait en clair sous un moteur installé — issue #22) et
/// du rapport pixel réel `dpr` (`Session::scale` /
/// `ApplyCtx.scale` — R-195 prérequis : MQ `resolution` et
/// densité `srcset` lisaient `1.0` à vie en `HiDPI` ; un `dpr`
/// invalide retombe sur `1.0`, jamais 0 ni négatif).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
// px CSS ≪ 2^32 en pratique — `as` sature déjà.
pub(crate) fn viewport_env(
    viewport: (f32, f32),
    color_scheme: explorer_css::ColorScheme,
    scripting: bool,
    dpr: f32,
) -> StyleEnv {
    StyleEnv {
        width_px: viewport.0.max(0.0).round() as u32,
        height_px: viewport.1.max(0.0).round() as u32,
        color_scheme,
        scripting,
        dpr: if dpr.is_finite() && dpr > 0.0 {
            dpr
        } else {
            1.0
        },
        ..StyleEnv::default()
    }
}

/// Feuille déjà parsée dans le cache texte→`Stylesheet`
/// (étape 227). `None` = cache plein et texte absent —
/// l'appelant re-parse à l'usage sans stocker.
fn sheet_parsed<'a>(
    cache: &'a mut HashMap<String, Stylesheet>,
    css: &str,
) -> Option<&'a Stylesheet> {
    if !cache.contains_key(css) && cache.len() < types::MAX_SHEET_CACHE {
        cache.insert(css.to_string(), parse_stylesheet(css));
    }
    cache.get(css)
}

/// `true` si un prélude sélecteur porte `:has(` — le restyle
/// scopé est alors interdit (étape 228) : un nœud hors du
/// sous-arbre sale peut dépendre de la mutation. Le prélude
/// est la chaîne brute — un `:has(` en argument
/// d'attribut/pseudo faussement positif retombe sur la
/// cascade complète (conservateur, jamais de faux négatif).
fn prelude_has_relational(prelude: &str) -> bool {
    prelude.contains(":has(")
}

/// Récursion `Rule`/`AtRule` — les blocs `@media`/
/// `@supports`/`@container`/`@layer`/`@scope`/`@starting-
/// style` portent leurs règles imbriquées.
fn rules_have_relational(rules: &[explorer_css::Rule]) -> bool {
    rules.iter().any(|r| match r {
        explorer_css::Rule::Qualified(q) => prelude_has_relational(&q.prelude),
        explorer_css::Rule::At(a) => {
            let nested: &Vec<explorer_css::Rule> = match &a.kind {
                explorer_css::AtKind::Container { rules, .. }
                | explorer_css::AtKind::Media { rules, .. }
                | explorer_css::AtKind::Layer { rules, .. }
                | explorer_css::AtKind::Scope { rules }
                | explorer_css::AtKind::StartingStyle { rules } => rules,
                _ => return false,
            };
            rules_have_relational(nested)
        }
    })
}

/// `:has(` quelque part dans la feuille (règles top-level
/// et imbriquées).
fn sheet_has_relational(sheet: &Stylesheet) -> bool {
    rules_have_relational(&sheet.rules)
}

/// `StyleEngine::add_stylesheet` via le cache de la
/// payload — saute la tokenisation+parse des textes déjà
/// vus (la collecte des règles dans `eng` reste par
/// passe : elle porte l'ordre de cascade). Retourne le
/// drapeau `:has(` de la feuille (étape 228).
fn add_sheet(
    eng: &mut StyleEngine,
    cache: &mut HashMap<String, Stylesheet>,
    css: &str,
    origin: Origin,
) -> bool {
    if let Some(s) = sheet_parsed(cache, css) {
        eng.add_stylesheet(s, origin);
        sheet_has_relational(s)
    } else {
        let s = parse_stylesheet(css);
        let rel = sheet_has_relational(&s);
        eng.add_stylesheet(&s, origin);
        rel
    }
}

/// Variante confinée shadow (R-96) de [`add_sheet`].
fn add_sheet_scoped(
    eng: &mut StyleEngine,
    cache: &mut HashMap<String, Stylesheet>,
    css: &str,
    origin: Origin,
    scope: explorer_dom::NodeId,
) -> bool {
    if let Some(s) = sheet_parsed(cache, css) {
        eng.add_scoped_stylesheet(s, origin, scope);
        sheet_has_relational(s)
    } else {
        let s = parse_stylesheet(css);
        let rel = sheet_has_relational(&s);
        eng.add_scoped_stylesheet(&s, origin, scope);
        rel
    }
}

/// Signature du jeu de feuilles d'une passe (étape 228) :
/// UA + `<style>` par (scope, texte substitué) + `linked` +
/// `adopted`, dans l'ordre de cascade — `style_eng` est
/// réutilisable ssi cette signature tient.
fn sheets_sig(
    sheets: &[(Option<explorer_dom::NodeId>, String)],
    linked: &[String],
    adopted: &[String],
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    MEDIA_UA_CSS.hash(&mut h);
    for (scope, css) in sheets {
        scope.hash(&mut h);
        css.hash(&mut h);
    }
    for css in linked.iter().chain(adopted) {
        css.hash(&mut h);
    }
    h.finish()
}

/// Cascade + transitions/animations sur payload — la passe
/// de styles partagée par reflow complet et relayout
/// incrémental. Écrit `prev_styles`/`transitions`/
/// `animations` de la payload.
///
/// Étape 228 — `scope` = racine de restyle scopé (voie
/// journal in-place : `parent` de la cible — couvre les
/// combinators `~`/`+`/`:nth-*` sur les frères et le
/// `:empty` du parent ; seul `:has(` force le repli). La
/// map persistée `p.styles` sert de base : le restyle
/// écrit seulement le sous-arbre (`apply_scoped`), hors
/// portée les entrées survivent. Replis cascade complète :
/// map/env absents ou changés, `:has(` dans les feuilles,
/// racine morte.
pub(crate) fn styles_payload(
    p: &mut PaintPayload,
    dom: &Dom,
    viewport: (f32, f32),
    color_scheme: explorer_css::ColorScheme,
    scripting: bool,
    dpr: f32,
    scope: Option<explorer_dom::NodeId>,
) -> (StyleMap, StyleEngine) {
    // Collecte + substitution `inline_css_expanded` en une
    // passe (étape 218 — le DOM a pu muter depuis la
    // collecte ; substitution par contenu, robuste aux
    // `<style>` insérés par JS) : la signature et les
    // ajouts au moteur portent le même texte.
    let sheets: Vec<(Option<explorer_dom::NodeId>, String)> = collect_style_sheets(dom, &p.csp)
        .into_iter()
        .map(|(s, css)| (s, p.inline_css_expanded.get(&css).cloned().unwrap_or(css)))
        .collect();
    let sig = sheets_sig(&sheets, &p.linked_sheets, &p.adopted_sheets);
    // `style_eng` réutilisé tant que le jeu de feuilles est
    // identique — saute la collecte des règles entière en
    // hit (ordre de cascade), pas seulement le parse
    // (étape 227 garde `sheet_cache` pour la reconstruction).
    let eng = match p.style_eng.take() {
        Some(e) if p.style_sig == Some(sig) => e,
        _ => {
            let mut e = StyleEngine::new();
            let mut rel = false;
            let cache = &mut p.sheet_cache;
            rel |= add_sheet(&mut e, cache, MEDIA_UA_CSS, Origin::UserAgent);
            // R-96 — chaque `<style>` se charge dans son
            // arbre : `scope = Some(racine)` → feuille
            // confinée shadow, `None` → feuille document.
            for (s, css) in &sheets {
                rel |= match s {
                    Some(root) => add_sheet_scoped(&mut e, cache, css, Origin::Author, *root),
                    None => add_sheet(&mut e, cache, css, Origin::Author),
                };
            }
            for css in p.linked_sheets.iter().chain(&p.adopted_sheets) {
                rel |= add_sheet(&mut e, cache, css, Origin::Author);
            }
            p.style_sig = Some(sig);
            p.eng_has_relational = rel;
            e
        }
    };
    // Media queries évaluées contre le viewport RÉEL de la
    // session (demande PIPE→NAV : `max-width:1007` doit être
    // vrai à 800px, pas lue sur le 1024×768 par défaut) et
    // le schéma de couleurs réglé (étape 97).
    let env = viewport_env(viewport, color_scheme, scripting, dpr);
    // Restyle scopé (étape 228) : la map persistée est la
    // base ; `compute_scoped` ne réécrit que le sous-arbre
    // de `scope` (parent de la cible journal — couvre les
    // combinators `~`/`+`/`:nth-*` sur les frères, le
    // `:empty` du parent et les descendants). Replis
    // cascade complète : map absente, `StyleEnv` changé,
    // `:has(` dans les feuilles (portée non bornable) ou
    // racine morte (cible retirée par le journal).
    let scoped_ok = scope.is_some()
        && !p.eng_has_relational
        && p.styles_env.as_ref().is_some_and(|e| *e == env)
        && scope.is_some_and(|r| dom.parent(r).is_ok());
    let mut styles = match (scope, p.styles.take()) {
        (Some(root), Some(mut map)) if scoped_ok => {
            let seed = dom.parent(root).ok().flatten().and_then(|par| map.get(par));
            let scoped = eng.compute_scoped(dom, root, &env, seed);
            map.apply_scoped(scoped);
            map
        }
        _ => eng.compute_tree(dom, dom.document(), &env),
    };
    let ord_of = element_ordinals(dom);
    let raw: HashMap<usize, ComputedStyle> = styles
        .iter()
        .filter_map(|(n, c)| ord_of.get(&n).map(|o| (*o, c.clone())))
        .collect();
    let now = Instant::now();
    update_transitions(&p.prev_styles, &mut p.transitions, &styles, &ord_of, now);
    let births = sync_animations(&mut p.animations, &styles, &ord_of, &eng, now);
    if p.anim_births.len() < types::MAX_ANIM_BIRTHS {
        p.anim_births.extend(births);
        p.anim_births.truncate(types::MAX_ANIM_BIRTHS);
    }
    apply_anim_overrides(&p.transitions, &mut p.animations, &mut styles, &ord_of, now);
    p.prev_styles = raw;
    p.styles_env = Some(env);
    (styles, eng)
}

/// `relayout_dom` sur payload — style → boîtes → layout →
/// display list → raster. `canvas_ops` = op-lists du miroir
/// (capturées sous verrou — la payload n'a pas `entry.js`).
#[allow(clippy::too_many_lines)] // passe 2 @container — séquentiel, pas de découpe naturelle.
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
    dlv_info!("apply: relayout");
    // `scope = None` : passe complète — la map produite
    // devient la base persistée des restyles scopés de la
    // voie journal (étape 228).
    let (mut styles, eng) = styles_payload(
        p,
        dom,
        ctx.viewport,
        ctx.color_scheme,
        ctx.scripting,
        ctx.scale,
        None,
    );
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
    // R-203 — couches `data:` indexées comme les http (même
    // contrat `emit_bg_layers` : couche puis repli).
    for (n, v) in data_imgs.background_layers {
        let e = p.images.background_layers.entry(n).or_default();
        for (idx, img) in v {
            e.retain(|(i, _)| *i != idx);
            e.push((idx, img));
        }
    }
    // Même merge `masks_url` que la voie incrémentale (voir
    // plus haut) : masques `data:` sinon invisibles en
    // session depuis R-52.
    p.images.masks_url.extend(data_imgs.masks_url);
    insert_fetched_images(p, &styles, ctx.image_max_px, ctx.image_max_bytes);
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
    // R-133 : sources XML conservées pour le re-raster à
    // taille utilisée dans `paint_replaced` (§B-32/§B-36).
    let (mut svgs, mut svg_sources) =
        explorer_render::svg::collect_svg_styled(dom, root_el, &styles, ctx.image_max_px);
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
    // R-190 — passe 2 `@container` (demande PIPE journalisée,
    // branchement NAV) : les règles taguées `container:` sont
    // inertes en passe 1 ; le layout vient de poser les
    // conteneurs — mesurer, re-calculer le style avec le ctx,
    // puis rebuild + re-layout **une seule fois**. Pas
    // d'itération (queries `min|max-width` stables ;
    // oscillation height documentée au journal). Rebuild
    // inconditionnel dès qu'un conteneur mesuré existe : les
    // styles de `::before`/`::after` ne sont pas comparables
    // depuis NAV (`StyleMap` n'expose pas leur itérateur) —
    // une passe bornée plutôt qu'un changement de pseudo
    // raté. Transitions sur bascule `@container` : absentes
    // (le snapshot `prev_styles` reste celui de la passe 1 —
    // borne documentée).
    if !eng.container_queries.is_empty() {
        let measures = explorer_layout::measure_containers(&bx);
        if !measures.is_empty() {
            let env = viewport_env(ctx.viewport, ctx.color_scheme, ctx.scripting, ctx.scale);
            let mut styles2 =
                eng.compute_tree_with_ctx(dom, dom.document(), &env, &|_| false, Some(&measures));
            // Style brut : les overrides d'animation doivent
            // être ré-appliqués (journal R-190 — sinon les
            // nœuds retagués perdent l'effet courant).
            let ord_of = element_ordinals(dom);
            apply_anim_overrides(
                &p.transitions,
                &mut p.animations,
                &mut styles2,
                &ord_of,
                Instant::now(),
            );
            // Re-collecte identique : une règle qui bascule
            // `background-image`/`content` charge son asset.
            let data_imgs = collect_data_images(dom, &styles2, ctx.image_max_px);
            p.images.replaced.extend(data_imgs.replaced);
            p.images.backgrounds.extend(data_imgs.backgrounds);
            // R-203 — couches `data:` indexées comme les http
            // (même contrat `emit_bg_layers` : couche puis repli).
            for (n, v) in data_imgs.background_layers {
                let e = p.images.background_layers.entry(n).or_default();
                for (idx, img) in v {
                    e.retain(|(i, _)| *i != idx);
                    e.push((idx, img));
                }
            }
            p.images.masks_url.extend(data_imgs.masks_url);
            insert_fetched_images(p, &styles2, ctx.image_max_px, ctx.image_max_bytes);
            let mut sizes = image_sizes(dom, &p.images.replaced);
            sizes.extend(explorer_render::svg::svg_sizes(dom, root_el));
            for n in composite::iframe_nodes(dom) {
                sizes
                    .entry(n)
                    .or_insert_with(|| composite::iframe_intrinsic(dom, n));
            }
            let (svgs2, svg_sources2) =
                explorer_render::svg::collect_svg_styled(dom, root_el, &styles2, ctx.image_max_px);
            if let Some(mut bx2) = build_box_tree_sized(dom, root_el, &styles2, &sizes) {
                layout_tree(&mut bx2, vw, &block);
                tracing::info!(
                    ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX),
                    "apply: layout p2 @container"
                );
                svgs = svgs2;
                svg_sources = svg_sources2;
                // La map persistée porte les bascules
                // `@container` — sinon les restyles
                // scopés suivants compareraient à une
                // base passe-1.
                styles = styles2;
                bx = bx2;
            }
        }
    }
    // Étape 228 — map + moteur persistés : base du restyle
    // scopé de la prochaine voie journal (mêmes feuilles →
    // `style_eng` réutilisé, parse ET collecte sautés).
    p.styles = Some(styles);
    p.style_eng = Some(eng);
    restitute_font_cache(p, &block);
    // Hauteur défilante ≠ hauteur de boîte : le contenu qui
    // déborde un ancêtre à hauteur spécifiée (`height:100%` de
    // vector-2022) reste scrollable — R-82.
    p.doc_h = bx.rect.height.max(scrollable_overflow_bottom(&bx));
    p.tree = Some(bx);
    p.tree_epoch = p.tree_epoch.wrapping_add(1);
    p.images.svgs = svgs;
    p.images.svg_sources = svg_sources;
    prune_paint_images(p, ctx.image_max_bytes);
    // `<canvas>` 2D (JS-C28) : op-lists cumulées du miroir,
    // re-résolues par chemin d'élément dans `dom`.
    p.images.canvases = canvas_table_from_ops(canvas_ops, dom);
    // Sous-contextes `<iframe>` (étape 129) : chaque
    // `SubFrame` fetchée est rasterisée dans sa surface
    // (viewport = content box de sa boîte) et blittée en
    // `images.replaced` — `paint_replaced` émet l'`Image`
    // à la peinture qui suit.
    tracing::info!("apply: composite_frames");
    composite::composite_frames(p, ctx, dom);
    tracing::info!("apply: repaint_payload");
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
#[allow(clippy::too_many_lines)]
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
    // §B-118 / R-199 — sonde `EXPLORER_EMIT_STATS=1` : variante
    // stats (compteurs boîte→item + chemins des ancêtres
    // fautifs) — la classe fautive se nomme en une lecture.
    let stats_on = std::env::var_os("EXPLORER_EMIT_STATS").is_some();
    let (mut list, emit_stats) = if stats_on {
        let (l, st) = explorer_render::build_display_list_incremental_stats(
            bx,
            vw,
            vh,
            (0.0, p.scroll_y),
            &p.images,
            p.list.as_ref(),
            p.tree_epoch,
        );
        (l, Some(st))
    } else {
        (
            explorer_render::build_display_list_incremental(
                bx,
                vw,
                vh,
                (0.0, p.scroll_y),
                &p.images,
                p.list.as_ref(),
                p.tree_epoch,
            ),
            None,
        )
    };
    if let Some(st) = &emit_stats {
        tracing::info!(
            target: "paint_stats",
            boxes = st.boxes,
            items = st.items,
            by_kind = ?st.by_kind,
            off_canvas = st.off_canvas,
            opacity0 = st.drop_opacity0,
            backface = st.drop_backface,
            visibility = st.skip_visibility,
            deferred = st.deferred_emitted,
            cap = st.drop_cap,
            spans = st.spans_copied,
            opacity0_path = ?st.opacity0_path,
            backface_path = ?st.backface_path,
            visibility_path = ?st.visibility_path,
            tf_offsets = ?st.tf_offsets,
            off_first = ?st.off_canvas_first,
            off_tf = ?st.off_canvas_tf,
            glyphs = st.glyphs,
            invisible = st.invisible_glyphs,
            "apply: emit_stats"
        );
        // §B-118 — la liste complète au debug : rect/couleur/
        // clip de chaque item (145 items ≈ autant de lignes).
        tracing::info!(target: "paint_stats", "apply: display_dump\n{}", list.dump());
    }
    tracing::info!(items = list.items.len(), "apply: display_list");
    if list.dropped_emits > 0 {
        // WARN une fois par chemin fautif (la borne absorbe à
        // chaque apply — vol 67 : 994 WARN identiques), `debug`
        // ensuite ; `dropped_emits` cumulé reste compté dans
        // la liste pour la télémétrie.
        let fresh = list
            .first_cycle_path
            .as_ref()
            .is_none_or(|path| p.cycle_warned.insert(path.clone().into_boxed_slice()));
        if fresh {
            tracing::warn!(
                dropped = list.dropped_emits,
                path = ?list.first_cycle_path,
                "apply: cycle de report cassé (§B-26)"
            );
        } else {
            tracing::debug!(
                dropped = list.dropped_emits,
                path = ?list.first_cycle_path,
                "apply: cycle de report cassé (§B-26)"
            );
        }
    }
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
        list.items.push(DisplayItem::FillRect {
            rect,
            color,
            radii: [0.0; 4],
            corners: [1.0; 4],
        });
    }
    // Étape 180 (P4) — le framebuffer ne couvre plus que la
    // tranche viewport : tout document plus haut que
    // `vh.max(MIN_FULL_DOC_CSS)` (px CSS) passe en raster
    // fenêtré — coût de repaint borné O(vh) au lieu de
    // O(doc_h), frame ~5 Mio constante au lieu de ~50+ Mio
    // sur pages hautes (repaint 16 s guardian, PNG >55 Mo
    // inencodable). Sous le cap le chemin plein-doc + diff
    // dirty reste (petites pages, mutations localisées).
    if list.height > vh.max(MIN_FULL_DOC_CSS) {
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
    tracing::info!(regions = regions.len(), "apply: raster");
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
    p.windowed = false;
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
    p.windowed = true;
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
            list.items.push(DisplayItem::FillRect {
                rect,
                color,
                radii: [0.0; 4],
                corners: [1.0; 4],
            });
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
            image_max_bytes: self.image_max_bytes,
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
        // §B-17 — l'apply évincé emportait sa couverture
        // `dirty` : des mutations purement structurelles
        // (insertions sans changement d'attribut) n'étaient
        // jamais re-posées. Le `dirty` survit en dette —
        // le prochain relayout le fusionne à sa cible.
        self.pending_applies.retain(|p| {
            if p.tab != tab {
                return true;
            }
            if let Some(d) = &p.dirty {
                entry.style_dirty.push(d.clone());
            }
            false
        });
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
        // §B-4a : un journal est une **séquence ordonnée de
        // deltas**, pas un snapshot — évincer un job journal en
        // file (comme le `retain` de `queue_apply_keyed`) perd
        // ses ops à jamais : le `dirty` miroir est déjà consommé
        // au drain, rien ne resynchronise → divergence
        // permanente miroir/DOM (rAF livré au miroir, jamais au
        // DOM commis — « avantTI » vs « avantTIR »). Le job en
        // queue, s'il est du même onglet et déjà journal, est
        // prolongé (les ops rejouent dans l'ordre ; l'empreinte
        // la plus récente fait foi — même règle que
        // `freshen_apply`). Un keyed en file avant subsume le
        // passé : le journal s'ajoute après, ordre respecté.
        if let Some(back) = self.pending_applies.back_mut() {
            if back.tab == tab {
                if let Some(j) = &mut back.journal {
                    j.ops.extend(journal.ops);
                    j.fingerprint = journal.fingerprint;
                    return;
                }
            }
        }
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
        // B-52 : une série de rejets `UnmappedKey` fait payer
        // la tentative journal à chaque cycle (rafale ~1×/5 s
        // sans wedge — churn). Au-delà de 8 consécutifs, la
        // voie journal est sautée : re-parse direct, comme si
        // le journal n'existait pas — la série retombe à un
        // succès journal (`commit_apply_detached`).
        if job.journal.is_some() && self.unmapped_streaks.get(&tab).copied().unwrap_or(0) >= 8 {
            job.journal = None;
            job.html.clear();
            job.ids.clear();
            job.tags.clear();
        }
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
            // §B-111 — `pump:` pré-apply (morgue C406) — même
            // fenêtre silencieuse que `run_apply` côté voie
            // détachée : reclaim → ctx → freshen → take_paint.
            dlv_info!("pump: take tab={}", job.tab);
            self.reclaim_orphaned_paint(job.tab);
            dlv_info!("pump: reclaim tab={}", job.tab);
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
            dlv_info!("pump: freshen tab={}", job.tab);
            self.freshen_apply(job.tab, &mut job);
            // B-25 (caf) : un job baseline sans `html` —
            // rejet de journal re-enfilé dont la resync n'a
            // rien livré (moteur mort, stall, dirty absent).
            // Un apply « vide » commettrait un document vide
            // qui détruit la payload vivante : `/dom` 404
            // permanent. On jette le job plutôt que la page.
            if job.journal.is_none() && job.html.is_empty() {
                tracing::warn!(
                    tab = job.tab,
                    "baseline apply sans html livré — job jeté (resync vide)"
                );
                continue;
            }
            let entry = self.tabs.get_mut(&job.tab)?;
            entry.paint_detached = true;
            dlv_info!("pump: paint tab={}", job.tab);
            let p = entry.take_paint();
            let g = DetachedPaint::new(
                std::sync::Arc::clone(&self.orphaned_paints),
                job.tab,
                job.epoch,
                p,
            );
            self.applies_started += 1;
            self.paint_in_flight
                .insert(job.tab, std::time::Instant::now());
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
        o: &ApplyOutcome,
    ) {
        self.applies_fin += 1;
        self.paint_in_flight.remove(&tab);
        self.commit_apply_detached_inner(tab, epoch, payload, o);
    }

    fn commit_apply_detached_inner(
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

    /// Réinsère la payload d'un tick anim détaché (Étape
    /// 3c — pendant allégé de [`Self::commit_apply_detached`],
    /// sans `ApplyOutcome`) : `produced` = le worker a mené
    /// relayout+repaint à terme → `FrameInvalidated` émis.
    /// Garde epoch : une navigation supplantée jette la
    /// passe (sa propre peinture arrive). Un scroll survenu
    /// pendant le worker déclenche le repaint de
    /// repositionnement (`p.scroll_y` date du take ; le
    /// `scroll_y` de `Tab` fait foi — même règle que
    /// l'apply détaché).
    pub(crate) fn commit_anim_tick(
        &mut self,
        tab: TabId,
        epoch: u64,
        payload: PaintPayload,
        produced: bool,
    ) {
        self.paint_in_flight.remove(&tab);
        self.reclaim_orphaned_paint(tab);
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return; // onglet fermé pendant le worker
        };
        if !entry.paint_detached {
            // Drapeau déjà libéré (réarmement d'urgence) :
            // la payload est parquée plutôt que jetée —
            // la prochaine réclamation la réinsère si
            // encore vivante (issue #23).
            let _ = self
                .orphaned_paints
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(tab, (epoch, payload));
            return;
        }
        entry.paint_detached = false;
        if entry.nav_epoch == epoch {
            let scrolled = (entry.scroll_y - payload.scroll_y).abs() > f32::EPSILON;
            entry.put_paint(payload);
            if produced {
                self.emit_invalidated(tab);
            }
            if scrolled && self.repaint(tab).is_ok() {
                self.emit_invalidated(tab);
            }
            // Étape 91 : les entrées différées pendant la
            // fenêtre détachée rejouent une fois `dom`/`tree`
            // réinsérés (focus/édition restaurés).
            self.drain_deferred_inputs(tab);
        }
        // Epoch changée : supplantée — la nouvelle nav porte
        // sa propre peinture (même règle que l'apply détaché).
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
        self.applies_fin += 1;
        self.paint_in_flight.remove(&tab);
        self.journal_fallbacks += 1;
        if matches!(why, JournalReject::UnmappedKey) {
            let n = self.unmapped_streaks.entry(tab).or_insert(0);
            *n += 1;
            if *n == 8 {
                tracing::warn!(
                    tab,
                    "série UnmappedKey — voie journal sautée pour cet onglet (B-52)"
                );
            }
        } else {
            self.unmapped_streaks.insert(tab, 0);
        }
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
        self.applies_fin += 1;
        self.paint_in_flight.remove(&tab);
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
        // §B-111 — `pump:` pré-`apply: début` (morgue C406) :
        // la fenêtre silencieuse = reclaim (drop d'une
        // PaintPayload orpheline) + apply_ctx ; `apply: début`
        // n'émettant jamais, la mort vit ici.
        dlv_info!("pump: rapply tab={tab}");
        self.reclaim_orphaned_paint(tab);
        dlv_info!("pump: reclaim tab={tab}");
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
        dlv_info!("pump: freshen tab={tab}");
        self.freshen_apply(tab, job);
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        let old_title = entry.title.clone();
        let epoch = job.epoch;
        dlv_info!("pump: paint tab={tab}");
        let mut p = entry.take_paint();
        // ADR-0025 : rejeu du journal en place — `NodeId`
        // stables, zéro re-parse. Tout rejet bascule sur la
        // sérialisation complète (garde-fou auto-réparant :
        // le job est re-enfilé en mode parse, `freshen_apply`
        // tirera la baseline cléée la plus fraîche).
        match run_apply_job(&mut p, &ctx, job, old_title.as_deref()) {
            Ok(out) => {
                // JS-C316 — `journal_applies` = rejouées
                // **réelles** : les journaux vides (drain
                // propre C293) ne comptent pas — sinon la
                // métrique inonde à ~1/pompe sur page saine.
                if job.journal.as_ref().is_some_and(|j| !j.ops.is_empty()) {
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
        // `<img loading="lazy">` (étape 229) — les applies
        // viennent de poser/déplacer des boîtes : promouvoir
        // ce qui a rejoint le voisinage du viewport.
        self.promote_lazy_all();
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
/// détachée sont insérés dans `images.backgrounds`,
/// `images.background_layers` (R-203 — chaque couche
/// `url()` sous son index) et `images.masks_url`. Pour les
/// fonds, le `NodeId` propriétaire est retrouvé par les
/// `url()` de la valeur `background-image` calculée — via
/// `bg_aliases` (texte `url(...)` brut → URL résolue ; la
/// feuille d'origine ne survit pas à la cascade) et
/// `background_layer_urls` (énumération par index de
/// couche). Pour les masques, la clé est le
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
/// Budget octets global sur les bitmaps décodés retenus par
/// onglet (§B-59, `SessionConfig::image_max_bytes`) : un même
/// bitmap peut être partagé (`Arc`) entre `images.*` (peint),
/// `img_decoded`/`bg_decoded` (caches par clé) et
/// `masks_url` — l'éviction retire le bitmap de **toutes**
/// les tables, sinon l'`Arc` résiduel le garde en vie. Les
/// bitmaps les plus tardifs du document partent en premier
/// (le haut de page, visible, reste) ; les entrées de cache
/// non peintes (nœud supprimé par mutation) sont évincées
/// avant toute image affichée. Les corps compressés
/// `img_bodies` survivent — un apply ultérieur peut
/// re-décoder ; le nœud évincé peint son repli `alt`,
/// jamais de croissance hors budget.
#[allow(clippy::too_many_lines)] // motif maison : la collecte par table reste linéaire et lisible en un bloc.
fn prune_paint_images(p: &mut PaintPayload, max_bytes: usize) {
    #[derive(Default)]
    struct Ent {
        bytes: usize,
        /// Plus petit ordinal de nœud porteur — `usize::MAX`
        /// = orphelin de cache, évincé en premier.
        idx: usize,
        replaced: Vec<explorer_dom::NodeId>,
        backgrounds: Vec<explorer_dom::NodeId>,
        bg_layers: Vec<explorer_dom::NodeId>,
        svgs: Vec<explorer_dom::NodeId>,
        masks: Vec<explorer_dom::NodeId>,
        mask_urls: Vec<String>,
        img_keys: Vec<String>,
        bg_keys: Vec<String>,
    }
    fn slot<'a>(by_ptr: &'a mut HashMap<usize, Ent>, img: &Arc<DecodedImage>) -> &'a mut Ent {
        by_ptr
            .entry(Arc::as_ptr(img) as usize)
            .or_insert_with(|| Ent {
                idx: usize::MAX,
                ..Ent::default()
            })
    }
    let mut by_ptr: HashMap<usize, Ent> = HashMap::new();
    for (n, img) in &p.images.replaced {
        let e = slot(&mut by_ptr, img);
        e.bytes = img.rgba.len();
        e.idx = e.idx.min(n.index() as usize);
        e.replaced.push(*n);
    }
    for (n, img) in &p.images.backgrounds {
        let e = slot(&mut by_ptr, img);
        e.bytes = img.rgba.len();
        e.idx = e.idx.min(n.index() as usize);
        e.backgrounds.push(*n);
    }
    for (n, layers) in &p.images.background_layers {
        for (_, img) in layers {
            let e = slot(&mut by_ptr, img);
            e.bytes = img.rgba.len();
            e.idx = e.idx.min(n.index() as usize);
            e.bg_layers.push(*n);
        }
    }
    for (n, img) in &p.images.svgs {
        let e = slot(&mut by_ptr, img);
        e.bytes = img.rgba.len();
        e.idx = e.idx.min(n.index() as usize);
        e.svgs.push(*n);
    }
    for (n, img) in &p.images.masks {
        let e = slot(&mut by_ptr, img);
        e.bytes = img.rgba.len();
        e.idx = e.idx.min(n.index() as usize);
        e.masks.push(*n);
    }
    // `masks_url` : utilisée au paint par URL (pseudos sans
    // `NodeId`) — petits bitmaps alpha, gardés (idx 0).
    for (k, img) in &p.images.masks_url {
        let e = slot(&mut by_ptr, img);
        e.bytes = img.rgba.len();
        e.idx = 0;
        e.mask_urls.push(k.clone());
    }
    // Caches persistants : un bitmap qui n'est peint nulle
    // part reste de la mémoire — orphelin (`MAX`), évincé
    // avant les images affichées ; partagé → indice du
    // nœud peint.
    for (k, img) in &p.img_decoded {
        let e = slot(&mut by_ptr, img);
        e.bytes = img.rgba.len();
        e.img_keys.push(k.clone());
    }
    for (k, img) in p
        .bg_decoded
        .iter()
        .filter_map(|(k, v)| v.as_ref().map(|i| (k, i)))
    {
        let e = slot(&mut by_ptr, img);
        e.bytes = img.rgba.len();
        e.bg_keys.push(k.clone());
    }
    let mut total: usize = by_ptr.values().map(|e| e.bytes).sum();
    if total <= max_bytes {
        return;
    }
    let mut order: Vec<(usize, usize)> = by_ptr.iter().map(|(&ptr, e)| (e.idx, ptr)).collect();
    order.sort_unstable_by_key(|a| std::cmp::Reverse(a.0));
    let mut evicted = 0usize;
    let mut freed = 0usize;
    for (_, ptr) in order {
        if total <= max_bytes {
            break;
        }
        let Some(e) = by_ptr.remove(&ptr) else {
            continue;
        };
        for n in e.replaced {
            p.images.replaced.remove(&n);
        }
        for n in e.backgrounds {
            p.images.backgrounds.remove(&n);
        }
        for n in e.bg_layers {
            // Éviction fine par bitmap : le nœud peut porter
            // d'autres couches encore budgétées — on ne
            // retire que l'entrée du bitmap évincé.
            if let Some(v) = p.images.background_layers.get_mut(&n) {
                v.retain(|(_, i)| Arc::as_ptr(i) as usize != ptr);
                if v.is_empty() {
                    p.images.background_layers.remove(&n);
                }
            }
        }
        for n in e.svgs {
            p.images.svgs.remove(&n);
        }
        for n in e.masks {
            p.images.masks.remove(&n);
        }
        for k in e.mask_urls {
            p.images.masks_url.remove(&k);
        }
        for k in e.img_keys {
            p.img_decoded.remove(&k);
        }
        for k in e.bg_keys {
            p.bg_decoded.remove(&k);
        }
        evicted += 1;
        freed += e.bytes;
        total = total.saturating_sub(e.bytes);
    }
    tracing::info!(
        evicted,
        freed,
        kept = total,
        budget = max_bytes,
        "apply: images élaguées (budget octets)"
    );
}

fn insert_fetched_images(p: &mut PaintPayload, styles: &StyleMap, max_px: u32, max_bytes: usize) {
    if p.bg_aliases.is_empty() && p.mask_aliases.is_empty() {
        return;
    }
    // Drain unique : corps → cache décodé **+ magasin retenu
    // `img_bodies`** (étape 133 — source du re-décodage au
    // retour bfcache). Les applys suivants trouvent
    // `bg_bodies` vide et ne coûtent que la boucle de
    // correspondance styles → cache.
    // Budget octets global sur les bitmaps retenus (§B-59) :
    // au-delà, le décodage n'entre pas en cache — le fond
    // n'est pas peint (dégradation), jamais de dépassement.
    let mut running = net::decoded_cache_bytes(&p.img_decoded, &p.bg_decoded);
    for (resolved, body) in std::mem::take(&mut p.bg_bodies) {
        net::retain_img_body(&mut p.img_bodies, &resolved, body.clone());
        if let std::collections::hash_map::Entry::Vacant(e) = p.bg_decoded.entry(resolved) {
            let img = decode_image(&body, max_px).map(Arc::new).filter(|i| {
                let b = i.rgba.len();
                if running.saturating_add(b) > max_bytes {
                    false
                } else {
                    running += b;
                    true
                }
            });
            e.insert(img);
        }
    }
    for (node, st) in styles.iter() {
        // R-203 — chaque couche `url()` est indexée dans
        // `background_layers` sous son index de couche
        // (`emit_bg_layers` la consulte avant le repli
        // `backgrounds`) ; la première couche url garde
        // aussi l'entrée `backgrounds` — contrat
        // historique inchangé pour les consommateurs
        // existants (`emit_image_marker`, fetchers
        // mono-couche). `retain` par index : la passe 2
        // `@container` ré-insère sous les mêmes `NodeId`
        // sans dupliquer.
        let layers = explorer_render::background_layer_urls(st);
        let first_idx = layers.first().map(|(i, _)| *i);
        for (idx, raw) in layers {
            let Some(resolved) = p.bg_aliases.get(raw.as_str()) else {
                continue;
            };
            if let Some(Some(img)) = p.bg_decoded.get(resolved.as_str()) {
                if Some(idx) == first_idx {
                    p.images.backgrounds.insert(node, img.clone());
                }
                let v = p.images.background_layers.entry(node).or_default();
                v.retain(|(i, _)| *i != idx);
                v.push((idx, img.clone()));
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// §B-59 — le budget octets borne les bitmaps retenus :
    /// orphelins de cache évincés en premier, puis les images
    /// les plus tardives du document ; un bitmap partagé
    /// (`Arc`) disparaît de toutes les tables à la fois.
    #[test]
    fn prune_paint_images_budget_octets() {
        let img = |n: usize| {
            Arc::new(DecodedImage {
                width: 1,
                height: 1,
                rgba: vec![0u8; n],
            })
        };
        let dom = TreeBuilder::parse_scripting(
            "<html><body><div><p>x</p><p>y</p></div></body></html>",
            false,
        );
        let ids = engine_preorder(&dom);
        let (n1, n2) = (ids[2], ids[3]);
        let mut p = PaintPayload::default();
        let a = img(100);
        p.images.replaced.insert(n1, a.clone());
        p.img_decoded.insert("a".into(), a);
        let b = img(100);
        p.images.replaced.insert(n2, b.clone());
        p.bg_decoded.insert("b".into(), Some(b));
        p.img_decoded.insert("orphan".into(), img(100));
        p.images.masks_url.insert("mask".into(), img(10));
        prune_paint_images(&mut p, 200);
        // 310 octets → 200 : orphelin (100) puis nœud tardif
        // (100) évincés ; premier nœud + masque gardés.
        assert!(p.images.replaced.contains_key(&n1));
        assert!(!p.images.replaced.contains_key(&n2));
        assert!(p.img_decoded.contains_key("a"));
        assert!(!p.img_decoded.contains_key("orphan"));
        assert!(!p.bg_decoded.contains_key("b"));
        assert!(p.images.masks_url.contains_key("mask"));
        // Déjà sous le budget : no-op.
        let before = p.images.replaced.len();
        prune_paint_images(&mut p, 200);
        assert_eq!(p.images.replaced.len(), before);
    }

    /// JS-C129 — le rejeu `Insert` apparie `créés (moteur)` ↔
    /// `ids/tags (miroir)` **par tag**, pas positionnellement :
    /// un nœud que le re-parse moteur synthétise ou garde sans
    /// reflet miroir (ex. `<tbody>` sous `<table>`) ne décale
    /// plus les clés suivantes — avant : zip tronqué → clés
    /// dérivées sur de mauvais nœuds, `UnmappedKey` en cascade
    /// → repli re-parse (~16 s terrain, classe « partout »).
    #[test]
    fn align_keyed_skips_engine_only_nodes() {
        // Moteur : div > p(x), span(synthèse), b(y).
        let dom = TreeBuilder::parse_scripting(
            "<html><body><div><p>x</p><span>s</span><b>y</b></div></body></html>",
            false,
        );
        let engine = engine_preorder(&dom);
        // miroir : div, p, #text x, b, #text y — le `span`
        // moteur n'a pas de clé miroir (synthèse parse).
        let tags = [
            "div".to_string(),
            "p".to_string(),
            format!("#text\x00{:016x}", fnv64("x")),
            "b".to_string(),
            format!("#text\x00{:016x}", fnv64("y")),
        ];
        let ids = [10u64, 11, 12, 13, 14];
        let mut map = HashMap::new();
        align_keyed(&mut map, &dom, &engine, &ids, &tags);
        // `b` miroir (clé 13) doit tomber sur le VRAI `b`
        // moteur — le zip positionnel l'aurait posé sur `span`.
        let b = map[&13];
        assert_eq!(dom.tag_name(b).unwrap(), "b");
        assert_eq!(map.len(), 5, "5 clés miroir attendues : {map:?}");
    }

    /// §B-111 — soundcloud : un `Insert` hydraté très profond
    /// débordait la pile worker (`tokio-rt-worker` mort,
    /// `0xC00000FD`) — `graft_subtree` récursait natif par
    /// niveau de sous-arbre. La greffe tourne désormais sous
    /// `deep` : la profondeur devient un coût tas. Verrou :
    /// ~10 000 niveaux sur une pile de 2 Mio — sans la garde,
    /// l'overflow tue le thread (en réel : le process).
    #[test]
    fn graft_subtree_deep_survives() {
        const DEPTH: usize = 10_000;
        let graft = std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(|| {
                let mut src = Dom::with_max_nodes(DEPTH + 16);
                let root = src.create_element("div").unwrap();
                let mut cur = root;
                for _ in 0..DEPTH {
                    let n = src.create_element("div").unwrap();
                    src.append_child(cur, n).unwrap();
                    cur = n;
                }
                let mut dst = Dom::with_max_nodes(DEPTH + 16);
                let par = dst.create_element("div").unwrap();
                let new = graft_subtree(&mut dst, &src, root, par, 0).expect("greffe profonde");
                // La chaîne complète est greffée : on redescend
                // itératif jusqu'au 10 000ᵉ descendant.
                let mut depth = 0usize;
                let mut cur = new;
                while let Some(c) = dst.children_iter(cur).unwrap().next() {
                    depth += 1;
                    cur = c;
                }
                assert_eq!(depth, DEPTH);
            })
            .expect("spawn");
        graft.join().expect("thread greffe survécu");
    }

    /// Le texte miroir sans concordant de contenu à la
    /// position courante reste non mappé — `UnmappedKey` sûr,
    /// jamais un `SetText` sur le mauvais nœud.
    #[test]
    fn align_keyed_text_requires_content_match() {
        let dom = TreeBuilder::parse_scripting("<html><body><div>ab</div></body></html>", false);
        let engine = engine_preorder(&dom);
        // Miroir : div, #text "a" (le moteur a fusionné
        // `a`+`b` en "ab" — hash différent → pas de map).
        let tags = ["div".to_string(), format!("#text\x00{:016x}", fnv64("a"))];
        let ids = [20u64, 21];
        let mut map = HashMap::new();
        align_keyed(&mut map, &dom, &engine, &ids, &tags);
        assert!(map.contains_key(&20));
        assert!(!map.contains_key(&21), "texte divergent non mappé");
    }

    /// JS-C289 — `attachShadow` programmatique : la racine
    /// fragment (aucun `<template>` repère en light DOM) et
    /// son ombre entrent dans le pré-ordre moteur sous le tag
    /// `template` synthétique, AVANT les enfants light —
    /// ordre sérialisation miroir (R-96). Avant : clés shadow
    /// non mappées → `UnmappedKey`/`Fingerprint` → re-parse
    /// en boucle (mur journal mesuré sur youtube.com : le
    /// bundle 10,8 Mo se ré-évaluait à chaque rejet).
    #[test]
    fn programmatic_shadow_aligns_and_fingerprints() {
        let mut dom = TreeBuilder::parse_scripting(
            "<html><body><my-el><p>light</p></my-el></body></html>",
            false,
        );
        let host = engine_preorder(&dom)
            .into_iter()
            .find(|&n| dom.tag_name(n).ok() == Some("my-el"))
            .unwrap();
        // attachShadow programmatique — fragment nu, pas de
        // repère `<template shadowrootmode>` en light DOM.
        let root = dom.create_fragment().unwrap();
        dom.attach_shadow_root(host, root, true);
        let span = dom.create_element("span").unwrap();
        dom.insert_before(root, span, None).unwrap();
        // Miroir : my-el, template(racine), span, p — ordre
        // [shadow][light].
        let engine = engine_preorder(&dom);
        let tags = [
            "my-el".to_string(),
            "template".to_string(),
            "span".to_string(),
            "p".to_string(),
        ];
        let ids = [30u64, 31, 32, 33];
        let mut map = HashMap::new();
        align_keyed(&mut map, &dom, &engine, &ids, &tags);
        assert_eq!(map.len(), 4, "toutes les clés mappées : {map:?}");
        assert_eq!(map[&31], root, "la clé racine lie le fragment");
        assert_eq!(map[&32], span);
        // L'ombre est bien émise avant le light dans le
        // pré-ordre (span index < p index).
        let pos = |n| engine.iter().position(|&e| e == n).unwrap();
        assert!(pos(root) < pos(span) && pos(span) < pos(map[&33]));
        // Empreinte miroir (JS-C315) : somme des hash de tag —
        // le fragment racine compte `template`, l'ordre
        // n'influence plus la valeur.
        let mut want = 0xcbf2_9ce4_8422_2325u64;
        for t in ["my-el", "template", "span", "p"] {
            want = want.wrapping_add(super::fp_tag(t));
        }
        assert_eq!(engine_fingerprint(&dom, &map), want);
    }

    /// JS-C289 bis — DSD : le `<template shadowrootmode>`
    /// consommé reste repère (son `template_contents` EST la
    /// racine) — le pré-ordre ne doit PAS réémettre l'ombre
    /// une seconde fois (ordre déjà couvert par la descente
    /// template canonique).
    #[test]
    fn dsd_shadow_not_duplicated() {
        let dom = TreeBuilder::parse_scripting(
            "<html><body><my-el><template shadowrootmode=\"open\"><b>s</b></template><p>light</p></my-el></body></html>",
            false,
        );
        let engine = engine_preorder(&dom);
        // my-el, template(repère), b (ombre via
        // template_contents), p — un seul passage.
        let names: Vec<String> = engine
            .iter()
            .map(|&id| {
                if dom.is_shadow_root(id) {
                    "root".to_string()
                } else {
                    dom.tag_name(id).unwrap_or("#").to_string()
                }
            })
            .collect();
        assert_eq!(
            names,
            ["html", "head", "body", "my-el", "template", "b", "#", "p", "#"],
            "DSD sans doublon (textes « s »/« light ») : {names:?}"
        );
    }
}

#[cfg(test)]
mod reloc_tests {
    use super::relocate_obreloc;
    use explorer_html::TreeBuilder;

    fn kids_tags(dom: &explorer_dom::Dom, id: explorer_dom::NodeId) -> Vec<String> {
        dom.children_iter(id)
            .unwrap()
            .map(|n| dom.tag_name(n).unwrap_or("?").to_string())
            .collect()
    }

    fn find_child(
        dom: &explorer_dom::Dom,
        id: explorer_dom::NodeId,
        tag: &str,
    ) -> explorer_dom::NodeId {
        dom.children_iter(id)
            .unwrap()
            .find(|&n| dom.tag_name(n).is_ok_and(|t| t == tag))
            .unwrap()
    }

    /// Verrouille le comportement **spec** du tree-builder que la
    /// couture contourne : un `<iron-iconset-svg>` sous `<head>`
    /// est foster-parenté dans `<body>` (mur youtube.com — JS-C291).
    #[test]
    fn c291_head_foster_par_le_parseur() {
        let html = "<html><head><title>t</title><iron-iconset-svg><svg><defs></defs></svg></iron-iconset-svg></head><body><script>1</script></body></html>";
        let dom = TreeBuilder::parse_scripting(html, false);
        let root = dom.document_element().unwrap();
        let head = find_child(&dom, root, "head");
        let body = find_child(&dom, root, "body");
        assert_eq!(kids_tags(&dom, head), ["title"]);
        assert_eq!(kids_tags(&dom, body), ["iron-iconset-svg", "script"]);
    }

    /// JS-C291 — `data-obreloc` : les enfants illégaux de `head`
    /// voyagent dans le marqueur et sont resplicés à l'index.
    #[test]
    fn c291_reloc_inline_sous_head() {
        let html = "<html><head><title>t</title><template data-obreloc><iron-iconset-svg><svg><defs></defs></svg></iron-iconset-svg><div>x</div></template><script>1</script></head><body><p>y</p></body></html>";
        let mut dom = TreeBuilder::parse_scripting(html, false);
        relocate_obreloc(&mut dom);
        let root = dom.document_element().unwrap();
        let head = find_child(&dom, root, "head");
        assert_eq!(
            kids_tags(&dom, head),
            ["title", "iron-iconset-svg", "div", "script"]
        );
    }

    /// JS-C291 — `data-obreloc-x` : l'enfant à contexte requis
    /// (`tr` sous `div`) voyage enveloppé ; l'enveloppe est
    /// déballée, le `tr` (avec son `td`) atterrit sous `div`.
    #[test]
    fn c291_reloc_contexte_requis() {
        let html = "<html><body><div><template data-obreloc-x=\"table>tbody\"><table><tbody><tr><td>x</td></tr></tbody></table></template><span>y</span></div></body></html>";
        let mut dom = TreeBuilder::parse_scripting(html, false);
        relocate_obreloc(&mut dom);
        let body = find_child(&dom, dom.document_element().unwrap(), "body");
        let div = find_child(&dom, body, "div");
        assert_eq!(kids_tags(&dom, div), ["tr", "span"]);
        let tr = find_child(&dom, div, "tr");
        assert_eq!(kids_tags(&dom, tr), ["td"]);
    }

    /// JS-C291 — `data-obreloc-prev` : les enfants éléments d'un
    /// parent rawtext (`script`) suivent en marqueur-frère et
    /// reviennent sous le `script`.
    #[test]
    fn c291_reloc_rawtext_prev() {
        let html = "<html><head><script>x=1</script><template data-obreloc-prev><div>w</div></template></head><body></body></html>";
        let mut dom = TreeBuilder::parse_scripting(html, false);
        relocate_obreloc(&mut dom);
        let head = find_child(&dom, dom.document_element().unwrap(), "head");
        assert_eq!(kids_tags(&dom, head), ["script"]);
        let script = find_child(&dom, head, "script");
        // `?` = le texte `x=1` (rawtext) conservé en place,
        // `div` resplicé après lui.
        assert_eq!(kids_tags(&dom, script), ["?", "div"]);
    }

    /// JS-C291 — `data-obreloc-html` : enfant hors head/body/
    /// frameset sous `html` — resplicé à l'index porté.
    #[test]
    fn c291_reloc_sous_html() {
        let html = "<html><head></head><body><template data-obreloc-html=\"2\"><div>z</div></template></body></html>";
        let mut dom = TreeBuilder::parse_scripting(html, false);
        relocate_obreloc(&mut dom);
        let root = dom.document_element().unwrap();
        assert_eq!(kids_tags(&dom, root), ["head", "body", "div"]);
    }

    /// JS-C292 — un `Remove` de clé inconnue es un no-op : el
    /// nodo se creó y retiró entre dos drains, nunca entró a
    /// una serialización. La empreinte final sigue arbitrando
    /// un desajuste real.
    #[test]
    fn c292_remove_cle_orpheline_tolerada() {
        use super::{apply_journal_op, ApplyCtx, PaintPayload, SharedFonts};
        use explorer_layout::font::{FontRequest, FontSource};
        use std::sync::{Arc, Mutex};

        struct NoFont;
        impl FontSource for NoFont {
            fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
                None
            }
        }
        let ctx = ApplyCtx {
            fonts: SharedFonts(Arc::new(Mutex::new(explorer_render::WebFontSource::new(
                Arc::new(NoFont),
                4,
            )))),
            viewport: (800.0, 600.0),
            scale: 1.0,
            image_max_px: 0,
            image_max_bytes: 0,
            color_scheme: explorer_css::ColorScheme::Light,
            dom_journal_max_ops: 1000,
            scripting: false,
        };
        let dom = TreeBuilder::parse_scripting("<html><body><p>x</p></body></html>", false);
        let mut p = PaintPayload {
            dom: Some(dom),
            ..PaintPayload::default()
        };
        let e = super::engine_preorder(p.dom.as_ref().unwrap());
        p.keymap.insert(1, e[0]);
        let mut touched = Vec::new();
        let op = explorer_js::DomMutation::Remove { key: 9999 };
        assert!(apply_journal_op(&mut p, &ctx, &op, &mut touched).is_ok());
        // Con clave conocida el Remove sigue funcionando.
        let op = explorer_js::DomMutation::Remove { key: 1 };
        assert!(apply_journal_op(&mut p, &ctx, &op, &mut touched).is_ok());
        assert!(p.keymap.is_empty());
    }

    fn c293_ctx() -> super::ApplyCtx {
        use super::{ApplyCtx, SharedFonts};
        use explorer_layout::font::{FontRequest, FontSource};
        use std::sync::{Arc, Mutex};
        struct NoFont;
        impl FontSource for NoFont {
            fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
                None
            }
        }
        ApplyCtx {
            fonts: SharedFonts(Arc::new(Mutex::new(explorer_render::WebFontSource::new(
                Arc::new(NoFont),
                4,
            )))),
            viewport: (800.0, 600.0),
            scale: 1.0,
            image_max_px: 0,
            image_max_bytes: 0,
            color_scheme: explorer_css::ColorScheme::Light,
            dom_journal_max_ops: 1000,
            scripting: false,
        }
    }

    /// JS-C293+JS-C315 — journal vide (drain propre) :
    /// l'empreinte reste l'arbitre — mais en **multiset**
    /// (JS-C315) : la resplice `data-obreloc*` (mêmes nœuds,
    /// ordre différent) ne diverge plus ; une vraie
    /// divergence (compte/tags) rejette `Fingerprint`,
    /// vide ou non.
    #[test]
    fn c293_journal_vide_empreinte_arbitre() {
        use crate::session::apply::PaintPayload;
        let dom = TreeBuilder::parse_scripting("<html><body><p>x</p></body></html>", false);
        let mut p = PaintPayload {
            dom: Some(dom),
            ..PaintPayload::default()
        };
        let e = super::engine_preorder(p.dom.as_ref().unwrap());
        for (i, id) in e.iter().enumerate() {
            p.keymap.insert(i as u64 + 1, *id);
        }
        let ctx = c293_ctx();
        let job = super::PendingApply {
            tab: 0,
            epoch: 0,
            html: String::new(),
            dirty: None,
            canvas_ops: super::CanvasOpsTable::new(),
            relayout: true,
            journal: None,
            ids: Vec::new(),
            tags: Vec::new(),
        };
        let fp = super::engine_fingerprint(p.dom.as_ref().unwrap(), &p.keymap);
        let j = explorer_js::DomJournal {
            ops: Vec::new(),
            fingerprint: fp,
        };
        assert!(super::apply_journal(&mut p, &ctx, &job, &j, None, 100).is_ok());
        // JS-C314 (restauré — la garde avait sauté dans la
        // réécriture C315) : journal **vide** — pas d'arbitrage,
        // match OU mismatch → Ok sans relayout. Rien n'a été
        // rejoué → rien à vérifier ; l'empreinte session diverge
        // durablement du miroir sur pages `data-obreloc`, un
        // check y re-parse à chaque pompe.
        let j = explorer_js::DomJournal {
            ops: Vec::new(),
            fingerprint: fp ^ 1,
        };
        assert!(super::apply_journal(&mut p, &ctx, &job, &j, None, 100).is_ok());
        // L'arbitre reste entier sur de vraies ops : `SetAttr`
        // rejoué + empreinte divergente = repli `Fingerprint`.
        let j = explorer_js::DomJournal {
            ops: vec![explorer_js::DomMutation::SetAttr {
                key: 1,
                name: "data-x".to_string(),
                value: Some("y".to_string()),
            }],
            fingerprint: fp ^ 1,
        };
        assert!(matches!(
            super::apply_journal(&mut p, &ctx, &job, &j, None, 100),
            Err(super::JournalReject::Fingerprint)
        ));
    }

    /// Étape 208 (AR-3) — la voie journal réclame l'arène :
    /// un `Remove` détachait sans libérer (`live`
    /// monotone-croissant avant R-142). Marque `0` au départ
    /// → première passe `collect` balaie le sous-arbre ;
    /// `images.*` perd ses clés mortes (`retain_alive`) et
    /// la marque double (amorti des passes suivantes).
    #[test]
    fn etape208_journal_remove_reclame_l_arene() {
        use crate::session::apply::PaintPayload;
        use std::sync::Arc;
        let dom = TreeBuilder::parse_scripting("<html><body><p>x</p></body></html>", false);
        let mut p = PaintPayload {
            dom: Some(dom),
            ..PaintPayload::default()
        };
        let e = super::engine_preorder(p.dom.as_ref().unwrap());
        for (i, id) in e.iter().enumerate() {
            p.keymap.insert(i as u64 + 1, *id);
        }
        // Le `<p>` et sa clé miroir — sous-arbre retiré.
        let p_id = e
            .iter()
            .copied()
            .find(|&id| p.dom.as_ref().unwrap().tag_name(id).is_ok_and(|t| t == "p"))
            .expect("un <p>");
        let p_key = p
            .keymap
            .iter()
            .find(|(_, v)| **v == p_id)
            .map(|(k, _)| *k)
            .expect("clé du <p>");
        let live_before = p.dom.as_ref().unwrap().live_count();
        // Empreinte post-Remove : somme des tags mappés —
        // `<p>` sort de la keymap (son texte n'y compte pas).
        let fp_before = super::engine_fingerprint(p.dom.as_ref().unwrap(), &p.keymap);
        let fp = fp_before.wrapping_sub(super::fp_tag("p"));
        // Bitmap accrochée au nœud mourant — la purge
        // post-collect la retire (entrée jamais relue).
        p.images.replaced.insert(
            p_id,
            Arc::new(explorer_render::DecodedImage {
                width: 1,
                height: 1,
                rgba: vec![0, 0, 0, 255],
            }),
        );
        let ctx = c293_ctx();
        let job = super::PendingApply {
            tab: 0,
            epoch: 0,
            html: String::new(),
            dirty: None,
            canvas_ops: super::CanvasOpsTable::new(),
            relayout: false,
            journal: None,
            ids: Vec::new(),
            tags: Vec::new(),
        };
        let j = explorer_js::DomJournal {
            ops: vec![explorer_js::DomMutation::Remove { key: p_key }],
            fingerprint: fp,
        };
        assert!(super::apply_journal(&mut p, &ctx, &job, &j, None, 100).is_ok());
        let dom = p.dom.as_ref().unwrap();
        assert_eq!(dom.live_count(), live_before - 2); // <p> + son texte
        assert!(!dom.is_alive(p_id));
        assert!(p.images.replaced.is_empty());
        // Marque doublée — les passes suivantes sont amorties.
        assert_eq!(p.dom_gc_mark, dom.live_count() * 2);
    }
}

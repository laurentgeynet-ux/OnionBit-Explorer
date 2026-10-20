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
use std::time::Instant;

use explorer_css::value::Color;
use explorer_css::{ComputedStyle, Origin, StyleEngine, StyleEnv, StyleMap};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;
use explorer_js::CanvasOpsTable;
use explorer_layout::font::FontSource;
use explorer_layout::{
    build_box_tree_sized, layout_tree, relayout_subtree, BlockCtx, LayoutBox, Rect, Relayout,
};
use explorer_render::canvas::CanvasTable;
use explorer_render::{
    collect_data_images, rasterize, DisplayItem, DisplayList, Frame, PaintImages, WebFontSource,
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
}

/// État de peinture d'un onglet sorti pour un apply
/// (étape 55h) — exactement les champs que les phases
/// lisent/mutent ; la comptabilité de l'onglet (`hist`,
/// `js`, `focused`, `pending_nav`…) reste sur `Tab` et
/// répond normalement pendant la fenêtre.
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
    /// PNG paresseux (cache de `frame_png`).
    pub png: Option<Vec<u8>>,
    /// Popup `<select>` ouvert.
    pub popup: Option<SelectPopup>,
    /// Recherche en cours (rectangles de surlignage).
    pub find: Option<FindState>,
    /// Bitmaps indexés par `NodeId`.
    pub images: PaintImages,
    /// Feuilles `<link>` concaténées (cascade).
    pub linked_css: String,
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
}

/// Titre courant de l'onglet — lu au début d'apply pour
/// détecter le changement (le champ reste sur `Tab` ; la
/// comparaison s'appuie sur ce snapshot).
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
            png: self.png.take(),
            popup: self.popup.take(),
            find: self.find.take(),
            images: std::mem::take(&mut self.images),
            linked_css: std::mem::take(&mut self.linked_css),
            prev_styles: std::mem::take(&mut self.prev_styles),
            transitions: std::mem::take(&mut self.transitions),
            animations: std::mem::take(&mut self.animations),
            doc_h: self.doc_h,
            scroll_y: self.scroll_y,
            spec_paint: std::mem::take(&mut self.spec_paint),
            dirty: std::mem::take(&mut self.dirty),
            emitted: self.emitted,
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
        self.png = p.png;
        self.popup = p.popup;
        self.find = p.find;
        self.images = p.images;
        self.linked_css = p.linked_css;
        self.prev_styles = p.prev_styles;
        self.transitions = p.transitions;
        self.animations = p.animations;
        self.doc_h = p.doc_h;
        self.spec_paint = p.spec_paint;
        self.dirty = p.dirty;
        self.emitted = p.emitted;
    }
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
    let new_dom = TreeBuilder::parse(&job.html);
    let new_title = extract_title(&new_dom);
    // Relayout incrémental (étape 55f) avant tout
    // remplacement — il a besoin de l'ancien DOM et de
    // l'ancien arbre, tous deux dans la payload.
    let incremental = job.relayout
        && job
            .dirty
            .as_deref()
            .is_some_and(|path| try_incr_payload(p, ctx, &new_dom, path, &job.canvas_ops));
    out.incremental = incremental;
    // Les op-lists `<canvas>` survivent au re-parse : le
    // miroir les conserve (bitmap cumulative) — la table est
    // construite sur `new_dom` depuis les ops capturées à
    // l'enfilement (chemins → `NodeId` du DOM commis).
    let canvases = canvas_table_from_ops(&job.canvas_ops, &new_dom);
    p.dom = Some(new_dom);
    p.images = PaintImages::default();
    p.images.canvases = canvases;
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

/// `try_sync_incremental` sur payload — mêmes gardes que
/// la version session : cible résolue dans les deux DOM,
/// remap des ids conservés, repli `false` → reflow global.
fn try_incr_payload(
    p: &mut PaintPayload,
    ctx: &ApplyCtx,
    new_dom: &Dom,
    path: &[(String, u32)],
    canvas_ops: &CanvasOpsTable,
) -> bool {
    // Phase 1 — résolution de la cible sous emprunts
    // immuables (DOM + arbre présents, cible dans les deux
    // DOM, racine posable).
    let target_old = {
        let (Some(old_dom), true) = (p.dom.as_ref(), p.tree.is_some()) else {
            return false;
        };
        let Some(t) = dom_path_node(old_dom, path) else {
            return false;
        };
        t
    };
    let Some(target_new) = dom_path_node(new_dom, path) else {
        return false;
    };
    let Some(root_el) = new_dom.document_element() else {
        return false;
    };
    let mut sizes = image_sizes(new_dom, &p.images.replaced);
    let styles = styles_payload(p, new_dom);
    sizes.extend(explorer_render::svg::svg_sizes(new_dom, root_el));
    let svgs = explorer_render::svg::collect_svg(new_dom, root_el, ctx.image_max_px);
    let block = BlockCtx {
        fonts: Some(&ctx.fonts as &dyn FontSource),
        ..BlockCtx::default()
    };
    // Phase 2 — remap + relayout local (le remap précède :
    // `bx.node == target` compare des ids du nouveau DOM).
    let doc_h = {
        let (Some(old_dom), Some(tree)) = (p.dom.as_ref(), p.tree.as_mut()) else {
            return false;
        };
        if !remap_box_nodes(old_dom, new_dom, tree, target_old, target_new) {
            return false;
        }
        if relayout_subtree(tree, new_dom, &styles, &sizes, target_new, &block) != Relayout::Done {
            return false;
        }
        tree.rect.height
    };
    p.doc_h = doc_h;
    p.images.svgs = svgs;
    p.images.canvases = canvas_table_from_ops(canvas_ops, new_dom);
    repaint_payload(p, ctx).is_ok()
}

/// Cascade + transitions/animations sur payload — la passe
/// de styles partagée par reflow complet et relayout
/// incrémental. Écrit `prev_styles`/`transitions`/
/// `animations` de la payload.
pub(crate) fn styles_payload(p: &mut PaintPayload, dom: &Dom) -> StyleMap {
    let mut eng = StyleEngine::new();
    eng.add_css(MEDIA_UA_CSS, Origin::UserAgent);
    eng.add_css(&collect_style_css(dom), Origin::Author);
    eng.add_css(&p.linked_css, Origin::Author);
    let mut styles = eng.compute_tree(dom, dom.document(), &StyleEnv::default());
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
    let styles = styles_payload(p, dom);
    let Some(root_el) = dom.document_element() else {
        return Err(CoreError::EmptyDocument);
    };
    let data_imgs = collect_data_images(dom, &styles, ctx.image_max_px);
    p.images.replaced.extend(data_imgs.replaced);
    p.images.backgrounds.extend(data_imgs.backgrounds);
    let mut sizes = image_sizes(dom, &p.images.replaced);
    sizes.extend(explorer_render::svg::svg_sizes(dom, root_el));
    let svgs = explorer_render::svg::collect_svg(dom, root_el, ctx.image_max_px);
    let Some(mut bx) = build_box_tree_sized(dom, root_el, &styles, &sizes) else {
        return Err(CoreError::NothingToPaint);
    };
    let (vw, _vh) = ctx.viewport;
    let block = BlockCtx {
        fonts: Some(&ctx.fonts as &dyn FontSource),
        ..BlockCtx::default()
    };
    layout_tree(&mut bx, vw, &block);
    p.doc_h = bx.rect.height;
    p.tree = Some(bx);
    p.tree_epoch = p.tree_epoch.wrapping_add(1);
    p.images.svgs = svgs;
    // `<canvas>` 2D (JS-C28) : op-lists cumulées du miroir,
    // re-résolues par chemin d'élément dans `dom`.
    p.images.canvases = canvas_table_from_ops(canvas_ops, dom);
    repaint_payload(p, ctx)
}

/// `repaint` sur payload — display list + raster + diff
/// dirty. Les overlays (find, popup `<select>`) suivent la
/// payload — le find est effacé au re-parse (`NodeId`s
/// morts) mais peut revivre via `find_step` sous verrou.
pub(crate) fn repaint_payload(p: &mut PaintPayload, ctx: &ApplyCtx) -> Result<(), CoreError> {
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
    let frame_cap = MAX_FRAME_DIM / ctx.scale.max(f32::EPSILON);
    if list.height > frame_cap {
        list.height = frame_cap;
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
            explorer_render::rasterize_dirty(
                &list,
                &ctx.fonts as &dyn FontSource,
                ctx.scale,
                prev,
                &regions,
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
                rasterize(&culled, &ctx.fonts as &dyn FontSource, ctx.scale)
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
        }
    }

    /// Enfile une resynchro DOM pour l'onglet (étape 55h) :
    /// une seule entrée par onglet — une sérialisation plus
    /// récente subsume l'ancienne (document complet). Les
    /// op-lists `<canvas>` sont capturées ici (le moteur
    /// reste sur `Tab` pendant l'apply détaché).
    pub(crate) fn queue_apply(
        &mut self,
        tab: TabId,
        html: String,
        dirty: Option<Vec<(String, u32)>>,
    ) {
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
            html,
            dirty,
            canvas_ops,
            relayout: true,
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
        if let Some((html, dirty)) = eng.sync_dom_path() {
            job.html = html;
            job.dirty = dirty;
            job.canvas_ops = eng.canvas_ops();
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
    ) -> Option<(PendingApply, PaintPayload, ApplyCtx, Option<String>)> {
        loop {
            let mut job = self.pending_applies.pop_front()?;
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
            return Some((job, entry.take_paint(), ctx, entry.title.clone()));
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
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return; // onglet fermé pendant l'apply
        };
        if !entry.paint_detached {
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
        }
        // Epoch changée : la page a été supplantée — la
        // payload est abandonnée (sa propre nav repeint).
    }

    /// Apply synchrone d'une entrée de file — take/put sous
    /// `&mut self` (hôtes `&mut` : compat `handle_input`,
    /// pompe synchrone, FFI/CLI/tests).
    fn run_apply(&mut self, job: &mut PendingApply) {
        let tab = job.tab;
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
        self.freshen_apply(tab, job);
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        let old_title = entry.title.clone();
        let mut p = entry.take_paint();
        let out = apply_payload(&mut p, &ctx, job, old_title.as_deref());
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.put_paint(p);
        }
        self.commit_apply(tab, &out);
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

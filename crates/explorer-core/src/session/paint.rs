// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Pipeline de peinture : cascade, layout, repaint, synchro
//! DOM incrémentale (R-12). Les corps vivent dans
//! [`super::apply`] depuis l'étape 55h — les méthodes ici
//! délèguent via take/put de la [`PaintPayload`] pour
//! servir indifféremment le chemin `&mut self` et le chemin
//! détaché `spawn_blocking`.
use super::*;
use std::time::Instant;

use explorer_net::http::Request;
use explorer_net::url::Url;

impl Session {
    /// Resynchronise le document depuis le miroir JS muté :
    /// re-parse du HTML sérialisé, relecture du titre
    /// (`TitleChanged` si changé). `focused`/`hovered` sont
    /// conservés — ce sont des chemins, pas des `NodeId`
    /// (étape 52). `relayout` contrôle
    /// l'invalidation immédiate — vrai depuis les événements,
    /// faux pendant la navigation (qui relayout juste après).
    /// `dirty_path` = chemin du sous-arbre sale rapporté par
    /// le miroir (étape 55f) — sert à viser le relayout
    /// incrémental au lieu du reflow global.
    ///
    /// Chemin synchrone `&mut self` — les hôtes partagés
    /// enfilent via [`Self::queue_apply_sync`] puis drainent hors
    /// verrou (étape 55h).
    pub(crate) fn apply_dom_sync(
        &mut self,
        tab: TabId,
        html: &str,
        dirty_path: Option<&[(String, u32)]>,
        relayout: bool,
        ids: Option<Vec<u64>>,
        tags: Option<Vec<String>>,
    ) {
        self.reclaim_orphaned_paint(tab);
        let ctx = self.apply_ctx();
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        let canvas_ops = entry.js.as_deref_mut().map_or_else(
            explorer_js::CanvasOpsTable::new,
            explorer_js::JsEngine::canvas_ops,
        );
        let old_title = entry.title.clone();
        let mut p = entry.take_paint();
        let job = apply::PendingApply {
            tab,
            epoch: entry.nav_epoch,
            html: html.to_string(),
            dirty: dirty_path.map(<[_]>::to_vec),
            canvas_ops,
            relayout,
            journal: None,
            ids: ids.unwrap_or_default(),
            tags: tags.unwrap_or_default(),
        };
        let out = apply::apply_payload(&mut p, &ctx, &job, old_title.as_deref());
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.put_paint(p);
        }
        self.commit_apply(tab, &out);
    }

    /// `Navigated` + `TitleChanged` (si le titre change) +
    /// `FrameInvalidated` — séquence d'événements post-rendu.
    /// `prev` = titre avant navigation (capturé par l'appelant,
    /// `navigate_to` l'ayant déjà écrasé).
    pub(crate) fn emit_navigated(
        &mut self,
        tab: TabId,
        url: &Url,
        title: Option<String>,
        prev: Option<&str>,
    ) {
        let _ = self.events.send(CoreEvent::Navigated {
            tab,
            url: url.to_string(),
            title: title.clone(),
        });
        if let Some(t) = title.filter(|t| prev != Some(t.as_str())) {
            let _ = self.events.send(CoreEvent::TitleChanged { tab, title: t });
        }
        // Surfaces média de la page posée (étape 71b) —
        // commit nav, restauration bfcache et page d'erreur
        // passent tous par ici ; `refresh` diffe et émet
        // `MediaOffered`/`MediaMoved`/`MediaGone`.
        self.refresh_media(tab);
        self.emit_invalidated(tab);
    }

    /// Page d'erreur **peinte par le moteur** (étape 69) :
    /// une nav qui échoue commet un document d'erreur
    /// (statut, URL, cause) au lieu de laisser la page
    /// précédente ou un vide. La barre montre l'URL tentée,
    /// `can_back` est préservé (historique intact), un
    /// reload retente la cible en `GET`.
    pub(crate) fn paint_error_page(
        &mut self,
        tab: TabId,
        req: &Request,
        push: bool,
        err: &CoreError,
    ) {
        let url = &req.url;
        let html = error_document(url, err);
        let mut tb = explorer_html::TreeBuilder::new();
        tb.push(&html);
        tb.finish();
        let dom = tb.into_dom();
        let prev = self.tabs.get(&tab).and_then(|e| e.title.clone());
        let title = Some("Erreur de chargement".to_string());
        // Purge interactive AVANT le relayout — les `find`
        // de la page précédente ne doivent pas cuire dans la
        // frame d'erreur (étape 75 : bande orange stale).
        let cap = self.bfcache_pages;
        {
            let Some(e) = self.tabs.get_mut(&tab) else {
                return;
            };
            e.popup = None;
            e.find = None;
            e.focused = None;
            e.focused_key = None;
            e.hovered = None;
            e.deferred_inputs.clear();
            e.spec_paint = false;
            // Étape 76 : la page quittée gèle à son index
            // avant que l'erreur ne la remplace — `back` la
            // restaure sans réseau, comme après une nav
            // réussie (une page d'erreur quittée ne gèle
            // jamais — sa traversée retente la requête).
            // Ni une page en vol (étape 84 : pas d'entrée,
            // pas de slot — geler empoisonnerait `hist_idx`).
            if !e.hist_pending && !e.hist.get(e.hist_idx).is_some_and(|h| h.failed) {
                e.freeze(cap);
            }
        }
        if self.relayout_dom(tab, &dom).is_err() {
            return;
        }
        {
            let Some(e) = self.tabs.get_mut(&tab) else {
                return;
            };
            e.title.clone_from(&title);
            e.url = Some(url.clone());
            // `last_req = None` → reload retombe sur la
            // requête `failed` de l'entrée — retente de la
            // requête tombée, pas d'un `GET` vide (étape 93).
            e.last_req = None;
            // Idem commit réussi : la file d'intentions
            // appartient au document mort — elle n'a pas à
            // tirer après l'atterrissage de la page d'erreur.
            e.pending_nav.clear();
            // Étape 76 : la navigation en échec prend son
            // entrée d'historique (Chrome/Firefox) — `back`
            // revient à la page précédente, `forward` et la
            // traversée **retentent** la cible (entrée
            // `failed` : jamais de restauration bfcache).
            if push {
                e.hist.truncate(e.hist_idx + 1);
                e.hist.push(HistEntry {
                    req: req.clone(),
                    failed: true,
                    same_doc: false,
                    state: None,
                });
                e.hist_idx = e.hist.len() - 1;
                // L'entrée `failed` résout la position (84) —
                // les files `pushState`/`replaceState`
                // différés (CAP-151) appartiennent au
                // document tombé : vidées.
                e.hist_pending = false;
                e.pending_push.clear();
                e.pending_replace = None;
            }
        }
        self.emit_navigated(tab, url, title, prev.as_deref());
    }

    /// Log + `PipelineError` sur le bus, renvoie l'erreur.
    pub(crate) fn emit_error(&mut self, tab: TabId, e: CoreError) -> CoreError {
        tracing::debug!(tab, error = %e, "pipeline en échec");
        let _ = self.events.send(CoreEvent::PipelineError {
            tab: Some(tab),
            message: e.to_string(),
        });
        e
    }

    /// Re-pose l'arbre et re-peint depuis l'état courant
    /// (resize, style muté) — sans re-fetch.
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`] ou [`CoreError::EmptyDocument`]
    /// si l'onglet n'a jamais été navigué.
    pub fn rerender(&mut self, tab: TabId) -> Result<(), CoreError> {
        if !self.tabs.contains_key(&tab) {
            return Err(CoreError::UnknownTab(tab));
        }
        self.relayout(tab)?;
        self.emit_invalidated(tab);
        Ok(())
    }

    /// Style → boîtes → layout → display list → raster → PNG.
    /// Délègue à [`apply::relayout_dom_payload`] via
    /// take/put (étape 55h) — `EmptyDocument` si la peinture
    /// est détachée pour un apply hors verrou.
    pub(crate) fn relayout(&mut self, tab: TabId) -> Result<(), CoreError> {
        self.reclaim_orphaned_paint(tab);
        let ctx = self.apply_ctx();
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return Err(CoreError::UnknownTab(tab));
        };
        let canvas_ops = entry.js.as_deref_mut().map_or_else(
            explorer_js::CanvasOpsTable::new,
            explorer_js::JsEngine::canvas_ops,
        );
        // Le DOM sort de la payload le temps du pipeline —
        // `relayout_dom_payload` veut `&mut p` et un `&Dom`
        // disjoint.
        let mut p = entry.take_paint();
        let Some(dom) = p.dom.take() else {
            entry.put_paint(p);
            return Err(CoreError::EmptyDocument);
        };
        let r = apply::relayout_dom_payload(&mut p, &ctx, &dom, &canvas_ops);
        p.dom = Some(dom);
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.put_paint(p);
        }
        r
    }

    /// Relayout ciblé sur les éléments animés (étape 139 —
    /// la mesure profil `commit_lock` 33,5 s sur github
    /// attribuait le coût à un **reflow complet par tick
    /// d'animation**) : les ordinals `elem` des transitions/
    /// animations actives sont résolus en `NodeId` par
    /// `element_ordinals`, `journal_cover` pose
    /// `relayout_subtree` sur leur ancêtre commun — repli
    /// reflow complet si la cible n'est pas trouvable.
    pub(crate) fn relayout_animated(
        &mut self,
        tab: TabId,
        ords: &[usize],
    ) -> Result<(), CoreError> {
        self.reclaim_orphaned_paint(tab);
        let ctx = self.apply_ctx();
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return Err(CoreError::UnknownTab(tab));
        };
        let canvas_ops = entry.js.as_deref_mut().map_or_else(
            explorer_js::CanvasOpsTable::new,
            explorer_js::JsEngine::canvas_ops,
        );
        let mut p = entry.take_paint();
        let r = anim_tick_payload(&mut p, &ctx, &canvas_ops, ords, tab);
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.put_paint(p);
        }
        r
    }

    /// Passe de styles partagée par le reflow complet et le
    /// relayout incrémental (étape 55f) : cascade `<style>` +
    /// `<link>`, calcul sur tout `dom`, surcharge des
    /// transitions en vol. Délègue à
    /// [`apply::styles_payload`] (étape 55h). Test/profil
    /// uniquement — les chemins livrés passent par
    /// `styles_payload` directement.
    #[cfg(test)]
    pub(crate) fn compute_styles(&mut self, tab: TabId, dom: &Dom) -> StyleMap {
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return StyleMap::default();
        };
        let mut p = entry.take_paint();
        let (styles, _eng) = apply::styles_payload(
            &mut p,
            dom,
            self.viewport,
            self.color_scheme,
            entry.js.is_some(),
            self.scale,
        );
        entry.put_paint(p);
        styles
    }

    /// Pipeline style → layout → raster sur `dom` (emprunté —
    /// `relayout` pour le DOM posé, `nav_paint` pour le DOM
    /// en construction). Délègue à
    /// [`apply::relayout_dom_payload`] (étape 55h).
    pub(crate) fn relayout_dom(&mut self, tab: TabId, dom: &Dom) -> Result<(), CoreError> {
        self.reclaim_orphaned_paint(tab);
        let ctx = self.apply_ctx();
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return Err(CoreError::UnknownTab(tab));
        };
        let canvas_ops = entry.js.as_deref_mut().map_or_else(
            explorer_js::CanvasOpsTable::new,
            explorer_js::JsEngine::canvas_ops,
        );
        let mut p = entry.take_paint();
        let r = apply::relayout_dom_payload(&mut p, &ctx, dom, &canvas_ops);
        entry.put_paint(p);
        r
    }

    /// Display list + raster + PNG depuis l'arbre posé —
    /// sans re-layout (scroll, invalidation partielle).
    /// Délègue à [`apply::repaint_payload`] (étape 55h).
    pub(crate) fn repaint(&mut self, tab: TabId) -> Result<(), CoreError> {
        self.reclaim_orphaned_paint(tab);
        let ctx = self.apply_ctx();
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return Err(CoreError::UnknownTab(tab));
        };
        let mut p = entry.take_paint();
        let r = apply::repaint_payload(&mut p, &ctx, None);
        entry.put_paint(p);
        r
    }
}

/// Corps du tick anim (Étape 3c) — exécutable sous
/// `&mut Session` ([`Session::relayout_animated`]) ou en
/// `spawn_blocking` sur payload détachée : les ordinals
/// `elem` des transitions/animations actives sont résolus
/// en `NodeId` par `element_ordinals`, `journal_cover`
/// pose `relayout_subtree` sur leur ancêtre commun — repli
/// reflow complet si la cible n'est pas trouvable. Mêmes
/// phases `anim.ordinals`/`anim.relayout` que la voie
/// synchrone (même instrumentation, hors verrou ici).
/// `Ok(())` = relayout + repaint produits ;
/// [`CoreError::EmptyDocument`] = peinture déjà détachée
/// par un apply/nav — tick sauté, la pompe suivante
/// rattrape.
pub(crate) fn anim_tick_payload(
    p: &mut apply::PaintPayload,
    ctx: &apply::ApplyCtx,
    ops: &explorer_js::CanvasOpsTable,
    ords: &[usize],
    tab: TabId,
) -> Result<(), CoreError> {
    let Some(dom) = p.dom.take() else {
        return Err(CoreError::EmptyDocument);
    };
    let mut t = Instant::now();
    let ord_to_node: std::collections::HashMap<usize, explorer_dom::NodeId> =
        anim_items::element_ordinals(&dom)
            .into_iter()
            .map(|(n, o)| (o, n))
            .collect();
    let touched: Vec<(usize, explorer_dom::NodeId)> = ords
        .iter()
        .filter_map(|o| ord_to_node.get(o).map(|n| (*o, *n)))
        .collect();
    t = sync_phase(tab, "anim.ordinals", t);
    let now = Instant::now();
    let r = if paint_only_patch(p, &dom, &touched, now) {
        // Toutes les props animées sont peinture-pures : le
        // tick saute cascade + relayout et ne ré-émet que la
        // display list (étape 4).
        t = sync_phase(tab, "anim.patch", t);
        apply::repaint_payload(p, ctx, Some(&dom))
    } else {
        let nodes: Vec<explorer_dom::NodeId> = touched.iter().map(|&(_, n)| n).collect();
        match apply::journal_cover(&dom, &nodes) {
            Some(cover) => apply::relayout_subtree_payload(p, ctx, tab, &dom, cover, ops),
            None => apply::relayout_dom_payload(p, ctx, &dom, ops),
        }
    };
    sync_phase(tab, "anim.relayout", t);
    p.dom = Some(dom);
    r
}

/// Flippe l'identité d'une boîte mutée in-place (R-37) : la
/// voie `same_obj` du repaint incrémental saute la signature
/// quand `children`/`lines` gardent leur allocation — les
/// champs peints (`style`, styles de fragments) n'y sont pas
/// mutables. Ré-allouer un Vec non vide (drain + collect =
/// memcpy des éléments, pas de deep-clone) fait retomber
/// `bx_ptr`-égal → la signature se compare → la boîte est
/// ré-émise, ses enfants restent réutilisables. Feuille :
/// `same_obj` déjà faux, rien à faire.
#[allow(clippy::drain_collect)]
// ^ la nouvelle allocation est le but : `mem::take` rendrait
// le même Vec (même ptr) et la voie `same_obj` resterait.
fn reemit(bx: &mut explorer_layout::LayoutBox) {
    if !bx.children.is_empty() {
        bx.children = bx.children.drain(..).collect();
    } else if !bx.lines.is_empty() {
        bx.lines = bx.lines.drain(..).collect();
    }
}

/// Prop changée entre le style posé (`bx.style`) et le style
/// recomputé du tick : `(prop, old, new)` — `old` sert au
/// Prop changée entre le style posé (`bx.style`) et le style
/// recomputé du tick : `(prop, old, new)` — `old` sert au
/// diff (et au `get` de la propagation par `set`, jamais de
/// remplacement complet).
type PropChange = (String, String, String);

/// Élément touché planifié : nœud + diff vs le style posé
/// (`bx.style`) — le patch applique les `changed` par `set`
/// sans toucher aux clés matérialisées au layout
/// (`font-size`…), absentes de la cascade brute.
struct PatchPlan {
    node: explorer_dom::NodeId,
    changed: Vec<PropChange>,
}

/// Fast path « peinture seule » du tick anim (Étape 4) —
/// évalue les overrides du tick contre `prev_styles` (la
/// base cascade *avant* overrides, pas besoin de reconstruire
/// le `StyleEngine`), diffe le résultat contre le style posé
/// et, si **toutes** les props changées sont peinture-pures,
/// patche `bx.style` en place + fragments de texte.
/// `true` = tick résolu par `repaint_payload` seul ;
/// `false` = une prop sort de la whitelist (ou un pré-requis
/// manque : style de base absent, boîte introuvable, sous-
/// arbre SVG) → repli relayout ciblé / global.
fn paint_only_patch(
    p: &mut apply::PaintPayload,
    dom: &Dom,
    touched: &[(usize, explorer_dom::NodeId)],
    now: Instant,
) -> bool {
    // Flags `applied` de toutes les animations — mêmes
    // règles qu'`apply_anim_overrides` : frames présentes +
    // phase active + élément encore touché ce tick.
    let mut phases: Vec<(usize, usize, f64)> = Vec::new();
    let mut applied = vec![false; p.animations.len()];
    for (ai, a) in p.animations.iter().enumerate() {
        if a.frames.is_empty() {
            continue;
        }
        let elapsed = a
            .clock(now)
            .saturating_duration_since(a.start)
            .as_secs_f64();
        if let Some(ph) = a.decl.phase(elapsed) {
            // `applied` reflète « la passe a écrit » : l'ord
            // doit avoir survécu au DOM (touched), sinon le
            // repli de fin n'a rien à replier.
            applied[ai] = touched.iter().any(|&(o, _)| o == a.elem);
            phases.push((ai, a.elem, ph));
        }
    }
    // Phase A — plans immutables : style recomputé + diff
    // whitelist. Tout rat → `false` (repli relayout).
    let mut plans: Vec<PatchPlan> = Vec::with_capacity(touched.len());
    for &(ord, node) in touched {
        let Some(base) = p.prev_styles.get(&ord) else {
            return false;
        };
        if subtree_has_svg(dom, node) {
            return false;
        }
        let mut cs = base.clone();
        // Keyframes d'abord (ordre de liste), transitions
        // par-dessus — même ordre qu'`apply_anim_overrides`.
        for &(ai, elem, ph) in &phases {
            if elem != ord {
                continue;
            }
            let a = &p.animations[ai];
            for (k, v) in explorer_css::animation::keyframe_style(&a.frames, ph, a.decl.timing, &cs)
            {
                let cur = cs.get(&k).unwrap_or_default();
                cs.set(
                    &k,
                    &explorer_css::animation::composite_value(&k, cur, &v, a.decl.composite),
                );
            }
        }
        for t in p.transitions.iter().filter(|t| t.elem == ord) {
            if let Some(v) = t.value_at(now) {
                cs.set(&t.prop, &v);
            }
        }
        let Some(bx) = p.tree.as_ref().and_then(|t| find_box(t, node)) else {
            return false;
        };
        // Props que l'animation peut écrire — déclarations
        // des frames + props de transitions. Une de ces props
        // absente du style recomputé ce tick = repli à
        // appliquer (la frame précédente l'avait écrite).
        let mut keys: Vec<String> = cs.iter().map(|(k, _)| k.to_string()).collect();
        for a in p.animations.iter().filter(|a| a.elem == ord) {
            for (_, decls) in &a.frames {
                for d in decls {
                    if !keys.contains(&d.name) {
                        keys.push(d.name.clone());
                    }
                }
            }
        }
        for t in p.transitions.iter().filter(|t| t.elem == ord) {
            if !keys.contains(&t.prop) {
                keys.push(t.prop.clone());
            }
        }
        // Diff `bx.style` (posé, matérialisations incluses)
        // vs `cs` (recomputé) — les clés propres à `bx`
        // sont des valeurs matérialisées au layout, jamais
        // des overrides : ignorées.
        let mut changed: Vec<PropChange> = Vec::new();
        for k in keys {
            let o = bx.style.get(&k).unwrap_or_default();
            let n = cs.get(&k).unwrap_or_default();
            if o != n {
                if !anim_items::prop_paint_safe(&k, o, n) {
                    tracing::debug!(
                        target: "sync_profile",
                        prop = %k,
                        old = %o,
                        new = %n,
                        "anim patch replié sur relayout"
                    );
                    return false;
                }
                changed.push((k, o.to_string(), n.to_string()));
            }
        }
        plans.push(PatchPlan { node, changed });
    }
    // Phase B — application : boîtes propres + anonymes du
    // target, puis fragments (passe globale : les runs du
    // target vivent dans les lignes d'un conteneur ancêtre —
    // `owner_element` filtre les runs d'éléments descendants).
    if let Some(tree) = p.tree.as_mut() {
        for plan in &plans {
            patch_element(tree, plan);
        }
        patch_all_frags(tree, &plans, dom);
    }
    for (ai, a) in p.animations.iter_mut().enumerate() {
        a.applied = applied[ai];
    }
    // Transitions terminées sorties — même règle que
    // `update_transitions` (`end() > now`).
    p.transitions.retain(|t| t.end() > now);
    true
}

/// Première boîte de `node` dans l'arbre (ordre pré —
/// l'élément n'a qu'une boîte propre ; les suivantes sont
/// des wrappers sans `node`).
pub(crate) fn find_box(
    bx: &explorer_layout::LayoutBox,
    node: explorer_dom::NodeId,
) -> Option<&explorer_layout::LayoutBox> {
    if bx.node == Some(node) {
        return Some(bx);
    }
    bx.children.iter().find_map(|c| find_box(c, node))
}

/// `node` ou un descendant est `<svg>` — l'îlot SVG est
/// sérialisé à la pose : ses styles ne se patchent pas en
/// place → repli relayout.
fn subtree_has_svg(dom: &Dom, node: explorer_dom::NodeId) -> bool {
    if dom.tag_name(node).ok() == Some("svg") {
        return true;
    }
    dom.descendants(node)
        .is_ok_and(|mut it| it.any(|n| dom.tag_name(n).ok() == Some("svg")))
}

/// Applique les `changed` de `plan` sur les boîtes propres
/// de `plan.node` (par `set` — les clés matérialisées au
/// layout survivent), puis sur ses boîtes anonymes (clones
/// du computed du déclarant — même traitement). Les
/// descendants éléments **ne** sont pas propagés : fidèle
/// à `apply_anim_overrides`, dont les surcharges restent
/// locales à l'élément animé.
fn patch_element(bx: &mut explorer_layout::LayoutBox, plan: &PatchPlan) {
    if bx.node == Some(plan.node) {
        for (p, _, n) in &plan.changed {
            bx.style.set(p, n);
        }
        reemit(bx);
        for c in &mut bx.children {
            patch_owned_anons(c, &plan.changed);
        }
        return;
    }
    for c in &mut bx.children {
        patch_element(c, plan);
    }
}

/// Chaîne de boîtes anonymes sous la boîte target — elles
/// portent une copie de son style posé : mêmes `changed`.
/// S'arrête au premier élément descendant (style propre,
/// figé — même contrat que `apply_anim_overrides`).
fn patch_owned_anons(bx: &mut explorer_layout::LayoutBox, changed: &[PropChange]) {
    if bx.node.is_some() {
        return;
    }
    for (p, _, n) in changed {
        bx.style.set(p, n);
    }
    reemit(bx);
    for c in &mut bx.children {
        patch_owned_anons(c, changed);
    }
}

/// Élément propriétaire d'un run de texte : le plus proche
/// ancêtre élément de `fnode` (un nœud texte remonte à son
/// élément ; un nœud élément vaut pour lui-même). Sert à
/// n'appliquer les surcharges qu'aux runs **du target** —
/// les runs d'un élément descendant portent son propre
/// style figé (`apply_anim_overrides` ne propage pas).
pub(crate) fn owner_element(
    dom: &Dom,
    mut n: explorer_dom::NodeId,
) -> Option<explorer_dom::NodeId> {
    loop {
        if dom.tag_name(n).is_ok() {
            return Some(n);
        }
        n = dom.parent(n).ok()??;
    }
}

/// Fragments de texte — chaque ligne du document est
/// scannée : un run dont l'**élément propriétaire** est le
/// target porte son style posé → toutes les `changed`
/// s'appliquent. Les runs d'un élément descendant gardent
/// leur style figé (contrat `apply_anim_overrides`).
fn patch_all_frags(bx: &mut explorer_layout::LayoutBox, plans: &[PatchPlan], dom: &Dom) {
    let mut hit_any = false;
    for line in &mut bx.lines {
        for frag in &mut line.fragments {
            let Some(fnode) = frag.node else {
                continue;
            };
            for plan in plans {
                if owner_element(dom, fnode) != Some(plan.node) {
                    continue;
                }
                for (p, _, n) in &plan.changed {
                    frag.style.set(p, n);
                    hit_any = true;
                }
            }
        }
    }
    // `lines` est whitelisté R-37 mais la signature hache
    // `f.style` : la boîte porteuse doit quitter la voie
    // `same_obj` pour être ré-émise.
    if hit_any {
        reemit(bx);
    }
    for c in &mut bx.children {
        patch_all_frags(c, plans, dom);
    }
}

#[cfg(test)]
use explorer_css::StyleMap;
use explorer_dom::Dom;

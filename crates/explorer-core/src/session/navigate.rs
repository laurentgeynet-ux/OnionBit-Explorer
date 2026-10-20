// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Navigation : historique, bfcache, fragments, file des
//! navigations scriptées et commit du pipeline.
use super::*;
use std::collections::HashMap;
use std::sync::Arc;

use explorer_dom::Dom;
use explorer_js::NavRequest;
use explorer_net::http::{Method, Request};
use explorer_net::policy::{initiator_headers, is_mixed_content};
use explorer_net::url::Url;
use explorer_render::PaintImages;

impl Session {
    /// Navigue : fetch → decode → parse → style → layout →
    /// display list → raster → PNG, puis
    /// `Navigated` + `FrameInvalidated`.
    ///
    /// Toute étape qui échoue produit `PipelineError` sur le
    /// bus et une `Err` typée — la session reste saine.
    ///
    /// # Errors
    /// [`CoreError::Url`] (URL invalide), [`CoreError::Fetch`]
    /// (transport), [`CoreError::HttpStatus`] (non-2xx),
    /// [`CoreError::EmptyDocument`]/[`CoreError::NothingToPaint`]
    /// (page sans contenu peint), [`CoreError::FrameTooBig`]
    /// (allocation refusée), [`CoreError::UnknownTab`].
    pub async fn navigate(&mut self, tab: TabId, url: &str) -> Result<(), CoreError> {
        if !self.tabs.contains_key(&tab) {
            return Err(CoreError::UnknownTab(tab));
        }
        let url = Url::parse(url).map_err(|e| CoreError::Url(e.to_string()))?;
        // `push` = l'entrée rejoint l'historique ; `Navigated`
        // est émis par `navigate_request` si le document final
        // est bien celui demandé (redirection script exclue).
        match self.navigate_request(tab, Request::get(url), true).await {
            Ok(_) => {
                self.drain_script_nav(tab).await;
                Ok(())
            }
            Err(e) => Err(self.emit_error(tab, e)),
        }
    }

    /// Recharge la page courante (re-fetch complet,
    /// historique inchangé) — réémet la requête d'origine
    /// `last_req` : une page servie par POST est re-POSTée
    /// (étape 56 — les navigateurs réémettent la requête,
    /// quitte à confirmer la resoumission côté UI).
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`], [`CoreError::EmptyDocument`]
    /// si l'onglet n'a jamais été navigué, ou erreur de fetch.
    pub async fn reload(&mut self, tab: TabId) -> Result<(), CoreError> {
        let Some(req) = self.tabs.get(&tab).and_then(|t| {
            t.last_req
                .clone()
                .or_else(|| t.url.clone().map(Request::get))
        }) else {
            return Err(if self.tabs.contains_key(&tab) {
                CoreError::EmptyDocument
            } else {
                CoreError::UnknownTab(tab)
            });
        };
        match self.navigate_request(tab, req, false).await {
            Ok(_) => {
                self.drain_script_nav(tab).await;
                Ok(())
            }
            Err(e) => Err(self.emit_error(tab, e)),
        }
    }

    /// Recule dans l'historique local de l'onglet — restaure
    /// le snapshot bfcache s'il existe (instantané, étape 56)
    /// sinon rejoue la requête d'origine (méthode + corps —
    /// un POST historié re-POSTe).
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`] ou [`CoreError::EmptyDocument`]
    /// s'il n'y a pas de page précédente.
    pub async fn go_back(&mut self, tab: TabId) -> Result<(), CoreError> {
        self.history_move(tab, -1).await?;
        self.drain_script_nav(tab).await;
        Ok(())
    }

    /// Avance dans l'historique local de l'onglet — mêmes
    /// sémantiques que [`Self::go_back`] (bfcache puis rejeu
    /// de la requête d'origine).
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`] ou [`CoreError::EmptyDocument`]
    /// s'il n'y a pas de page suivante.
    pub async fn go_forward(&mut self, tab: TabId) -> Result<(), CoreError> {
        self.history_move(tab, 1).await?;
        self.drain_script_nav(tab).await;
        Ok(())
    }

    /// Tentative de restauration bfcache (étape 56) : la
    /// position `idx` est gelée → tout l'état revient sans
    /// réseau ; la page quittée est gelée à son tour
    /// (l'aller-retour ne re-télécharge jamais). `Navigated`,
    /// `TitleChanged` et `FrameInvalidated` émis comme après
    /// une navigation classique.
    ///
    /// `false` = pas de snapshot → fetch normal.
    pub(crate) fn try_bfcache(&mut self, tab: TabId, idx: usize) -> bool {
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return false;
        };
        let prev = entry.title.clone();
        let Some(snap) = entry.take_snapshot(idx) else {
            return false;
        };
        // La page quittée gèle son état affiché — une
        // peinture spéculative en vol ne doit pas être
        // embarquée (étape 56c).
        self.unspec(tab);
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return false;
        };
        entry.freeze(self.bfcache_pages);
        entry.restore(snap);
        entry.hist_idx = idx;
        // Snapshot gelé avant sa première peinture (étape
        // 56b — fenêtre commit→repaint) ou produit absent :
        // le DOM restauré suffit à re-peindre plutôt que de
        // servir un `/frame` 204 persistant avec hit-test
        // mort et scroll bloqué à 0.
        let needs_paint = entry.dom.is_some() && entry.frame.is_none();
        let url = entry.url.clone();
        let title = entry.title.clone();
        if needs_paint {
            let _ = self.relayout(tab);
        }
        if let Some(u) = url {
            // Mêmes événements qu'une navigation classique :
            // `Navigated` + `TitleChanged` si le titre
            // restauré diffère + `FrameInvalidated`.
            self.emit_navigated(tab, &u, title, prev.as_deref());
        }
        true
    }

    /// Navigation **même-document** vers un `#fragment`
    /// (étape 56d — `href="#id"`, footnotes Wikipedia) :
    /// l'URL hors fragment identique à la page commise →
    /// jamais de re-fetch — on pousse l'entrée d'historique,
    /// on émet `Navigated`, puis on scrolle vers la cible
    /// (`id` d'abord, `<a name>` ensuite, `#` nu → haut de
    /// page). URL différente → navigation complète ; le
    /// fragment est appliqué post-commit dans `sub_commit`.
    pub(crate) fn try_fragment_nav(&mut self, tab: TabId, req: &Request) -> bool {
        if req.method != Method::Get || req.body.is_some() {
            return false;
        }
        let Some(frag) = req.url.fragment() else {
            return false;
        };
        let same = self.tabs.get(&tab).is_some_and(|e| {
            e.url
                .as_ref()
                .is_some_and(|u| u.as_str().split('#').next() == req.url.as_str().split('#').next())
        });
        if !same {
            return false;
        }
        let prev = self.tabs.get(&tab).and_then(|e| e.title.clone());
        if let Some(e) = self.tabs.get_mut(&tab) {
            e.hist.truncate(e.hist_idx + 1);
            e.hist.push(req.clone());
            e.hist_idx = e.hist.len() - 1;
            e.url = Some(req.url.clone());
        }
        let title = self.tabs.get(&tab).and_then(|e| e.title.clone());
        self.emit_navigated(tab, &req.url, title, prev.as_deref());
        if frag.is_empty() {
            // `#` nu = haut de page.
            let _ = self.scroll_to_y(tab, 0.0);
        } else {
            self.scroll_to_fragment(tab, frag);
        }
        true
    }

    /// Scrolle vers la cible d'un `#fragment` : `id` puis
    /// `<a name>` (ordre navigateur). Position = le haut de
    /// la première boîte posée portant le nœud — bornée
    /// comme `scroll_by`. No-op si la cible est introuvable
    /// ou non rendue (dégradation, jamais d'erreur).
    pub(crate) fn scroll_to_fragment(&mut self, tab: TabId, frag: &str) {
        let Some(entry) = self.tabs.get(&tab) else {
            return;
        };
        let (Some(dom), Some(tree)) = (entry.dom.as_ref(), entry.tree.as_ref()) else {
            return;
        };
        let target = find_by_id(dom, frag).or_else(|| find_named_anchor(dom, frag));
        let Some(y) = target.and_then(|n| box_y_of_node(tree, n)) else {
            return;
        };
        let _ = self.scroll_to_y(tab, y);
    }

    /// Scroll absolu borné `[0, doc_h − viewport]` avec
    /// repaint et `FrameInvalidated` — variante absolue de
    /// `scroll_by` pour la navigation fragment (56d).
    pub(crate) fn scroll_to_y(&mut self, tab: TabId, y: f32) -> Result<(), CoreError> {
        {
            let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            let max = (entry.doc_h - self.viewport.1).max(0.0);
            let next = y.clamp(0.0, max);
            if (next - entry.scroll_y).abs() < f32::EPSILON {
                return Ok(());
            }
            entry.scroll_y = next;
        }
        self.repaint(tab)?;
        self.emit_invalidated(tab);
        Ok(())
    }

    /// `navigate` vers l'URL de l'entrée **forward** (étape
    /// 56c, repro A→B→back→`navigate B`) : restaurer le
    /// snapshot bfcache — un commit complet de l'entrée —
    /// plutôt qu'un fetch neuf dont la peinture progressive
    /// afficherait B pendant que l'entrée commise reste A
    /// (état hybride : frame B + hit-test/`status` sur A).
    /// Seul un GET sans corps vers l'URL exacte de
    /// l'entrée forward est réécrit en restauration ; un
    /// POST ou une URL différente navigue normalement, et
    /// l'absence de snapshot retombe sur le chemin classique.
    pub(crate) fn try_forward_nav(&mut self, tab: TabId, req: &Request) -> bool {
        if req.method != Method::Get || req.body.is_some() {
            return false;
        }
        let fwd = self
            .tabs
            .get(&tab)
            .map(|e| e.hist_idx + 1)
            .unwrap_or_default();
        let matches = self
            .tabs
            .get(&tab)
            .is_some_and(|e| e.hist.get(fwd).is_some_and(|r| r.url == req.url));
        matches && self.try_bfcache(tab, fwd)
    }

    /// Déplacement dans l'historique — `delta` = ±1.
    pub(crate) async fn history_move(&mut self, tab: TabId, delta: isize) -> Result<(), CoreError> {
        let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
        let next = entry.hist_idx.checked_add_signed(delta);
        let Some(idx) = next.filter(|&i| i < entry.hist.len()) else {
            return Err(CoreError::EmptyDocument);
        };
        let req = entry.hist[idx].clone();
        // Hit bfcache (étape 56) : restauration instantanée,
        // zéro réseau.
        if self.try_bfcache(tab, idx) {
            return Ok(());
        }
        // Miss → on rejoue la requête d'origine (méthode +
        // corps — un POST historié re-POSTe, étape 56).
        // `Navigated` est émis par `navigate_request` — sauf
        // redirection script (alors `entry.url` diffère et la
        // cible a émis pour elle-même).
        match self.navigate_request(tab, req, false).await {
            Ok(_) => {
                if let Some(entry) = self.tabs.get_mut(&tab) {
                    entry.hist_idx = idx;
                }
                // Pas de drain ici : les appelants publics
                // (`go_back/forward`) le font après — sinon
                // `drain_script_nav` → `history_move` →
                // `drain_script_nav` bouclerait l'inférence.
                Ok(())
            }
            Err(e) => Err(self.emit_error(tab, e)),
        }
    }

    /// Fetch → parse → état de l'onglet → rendu. Ne touche
    /// pas à l'historique (reload, déplacements). Renvoie le
    /// titre de la page.
    pub(crate) async fn navigate_to(
        &mut self,
        tab: TabId,
        url: Url,
    ) -> Result<Option<String>, CoreError> {
        self.navigate_request(tab, Request::get(url), false).await
    }

    /// Variante [`navigate_to`](Self::navigate_to) prenant une
    /// requête complète — soumission de formulaire POST
    /// (corps urlencodé) ; GET passe par le wrapper.
    /// `push` = l'URL rejoint l'historique après le rendu
    /// (navigate, soumission) ; `false` = déplacement/reload.
    ///
    /// Chemin compat `&mut self` (étape 53d) : les phases
    /// [`nav_begin`](Self::nav_begin) → [`NavJob::run`] →
    /// [`nav_apply`](Self::nav_apply) → [`SubJob::run`] →
    /// [`sub_commit`](Self::sub_commit) sont exactement celles
    /// que [`navigate_shared`] conduit sur une session
    /// partagée — ici `self` reste emprunté pendant tout le
    /// parcours (CLI, FFI, tests).
    pub(crate) async fn navigate_request(
        &mut self,
        tab: TabId,
        req: Request,
        push: bool,
    ) -> Result<Option<String>, CoreError> {
        // `navigate` vers l'entrée forward = restauration
        // bfcache, jamais une peinture non commise (56c).
        if push && self.try_forward_nav(tab, &req) {
            return Ok(self.tabs.get(&tab).and_then(|e| e.title.clone()));
        }
        // `#fragment` même document = scroll + historique,
        // jamais de re-fetch (56d).
        if self.try_fragment_nav(tab, &req) {
            return Ok(self.tabs.get(&tab).and_then(|e| e.title.clone()));
        }
        let mut job = self.nav_begin(tab, req, push)?;
        let epoch = job.epoch;
        let (done, tail) = match job
            .run(|dom: &Dom| -> PaintFut<'_> {
                self.nav_paint(tab, epoch, dom);
                Box::pin(async {})
            })
            .await
        {
            Ok(out) => out,
            Err(e) => {
                // Le document en vol a pu être peint
                // progressivement — on réverte avant de
                // propager (étape 56c).
                self.unspec(tab);
                return Err(e);
            }
        };
        let Some(mut sub) = self.nav_apply(tab, tail, done)? else {
            return Ok(None);
        };
        sub.run().await;
        self.sub_commit(tab, sub).await
    }

    /// Phase 1 de la navigation éclatée (étape 53d) :
    /// valide l'onglet, arme l'epoch (les phases suivantes
    /// refusent tout commit d'une navigation supplantée) et
    /// capture le titre courant pour `TitleChanged`. Aucune
    /// écriture visible — URL, historique et DOM ne bougent
    /// qu'au commit.
    pub(crate) fn nav_begin(
        &mut self,
        tab: TabId,
        req: Request,
        push: bool,
    ) -> Result<NavJob, CoreError> {
        let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
        entry.nav_epoch += 1;
        Ok(NavJob {
            epoch: entry.nav_epoch,
            req,
            push,
            prev_title: entry.title.clone(),
            transport: Arc::clone(&self.transport),
            paint_chunk_bytes: self.paint_chunk_bytes,
            paint_interval: self.paint_interval,
        })
    }

    /// Peinture progressive epoch-gardée (étape 53d) : un
    /// snapshot émis par [`NavJob::run`] n'est posé que si
    /// la navigation est encore la plus récente de l'onglet —
    /// sinon il est silencieusement jeté.
    pub(crate) fn nav_paint(&mut self, tab: TabId, epoch: u64, dom: &Dom) {
        let live = self.tabs.get(&tab).is_some_and(|e| e.nav_epoch == epoch);
        if live && self.relayout_dom(tab, dom).is_ok() {
            // Peinture spéculative (étape 56c) : l'arbre et la
            // frame montrent un document non commis — marqué
            // pour réversion au gel/abandon/échec.
            if let Some(e) = self.tabs.get_mut(&tab) {
                e.spec_paint = true;
            }
            self.emit_invalidated(tab);
        }
    }

    /// Réverte une peinture spéculative (étape 56c) : la
    /// frame provenait d'un document en vol — repeint depuis
    /// le DOM commis pour que frame et DOM redeviennent
    /// cohérents. No-op si rien de spéculatif.
    pub(crate) fn unspec(&mut self, tab: TabId) {
        let spec = self.tabs.get(&tab).is_some_and(|e| e.spec_paint);
        if !spec {
            return;
        }
        if let Some(e) = self.tabs.get_mut(&tab) {
            e.spec_paint = false;
        }
        if self.relayout(tab).is_ok() {
            self.emit_invalidated(tab);
        } else if let Some(e) = self.tabs.get_mut(&tab) {
            // Rien de commis à repeindre (onglet jamais
            // navigué) : on jette l'arbre et la frame
            // spéculatifs plutôt que de laisser un état
            // sans DOM derrière.
            e.tree = None;
            e.list = None;
            e.frame = None;
            e.png = None;
        }
    }

    /// Phase 2 — commit du DOM téléchargé : écrit l'état de
    /// l'onglet, planifie les sous-ressources, arme le moteur
    /// JS et emballe le [`SubJob`] détachable. `None` = epoch
    /// supplantée (navigation abandonnée — rien à commettre).
    pub(crate) fn nav_apply(
        &mut self,
        tab: TabId,
        tail: NavTail,
        done: NavDone,
    ) -> Result<Option<SubJob>, CoreError> {
        {
            let entry = self.tabs.get(&tab).ok_or(CoreError::UnknownTab(tab))?;
            if entry.nav_epoch != tail.epoch {
                self.unspec(tab);
                return Ok(None);
            }
        }
        let NavDone {
            url,
            dom,
            html,
            title,
        } = done;
        if let Some(s) = &self.store {
            s.record_visit(&url.to_string(), title.as_deref());
        }
        let scripting = self.js_factory.is_some();
        let (sheet_urls, scripts, img_urls) = self.plan_subresources(&dom, &url, scripting);
        let doc_css = collect_style_css(&dom);
        // Le gel capture l'état affiché — une peinture
        // spéculative du document en vol ne doit pas être
        // embarquée dans le snapshot (étape 56c).
        self.unspec(tab);
        {
            let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            // bfcache (étape 56) : la page quittée est gelée
            // à sa position d'historique — un retour
            // arrière la restaurera sans réseau.
            entry.freeze(self.bfcache_pages);
            entry.last_req = Some(tail.req.clone());
            entry.url = Some(url.clone());
            entry.title.clone_from(&title);
            entry.scroll_y = 0.0;
            entry.focused = None;
            entry.hovered = None;
            entry.popup = None;
            entry.find = None;
            entry.js = None;
            entry.images = PaintImages::default();
            entry.linked_css.clear();
            entry.prev_styles = HashMap::new();
            entry.transitions.clear();
            entry.sync_floor = None;
            entry.dom = Some(dom);
        }
        self.webfonts.clear();
        let eng = self.spawn_engine(tab, &html, !scripts.is_empty());
        Ok(Some(SubJob {
            tab,
            epoch: tail.epoch,
            url: url.clone(),
            push: tail.push,
            prev_title: tail.prev_title,
            req: tail.req,
            title,
            transport: Arc::clone(&self.transport),
            events: self.events.clone(),
            page: url,
            sheet_urls,
            scripts,
            img_urls,
            doc_css,
            eng,
            css_slots: Vec::new(),
            script_srcs: Vec::new(),
            img_bodies: HashMap::new(),
            async_mods: Vec::new(),
            fonts: Vec::new(),
            arrival_evals: Vec::new(),
            post_evals: Vec::new(),
            max_inflight: self.max_inflight,
            max_modules: self.max_modules,
            font_budget: self.max_fonts,
        }))
    }

    /// Dépile la prochaine intention de navigation.
    pub(crate) fn pop_pending_nav(&mut self, tab: TabId) -> Option<DrainAct> {
        self.tabs
            .get_mut(&tab)
            .and_then(|e| e.pending_nav.pop_front())
    }

    /// Enfile une intention — `false` si la file est pleine
    /// (`MAX_PENDING_NAV`) : l'appelant cesse alors de
    /// vider le moteur (un flot hostile ne fait pas gonfler
    /// la mémoire, et aucune intention n'est jetée en
    /// silence — celles encore dans le moteur restent
    /// drainables au prochain tour).
    pub(crate) fn enqueue_nav_act(&mut self, tab: TabId, act: DrainAct) -> bool {
        const MAX_PENDING_NAV: usize = 8;
        if let Some(e) = self.tabs.get_mut(&tab) {
            if e.pending_nav.len() < MAX_PENDING_NAV {
                e.pending_nav.push_back(act);
                return true;
            }
        }
        false
    }

    /// Déverse les intentions script du moteur dans
    /// `pending_nav` (étape 53d) — les handlers consultent
    /// ensuite [`Self::nav_pending`] au lieu de conduire les
    /// navigations sous `&mut self`. Stoppe quand la file
    /// est pleine : le reste reste dans le moteur pour le
    /// prochain drain.
    pub(crate) fn collect_script_navs(&mut self, tab: TabId) {
        while let Some(n) = self.next_script_nav(tab) {
            let act = match n {
                ScriptNav::To(u) => DrainAct::Req(Box::new(Request::get(u)), true),
                ScriptNav::Go(d) => DrainAct::Go(d),
                ScriptNav::Submit(req) => DrainAct::Req(Box::new(req), true),
            };
            if !self.enqueue_nav_act(tab, act) {
                break;
            }
        }
    }

    /// Cible `(index, requête)` d'un déplacement
    /// d'historique — lecture pure pour les drivers partagés
    /// (53d) ; la requête d'origine est rejouée telle quelle
    /// hors bfcache (étape 56 — POST rejoué).
    pub(crate) fn hist_target(
        &self,
        tab: TabId,
        delta: isize,
    ) -> Result<(usize, Request), CoreError> {
        let entry = self.tabs.get(&tab).ok_or(CoreError::UnknownTab(tab))?;
        let Some(idx) = entry
            .hist_idx
            .checked_add_signed(delta)
            .filter(|&i| i < entry.hist.len())
        else {
            return Err(CoreError::EmptyDocument);
        };
        Ok((idx, entry.hist[idx].clone()))
    }

    /// Fixe l'index courant d'historique (commit côté driver
    /// partagé, 53d).
    pub(crate) fn set_hist_idx(&mut self, tab: TabId, idx: usize) {
        if let Some(e) = self.tabs.get_mut(&tab) {
            e.hist_idx = idx;
        }
    }

    /// La session a-t-elle des intentions de navigation en
    /// attente pour `tab` (input ou script) — sonde des
    /// drivers partagés (53d).
    #[must_use]
    pub fn nav_pending(&self, tab: TabId) -> bool {
        self.tabs
            .get(&tab)
            .is_some_and(|e| !e.pending_nav.is_empty())
    }

    /// Première intention de navigation script exploitable
    /// (`NavRequest` — étape 44, côté moteur) : `Navigate`
    /// résolu contre l'URL courante, `HistoryGo` en delta.
    /// Les intentions inapplicables (URL irrésolvable) sont
    /// consommées et sautées ; le reste de la file sera
    /// re-lu au tour suivant de [`drain_script_nav`].
    pub(crate) fn next_script_nav(&mut self, tab: TabId) -> Option<ScriptNav> {
        let reqs = self
            .tabs
            .get_mut(&tab)
            .and_then(|e| e.js.as_mut())
            .map_or_else(Vec::new, |e| e.drain_nav_requests());
        for r in reqs {
            match r {
                NavRequest::Navigate(href) => {
                    let url = self
                        .tabs
                        .get(&tab)
                        .and_then(|t| t.url.as_ref())
                        .and_then(|b| Url::resolve(b, &href).ok());
                    if let Some(u) = url {
                        return Some(ScriptNav::To(u));
                    }
                }
                NavRequest::HistoryGo(d) => {
                    // i32 → isize : lossless sur toutes les
                    // cibles réelles ; sinon MAX garantit le
                    // refus de borne dans `history_move`.
                    let delta = isize::try_from(d).unwrap_or(isize::MAX);
                    return Some(ScriptNav::Go(delta));
                }
                NavRequest::FormSubmit {
                    method,
                    action,
                    enctype,
                    fields,
                } => {
                    let base = self.tabs.get(&tab).and_then(|t| t.url.as_ref());
                    if let Some(req) = base
                        .and_then(|b| script_form_request(b, &method, &action, &enctype, &fields))
                    {
                        return Some(ScriptNav::Submit(req));
                    }
                }
            }
        }
        None
    }

    /// Drainage des intentions de navigation accumulées —
    /// input (`pending_nav`, étape 53d) puis script
    /// (`location`, `history`, `form.submit()`) — appelé
    /// après tout point de synchro producteur : événements,
    /// fin de chargement, tick timers (étape 52 — les hôtes
    /// la relancent après [`Self::pump_timers`]). Publique
    /// pour les pompes des hôtes `&mut self` (CLI, FFI,
    /// tests) — les hôtes à session partagée passent par
    /// [`drain_navs_shared`]. Boucle bornée par
    /// `MAX_NAV_HOPS`, jamais de récursion — chaque
    /// navigation est conduite par `navigate_request` en
    /// feuille du graphe d'appels (seul `history_move`,
    /// boxé, boucle pour la taille du future). `true` si au
    /// moins une navigation a été appliquée.
    pub async fn drain_script_nav(&mut self, tab: TabId) -> bool {
        const MAX_NAV_HOPS: u8 = 8;
        let mut applied = false;
        for _ in 0..MAX_NAV_HOPS {
            self.collect_script_navs(tab);
            let Some(act) = self.pop_pending_nav(tab) else {
                break;
            };
            match act {
                DrainAct::Req(req, push) => {
                    let _ = self.navigate_request(tab, *req, push).await;
                }
                DrainAct::Go(0) => {
                    // `history.go(0)` = rechargement de
                    // l'entrée courante (spec HTML).
                    if let Some(u) = self.tabs.get(&tab).and_then(|t| t.url.clone()) {
                        let _ = self.navigate_to(tab, u).await;
                    }
                }
                DrainAct::Go(d) => {
                    // Box : `history_move` appelle aussi
                    // `drain_script_nav` — récursion mutuelle
                    // cassée ici (taille + inférence `Send`).
                    let mv: std::pin::Pin<Box<dyn std::future::Future<Output = _> + Send + '_>> =
                        Box::pin(self.history_move(tab, d));
                    let _ = mv.await;
                }
            }
            applied = true;
        }
        applied
    }

    /// `NetRequest::Fetch` → `Request` émettable (étape 51),
    /// `None` = bloquée : URL irrésoluble, mixed-content,
    /// verbe inconnu, cross-origin non-safelistée (ADR-0019).
    /// `Referer`/`Origin` posés à l'émission — une requête
    /// CORS porte `Origin` même en verbe safelisté (spec
    /// Fetch) ; `Cookie` injecté same-origin uniquement
    /// (`credentials` cross-origin hors MVP).
    pub(crate) fn script_request(
        &self,
        page: &Url,
        method: &str,
        raw_url: &str,
        headers: &[(String, String)],
        body: Option<&str>,
    ) -> Option<(Request, bool)> {
        let url = Url::resolve(page, raw_url.trim()).ok()?;
        if is_mixed_content(page, &url) {
            return None;
        }
        let m = parse_method(method)?;
        let cross = page.origin() != url.origin();
        // CORS minimal : cross-origin limité aux requêtes
        // simples — en-tête non safelisté = preflight requis,
        // bloqué au MVP (déviation documentée).
        if cross
            && !(cors_simple_method(m) && headers.iter().all(|(n, _)| cors_safelisted_header(n)))
        {
            return None;
        }
        let mut req = Request {
            method: m,
            url: url.clone(),
            headers: initiator_headers(page, &url, m),
            body: body.map(|b| b.as_bytes().to_vec()),
        };
        if cross
            && !req
                .headers
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case("origin"))
        {
            req.headers
                .push(("origin".to_string(), origin_header_value(page)));
        }
        req.headers.extend(
            headers
                .iter()
                .filter(|(n, _)| !forbidden_request_header(n))
                .cloned(),
        );
        if !cross {
            let c = self.cookies.header_for(&url, true);
            if !c.is_empty() {
                req.headers.push(("Cookie".to_string(), c));
            }
        }
        Some((req, cross))
    }
}

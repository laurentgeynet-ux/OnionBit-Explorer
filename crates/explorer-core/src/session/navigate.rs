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
use explorer_net::http::{FetchDest, FetchMode, Initiator, Method, Request};
use explorer_net::policy::{initiator_headers, is_mixed_content, sec_fetch_headers};
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
        let mut req = Request::get(url);
        // Navigation explicite (barre d'URL, API) = top-level
        // sans page initiatrice — `SameSite` tout passe (66).
        req.initiator = Some(Initiator::TopLevelNav);
        match self.navigate_request(tab, req, true).await {
            Ok(_) => {
                self.drain_script_nav(tab).await;
                Ok(())
            }
            Err(e) => Err(self.emit_error(tab, e)),
        }
    }

    /// Requête qu'émettrait [`Self::reload`] — la dernière
    /// requête servie (`last_req`, un POST rejoue le POST,
    /// étape 56), la requête `failed` de la position
    /// courante (page d'erreur : retenter la requête tombée
    /// plutôt qu'un `GET` vide — étape 93), sinon un `GET`
    /// top-level sur l'URL courante. L'API s'en sert pour
    /// exiger un `confirm` avant de rejouer une méthode non
    /// sûre.
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`], [`CoreError::EmptyDocument`]
    /// si l'onglet n'a jamais été navigué.
    pub fn reload_request(&self, tab: TabId) -> Result<Request, CoreError> {
        let Some(t) = self.tabs.get(&tab) else {
            return Err(CoreError::UnknownTab(tab));
        };
        if let Some(req) = t.last_req.clone().or_else(|| {
            t.hist
                .get(t.hist_idx)
                .filter(|h| h.failed)
                .map(|h| h.req.clone())
        }) {
            return Ok(req);
        }
        let Some(u) = &t.url else {
            return Err(CoreError::EmptyDocument);
        };
        // Reload direct = navigation top-level (66).
        let mut r = Request::get(u.clone());
        r.initiator = Some(Initiator::TopLevelNav);
        Ok(r)
    }

    /// Recharge la page courante (re-fetch complet,
    /// historique inchangé) — réémet la requête d'origine
    /// `last_req` : une page servie par POST est re-POSTée
    /// (étape 56 — les navigateurs réémettent la requête,
    /// quitte à confirmer la resoumission côté UI — l'API
    /// exige `confirm`, étape 93).
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`], [`CoreError::EmptyDocument`]
    /// si l'onglet n'a jamais été navigué, ou erreur de fetch.
    pub async fn reload(&mut self, tab: TabId) -> Result<(), CoreError> {
        let req = self.reload_request(tab)?;
        match self.navigate_request(tab, req, false).await {
            Ok(_) => {
                // Reload réussi sur une page d'erreur : la
                // position courante n'est plus en échec
                // (étape 76).
                let idx = self.tabs.get(&tab).map(|e| e.hist_idx);
                if let Some(i) = idx {
                    self.mark_hist_ok(tab, i);
                }
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
        // Une entrée en échec n'est jamais restaurée
        // (étape 76) — sa « page d'erreur » n'est pas un
        // snapshot : la traversée retente la requête.
        if entry.hist.get(idx).is_some_and(|h| h.failed) {
            return false;
        }
        let prev = entry.title.clone();
        let Some(snap) = entry.take_snapshot(idx) else {
            return false;
        };
        // La page quittée gèle son état affiché — une
        // peinture spéculative en vol ne doit pas être
        // embarquée (étape 56c).
        self.unspec(tab);
        let cur_failed = self
            .tabs
            .get(&tab)
            .is_some_and(|e| e.hist.get(e.hist_idx).is_some_and(|h| h.failed));
        let cur_pending = self.tabs.get(&tab).is_some_and(|e| e.hist_pending);
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return false;
        };
        // Une page d'erreur quittée ne gèle pas (étape 76) :
        // son slot servirait une erreur périmée à la
        // traversée — la retente est la sémantique voulue.
        // Une page en vol ne gèle pas non plus (étape 84) :
        // sans entrée d'historique elle n'a pas de slot —
        // la geler sous `hist_idx` empoisonnerait la page
        // quittée (son snapshot vient d'être retiré).
        if !cur_failed && !cur_pending {
            entry.freeze(self.bfcache_pages);
        }
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
        // La page quittée (gelée à l'instant ou page
        // d'erreur non gelable) meurt — ses surfaces média
        // sont closes ; le snapshot restauré n'en porte
        // pas, `refresh` ré-offrira ids neufs (71b).
        self.close_media(tab);
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
            e.hist.push(HistEntry {
                req: req.clone(),
                failed: false,
            });
            e.hist_idx = e.hist.len() - 1;
            // L'entrée fragment résout la position — la page
            // affichée a désormais son entrée, `hist_pending`
            // tombe. La nav en vol poussera quand même la
            // sienne à son commit (au-dessus, futur tronqué).
            e.hist_pending = false;
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
    /// Retour : `true` si la position a bougé (repaint fait) —
    /// l'appelant peut repeindre lui-même sinon quand il a un
    /// changement de frame indépendant du déplacement (find,
    /// étape 68).
    pub(crate) fn scroll_to_y(&mut self, tab: TabId, y: f32) -> Result<bool, CoreError> {
        {
            let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            let max = (entry.doc_h - self.viewport.1).max(0.0);
            let next = y.clamp(0.0, max);
            if (next - entry.scroll_y).abs() < f32::EPSILON {
                return Ok(false);
            }
            entry.scroll_y = next;
        }
        self.repaint(tab)?;
        self.emit_invalidated(tab);
        Ok(true)
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
        // Nav `push` en vol : le futur va être tronqué au
        // commit — il n'y a plus d'entrée « forward » (84).
        if self.tabs.get(&tab).is_some_and(|e| e.hist_pending) {
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
            .is_some_and(|e| e.hist.get(fwd).is_some_and(|r| r.req.url == req.url));
        matches && self.try_bfcache(tab, fwd)
    }

    /// Déplacement dans l'historique — `delta` = ±1.
    pub(crate) async fn history_move(&mut self, tab: TabId, delta: isize) -> Result<(), CoreError> {
        let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
        // Nav `push` en vol (URL commise, entrée pas encore
        // poussée) : la page affichée est logiquement une place
        // au-delà — `back` vise alors `hist_idx`, la page
        // quittée (étape 84).
        let pos = entry.hist_idx + usize::from(entry.hist_pending);
        let next = pos.checked_add_signed(delta);
        let Some(idx) = next.filter(|&i| i < entry.hist.len()) else {
            return Err(CoreError::EmptyDocument);
        };
        let mut req = entry.hist[idx].req.clone();
        // Back/forward = navigation top-level pilotée par le
        // navigateur — le contexte initiateur historié ne
        // s'applique plus (étape 66).
        req.initiator = Some(Initiator::TopLevelNav);
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
                    // Rejeu d'une entrée `failed` ayant abouti :
                    // la position héberge une vraie page —
                    // bfcache redevient éligible (étape 76).
                    if let Some(h) = entry.hist.get_mut(idx) {
                        h.failed = false;
                    }
                }
                // Pas de drain ici : les appelants publics
                // (`go_back/forward`) le font après — sinon
                // `drain_script_nav` → `history_move` →
                // `drain_script_nav` bouclerait l'inférence.
                Ok(())
            }
            Err(e) => {
                // La page d'erreur est commise à la position de
                // l'entrée — la traversée a « atterri » (étape 76 :
                // back depuis l'erreur revient à la source, pas à
                // l'entrée d'avant).
                if let Some(entry) = self.tabs.get_mut(&tab) {
                    entry.hist_idx = idx;
                }
                Err(self.emit_error(tab, e))
            }
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
        let t_nav = std::time::Instant::now();
        tracing::info!(tab, "nav: fetch+parse…");
        let (done, tail) = match job
            .run(|dom: &Dom| -> PaintFut<'_> {
                self.nav_paint(tab, epoch, dom);
                Box::pin(async {})
            })
            .await
        {
            // Téléchargement (étape 70) : corps bufferisé
            // proposé à l'UI — rien n'est committé, la page
            // courante reste en place.
            Ok(NavOutcome::Download(dl)) => {
                self.offer_download(tab, dl);
                return Ok(None);
            }
            Ok(NavOutcome::Document(d, t)) => (d, t),
            Err(e) => {
                // Le document en vol a pu être peint
                // progressivement — on réverte avant de
                // propager (étape 56c), puis la page
                // d'erreur est commise (étape 69 : 404/5xx,
                // fetch en échec, document vide — plus de
                // vide, `can_back` préservé).
                self.unspec(tab);
                self.paint_error_page(tab, &job.req, job.push, &e);
                return Err(e);
            }
        };
        // Erreurs post-fetch (pose DOM, commit) : la couverture
        // de l'étape 69 s'étend — la page d'erreur est peinte
        // avec l'URL effectivement naviguée (étape 75), et
        // l'entrée d'historique en échec est poussée (76).
        let err_req = {
            let mut r = tail.req.clone();
            r.url = done.url.clone();
            r
        };
        let push = tail.push;
        tracing::info!(
            tab,
            ms = u64::try_from(t_nav.elapsed().as_millis()).unwrap_or(u64::MAX),
            "nav: fetch+parse ok → peinture"
        );
        let mut sub = match self.nav_apply(tab, tail, done) {
            Ok(Some(s)) => s,
            Ok(None) => return Ok(None),
            Err(e) => {
                self.paint_error_page(tab, &err_req, push, &e);
                return Err(e);
            }
        };
        tracing::info!(
            tab,
            ms = u64::try_from(t_nav.elapsed().as_millis()).unwrap_or(u64::MAX),
            "nav: peinture ok → sous-ressources"
        );
        sub.run().await;
        tracing::info!(
            tab,
            ms = u64::try_from(t_nav.elapsed().as_millis()).unwrap_or(u64::MAX),
            "nav: sous-ressources ok → commit"
        );
        match self.sub_commit(tab, sub).await {
            Ok(t) => Ok(t),
            Err(e) => {
                self.paint_error_page(tab, &err_req, push, &e);
                Err(e)
            }
        }
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
        mut req: Request,
        push: bool,
    ) -> Result<NavJob, CoreError> {
        // Défaut « site pour cookies » (étape 66) : une nav
        // sans initiateur posé est initiée par la page
        // courante (lien, `location`, script-nav) ; onglet
        // vierge = navigation top-level.
        if req.initiator.is_none() {
            req.initiator = Some(
                self.tabs
                    .get(&tab)
                    .and_then(|t| t.url.clone())
                    .map_or(Initiator::TopLevelNav, Initiator::NavFrom),
            );
        }
        // Métadonnées `Sec-Fetch-*` de navigation (étape 125) :
        // `dest=document`/`mode=navigate` dérivés ici ;
        // `TopLevelNav` = geste utilisateur implicite (saisie,
        // favori, reload, back/forward) ; `NavFrom` ne porte
        // `?1` que si l'émetteur l'a marqué (clic/submit vs
        // `location.href` — `sec-fetch_headers` n'émet `User`
        // que sur navigation marquée).
        req.fetch_dest = FetchDest::Document;
        if matches!(req.initiator, Some(Initiator::TopLevelNav)) {
            req.fetch_user = true;
        }
        if !req
            .headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("sec-fetch-site"))
        {
            req.headers.extend(sec_fetch_headers(&req));
        }
        let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
        entry.nav_epoch += 1;
        // Un id de téléchargement est réservé par navigation
        // (étape 70) : la progression hors verrou porte l'id
        // final dès le premier morceau — non-consécutivité
        // volontaire et documentée.
        let dl_id = self.next_download_id;
        self.next_download_id += 1;
        Ok(NavJob {
            tab,
            epoch: entry.nav_epoch,
            req,
            push,
            prev_title: entry.title.clone(),
            transport: Arc::clone(&self.transport),
            paint_chunk_bytes: self.paint_chunk_bytes,
            paint_interval: self.paint_interval,
            events: self.events.clone(),
            max_download_bytes: self.max_download_bytes,
            next_download_id: dl_id,
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
        let doc_sheets = collect_style_sheets(&dom);
        // Issue #6 : les `<script type="importmap">` pilotent le
        // prefetch des bare specifiers (`import_specs` — même
        // réécriture que le moteur) et sont poussés au moteur
        // avant le premier `eval_module` (`import_maps`).
        let import_maps = if scripting {
            collect_import_maps(&dom)
        } else {
            Vec::new()
        };
        let import_specs = merge_import_maps(&import_maps);
        // JS-C45 : les handlers inline `on*` exigent un moteur
        // même sans `<script>` — sinon clic muet (page à
        // `onclick=` seul : premier bug site-réel mesuré M24).
        let want_js = !scripts.is_empty() || has_inline_handlers(&dom);
        // Le gel capture l'état affiché — une peinture
        // spéculative du document en vol ne doit pas être
        // embarquée dans le snapshot (étape 56c).
        self.unspec(tab);
        // La page quittée meurt : ses surfaces média sont
        // closes avant le gel — le snapshot ne les porte
        // pas, la restauration ré-offre ids neufs (71b).
        self.close_media(tab);
        {
            let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            // bfcache (étape 56) : la page quittée est gelée
            // à sa position d'historique — un retour
            // arrière la restaurera sans réseau. Une page
            // d'erreur (entrée `failed`, étape 76) n'est pas
            // gelée : sa traversée retente la requête. Ni
            // une page **en vol** (étape 84) : sans entrée
            // elle n'a pas de slot — la geler sous
            // `hist_idx` écraserait le snapshot de la page
            // d'avant avec le document entrant.
            if !entry.hist_pending && !entry.hist.get(entry.hist_idx).is_some_and(|h| h.failed) {
                entry.freeze(self.bfcache_pages);
            }
            // Fenêtre « URL commise, entrée pas poussée »
            // (étape 84) : `sub_commit` poussera après la
            // vague de sous-ressources — `hist_target`
            // compte la page affichée comme position
            // `hist_idx + 1` jusque-là.
            entry.hist_pending = tail.push;
            entry.last_req = Some(tail.req.clone());
            entry.url = Some(url.clone());
            entry.title.clone_from(&title);
            entry.scroll_y = 0.0;
            entry.focused = None;
            entry.focused_key = None;
            entry.hovered = None;
            entry.deferred_inputs.clear();
            entry.popup = None;
            entry.find = None;
            // Les intentions en file appartiennent au document
            // mort (clic lien non drainé, reste >MAX_NAV_HOPS)
            // — comme `restore()` : elles ne doivent pas tirer
            // après le commit (terrain 23/10 : une intention
            // périmée rejouait une autre page au drain suivant).
            entry.pending_nav.clear();
            entry.js = None;
            entry.images = PaintImages::default();
            entry.bg_aliases.clear();
            entry.bg_bodies.clear();
            entry.mask_aliases.clear();
            entry.img_decoded.clear();
            entry.bg_decoded.clear();
            entry.linked_sheets.clear();
            entry.prev_styles = HashMap::new();
            entry.transitions.clear();
            entry.sync_floor = None;
            entry.dom = Some(dom);
        }
        if let Ok(mut wf) = self.webfonts.lock() {
            wf.clear();
        }
        let eng = self.spawn_engine(tab, &html, want_js);
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
            import_maps,
            import_specs,
            doc_sheets,
            eng,
            css_slots: Vec::new(),
            script_srcs: Vec::new(),
            img_bodies: HashMap::new(),
            bg_aliases: HashMap::new(),
            bg_bodies: HashMap::new(),
            mask_aliases: HashMap::new(),
            async_mods: Vec::new(),
            fonts: Vec::new(),
            arrival_evals: Vec::new(),
            post_evals: Vec::new(),
            viewport: self.viewport,
            color_scheme: self.color_scheme,
            max_inflight: self.max_inflight,
            max_modules: self.max_modules,
            font_budget: self.max_fonts,
            max_imports: self.max_imports,
            max_images: self.max_images,
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
                ScriptNav::Submit(req) => DrainAct::Req(req, true),
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
        // Même correction que `history_move` (étape 84) :
        // la page affichée par une nav `push` en vol compte
        // pour une place — `back` retombe sur `hist_idx`,
        // la page quittée, au lieu d'échouer `EmptyDocument`.
        let pos = entry.hist_idx + usize::from(entry.hist_pending);
        let Some(idx) = pos
            .checked_add_signed(delta)
            .filter(|&i| i < entry.hist.len())
        else {
            return Err(CoreError::EmptyDocument);
        };
        Ok((idx, entry.hist[idx].req.clone()))
    }

    /// Fixe l'index courant d'historique (commit côté driver
    /// partagé, 53d).
    pub(crate) fn set_hist_idx(&mut self, tab: TabId, idx: usize) {
        if let Some(e) = self.tabs.get_mut(&tab) {
            e.hist_idx = idx;
        }
    }

    /// Rejeu réussi (étape 76) : l'entrée `idx` — traversée ou
    /// reload d'une page d'erreur — héberge désormais une vraie
    /// page ; le marqueur `failed` tombe et le bfcache redevient
    /// éligible pour elle.
    pub(crate) fn mark_hist_ok(&mut self, tab: TabId, idx: usize) {
        if let Some(e) = self.tabs.get_mut(&tab) {
            if let Some(h) = e.hist.get_mut(idx) {
                h.failed = false;
            }
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
                        return Some(ScriptNav::Submit(Box::new(req)));
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
    /// Draine `pending_nav` — renvoie le nombre de
    /// navigations **commises** (étape 64 : les appelants
    //  distinguent intention et commit effectif — un fetch
    /// mort ne rapporte pas « navigated »).
    pub async fn drain_script_nav(&mut self, tab: TabId) -> usize {
        const MAX_NAV_HOPS: u8 = 8;
        let mut committed = 0usize;
        for _ in 0..MAX_NAV_HOPS {
            self.collect_script_navs(tab);
            let Some(act) = self.pop_pending_nav(tab) else {
                break;
            };
            let ok = match act {
                DrainAct::Req(req, push) => self.navigate_request(tab, *req, push).await.is_ok(),
                DrainAct::Go(0) => {
                    // `history.go(0)` = rechargement de
                    // l'entrée courante (spec HTML).
                    if let Some(u) = self.tabs.get(&tab).and_then(|t| t.url.clone()) {
                        self.navigate_to(tab, u).await.is_ok()
                    } else {
                        false
                    }
                }
                DrainAct::Go(d) => {
                    // Box : `history_move` appelle aussi
                    // `drain_script_nav` — récursion mutuelle
                    // cassée ici (taille + inférence `Send`).
                    let mv: std::pin::Pin<Box<dyn std::future::Future<Output = _> + Send + '_>> =
                        Box::pin(self.history_move(tab, d));
                    mv.await.is_ok()
                }
            };
            committed += usize::from(ok);
        }
        committed
    }

    /// `NetRequest::Fetch` → `Request` émettable (étape 51),
    /// `None` = bloquée : URL irrésoluble, mixed-content,
    /// verbe inconnu, `mode:same-origin` sur cible cross
    /// (ADR-0019). `Referer`/`Origin` posés à l'émission — une
    /// requête CORS porte `Origin` même en verbe safelisté
    /// (spec Fetch). `fetch_mode`/`credentials` viennent du
    /// script (JS-C77) : `no-cors` cross = émission bornée
    /// safelist, réponse opaque, pas de protocole CORS.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn script_request(
        &self,
        page: &Url,
        method: &str,
        raw_url: &str,
        headers: &[(String, String)],
        body: Option<&str>,
        fetch_mode: explorer_js::FetchMode,
        credentials: explorer_js::FetchCredentials,
    ) -> Option<(Request, bool)> {
        let url = Url::resolve(page, raw_url.trim()).ok()?;
        if is_mixed_content(page, &url) {
            return None;
        }
        let m = parse_method(method)?;
        let cross = page.origin() != url.origin();
        // JS-C77 — miroir `fetch(mode)`/`xhr.withCredentials` →
        // `Request.fetch_mode`/`CorsRequest.credentials` (types
        // jscore → enums net : même lexique, traduction pure).
        let mode = match fetch_mode {
            explorer_js::FetchMode::Cors => FetchMode::Cors,
            explorer_js::FetchMode::NoCors => FetchMode::NoCors,
            explorer_js::FetchMode::SameOrigin => FetchMode::SameOrigin,
        };
        let creds = match credentials {
            explorer_js::FetchCredentials::Omit => explorer_net::http::FetchCredentials::Omit,
            explorer_js::FetchCredentials::SameOrigin => {
                explorer_net::http::FetchCredentials::SameOrigin
            }
            explorer_js::FetchCredentials::Include => explorer_net::http::FetchCredentials::Include,
        };
        // `same-origin` sur cible cross → bloquée (spec ; le
        // miroir rejette déjà en `TypeError` avant émission —
        // garde défensive pour tout appelant non-miroir).
        if mode == FetchMode::SameOrigin && cross {
            return None;
        }
        // `no-cors` : en-têtes auteur bornés à la safelist
        // (défense — le miroir strippe déjà à l'émission) ;
        // hors safelist = simplement omis, jamais de preflight.
        let author_headers: Vec<(String, String)> = if mode == FetchMode::NoCors {
            headers
                .iter()
                .filter(|(n, v)| {
                    cors_safelisted_header(n)
                        && (!n.trim().eq_ignore_ascii_case("content-type")
                            || explorer_net::policy::cors_safelisted_content_type(
                                v.split(';').next().unwrap_or("").trim(),
                            ))
                })
                .cloned()
                .collect()
        } else {
            headers.to_vec()
        };
        let mut req = Request {
            method: m,
            url: url.clone(),
            headers: initiator_headers(page, &url, m),
            body: body.map(|b| b.as_bytes().to_vec()),
            // `fetch`/`XHR` = sous-ressource de la page —
            // `SameSite=Strict`/`Lax` exigent same-site (66).
            initiator: Some(Initiator::Subresource(page.clone())),
            fetch_dest: FetchDest::Empty,
            fetch_mode: mode,
            fetch_user: false,
            cors: None,
        };
        if cross && mode == FetchMode::Cors {
            // Enforcement CORS (étape 126) : les en-têtes
            // auteur non safelistés (ou `Content-Type` hors
            // types « form ») déclenchent le preflight
            // `OPTIONS` du `CorsTransport` — plus bloqués à
            // l'émission (MVP documenté remplacé).
            let mut unsafe_headers: Vec<String> = headers
                .iter()
                .filter(|(n, _)| !forbidden_request_header(n))
                .filter(|(n, v)| {
                    !cors_safelisted_header(n)
                        || (n.trim().eq_ignore_ascii_case("content-type")
                            && !explorer_net::policy::cors_safelisted_content_type(
                                v.split(';').next().unwrap_or("").trim(),
                            ))
                })
                .map(|(n, _)| n.trim().to_ascii_lowercase())
                .collect();
            unsafe_headers.sort();
            unsafe_headers.dedup();
            req.cors = Some(explorer_net::http::CorsRequest {
                origin: origin_header_value(page),
                // `credentials` du script (JS-C77 — `omit`/
                // `same-origin`/`include` honorés).
                credentials: creds,
                unsafe_headers,
            });
        }
        req.headers.extend(sec_fetch_headers(&req));
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
            author_headers
                .iter()
                .filter(|(n, _)| !forbidden_request_header(n))
                .cloned(),
        );
        // `credentials` du script (JS-C77) : `omit` ne pose ni
        // ne capture jamais de cookies ; `include` envoie sous
        // réserve `SameSite` — y compris sur `no-cors` cross
        // (`sendBeacon`, réponse opaque : l'agent traite le
        // `Set-Cookie`, le script ne lit rien).
        let include = creds == explorer_net::http::FetchCredentials::Include;
        let omit = creds == explorer_net::http::FetchCredentials::Omit;
        if !omit && (!cross || include) {
            let c = self.cookies.header_for_ctx(&req, true);
            if !c.is_empty() {
                req.headers.push(("Cookie".to_string(), c));
            }
        }
        Some((req, cross))
    }
}

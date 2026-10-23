// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `impl Session` réseau : commits de sous-ressources,
//! livraisons JS, pompe de transports.
use super::*;
use apply::PaintPayload;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use explorer_dom::Dom;
use explorer_js::{NetRequest, NetResponse};
use explorer_net::http::HttpTransport;
use explorer_net::policy::is_mixed_content;
use explorer_net::url::Url;
use explorer_render::{decode_image, DecodedImage};

impl Session {
    /// Le commit complet sous `&mut self` (hôtes synchrones :
    /// CLI, FFI, tests) = tête + rendu inline + queue. Les
    /// hôtes partagés appellent les trois morceaux à part et
    /// exécutent le rendu détaché hors verrou (étape 90).
    /// `None` = epoch supplantée, tout est jeté.
    pub(crate) async fn sub_commit(
        &mut self,
        tab: TabId,
        job: SubJob,
    ) -> Result<Option<String>, CoreError> {
        let Some(mut tail) = self.sub_commit_head(tab, job).await? else {
            return Ok(None);
        };
        self.insert_images(tab, &tail.url, &tail.img_bodies);
        self.stage_backgrounds(tab, &mut tail);
        self.relayout(tab)?;
        Ok(self.sub_commit_tail(tab, tail))
    }

    /// Fonds `url(http…)` fetchés pendant la vague (étape 98)
    /// posés sur l'onglet avant le relayout — l'insertion
    /// dans `images.backgrounds` attend la cascade (le
    /// `NodeId` propriétaire n'est connu qu'au style calculé).
    /// `mask_aliases` suit le même rail (étape 121 — ses
    /// corps sont partagés dans `bg_bodies`). `take_paint`
    /// les fera voyager avec la payload.
    fn stage_backgrounds(&mut self, tab: TabId, tail: &mut SubCommitTail) {
        if tail.bg_bodies.is_empty() {
            return;
        }
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        entry.bg_aliases = std::mem::take(&mut tail.bg_aliases);
        entry.bg_bodies = std::mem::take(&mut tail.bg_bodies);
        entry.mask_aliases = std::mem::take(&mut tail.mask_aliases);
    }

    /// Tête du commit — sous `&mut self` : garde epoch,
    /// feuilles liées, fontes, évaluations JS (scripts
    /// parse/post, modules, `run_pending`, resynchro miroir),
    /// réinsertion du moteur, drains post-script et pompe
    /// réseau. Le rendu (`insert_images` + `relayout`) n'y
    /// figure pas : il peut coûter des dizaines de secondes
    /// sur une cascade de terrain — les hôtes partagés le
    /// détachent (étape 90). `Ok(None)` = epoch supplantée.
    pub(crate) async fn sub_commit_head(
        &mut self,
        tab: TabId,
        job: SubJob,
    ) -> Result<Option<SubCommitTail>, CoreError> {
        {
            let entry = self.tabs.get(&tab).ok_or(CoreError::UnknownTab(tab))?;
            if entry.nav_epoch != job.epoch {
                self.unspec(tab);
                return Ok(None);
            }
        }
        let SubJob {
            url,
            push,
            prev_title,
            req,
            title,
            css_slots,
            scripts,
            script_srcs,
            img_bodies,
            bg_aliases,
            bg_bodies,
            mask_aliases,
            mut eng,
            fonts,
            arrival_evals,
            post_evals,
            import_maps,
            ..
        } = job;
        // Feuilles liées dans l'ordre document (slot vide =
        // feuille en échec → absente de la cascade). Une
        // entrée par feuille (étape 104) : chaque feuille est
        // parsée isolément à la cascade.
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.linked_sheets = css_slots.into_iter().flatten().collect();
        }
        if let Ok(mut wf) = self.webfonts.lock() {
            for (family, weight, italic, data) in fonts {
                wf.register(&family, weight, italic, data);
            }
        }
        if let Some(eng) = eng.as_mut() {
            // Issue #6 : les import maps arment la résolution des
            // bare specifiers — la spec les veut enregistrées avant
            // le premier module évalué (le prefetch, lui, a déjà
            // réécrit les spécificateurs via `import_specs`).
            for json in &import_maps {
                eng.set_import_map(json);
            }
            for item in scripts.iter().filter(|i| i.exec == ScriptExec::Parse) {
                let Some(src) = item_source(item, &script_srcs) else {
                    continue;
                };
                let url = item.src.as_ref().map(Url::as_str);
                self.eval_or_report(tab, eng.as_mut(), src, url.as_deref());
            }
            for me in arrival_evals {
                self.eval_module_graph(tab, eng.as_mut(), me);
            }
            let mut post_evals = post_evals.into_iter();
            for item in scripts.iter().filter(|i| i.exec == ScriptExec::Post) {
                if item.module {
                    if let Some(me) = post_evals.next() {
                        self.eval_module_graph(tab, eng.as_mut(), me);
                    }
                } else if let Some(src) = item_source(item, &script_srcs) {
                    let url = item.src.as_ref().map(Url::as_str);
                    self.eval_or_report(tab, eng.as_mut(), src, url.as_deref());
                }
            }
            let n = eng.run_pending();
            tracing::trace!(tab, jobs = n, "jobs js exécutés");
            // Miroir muté ? → re-parse : le relayout qui suit
            // rend les modifications (dirty bit consommé côté
            // moteur).
            if let Some(k) = eng.sync_dom_keyed() {
                self.apply_dom_sync(tab, &k.html, None, false, Some(k.ids), Some(k.tags));
            } else if let Some((mutated, _)) = eng.sync_dom_path() {
                self.apply_dom_sync(tab, &mutated, None, false, None, None);
            }
        }
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.js = eng;
        }
        // Étape 51 : XHR/fetch émis pendant le chargement →
        // spawned ; une pompe livre les réponses déjà là,
        // l'hôte pompe périodiquement pour le reste.
        self.after_script(tab);
        self.pump_net().await;
        Ok(Some(SubCommitTail {
            url,
            push,
            prev_title,
            req,
            title,
            img_bodies,
            bg_aliases,
            bg_bodies,
            mask_aliases,
        }))
    }

    /// Queue du commit — après le rendu (inline ou détaché) :
    /// historique, `Navigated`, persistance des cookies et
    /// scroll de fragment. Renvoie le titre extrait au parse.
    pub(crate) fn sub_commit_tail(&mut self, tab: TabId, tail: SubCommitTail) -> Option<String> {
        let SubCommitTail {
            url,
            push,
            prev_title,
            req,
            title,
            ..
        } = tail;
        // La page chargée prend sa place dans l'historique
        // AVANT le drain script : un `history.go` en vol la
        // voit (index réel), une redirection script pousse sa
        // propre entrée après la sienne.
        if push {
            if let Some(entry) = self.tabs.get_mut(&tab) {
                entry.hist.truncate(entry.hist_idx + 1);
                // La **requête** complète est historiée
                // (étape 56) : un POST de formulaire devient
                // une position rejouable — pas de dédup
                // d'URL, le résultat est une page à part
                // entière comme chez les navigateurs.
                entry.hist.push(HistEntry {
                    req: req.clone(),
                    failed: false,
                });
                entry.hist_idx = entry.hist.len() - 1;
                // Fenêtre « en vol » refermée (étape 84) :
                // la page commise a désormais son entrée.
                entry.hist_pending = false;
            }
        }
        // Cycle de vie document (issue #11) : les scripts
        // différés/modules ont couru dans `sub_commit_head` —
        // `DOMContentLoaded` se dispatche au commit, avant
        // `Navigated`. Les mutations de ses listeners sont
        // enfilées puis appliquées au prochain drain — même
        // voie que toute mutation script (le drain n'est pas
        // forcé ici : sous hôte partagé il tournerait sous
        // verrou, exactement ce que l'étape 55h a retiré).
        self.dispatch_doc_event(tab, "DOMContentLoaded");
        // `Navigated` seulement si le document final est bien
        // celui demandé : une redirection script a déjà émis
        // pour sa cible. Le titre annoncé = `entry.title` —
        // les scripts ont pu le muter par `document.title=`
        // (`apply_dom_sync` le tient à jour) : le titre extrait
        // au parse serait alors périmé et ré-émettrait
        // l'ancien (52b).
        let final_url = self.tabs.get(&tab).and_then(|t| t.url.clone());
        if final_url.as_ref() == Some(&url) {
            let live = self.tabs.get(&tab).and_then(|t| t.title.clone());
            self.emit_navigated(tab, &url, live, prev_title.as_deref());
        }
        // Les `Set-Cookie` du document et des sous-ressources
        // ont enrichi le jar partagé (CookieTransport agit
        // hors de la vue session) — checkpoint de
        // persistance (étape 56).
        self.persist_cookies();
        // Fragment cross-document (étape 56d) : la page est
        // posée — scrolle vers `id`/`name` si l'URL en porte.
        if let Some(frag) = url.fragment() {
            let frag = frag.to_string();
            self.scroll_to_fragment(tab, &frag);
        }
        // `load` : tout est commis (sous-ressources, peinture,
        // historique) — dernier événement du cycle de vie.
        self.dispatch_doc_event(tab, "load");
        title
    }

    /// Draine les `NetRequest` du moteur (étape 51) : chaque
    /// `Fetch` admis devient une tâche tokio qui livrera sa
    /// réponse sur le canal (`pump_net` aux points de
    /// synchro) ; `Abort` tue la tâche corrélée. Quota
    /// `max_script_net` et requêtes bloquées → livraison
    /// immédiate d'un échec `status: 0` (`error` côté
    /// script — la page reste vivante, jamais de panic).
    pub(crate) fn drain_script_net(&mut self, tab: TabId) {
        let Some((page, reqs)) = self.tabs.get_mut(&tab).and_then(|e| {
            e.js.as_mut()
                .map(|eng| (e.url.clone(), eng.drain_net_requests()))
        }) else {
            return;
        };
        let Some(page) = page else { return };
        let mut failed = Vec::new();
        for r in reqs {
            match r {
                NetRequest::Fetch {
                    handle,
                    method,
                    url,
                    headers,
                    body,
                } => {
                    let inflight = self.net_tasks.keys().filter(|(t, _)| *t == tab).count();
                    let built = if inflight >= self.max_script_net {
                        tracing::trace!(tab, handle, "quota requêtes script atteint");
                        None
                    } else {
                        self.script_request(&page, &method, &url, &headers, body.as_deref())
                    };
                    let Some((req, cross)) = built else {
                        failed.push(handle);
                        continue;
                    };
                    let tr = self.transport.clone();
                    let tx = self.net_tx.clone();
                    let page_origin = origin_header_value(&page);
                    let req_url = req.url.clone();
                    let credentials = req
                        .cors
                        .as_ref()
                        .map_or(explorer_net::http::FetchCredentials::SameOrigin, |c| {
                            c.credentials
                        });
                    let j = tokio::spawn(async move {
                        let out = tr.fetch(&req).await;
                        let _ = tx.send(NetDone {
                            tab,
                            handle,
                            cross,
                            credentials,
                            page_origin,
                            req_url,
                            out,
                        });
                    });
                    self.net_tasks.insert((tab, handle), j);
                }
                NetRequest::Abort { handle } => {
                    if let Some(j) = self.net_tasks.remove(&(tab, handle)) {
                        j.abort();
                    }
                }
            }
        }
        // Échecs immédiats (bloquée/quota) : livrés comme un
        // échec réseau — déclenche `error`, jamais de panic.
        for handle in failed {
            self.deliver_net_response(
                tab,
                NetResponse {
                    handle,
                    status: 0,
                    status_text: String::new(),
                    headers: Vec::new(),
                    body: Vec::new(),
                },
            );
        }
    }

    /// Livre une `NetResponse` au moteur puis applique le
    /// tour post-script : microtasks, resynchro DOM +
    /// relayout, drains (réseau chaîné, cookies, storage).
    pub(crate) fn deliver_net_response(&mut self, tab: TabId, resp: NetResponse) {
        let mutated = {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return;
            };
            // Moteur sorti pour exécution hors verrou (53f) —
            // la réponse est tamponnée et livrée à sa
            // réinsertion ; la jeter ici pendrait le `fetch`/
            // XHR émetteur à jamais.
            if entry.js_detached.is_some() {
                entry.js_pending_net.push(resp);
                return;
            }
            let Some(eng) = entry.js.as_mut() else {
                return;
            };
            if let Err(e) = eng.deliver_net_response(resp) {
                tracing::debug!(tab, error = %e, "livraison réseau js");
                let _ = self.events.send(CoreEvent::PipelineError {
                    tab: Some(tab),
                    message: e.to_string(),
                });
            }
            let n = eng.run_pending();
            tracing::trace!(tab, jobs = n, "jobs js après réponse réseau");
            Self::sync_for_apply(eng.as_mut())
        };
        if let Some(s) = mutated {
            self.queue_apply_sync(tab, s);
        }
        self.after_script(tab);
    }

    /// Points de synchro post-script (étape 51) : draine les
    /// écritures `document.cookie` vers le jar partagé
    /// (`HttpOnly` filtré — non posable par un script), le
    /// `localStorage` vers la map d'origine + le profil, et
    /// les nouvelles requêtes réseau vers le scheduler.
    pub(crate) fn after_script(&mut self, tab: TabId) {
        let Some((page, writes, map)) = self.tabs.get_mut(&tab).and_then(|e| {
            e.js.as_mut().map(|eng| {
                (
                    e.url.clone(),
                    eng.drain_cookie_writes(),
                    eng.drain_storage(),
                )
            })
        }) else {
            return;
        };
        if let Some(page) = page {
            for w in &writes {
                self.cookies.set_from_header(&page, &strip_http_only(w));
            }
            if !writes.is_empty() {
                self.persist_cookies();
            }
            let key = page.origin().to_string();
            // Persisté seulement sur changement réel : une
            // map vidée par script est sauvée (effacement),
            // un contexte qui n'a jamais touché storage ne
            // produit aucune écriture.
            let changed = match self.local_storage.get(&key) {
                Some(prev) => *prev != map,
                None => !map.is_empty(),
            };
            if changed {
                self.local_storage.insert(key.clone(), map.clone());
                if let Some(s) = &self.store {
                    s.save_storage(&key, &map);
                }
            }
        }
        self.drain_script_net(tab);
        self.drain_media_commands(tab);
    }

    /// Livre les réponses arrivées — synchrone, sans yield :
    /// appelée aux points de synchro internes (événements)
    /// **et** par les hôtes partagés — les resynchros DOM
    /// rapportées sont enfilées (étape 55h), à drainer via
    /// `drain_applies_shared` hors verrou ou `drain_applies`
    /// sous `&mut self`.
    pub fn poll_net(&mut self) -> usize {
        let mut done = Vec::new();
        while let Ok(d) = self.net_rx.try_recv() {
            done.push(d);
        }
        let n = done.len();
        for d in done {
            self.net_tasks.remove(&(d.tab, d.handle));
            // `Set-Cookie` capturé : same-origin sauf
            // `credentials:omit` ; cross-origin sous `include`
            // seulement (étape 126 — le transport a déjà exigé
            // `ACAC: true` pour qu'elle soit lisible).
            let cookie_ok = match d.credentials {
                explorer_net::http::FetchCredentials::Omit => false,
                explorer_net::http::FetchCredentials::SameOrigin => !d.cross,
                explorer_net::http::FetchCredentials::Include => true,
            };
            if cookie_ok {
                if let Ok(resp) = &d.out {
                    for v in resp.headers_all("set-cookie") {
                        self.cookies.set_from_header(&d.req_url, v);
                    }
                }
            }
            let resp = match &d.out {
                Ok(resp) if !d.cross || cors_readable(&d.page_origin, resp) => NetResponse {
                    handle: d.handle,
                    status: resp.status,
                    status_text: resp.reason.clone(),
                    headers: resp.headers.clone(),
                    body: resp.body.clone(),
                },
                _ => NetResponse {
                    handle: d.handle,
                    status: 0,
                    status_text: String::new(),
                    headers: Vec::new(),
                    body: Vec::new(),
                },
            };
            self.deliver_net_response(d.tab, resp);
        }
        // Des `Set-Cookie` XHR ont pu enrichir le jar —
        // persistance au même point de synchro.
        if n > 0 {
            self.persist_cookies();
        }
        n
    }

    /// Pompe les réponses des requêtes script-initiées
    /// (étape 51) — cède d'abord le tour pour laisser les
    /// tâches produire, puis livre ce qui est arrivé.
    /// Renvoie le nombre livré. La session n'a pas
    /// d'horloge propre : l'hôte appelle périodiquement
    /// (daemon : battement dédié ; CLI/tests : après
    /// navigation) — jamais de callback ré-entrant. Les
    /// resynchros rapportées par les livraisons sont
    /// enfilées puis drainées ici (étape 55h — sémantique
    /// synchrone `&mut` conservée ; les hôtes partagés
    /// livrent via `poll_net` interne et drainent hors
    /// verrou avec `drain_applies_shared`).
    pub async fn pump_net(&mut self) -> usize {
        tokio::task::yield_now().await;
        let n = self.poll_net();
        self.drain_applies();
        n
    }

    /// Requêtes script-initiées encore en vol (tous onglets)
    /// — condition d'arrêt des pompes bornées de l'hôte
    /// (CLI, tests).
    #[must_use]
    pub fn net_pending(&self) -> usize {
        self.net_tasks.len()
    }

    /// Collecte des sous-ressources du document (étape 50) :
    /// URLs résolues ici (émission propre), mixed-content
    /// filtré, images dédupliquées — `(feuilles, scripts,
    /// images)` bornés par la config.
    pub(crate) fn plan_subresources(
        &self,
        dom: &Dom,
        base: &Url,
        scripting: bool,
    ) -> (Vec<Url>, Vec<ScriptItem>, Vec<Url>) {
        // Étape 96 : `media=` évalué contre l'env viewport
        // AVANT la borne — une feuille hors-media (variante
        // `print`, `(prefers-color-scheme: …)`) n'est ni
        // fetchée ni comptée dans `max_sheets`. Dédup sur
        // l'URL résolue (ordre conservé — les pages à blocs
        // de liens générés répètent le même `href`, GitHub
        // ~7 doublons : un doublon brûle quota + fetch pour
        // un contenu identique en cascade).
        let env = super::apply::viewport_env(self.viewport, self.color_scheme);
        let mut seen_sheets = std::collections::HashSet::new();
        let sheets: Vec<Url> = collect_stylesheet_links(dom)
            .into_iter()
            .filter(|(_, m)| media_attr_matches(&env, m.as_deref().unwrap_or_default()))
            .filter_map(|(h, _)| Url::resolve(base, &h).ok())
            .filter(|u| !is_mixed_content(base, u) && seen_sheets.insert(u.to_string()))
            .take(self.max_sheets)
            .collect();
        let scripts = collect_scripts(dom, base, scripting, self.max_scripts);
        let mut seen_img = std::collections::HashSet::new();
        let imgs: Vec<Url> = collect_images(dom)
            .into_iter()
            .filter_map(|(_, src)| Url::resolve(base, &src).ok())
            // `data:` (et autres non-http) ne se fetch pas —
            // peints inline par `collect_data_images` (56f).
            .filter(|u| matches!(u.scheme(), "http" | "https"))
            .filter(|u| !is_mixed_content(base, u) && seen_img.insert(u.as_str()))
            .take(self.max_images)
            .collect();
        (sheets, scripts, imgs)
    }

    /// Remplit `graph` en BFS du graphe statique
    /// d'`entry_src` — vagues parallèles `max_inflight`,
    /// `sources` plafonné à `max_modules` (l'entrée incluse).
    /// `page` porte la politique (`Referer`, mixed-content) ;
    /// `entry_base` sert à résoudre les spécificateurs après
    /// réécriture des bare par l'import map (issue #6).
    /// Associée sans `&self` : `dyn JsEngine` n'est pas
    /// `Sync`, `&Session` ne peut donc pas traverser un
    /// `.await` dans une future `Send`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn fetch_module_sources(
        transport: &Arc<dyn HttpTransport>,
        max_inflight: usize,
        max_modules: usize,
        page: &Url,
        entry_base: &Url,
        entry_src: &str,
        import_specs: &[(String, String)],
        graph: &mut ModuleGraph,
    ) {
        let mut queued: std::collections::HashSet<String> =
            std::iter::once(entry_base.as_str()).collect();
        let mut frontier: Vec<(String, Url)> = Vec::new();
        for (spec, u) in module_deps(entry_src, entry_base, import_specs) {
            graph
                .alias
                .entry(spec.clone())
                .or_insert_with(|| u.as_str());
            if !is_mixed_content(page, &u) && queued.insert(u.as_str()) {
                frontier.push((spec, u));
            }
        }
        while !frontier.is_empty() && graph.sources.len() < max_modules {
            let room = (max_modules - graph.sources.len()).min(frontier.len());
            let mut jobs = VecDeque::new();
            for (_, u) in frontier.drain(..room) {
                jobs.push_back((
                    (u.as_str(), u.clone()),
                    sub_request(page, &u, explorer_net::http::FetchDest::Script),
                ));
            }
            let mut fetched = Vec::new();
            fetch_wave(transport, max_inflight, jobs, |tag, res| {
                fetched.push((tag, res));
            })
            .await;
            let mut next = Vec::new();
            for ((key, u), res) in fetched {
                let Ok(resp) = res else { continue };
                if !(200..300).contains(&resp.status) {
                    continue;
                }
                let text = decode_subresource(&resp.body, None);
                graph.sources.insert(key, text.clone());
                for (spec2, u2) in module_deps(&text, &u, import_specs) {
                    graph
                        .alias
                        .entry(spec2.clone())
                        .or_insert_with(|| u2.as_str());
                    if !is_mixed_content(page, &u2) && queued.insert(u2.as_str()) {
                        next.push((spec2, u2));
                    }
                }
            }
            frontier = next;
        }
    }

    /// Décode + insère les corps `<img>` dans la table de
    /// l'onglet — **après** la resynchro DOM : les `NodeId`
    /// changent au re-parse, les corps sont re-matchés par
    /// `src` résolue en ordre document (un fetch = tous les
    /// `<img>` homonymes, décodage dédupliqué).
    pub(crate) fn insert_images(
        &mut self,
        tab: TabId,
        base: &Url,
        bodies: &HashMap<String, Vec<u8>>,
    ) {
        if bodies.is_empty() {
            return;
        }
        let max_px = self.image_max_px;
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        let Some(dom) = entry.dom.as_ref() else {
            return;
        };
        insert_images_into(
            dom,
            &mut entry.images,
            base,
            bodies,
            max_px,
            &mut entry.img_decoded,
        );
    }
}

/// Boucle `insert_images` partagée — `dom` et `images`
/// proviennent de l'onglet (chemin synchrone) ou de la
/// payload détachée (étape 90). `decoded_cache` = cache
/// persistant `src`/`poster` → image (étape 108) : la table
/// `replaced` indexée `NodeId` meurt à chaque re-parse, le
/// cache survit et permet la ré-insertion sans re-fetch ni
/// re-décodage.
fn insert_images_into(
    dom: &Dom,
    images: &mut PaintImages,
    base: &Url,
    bodies: &HashMap<String, Vec<u8>>,
    max_px: u32,
    decoded_cache: &mut HashMap<String, Arc<DecodedImage>>,
) {
    let items = collect_images(dom);
    let mut decoded: HashMap<String, Option<Arc<DecodedImage>>> = HashMap::new();
    for (node, src) in items {
        let Ok(u) = Url::resolve(base, &src) else {
            continue;
        };
        let Some(body) = bodies.get(&u.as_str()) else {
            continue;
        };
        let img = decoded
            .entry(u.as_str())
            .or_insert_with(|| decode_image(body, max_px).map(Arc::new));
        if let Some(img) = img {
            images.replaced.insert(node, img.clone());
            decoded_cache.insert(src, img.clone());
        }
    }
}

/// `<img>`/affiches fetchés ré-insérés sur le DOM re-parsé
/// (étape 108) — miroir des fonds `bg_decoded`. Les `NodeId`
/// changent au re-parse mais l'attribut `src`/`poster`
/// survit : la table `replaced` se reconstruit par clé,
/// zéro fetch ni décodage.
pub(crate) fn reinsert_imgs_by_src(
    dom: &Dom,
    images: &mut PaintImages,
    decoded_cache: &HashMap<String, Arc<DecodedImage>>,
) {
    if decoded_cache.is_empty() {
        return;
    }
    for (node, src) in collect_images(dom) {
        if let Some(img) = decoded_cache.get(&src) {
            images.replaced.insert(node, img.clone());
        }
    }
}

/// Variante payload d'`insert_images` pour le rendu de
/// navigation détaché (étape 90) — même sémantique, hors
/// session : le DOM et la table `images` vivent dans `p`.
pub(crate) fn insert_images_payload(
    p: &mut PaintPayload,
    base: &Url,
    bodies: &HashMap<String, Vec<u8>>,
    max_px: u32,
) {
    if bodies.is_empty() {
        return;
    }
    let PaintPayload {
        dom,
        images,
        img_decoded,
        ..
    } = p;
    let Some(dom) = dom.as_ref() else {
        return;
    };
    insert_images_into(dom, images, base, bodies, max_px, img_decoded);
}

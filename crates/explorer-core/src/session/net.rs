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
use explorer_net::error::NetResult;
use explorer_net::http::{HttpTransport, Response};
use explorer_net::policy::is_mixed_content;
use explorer_net::url::Url;
use explorer_render::{data_url_bytes, decode_image, DecodedImage};

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
            link_events,
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
            frames,
            ..
        } = job;
        // Feuilles liées dans l'ordre document (slot vide =
        // feuille en échec → absente de la cascade). Une
        // entrée par feuille (étape 104) : chaque feuille est
        // parsée isolément à la cascade.
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.linked_sheets = css_slots.into_iter().flatten().collect();
            // Sous-contextes `<iframe>` de la vague (étape
            // 128) — ordre d'arrivée ; `frames` est réassigné
            // à chaque commit → les contextes du document
            // mort ne survivent pas.
            entry.frames = frames;
            // Surfaces composites (étape 129) : les bitmaps
            // clés `(url,w,h)` datent de la vague
            // précédente — même URL ≠ même contenu d'un
            // commit à l'autre, le cache est invalidé.
            entry.frame_bitmaps.clear();
        }
        if let Ok(mut wf) = self.webfonts.lock() {
            for (family, weight, italic, data) in fonts {
                wf.register(&family, weight, italic, data);
            }
        }
        if let Some(eng) = eng.as_mut() {
            // Étape 134 : un realm JS par `<iframe>` fetché —
            // créé avant les scripts du parent pour qu'un
            // `frames[n]`/`contentWindow` lu en cours de
            // parse résolve un vrai contexte.
            self.wire_frame_contexts(tab, eng.as_mut());
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
                self.eval_or_report(tab, eng.as_mut(), src, url.as_deref(), Some(item.el_ord));
            }
            for me in arrival_evals {
                self.eval_module_graph(tab, eng.as_mut(), &me);
            }
            let mut post_evals = post_evals.into_iter();
            for item in scripts.iter().filter(|i| i.exec == ScriptExec::Post) {
                if item.module {
                    if let Some(me) = post_evals.next() {
                        self.eval_module_graph(tab, eng.as_mut(), &me);
                    }
                } else if let Some(src) = item_source(item, &script_srcs) {
                    let url = item.src.as_ref().map(Url::as_str);
                    self.eval_or_report(tab, eng.as_mut(), src, url.as_deref(), Some(item.el_ord));
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
            // CAP-185 : `load`/`error` des `<link rel=stylesheet>`
            // statiques — livrés **après** les scripts du
            // document (les listeners `addEventListener` sont
            // enregistrés : le bootstrap `x-web` n'injecte son
            // entrée `type=module` qu'au `load` de la feuille),
            // **avant** `after_script` (les insertions
            // dynamiques qu'ils produisent sont drainées dans
            // la foulée).
            if let Some(eng) = entry.js.as_mut() {
                for (ord, ok) in link_events {
                    eng.deliver_static_link(explorer_js::JS_CTX_TOP, ord, ok);
                }
            }
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
                    same_doc: false,
                    state: None,
                });
                entry.hist_idx = entry.hist.len() - 1;
                // Fenêtre « en vol » refermée (étape 84) :
                // la page commise a désormais son entrée.
                entry.hist_pending = false;
                // `pushState`/`replaceState` reçus pendant
                // la fenêtre (CAP-151 — scripts de chargement
                // SPA : le document est celui commité) :
                // `replace` corrige la requête poussée (et
                // son état, CAP-158), les `push` s'empilent
                // au-dessus dans l'ordre.
                if let Some((rep, st)) = entry.pending_replace.take() {
                    if let Some(h) = entry.hist.last_mut() {
                        h.req.url = rep;
                        h.state = st;
                    }
                }
                let extras = std::mem::take(&mut entry.pending_push);
                for (u, st) in extras {
                    let mut r = Request::get(u);
                    r.initiator = Some(explorer_net::http::Initiator::TopLevelNav);
                    entry.hist.truncate(entry.hist_idx + 1);
                    // `pushState` différés : entrées
                    // same-document du document fraîchement
                    // commis (CAP-151) avec leur état
                    // (CAP-158).
                    entry.hist.push(HistEntry {
                        req: r,
                        failed: false,
                        same_doc: true,
                        state: st,
                    });
                    entry.hist_idx = entry.hist.len() - 1;
                }
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
        // CAP-170 : `readyState` « interactive » +
        // `readystatechange` juste avant `DOMContentLoaded`
        // (ordre spec — les scripts de la page ont déjà couru,
        // les sous-ressources peuvent encore charger).
        self.set_doc_ready_state(tab, explorer_js::JS_CTX_TOP, "interactive");
        self.dispatch_doc_event(tab, "readystatechange");
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
        // Étape 134 : `load` des sous-contextes d'abord (spec :
        // le `load` de la page attend ses `<iframe>`), puis
        // `load`/`onload` sur chaque élément porteur.
        self.frame_ctx_load_events(tab);
        // CAP-170 : `readyState` « complete » +
        // `readystatechange` **avant** `load` (spec — dans un
        // listener `load`, `document.readyState ===
        // "complete"`).
        self.set_doc_ready_state(tab, explorer_js::JS_CTX_TOP, "complete");
        self.dispatch_doc_event(tab, "readystatechange");
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
    /// Événements `WebSocket` (JS-C94) — multi-coups par
    /// flux ; `Close`/`Error` = fin de flux → le handle sort
    /// de `net_tasks`/`ws_sends` (comme un `NetDone`).
    fn drain_ws_events(&mut self) {
        let mut ws_done = Vec::new();
        while let Ok(d) = self.ws_rx.try_recv() {
            ws_done.push(d);
        }
        for d in ws_done {
            if matches!(
                d.ev,
                explorer_net::ws::WsEvent::Close { .. } | explorer_net::ws::WsEvent::Error(_)
            ) {
                self.net_tasks.remove(&(d.tab, d.handle));
                self.ws_sends.remove(&(d.tab, d.handle));
            }
            self.deliver_ws(d.tab, d.ctx, d.handle, d.ev);
        }
    }

    /// Événements `EventSource` (JS-C92) — multi-coups par
    /// flux ; `Error` = fin de tâche → le handle sort du
    /// registre (comme un `NetDone` complet).
    fn drain_es_events(&mut self) {
        let mut es_done = Vec::new();
        while let Ok(d) = self.es_rx.try_recv() {
            es_done.push(d);
        }
        for d in es_done {
            if matches!(d.ev, explorer_js::EsEvent::Error) {
                self.net_tasks.remove(&(d.tab, d.handle));
            }
            self.deliver_es(d.tab, d.ctx, d.handle, d.ev);
        }
    }

    /// Distribution des `NetRequest` drainées (fetch/abort/
    /// crypto/EventSource) — extrait de `drain_script_net`.
    fn dispatch_net_requests(
        &mut self,
        tab: TabId,
        page: &Url,
        base: &Url,
        reqs: Vec<explorer_js::NetRequest>,
    ) {
        let mut failed = Vec::new();
        self.dispatch_net_reqs_inner(tab, page, base, reqs, &mut failed);
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

    /// Boucle de dispatch des `NetRequest` (fetch/abort/crypto/
    /// `EventSource`) — extrait de `drain_script_net` (clippy
    /// `too_many_lines` : JS-C92 l'a fait passer à 115).
    #[allow(clippy::too_many_lines)] // dispatch séquentiel — chaque variante est documentée in situ
    fn dispatch_net_reqs_inner(
        &mut self,
        tab: TabId,
        page: &Url,
        base: &Url,
        reqs: Vec<explorer_js::NetRequest>,
        failed: &mut Vec<u64>,
    ) {
        for r in reqs {
            match r {
                NetRequest::Fetch {
                    handle,
                    method,
                    url,
                    headers,
                    body,
                    mode,
                    credentials,
                    ctx,
                } => {
                    if self
                        .spawn_script_fetch(
                            tab,
                            page,
                            base,
                            handle,
                            &method,
                            &url,
                            &headers,
                            body.as_deref(),
                            mode,
                            credentials,
                            ctx,
                        )
                        .is_err()
                    {
                        failed.push(handle);
                    }
                }
                NetRequest::Abort { handle } => {
                    if let Some(j) = self.net_tasks.remove(&(tab, handle)) {
                        j.abort();
                    }
                }
                // Opération `crypto.subtle` (JS-C79) — pas une
                // requête réseau : le calcul tourne en
                // `spawn_blocking` (CPU bornée par la taille
                // des données, copiées hors heap à l'émission)
                // et livre sur `crypto_tx`, drainé par
                // `poll_net` comme les `NetDone`. La promesse
                // reste non-réentrante : jamais de livraison
                // synchrone dans le drain.
                // `EventSource` (JS-C92) — flux streamé
                // multi-coups : `open` aux en-têtes
                // `text/event-stream`, un `message` par
                // événement SSE, `error` à la coupure. La
                // tâche vit dans `net_tasks` — `Abort`
                // (`es.close()`) la tue comme un fetch.
                NetRequest::EventSource {
                    handle,
                    url,
                    with_credentials,
                    ctx,
                } => {
                    if self
                        .spawn_es(&EsSpawnArgs {
                            tab,
                            page,
                            base,
                            handle,
                            url: &url,
                            with_credentials,
                            ctx,
                        })
                        .is_err()
                    {
                        let _ = self.es_tx.send(EsDone {
                            tab,
                            ctx,
                            handle,
                            ev: explorer_js::EsEvent::Error,
                        });
                    }
                }
                // `WebSocket` (JS-C94) — socket dédiée hors
                // pool : `WsOpen` dial via `WsDial`, `send`/
                // `close` routés par `ws_sends` vers la
                // tâche driver du flux. La livraison
                // `open`/`message`/`error`/`close` est
                // multi-coups sur `ws_tx`, comme `EsDone`.
                NetRequest::WsOpen {
                    handle,
                    url,
                    protocols,
                    ctx,
                } => {
                    if self
                        .spawn_ws(&WsSpawnArgs {
                            tab,
                            page,
                            base,
                            handle,
                            url: &url,
                            protocols: &protocols,
                            ctx,
                        })
                        .is_err()
                    {
                        let _ = self.ws_tx.send(WsDone {
                            tab,
                            ctx,
                            handle,
                            ev: explorer_net::ws::WsEvent::Error("ws: open refused".into()),
                        });
                    }
                }
                NetRequest::WsSend { handle, out } => {
                    let out = match out {
                        explorer_js::WsOut::Text(t) => explorer_net::ws::WsSend::Text(t),
                        explorer_js::WsOut::Binary(b) => explorer_net::ws::WsSend::Binary(b),
                        explorer_js::WsOut::Close { code, reason } => {
                            explorer_net::ws::WsSend::Close { code, reason }
                        }
                    };
                    if let Some(tx) = self.ws_sends.get(&(tab, handle)) {
                        let _ = tx.try_send(out);
                    }
                }
                NetRequest::WsClose {
                    handle,
                    code,
                    reason,
                } => {
                    if let Some(tx) = self.ws_sends.get(&(tab, handle)) {
                        let _ = tx.try_send(explorer_net::ws::WsSend::Close { code, reason });
                    }
                }
                NetRequest::Crypto {
                    handle,
                    op,
                    data,
                    key,
                } => {
                    let tx = self.crypto_tx.clone();
                    tokio::task::spawn_blocking(move || {
                        use explorer_net::crypto as nc;
                        fn digest_of(a: explorer_js::DigestAlgo) -> nc::CryptoAlgo {
                            match a {
                                explorer_js::DigestAlgo::Sha256 => nc::CryptoAlgo::Sha256,
                                explorer_js::DigestAlgo::Sha384 => nc::CryptoAlgo::Sha384,
                                explorer_js::DigestAlgo::Sha512 => nc::CryptoAlgo::Sha512,
                            }
                        }
                        fn key_of(a: explorer_js::KeyAlgo) -> nc::KeyAlgo {
                            match a {
                                explorer_js::KeyAlgo::Hmac { hash } => {
                                    nc::KeyAlgo::Hmac(digest_of(hash))
                                }
                                explorer_js::KeyAlgo::EcdsaP256 => nc::KeyAlgo::EcdsaP256,
                                explorer_js::KeyAlgo::Ed25519 => nc::KeyAlgo::Ed25519,
                            }
                        }
                        let result = match op {
                            explorer_js::CryptoOp::Digest(algo) => {
                                Ok(nc::digest(digest_of(algo), &data))
                            }
                            explorer_js::CryptoOp::ImportKey { format, algo } => {
                                let f = match format {
                                    explorer_js::ImportFormat::Raw => nc::KeyFormat::Raw,
                                    explorer_js::ImportFormat::Pkcs8 => nc::KeyFormat::Pkcs8,
                                    explorer_js::ImportFormat::Spki => nc::KeyFormat::Spki,
                                };
                                nc::import_key(key_of(algo), f, &data)
                            }
                            explorer_js::CryptoOp::Sign { algo } => {
                                nc::sign(key_of(algo), &key, &data)
                            }
                            explorer_js::CryptoOp::Verify { algo, sig } => {
                                nc::verify(key_of(algo), &key, &sig, &data)
                            }
                        };
                        let _ = tx.send(CryptoDone {
                            tab,
                            handle,
                            result,
                        });
                    });
                }
                // `Worker`/`importScripts` (JS-C132) — le
                // demandeur (thread moteur au premier
                // `new Worker`, thread worker ensuite) est
                // bloqué en `load()` : la réponse part par le
                // canal one-shot `reply` depuis la tâche
                // fetch, sans transiter par `deliver_*`.
                NetRequest::WorkerLoad {
                    url,
                    referrer,
                    ctx,
                    reply,
                } => {
                    self.spawn_worker_load(&WorkerLoadArgs {
                        tab,
                        page,
                        base,
                        spec: &url,
                        referrer: &referrer,
                        ctx,
                        reply,
                    });
                }
            }
        }
    }

    /// `NetRequest::WorkerLoad` — source de worker fetchée
    /// côté session : résolution contre `referrer` (page ou
    /// module émetteur — un worker reçoit l'URL de la page
    /// qui l'a créé), CSP `script-src` (repli spec de
    /// `worker-src`) + mixed-content, corps répondu par
    /// `reply` one-shot. `data:` est décodé sans requête,
    /// même pile que les sous-ressources dynamiques.
    fn spawn_worker_load(&mut self, a: &WorkerLoadArgs<'_>) {
        let Some((req_page, req_base, _policy, csp, _opaque)) =
            self.ctx_net_origin(a.tab, a.ctx, a.page, a.base)
        else {
            let _ = a.reply.0.send(Err("pas d'origine de contexte".to_string()));
            return;
        };
        if let Some(body) = data_url_bytes(a.spec) {
            let _ = a
                .reply
                .0
                .send(Ok(String::from_utf8_lossy(&body).into_owned()));
            return;
        }
        let rbase = Url::parse(a.referrer).ok().unwrap_or(req_base);
        let Some(url) = Url::resolve(&rbase, a.spec)
            .ok()
            .filter(|u| !is_mixed_content(&req_page, u))
            .filter(|u| {
                explorer_net::csp::csp_allows(
                    &csp,
                    &req_page,
                    explorer_net::http::FetchDest::Script,
                    u,
                )
            })
        else {
            let _ = a.reply.0.send(Err("résolution/CSP refusée".to_string()));
            return;
        };
        let req = sub_request(
            &req_page,
            &url,
            explorer_net::http::FetchDest::Script,
            None,
            Some(explorer_net::http::FetchCredentials::SameOrigin),
        );
        let tr = self.transport.clone();
        let reply = a.reply.clone();
        tokio::spawn(async move {
            let out = tr
                .fetch(&req)
                .await
                .map_err(|e| e.to_string())
                .and_then(|resp| {
                    if (200..300).contains(&resp.status) {
                        Ok(String::from_utf8_lossy(&resp.body).into_owned())
                    } else {
                        Err(format!("HTTP {}", resp.status))
                    }
                });
            let _ = reply.0.send(out);
        });
    }

    pub(crate) fn drain_script_net(&mut self, tab: TabId) {
        // Disjoncteur (vol 25) : les drains `Cmd` attendent
        // une réponse du worker — figé, chacun patienterait
        // 30 s sous verrou.
        if self.js_stalled(tab) {
            return;
        }
        // `WorkerLoad` (JS-C132) **avant** le drain `Cmd` :
        // le demandeur (thread moteur au premier `new Worker`)
        // est bloqué en `load()` — dispatcher le fetch async
        // le débloque ; le drain `Rc` suivant attend alors ≈
        // la durée du fetch au lieu d'expirer en timeout.
        let (wpage, wbase, wreqs) = self.tabs.get_mut(&tab).map_or_else(
            || (None, None, Vec::new()),
            |e| {
                (
                    e.url.clone(),
                    e.nav_base().cloned(),
                    e.js.as_mut()
                        .map_or_else(Vec::new, |eng| eng.drain_worker_requests()),
                )
            },
        );
        if let (Some(wp), Some(wb)) = (&wpage, &wbase) {
            if !wreqs.is_empty() {
                let mut wfailed = Vec::new();
                self.dispatch_net_reqs_inner(tab, wp, wb, wreqs, &mut wfailed);
            }
        }
        let Some((page, base, reqs, dyns, dyn_frames, dyn_imgs, dyn_links, adopted, ce_names)) =
            self.tabs.get_mut(&tab).and_then(|e| {
                let page = e.url.clone();
                let base = e.nav_base().cloned();
                e.js.as_mut().map(|eng| {
                    (
                        page,
                        base,
                        eng.drain_net_requests(),
                        eng.drain_dyn_scripts(),
                        eng.drain_dyn_frames(),
                        eng.drain_dyn_imgs(),
                        eng.drain_dyn_links(),
                        // `adoptedStyleSheets` (CAP-195 tranche
                        // 2) — snapshot cssText par feuille,
                        // cascadé à la passe suivante.
                        eng.adopted_sheets_css(),
                        // Registre `customElements` — poussé
                        // dans le `Dom` pour `:defined` (le
                        // pseudo est structurel, évalué à la
                        // cascade sur l'arbre session).
                        eng.defined_ce_names(),
                    )
                })
            })
        else {
            return;
        };
        let (Some(page), Some(base)) = (page, base) else {
            return;
        };
        if let Some(e) = self.tabs.get_mut(&tab) {
            e.adopted_sheets = adopted;
            e.defined_names = ce_names;
            if let Some(d) = e.dom.as_mut() {
                d.set_defined_names(e.defined_names.iter().cloned());
            }
        }
        self.dispatch_net_requests(tab, &page, &base, reqs);
        self.spawn_dyn_subresources(tab, &page, &base, dyns, dyn_frames, dyn_imgs, dyn_links);
        // `import()` dynamiques (JS-C100) — URL déjà résolue
        // par le chargeur hôte, `deliver_dyn_import` à la
        // réponse.
        for r in self
            .tabs
            .get_mut(&tab)
            .and_then(|e| e.js.as_mut())
            .map_or_else(Vec::new, |eng| eng.drain_dyn_imports())
        {
            tracing::debug!(tab, url = %r.url, seq = r.seq, "spawn dyn import");
            self.spawn_dyn_import(tab, &page, &base, &r);
        }
    }

    /// Distribution des requêtes dynamiques drainées
    /// (CAP-169/171/172) : `force-async` spec — fetch/éval
    /// dès l'arrivée, `load`/`error` sur l'élément à la
    /// livraison, jamais d'exécution synchrone dans le
    /// drain.
    #[allow(clippy::too_many_arguments)]
    fn spawn_dyn_subresources(
        &mut self,
        tab: TabId,
        page: &Url,
        base: &Url,
        dyns: Vec<explorer_js::DynScriptReq>,
        dyn_frames: Vec<explorer_js::DynFrameReq>,
        dyn_imgs: Vec<explorer_js::DynImgReq>,
        dyn_links: Vec<explorer_js::DynLinkReq>,
    ) {
        for d in dyns {
            self.spawn_dyn_script(tab, page, base, &d);
        }
        // `<iframe>` : fetch puis realm enfant —
        // `commit_dyn_frame` à la réponse.
        for d in dyn_frames {
            self.spawn_dyn_frame(tab, page, base, &d);
        }
        // `<img>` : source choisie (`srcset`/`sizes`/dpr ou
        // `src`), `data:` décodé sans fetch, sinon
        // `img-src` CSP + mixed-content puis fetch —
        // `commit_dyn_img` à la réponse.
        for d in dyn_imgs {
            self.spawn_dyn_img(tab, page, base, &d);
        }
        // `<link rel=stylesheet>` : fetch `style-src` CSP +
        // mixed-content puis `linked_sheets` + `load`/`error`
        // — `commit_dyn_link` à la réponse.
        tracing::debug!(tab, n = dyn_links.len(), "drain dyn links");
        for d in dyn_links {
            self.spawn_dyn_link(tab, page, base, &d);
        }
    }

    /// `import()` dynamique (JS-C100) — l'URL est déjà résolue
    /// par le chargeur hôte ; fetch `script-src` + mixed-content
    /// puis `deliver_dyn_import` au moteur (instancier+évaluer+
    /// résoudre la promesse pending).
    fn spawn_dyn_import(
        &mut self,
        tab: TabId,
        page: &Url,
        base: &Url,
        r: &explorer_js::DynImportReq,
    ) {
        let Some((req_page, req_base, doc_policy, csp, _opaque)) =
            self.ctx_net_origin(tab, r.ctx, page, base)
        else {
            tracing::warn!(tab, ctx = r.ctx, "dyn import : pas d'origine — rejet");
            self.deliver_dyn_import_req(tab, r.ctx, r.seq, None, Vec::new());
            return;
        };
        let Some(url) = Url::resolve(&req_base, &r.url)
            .ok()
            .filter(|u| !is_mixed_content(&req_page, u))
            .filter(|u| {
                explorer_net::csp::csp_allows(
                    &csp,
                    &req_page,
                    explorer_net::http::FetchDest::Script,
                    u,
                )
            })
        else {
            tracing::warn!(tab, url = %r.url, "dyn import : résolution/CSP refusé — rejet");
            self.deliver_dyn_import_req(tab, r.ctx, r.seq, None, Vec::new());
            return;
        };
        // `data:` — corps auto-porteur : décodé sans requête
        // (spec : `import()` accepte les URL `data:` — chunks
        // embarqués reddit). Le contrôle CSP `script-src`
        // (scheme-source `data:`) a déjà couru au filtre.
        if let Some(body) = data_url_bytes(&url.as_str()) {
            let src = String::from_utf8_lossy(&body).into_owned();
            self.deliver_dyn_import_req(tab, r.ctx, r.seq, Some(src), Vec::new());
            return;
        }
        tracing::debug!(tab, url = %url, "dyn import : fetch émis");
        let req = sub_request(
            &req_page,
            &url,
            explorer_net::http::FetchDest::Script,
            None,
            Some(explorer_net::http::FetchCredentials::SameOrigin),
        );
        let is_cross = req_page.origin() != url.origin();
        let fetch_mode = req.fetch_mode;
        let page_origin = origin_header_value(&req_page);
        let partition = explorer_net::cookies::request_partition(&req);
        let req_url = req.url.clone();
        let (ctx, seq) = (r.ctx, r.seq);
        let import_specs = r.import_specs.clone();
        let tr = self.transport.clone();
        let tx = self.net_tx.clone();
        let max_inflight = self.max_inflight;
        let max_modules = self.max_modules;
        let page_g = req_page.clone();
        let policy = doc_policy;
        let url_g = url.clone();
        tokio::spawn(async move {
            let out = tr.fetch(&req).await;
            // Entrée 2xx → graphe de deps fetché (JS-C101) :
            // le module livré fait `import {x} from 'lit'` —
            // `load_source` aval manquerait le préfetch.
            let (entry, sources) = match &out {
                Ok(resp) if (200..300).contains(&resp.status) => {
                    let entry_src = String::from_utf8_lossy(&resp.body).into_owned();
                    let mut graph = crate::session::net_items::ModuleGraph {
                        sources: std::collections::HashMap::new(),
                        alias: std::collections::HashMap::new(),
                    };
                    Self::fetch_module_sources(
                        &tr,
                        max_inflight,
                        max_modules,
                        &page_g,
                        policy,
                        &csp,
                        &url_g,
                        &entry_src,
                        &import_specs,
                        &mut graph,
                    )
                    .await;
                    (Some(entry_src), graph.sources.into_iter().collect())
                }
                _ => (None, Vec::new()),
            };
            let meta = crate::session::net_items::DynImportMeta {
                ctx,
                seq,
                entry,
                sources,
            };
            let _ = tx.send(NetDone {
                tab,
                handle: seq,
                cross: is_cross,
                mode: fetch_mode,
                credentials: explorer_net::http::FetchCredentials::SameOrigin,
                page_origin,
                req_url,
                partition,
                out,
                script: None,
                frame: None,
                img: None,
                link: None,
                dyn_module: None,
                import: Some(meta),
            });
        });
    }

    /// `deliver_dyn_import` au moteur du contexte émetteur —
    /// `Some(entry)` → instancier+évaluer ; `None` → rejet ;
    /// `deps` seedées dans `module_sources` avant (JS-C101).
    fn deliver_dyn_import_req(
        &mut self,
        tab: TabId,
        ctx: u32,
        seq: u64,
        src: Option<String>,
        deps: Vec<(String, String)>,
    ) {
        tracing::debug!(
            tab,
            seq,
            ok = src.is_some(),
            deps = deps.len(),
            "deliver dyn import"
        );
        if let Some(eng) = self.tabs.get_mut(&tab).and_then(|e| e.js.as_mut()) {
            eng.deliver_dyn_import(ctx, seq, src, deps);
        }
    }

    /// `NetDone` `import` → corps + graphe → `deliver_dyn_import`.
    fn commit_dyn_import(&mut self, tab: TabId, meta: &crate::session::net_items::DynImportMeta) {
        let deps = meta.sources.clone();
        self.deliver_dyn_import_req(tab, meta.ctx, meta.seq, meta.entry.clone(), deps);
    }

    /// Un `<script>` inséré dynamiquement (CAP-169) : inline →
    /// contrôle CSP `script-src` puis livraison synchrone ;
    /// `src` → résolution contre la base du document émetteur,
    /// `script-src` + mixed-content, puis fetch `sub_request`
    /// (initiateur = document émetteur — `Origin`/`Referer`/
    /// `Sec-Fetch-*`/`crossorigin` identiques aux classiques).
    /// Refus → `deliver_dyn_script(None)` = `error` jamais
    /// d'éval d'un corps refusé.
    #[allow(clippy::too_many_lines)] // branches src/inline × classique/module, chacune documentée in situ
    fn spawn_dyn_script(
        &mut self,
        tab: TabId,
        page: &Url,
        base: &Url,
        d: &explorer_js::DynScriptReq,
    ) {
        let Some((req_page, req_base, doc_policy, csp, _opaque)) =
            self.ctx_net_origin(tab, d.ctx, page, base)
        else {
            // Contexte mort entre émission et drain → `error`.
            self.deliver_dyn_script(tab, d.ctx, d.seq, None);
            return;
        };
        match (&d.src, &d.text) {
            (None, Some(text)) => {
                let ok = explorer_net::csp::csp_inline_allowed(
                    &csp,
                    explorer_net::csp::InlineKind::Script,
                    d.nonce.as_deref(),
                );
                // CAP-186 : un module inline est ensemencé
                // `inline:{seq}` puis son graphe de deps est
                // fetché — jamais l'éval classique.
                if d.module {
                    if ok {
                        self.spawn_dyn_module_graph(
                            tab,
                            d.ctx,
                            d.seq,
                            format!("inline:{}", d.seq),
                            text.clone(),
                            req_base.clone(),
                            req_page,
                            doc_policy,
                            csp,
                        );
                    } else {
                        self.deliver_dyn_module(
                            tab,
                            DynModuleDone {
                                ctx: d.ctx,
                                seq: d.seq,
                                name: String::new(),
                                graph: None,
                            },
                        );
                    }
                } else {
                    self.deliver_dyn_script(tab, d.ctx, d.seq, ok.then(|| text.clone()));
                }
            }
            (Some(_), _) => {
                self.spawn_dyn_script_src(tab, d, &req_page, &req_base, doc_policy, &csp);
            }
            // Ni `src` ni texte — balise inerte, rien à livrer.
            (None, None) => {}
        }
    }

    /// Branche `src` de `spawn_dyn_script` — résolution,
    /// CSP + mixed-content, fetch `sub_request` avec la
    /// méta `DynScriptMeta` (SRI/charset/module) et
    /// insertion `dyn_tasks`. Refus → `deliver_dyn_script
    /// (None)` = `error` honnête.
    fn spawn_dyn_script_src(
        &mut self,
        tab: TabId,
        d: &explorer_js::DynScriptReq,
        req_page: &Url,
        req_base: &Url,
        doc_policy: Option<explorer_net::http::ReferrerPolicy>,
        csp: &[explorer_net::csp::CspPolicy],
    ) {
        let Some(src) = &d.src else {
            return;
        };
        let Some(url) = Url::resolve(req_base, src.trim())
            .ok()
            .filter(|u| !is_mixed_content(req_page, u))
            .filter(|u| {
                explorer_net::csp::csp_allows(
                    csp,
                    req_page,
                    explorer_net::http::FetchDest::Script,
                    u,
                )
            })
        else {
            self.deliver_dyn_script(tab, d.ctx, d.seq, None);
            return;
        };
        let policy = d
            .referrerpolicy
            .as_deref()
            .and_then(explorer_net::http::ReferrerPolicy::parse)
            .or(doc_policy);
        let cross = d.crossorigin.map(|c| match c {
            explorer_js::FetchCredentials::Omit => explorer_net::http::FetchCredentials::Omit,
            explorer_js::FetchCredentials::SameOrigin => {
                explorer_net::http::FetchCredentials::SameOrigin
            }
            explorer_js::FetchCredentials::Include => explorer_net::http::FetchCredentials::Include,
        });
        let req = sub_request(
            req_page,
            &url,
            explorer_net::http::FetchDest::Script,
            policy,
            cross,
        );
        let is_cross = req_page.origin() != url.origin();
        let credentials = cross.unwrap_or(explorer_net::http::FetchCredentials::SameOrigin);
        let fetch_mode = req.fetch_mode;
        let page_origin = origin_header_value(req_page);
        let partition = explorer_net::cookies::request_partition(&req);
        let req_url = req.url.clone();
        let meta = DynScriptMeta {
            ctx: d.ctx,
            seq: d.seq,
            module: d.module,
            integrity: d.integrity.clone(),
            charset: d.charset.clone(),
            page: req_page.clone(),
        };
        let tr = self.transport.clone();
        let tx = self.net_tx.clone();
        let seq = d.seq;
        let j = tokio::spawn(async move {
            let out = tr.fetch(&req).await;
            let _ = tx.send(NetDone {
                tab,
                handle: seq,
                cross: is_cross,
                mode: fetch_mode,
                credentials,
                page_origin,
                req_url,
                partition,
                out,
                script: Some(meta),
                frame: None,
                img: None,
                link: None,
                dyn_module: None,
                import: None,
            });
        });
        self.dyn_tasks.insert((tab, seq), j);
    }

    /// Fetch du graphe d'un `<script type="module">`
    /// dynamique (CAP-186) — l'entrée est ensemencée
    /// (`sources[name]`, `alias[name] → base`) puis les
    /// dépendances sont fetchées hors session
    /// (`fetch_module_sources`, `max_inflight`/`max_modules`)
    /// avant envoi `dyn_module` → `eval_module_graph` +
    /// `load`/`error`. `import_specs` vide : un module
    /// dynamique à bare specifier hors import map est
    /// « unresolvable » honnête (issue #6).
    #[allow(clippy::too_many_arguments)]
    fn spawn_dyn_module_graph(
        &mut self,
        tab: TabId,
        ctx: u32,
        seq: u64,
        name: String,
        entry_src: String,
        entry_base: Url,
        page: Url,
        page_policy: Option<explorer_net::http::ReferrerPolicy>,
        csp: Vec<explorer_net::csp::CspPolicy>,
    ) {
        let tr = self.transport.clone();
        let tx = self.net_tx.clone();
        let max_inflight = self.max_inflight;
        let max_modules = self.max_modules;
        let req_url = entry_base.clone();
        let j = tokio::spawn(async move {
            let mut graph = crate::session::net_items::ModuleGraph {
                sources: std::collections::HashMap::from([(name.clone(), entry_src.clone())]),
                alias: std::collections::HashMap::from([(name.clone(), entry_base.to_string())]),
            };
            Self::fetch_module_sources(
                &tr,
                max_inflight,
                max_modules,
                &page,
                page_policy,
                &csp,
                &entry_base,
                &entry_src,
                &[],
                &mut graph,
            )
            .await;
            let _ = tx.send(NetDone {
                tab,
                handle: seq,
                cross: false,
                mode: explorer_net::http::FetchMode::NoCors,
                credentials: explorer_net::http::FetchCredentials::SameOrigin,
                page_origin: String::new(),
                req_url,
                partition: None,
                out: Err(explorer_net::error::NetError::InvalidResponse(
                    "dyn-module placeholder".into(),
                )),
                script: None,
                frame: None,
                img: None,
                link: None,
                dyn_module: Some(DynModuleDone {
                    ctx,
                    seq,
                    name,
                    graph: Some(graph),
                }),
                import: None,
            });
        });
        self.dyn_tasks.insert((tab, seq), j);
    }

    /// Livraison d'un `<script type="module">` dynamique
    /// (CAP-186) — même discipline que
    /// [`Self::deliver_dyn_script`] : tampon
    /// `js_pending_mod` sous détachement, `eval_module_graph`
    /// sur le graphe rempli puis `load`/`error` sur
    /// l'élément, `run_pending` + resynchro +
    /// `after_script`. `graph = None` → `error` (entrée
    /// refusée en amont).
    pub(crate) fn deliver_dyn_module(&mut self, tab: TabId, dm: DynModuleDone) {
        let DynModuleDone {
            ctx,
            seq,
            name,
            graph,
        } = dm;
        let mut eng = {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return;
            };
            if entry.js_detached.is_some() {
                entry
                    .js_pending_mod
                    .push((ctx, seq, graph.map(|graph| ModEval { name, graph })));
                return;
            }
            let Some(eng) = entry.js.take() else { return };
            eng
        };
        let ok = graph.is_some();
        if let Some(graph) = graph {
            self.eval_module_graph(tab, eng.as_mut(), &ModEval { name, graph });
        }
        eng.deliver_dyn_module(ctx, seq, ok);
        let n = eng.run_pending();
        tracing::trace!(tab, jobs = n, "jobs js après module dynamique");
        let mutated = Self::sync_for_apply(eng.as_mut());
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.js = Some(eng);
        }
        if let Some(s) = mutated {
            self.queue_apply_sync(tab, s);
        }
        self.after_script(tab);
    }

    /// Livraison d'un `<script>` dynamique (CAP-169) — même
    /// discipline que [`Self::deliver_net_response`] : tampon
    /// sous détachement, `run_pending` + resynchro DOM +
    /// `after_script` après le tour moteur (`load`/`error`
    /// peuvent émettre de nouvelles intentions).
    pub(crate) fn deliver_dyn_script(
        &mut self,
        tab: TabId,
        ctx: explorer_js::JsCtxId,
        seq: u64,
        body: Option<String>,
    ) {
        let mutated = {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return;
            };
            if entry.js_detached.is_some() {
                entry.js_pending_dyn.push((ctx, seq, body));
                return;
            }
            let Some(eng) = entry.js.as_mut() else {
                return;
            };
            eng.deliver_dyn_script(ctx, seq, body);
            let n = eng.run_pending();
            tracing::trace!(tab, jobs = n, "jobs js après script dynamique");
            Self::sync_for_apply(eng.as_mut())
        };
        if let Some(s) = mutated {
            self.queue_apply_sync(tab, s);
        }
        self.after_script(tab);
    }

    /// Un `<iframe>` inséré dynamiquement ou re-navigué par
    /// `src=`/`srcdoc=` (CAP-171) : `srcdoc` → sous-document
    /// inline, `commit_dyn_frame` synchrone ; `src` →
    /// résolution contre la base de l'émetteur, `frame-src`
    /// CSP + mixed-content, puis fetch `frame_request`
    /// (mode navigate — `Sec-Fetch-Dest: iframe` + Sec-Fetch
    /// nav, même pile que les cadres du parse). Refus →
    /// `error` sur l'élément, jamais de realm installé.
    fn spawn_dyn_frame(
        &mut self,
        tab: TabId,
        page: &Url,
        base: &Url,
        d: &explorer_js::DynFrameReq,
    ) {
        let sandbox = parse_sandbox(d.sandbox.as_deref());
        let Some((req_page, req_base, doc_policy, csp, _opaque)) =
            self.ctx_net_origin(tab, d.ctx, page, base)
        else {
            self.deliver_dyn_frame_el(tab, d.ctx, d.seq, false);
            return;
        };
        // Sous-documents synchrones : `srcdoc` (prioritaire,
        // spec) sinon `about:blank` quand `src` est absent ou
        // vide (un cadre nu a un realm `contentWindow` et un
        // `load` dès l'insertion — le parse saute ces cadres,
        // ici le script peut demander le realm).
        let src = d.src.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let sf = if let Some(html) = &d.srcdoc {
            let parent_scripting = self.tabs.get(&tab).is_some_and(|e| e.js.is_some());
            Some(store_srcdoc_frame(
                &req_page,
                html,
                sandbox,
                frame_scripting(parent_scripting, sandbox),
            ))
        } else if src.is_none() {
            let mut tb = explorer_html::TreeBuilder::new();
            tb.finish();
            Some(SubFrame {
                url: Url::parse("about:blank").unwrap_or_else(|_| req_page.clone()),
                sandbox,
                srcdoc: None,
                dom: tb.into_dom(),
                title: None,
                html: String::new(),
            })
        } else {
            None
        };
        if let Some(sf) = sf {
            let meta = DynFrameMeta {
                ctx: d.ctx,
                seq: d.seq,
                ord: d.ord,
                sandbox,
                url: sf.url.clone(),
                page: req_page,
            };
            self.commit_dyn_frame(tab, &meta, sf);
            return;
        }
        let Some(src) = src else { return };
        self.spawn_dyn_frame_fetch(tab, req_page, &req_base, doc_policy, &csp, d, src);
    }

    /// Fetch `src` d'un `<iframe>` dynamique (CAP-171) —
    /// résolution contre la base de l'émetteur, `frame-src`
    /// CSP + mixed-content, puis `frame_request` (mode
    /// navigate — `Sec-Fetch-Dest: iframe`, même pile que
    /// les cadres du parse). Refus → `error` sur l'élément.
    #[allow(clippy::too_many_arguments)]
    fn spawn_dyn_frame_fetch(
        &mut self,
        tab: TabId,
        req_page: Url,
        req_base: &Url,
        doc_policy: Option<explorer_net::http::ReferrerPolicy>,
        csp: &[explorer_net::csp::CspPolicy],
        d: &explorer_js::DynFrameReq,
        src: &str,
    ) {
        let Some(url) = Url::resolve(req_base, src)
            .ok()
            .filter(|u| !is_mixed_content(&req_page, u))
            .filter(|u| {
                explorer_net::csp::csp_allows(
                    csp,
                    &req_page,
                    explorer_net::http::FetchDest::Iframe,
                    u,
                )
            })
        else {
            self.deliver_dyn_frame_el(tab, d.ctx, d.seq, false);
            return;
        };
        let policy = d
            .referrerpolicy
            .as_deref()
            .and_then(explorer_net::http::ReferrerPolicy::parse)
            .or(doc_policy);
        let req = frame_request(&req_page, &url, policy);
        let is_cross = req_page.origin() != url.origin();
        let page_origin = origin_header_value(&req_page);
        let partition = explorer_net::cookies::request_partition(&req);
        let req_url = req.url.clone();
        let meta = DynFrameMeta {
            ctx: d.ctx,
            seq: d.seq,
            ord: d.ord,
            sandbox: parse_sandbox(d.sandbox.as_deref()),
            url,
            page: req_page,
        };
        let tr = self.transport.clone();
        let tx = self.net_tx.clone();
        let seq = d.seq;
        let j = tokio::spawn(async move {
            let out = tr.fetch(&req).await;
            let _ = tx.send(NetDone {
                tab,
                handle: seq,
                cross: is_cross,
                mode: req.fetch_mode,
                credentials: explorer_net::http::FetchCredentials::SameOrigin,
                page_origin,
                req_url,
                partition,
                out,
                script: None,
                frame: Some(meta),
                img: None,
                link: None,
                dyn_module: None,
                import: None,
            });
        });
        self.dyn_tasks.insert((tab, seq), j);
    }

    /// Un `<img>` dynamique (CAP-172) : `srcset` → candidat
    /// choisi par `resolve_sizes`/`pick_srcset` (même pile
    /// que `collect_images` — viewport/dpr de session), sinon
    /// `src` ; `data:` est décodé **sans requête** ; sinon
    /// résolution base émetteur + `img-src` CSP +
    /// mixed-content puis `sub_request`
    /// (`Sec-Fetch-Dest: image`, `crossorigin` CORS,
    /// `referrerpolicy`). Refus → `error` sur l'élément.
    fn spawn_dyn_img(&mut self, tab: TabId, page: &Url, base: &Url, d: &explorer_js::DynImgReq) {
        let Some((req_page, req_base, doc_policy, csp, _opaque)) =
            self.ctx_net_origin(tab, d.ctx, page, base)
        else {
            self.deliver_dyn_img_el(tab, d.ctx, d.seq, false);
            return;
        };
        // Choix de la source — `srcset` prime sur `src`
        // (spec) ; la clé est la chaîne telle que
        // `collect_images` la relira au prochain apply.
        let src_key = d
            .srcset
            .as_deref()
            .and_then(|ss| {
                let eff_w = resolve_sizes(d.sizes.clone(), self.viewport.0);
                pick_srcset(ss, eff_w, self.scale)
            })
            .or_else(|| {
                d.src
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            });
        let Some(src_key) = src_key else {
            self.deliver_dyn_img_el(tab, d.ctx, d.seq, false);
            return;
        };
        // `data:` — corps auto-porteur, aucune requête
        // (même pile que le decode statique).
        if let Some(body) = data_url_bytes(&src_key) {
            let url = Url::parse(&src_key).unwrap_or_else(|_| req_page.clone());
            self.commit_dyn_img_body(tab, d.ctx, d.seq, src_key, &url, body);
            return;
        }
        let Some(url) = Url::resolve(&req_base, &src_key)
            .ok()
            .filter(|u| !is_mixed_content(&req_page, u))
            .filter(|u| {
                explorer_net::csp::csp_allows(
                    &csp,
                    &req_page,
                    explorer_net::http::FetchDest::Image,
                    u,
                )
            })
        else {
            self.deliver_dyn_img_el(tab, d.ctx, d.seq, false);
            return;
        };
        let policy = d
            .referrerpolicy
            .as_deref()
            .and_then(explorer_net::http::ReferrerPolicy::parse)
            .or(doc_policy);
        let cross = d.crossorigin.map(|c| match c {
            explorer_js::FetchCredentials::Omit => explorer_net::http::FetchCredentials::Omit,
            explorer_js::FetchCredentials::SameOrigin => {
                explorer_net::http::FetchCredentials::SameOrigin
            }
            explorer_js::FetchCredentials::Include => explorer_net::http::FetchCredentials::Include,
        });
        let req = sub_request(
            &req_page,
            &url,
            explorer_net::http::FetchDest::Image,
            policy,
            cross,
        );
        let is_cross = req_page.origin() != url.origin();
        let credentials = cross.unwrap_or(explorer_net::http::FetchCredentials::SameOrigin);
        let fetch_mode = req.fetch_mode;
        let page_origin = origin_header_value(&req_page);
        let partition = explorer_net::cookies::request_partition(&req);
        let req_url = req.url.clone();
        let meta = DynImgMeta {
            ctx: d.ctx,
            seq: d.seq,
            src_key,
            url,
            page: req_page,
        };
        let tr = self.transport.clone();
        let tx = self.net_tx.clone();
        let seq = d.seq;
        let j = tokio::spawn(async move {
            let out = tr.fetch(&req).await;
            let _ = tx.send(NetDone {
                tab,
                handle: seq,
                cross: is_cross,
                mode: fetch_mode,
                credentials,
                page_origin,
                req_url,
                partition,
                out,
                script: None,
                frame: None,
                img: Some(meta),
                link: None,
                dyn_module: None,
                import: None,
            });
        });
        self.dyn_tasks.insert((tab, seq), j);
    }

    /// Réception d'un `<img>` dynamique fetché (CAP-172) —
    /// statut 2xx + sécurité sous-ressource (`nosniff`/CORP)
    /// exigés, puis le corps est rangé par
    /// [`Self::commit_dyn_img_body`] ; refus → `error` sur
    /// l'élément.
    fn commit_dyn_img(&mut self, tab: TabId, meta: DynImgMeta, out: &NetResult<Response>) {
        let body = match out {
            Ok(resp)
                if (200..300).contains(&resp.status)
                    && subresp_security_allows(
                        &meta.page,
                        &meta.url,
                        explorer_net::http::FetchDest::Image,
                        resp,
                    ) =>
            {
                Some(resp.body.clone())
            }
            _ => None,
        };
        match body {
            Some(b) => {
                self.commit_dyn_img_body(tab, meta.ctx, meta.seq, meta.src_key, &meta.url, b);
            }
            None => self.deliver_dyn_img_el(tab, meta.ctx, meta.seq, false),
        }
    }

    /// Corps accepté d'un `<img>` dynamique (CAP-172) :
    /// décodage `decode_image` → `img_decoded[src_key]` (la
    /// resynchro fait le reste — `collect_images` rematche
    /// la même clé sur le vrai DOM) + corps compressé retenu
    /// dans `img_bodies` sous l'URL résolue (bfcache).
    /// `load` part sur l'élément quoi qu'il arrive ;
    /// l'échec de **décodage** → `error` (spec — un corps
    /// invalide n'est pas une image chargée).
    fn commit_dyn_img_body(
        &mut self,
        tab: TabId,
        ctx: explorer_js::JsCtxId,
        seq: u64,
        src_key: String,
        url: &Url,
        body: Vec<u8>,
    ) {
        let max_px = self.image_max_px;
        let img = decode_image(&body, max_px).map(Arc::new);
        let ok = img.is_some();
        if let Some(img) = img {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return;
            };
            entry.img_decoded.insert(src_key, img.clone());
            entry.bg_decoded.insert(url.as_str(), Some(img));
            retain_img_body(&mut entry.img_bodies, &url.as_str(), body);
        }
        // Échec de décodage → `error` : un corps invalide
        // n'est pas une image chargée (spec), et rien n'est
        // retenu (un blob invalide ne re-servirait pas).
        self.deliver_dyn_img_el(tab, ctx, seq, ok);
    }

    /// Un `<link rel=stylesheet>` dynamique (CAP-173) :
    /// résolution base émetteur + `style-src` CSP +
    /// mixed-content puis `sub_request`
    /// (`Sec-Fetch-Dest: style`, `crossorigin` CORS,
    /// `referrerpolicy`) — le fetch a toujours lieu
    /// (spec), l'application est gated par `media` à la
    /// livraison. Refus → `error` sur l'élément.
    fn spawn_dyn_link(&mut self, tab: TabId, page: &Url, base: &Url, d: &explorer_js::DynLinkReq) {
        let Some((req_page, req_base, doc_policy, csp, _opaque)) =
            self.ctx_net_origin(tab, d.ctx, page, base)
        else {
            tracing::debug!(tab, seq = d.seq, "dyn link : origine ctx introuvable");
            self.deliver_dyn_link_el(tab, d.ctx, d.seq, false);
            return;
        };
        let Some(url) = Url::resolve(&req_base, &d.href)
            .ok()
            .filter(|u| !is_mixed_content(&req_page, u))
            .filter(|u| {
                explorer_net::csp::csp_allows(
                    &csp,
                    &req_page,
                    explorer_net::http::FetchDest::Style,
                    u,
                )
            })
        else {
            tracing::debug!(tab, seq = d.seq, href = %d.href, "dyn link : URL/CSP refusée");
            self.deliver_dyn_link_el(tab, d.ctx, d.seq, false);
            return;
        };
        let policy = d
            .referrerpolicy
            .as_deref()
            .and_then(explorer_net::http::ReferrerPolicy::parse)
            .or(doc_policy);
        let cross = d.crossorigin.map(|c| match c {
            explorer_js::FetchCredentials::Omit => explorer_net::http::FetchCredentials::Omit,
            explorer_js::FetchCredentials::SameOrigin => {
                explorer_net::http::FetchCredentials::SameOrigin
            }
            explorer_js::FetchCredentials::Include => explorer_net::http::FetchCredentials::Include,
        });
        let req = sub_request(
            &req_page,
            &url,
            explorer_net::http::FetchDest::Style,
            policy,
            cross,
        );
        let is_cross = req_page.origin() != url.origin();
        let credentials = cross.unwrap_or(explorer_net::http::FetchCredentials::SameOrigin);
        let fetch_mode = req.fetch_mode;
        let page_origin = origin_header_value(&req_page);
        let partition = explorer_net::cookies::request_partition(&req);
        let req_url = req.url.clone();
        let meta = DynLinkMeta {
            ctx: d.ctx,
            seq: d.seq,
            integrity: d.integrity.clone(),
            media: d.media.clone(),
            url,
            page: req_page,
        };
        let tr = self.transport.clone();
        let tx = self.net_tx.clone();
        let seq = d.seq;
        tracing::debug!(tab, seq, url = %req_url, "dyn link : fetch émis");
        let j = tokio::spawn(async move {
            let out = tr.fetch(&req).await;
            let _ = tx.send(NetDone {
                tab,
                handle: seq,
                cross: is_cross,
                mode: fetch_mode,
                credentials,
                page_origin,
                req_url,
                partition,
                out,
                script: None,
                frame: None,
                img: None,
                link: Some(meta),
                dyn_module: None,
                import: None,
            });
        });
        self.dyn_tasks.insert((tab, seq), j);
    }

    /// Réception d'un `<link>` dynamique fetché (CAP-173).
    /// Statut 2xx, sécurité sous-ressource (`nosniff`/CORP)
    /// et SRI (`integrity`, CAP-163) exigés ; la feuille
    /// décodée rejoint `linked_sheets` (la cascade la reprend
    /// au prochain apply, gated `media`), puis `load` part
    /// sur l'élément — sinon `error`.
    fn commit_dyn_link(&mut self, tab: TabId, meta: &DynLinkMeta, out: &NetResult<Response>) {
        let text = match out {
            Ok(resp)
                if (200..300).contains(&resp.status)
                    && subresp_security_allows(
                        &meta.page,
                        &meta.url,
                        explorer_net::http::FetchDest::Style,
                        resp,
                    )
                    && meta
                        .integrity
                        .as_deref()
                        .is_none_or(|m| explorer_net::crypto::integrity_allows(m, &resp.body)) =>
            {
                Some(decode_stylesheet(&resp.body, &resp.headers))
            }
            _ => None,
        };
        let ok = match (text, &meta.media) {
            // `media` présent et non-matching : le fetch a
            // lieu (spec) mais la feuille ne s'applique pas
            // — `load` part quand même (même traitement que
            // la pile statique).
            (Some(t), media) => {
                let scripting = self.tabs.get(&tab).is_some_and(|e| e.js.is_some());
                let env = super::apply::viewport_env(self.viewport, self.color_scheme, scripting);
                let applies = media.as_deref().is_none_or(|m| media_attr_matches(&env, m));
                if applies {
                    if let Some(entry) = self.tabs.get_mut(&tab) {
                        if entry.linked_sheets.len() < 64 {
                            entry.linked_sheets.push(t);
                        }
                    }
                }
                true
            }
            _ => false,
        };
        tracing::debug!(tab, seq = meta.seq, ok, url = %meta.url, "deliver dyn link");
        self.deliver_dyn_link_el(tab, meta.ctx, meta.seq, ok);
    }

    /// `load`/`error` sur l'élément `<link>` miroir
    /// (CAP-173) — relais mince de `deliver_dyn_link` avec
    /// resynchro (la livraison peut muter le DOM du ctx et
    /// la feuille rangée doit restyler).
    fn deliver_dyn_link_el(&mut self, tab: TabId, ctx: explorer_js::JsCtxId, seq: u64, ok: bool) {
        let mutated = {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return;
            };
            let Some(eng) = entry.js.as_mut() else {
                return;
            };
            eng.deliver_dyn_link(ctx, seq, ok);
            let n = eng.run_pending();
            tracing::trace!(tab, jobs = n, "jobs js après livraison link dyn");
            Self::sync_for_apply(eng.as_mut())
        };
        if let Some(s) = mutated {
            self.queue_apply_sync(tab, s);
        }
        self.after_script(tab);
    }

    /// `load`/`error` sur l'élément `<img>` miroir (CAP-172) —
    /// relais mince de `deliver_dyn_img` avec resynchro (la
    /// livraison peut muter le DOM du ctx).
    fn deliver_dyn_img_el(&mut self, tab: TabId, ctx: explorer_js::JsCtxId, seq: u64, ok: bool) {
        let mutated = {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return;
            };
            let Some(eng) = entry.js.as_mut() else {
                return;
            };
            eng.deliver_dyn_img(ctx, seq, ok);
            let n = eng.run_pending();
            tracing::trace!(tab, jobs = n, "jobs js après livraison img dyn");
            Self::sync_for_apply(eng.as_mut())
        };
        if let Some(s) = mutated {
            self.queue_apply_sync(tab, s);
        }
        self.after_script(tab);
    }

    /// Réception d'un `<iframe>` dynamique fetché (CAP-171) —
    /// la réponse passe `collect_frame` (XFO/`frame-ancestors`/
    /// `Set-Cookie`/parse — même pile que l'étape 128, sécurité
    /// sous-ressource + statut 2xx) puis rejoint
    /// `entry.frames` ; refus → `error` sur l'élément.
    fn commit_dyn_frame_resp(
        &mut self,
        tab: TabId,
        meta: &DynFrameMeta,
        out: &NetResult<Response>,
    ) {
        let parent_scripting = self.tabs.get(&tab).is_some_and(|e| e.js.is_some());
        let mut got = Vec::new();
        if let Ok(resp) = &out {
            if (200..300).contains(&resp.status)
                && subresp_security_allows(
                    &meta.page,
                    &meta.url,
                    explorer_net::http::FetchDest::Iframe,
                    resp,
                )
            {
                collect_frame(
                    &meta.page,
                    &self.cookies,
                    &meta.url,
                    resp,
                    meta.sandbox,
                    parent_scripting,
                    &mut got,
                );
            }
        }
        match got.pop() {
            Some(sf) => self.install_dyn_frame_ctx(tab, meta, sf),
            None => self.deliver_dyn_frame_el(tab, meta.ctx, meta.seq, false),
        }
    }

    /// Document `srcdoc` d'un `<iframe>` dynamique (CAP-171)
    /// — aucune requête : `SubFrame` inline puis même
    /// câblage que [`Self::commit_dyn_frame_resp`].
    fn commit_dyn_frame(&mut self, tab: TabId, meta: &DynFrameMeta, sf: SubFrame) {
        self.install_dyn_frame_ctx(tab, meta, sf);
    }

    /// Câblage du sous-contexte dynamique (CAP-171) — même
    /// séquence que `wire_frame_contexts` mais pour un cadre
    /// arrivé en cours de vie : realm enfant du ctx émetteur,
    /// `install_dom_ctx`, scripts inline, cycle `readyState`
    /// (`interactive` → `readystatechange` → DCL →
    /// `complete` → `readystatechange` → `load`, ordre spec)
    /// puis `load` sur l'élément. Le `FrameCtx` est poussé
    /// pour `ctx_net_origin`/drains postérieurs ; `node`
    /// est une sentinelle (le nœud réel n'existe qu'après
    /// resynchro — le `load` élément passe par le miroir).
    fn install_dyn_frame_ctx(&mut self, tab: TabId, meta: &DynFrameMeta, sf: SubFrame) {
        // `max_frames` partagé avec le plan statique (une
        // page hostile ne gonfle pas `entry.frames` par
        // insertions en boucle).
        if self
            .tabs
            .get(&tab)
            .is_some_and(|e| e.frames.len() >= self.max_frames)
        {
            self.deliver_dyn_frame_el(tab, meta.ctx, meta.seq, false);
            return;
        }
        let url = sf.url.clone();
        let html = sf.html.clone();
        let opaque = sf.sandbox.is_some_and(|s| !s.same_origin);
        // Phase 1 : `SubFrame` rangé, scripts inline collectés
        // (mêmes contrôles CSP/sandbox que le câblage
        // statique), realm créé — `frame_ord` relie l'élément
        // pour `contentWindow`.
        let Some((sub, scripts, ctx)) = self.prepare_dyn_frame(tab, meta, sf, &url) else {
            // Realm indisponible → `error` sur l'élément (le
            // `SubFrame` reste rendu, comme un cadre statique
            // sans JS).
            self.deliver_dyn_frame_el(tab, meta.ctx, meta.seq, false);
            return;
        };
        // Phase 2 : état d'origine — la clé opaque est liée
        // au `ctx` frais (même convention que
        // `wire_frame_contexts`).
        let (local, cookie) = if opaque {
            (
                self.local_storage
                    .get(&format!("opaque:{ctx}"))
                    .cloned()
                    .unwrap_or_default(),
                String::new(),
            )
        } else {
            self.origin_state(&url)
        };
        // Phase 3 : miroir installé, scripts évalués, cycle
        // `readyState` complet puis `load` sur l'élément.
        let evs = self.events.clone();
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        let Some(eng) = entry.js.as_mut() else {
            return;
        };
        eng.install_dom_ctx(ctx, &html, &url.to_string(), local, &cookie);
        // « loading » jusqu'au cycle interactive/complete —
        // retient les `postMessage` entrants pendant les
        // scripts du cadre dynamique (étape 130).
        eng.set_ready_state(ctx, "loading");
        for src in scripts {
            if let Err(e) = eng.eval_ctx(ctx, &src, None) {
                let _ = evs.send(CoreEvent::PipelineError {
                    tab: Some(tab),
                    message: format!("{e} — iframe dynamique"),
                });
            }
        }
        eng.set_ready_state(ctx, "interactive");
        let _ = eng.dispatch_event_ctx(ctx, "readystatechange", &[]);
        let _ = eng.dispatch_event_ctx(ctx, "DOMContentLoaded", &[]);
        eng.set_ready_state(ctx, "complete");
        let _ = eng.dispatch_event_ctx(ctx, "readystatechange", &[]);
        let _ = eng.dispatch_event_ctx(ctx, "load", &[]);
        entry.frame_ctxs.push(FrameCtx {
            ctx,
            sub,
            url,
            node: explorer_dom::NodeId::from_parts(u32::MAX, u32::MAX),
            opaque,
            load_fired: true,
        });
        // `load` sur l'élément — il vit dans le miroir du
        // ctx **émetteur** (`meta.ctx`), pas de l'enfant.
        eng.deliver_dyn_frame(meta.ctx, meta.seq, true);
        let n = eng.run_pending();
        tracing::trace!(tab, jobs = n, "jobs js après iframe dynamique");
        self.after_script(tab);
    }

    /// Phase 1 de [`Self::install_dyn_frame_ctx`] (CAP-171) :
    /// pousse le `SubFrame`, collecte ses scripts inline
    /// (mêmes contrôles CSP/`sandbox` que le câblage
    /// statique — `srcdoc` hérite la CSP de l'émetteur,
    /// `<script>` externes non fetchés à cette tranche) et
    /// crée le realm **sous le ctx émetteur**
    /// (`context_new_in` — `frame_ord` résolu dans son
    /// miroir). `(index frames, scripts, ctx)` — `None` =
    /// realm indisponible (le `SubFrame` reste rendu).
    fn prepare_dyn_frame(
        &mut self,
        tab: TabId,
        meta: &DynFrameMeta,
        sf: SubFrame,
        url: &Url,
    ) -> Option<(usize, Vec<String>, explorer_js::JsCtxId)> {
        let entry = self.tabs.get_mut(&tab)?;
        entry.frames.push(sf);
        let sub = entry.frames.len() - 1;
        let f = &entry.frames[sub];
        let (f_base, f_csp) = if f.srcdoc.is_some() {
            (
                entry
                    .nav_base()
                    .cloned()
                    .unwrap_or_else(|| entry.url.clone().unwrap_or_else(|| url.clone())),
                entry.csp.clone(),
            )
        } else {
            (
                document_base(&f.dom, &f.url)
                    .0
                    .unwrap_or_else(|| url.clone()),
                collect_meta_csp(&f.dom)
                    .iter()
                    .map(|v| explorer_net::csp::CspPolicy::parse(v, true))
                    .collect(),
            )
        };
        let scripts = if f.sandbox.is_some_and(|s| !s.scripts) {
            Vec::new()
        } else {
            collect_scripts(&f.dom, &f.url, &f_base, true, 0, &f_csp)
                .into_iter()
                .filter_map(|i| (!i.module).then_some(i.inline).flatten())
                .collect()
        };
        let ctx = entry
            .js
            .as_mut()
            .and_then(|eng| eng.context_new_in(meta.ctx, &url.to_string(), Some(meta.ord)))?;
        Some((sub, scripts, ctx))
    }

    /// `load`/`error` sur l'élément `<iframe>` miroir
    /// (CAP-171) — relais mince de `deliver_dyn_frame` avec
    /// resynchro (la livraison peut muter le DOM du ctx).
    fn deliver_dyn_frame_el(&mut self, tab: TabId, ctx: explorer_js::JsCtxId, seq: u64, ok: bool) {
        let mutated = {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return;
            };
            let Some(eng) = entry.js.as_mut() else {
                return;
            };
            eng.deliver_dyn_frame(ctx, seq, ok);
            let n = eng.run_pending();
            tracing::trace!(tab, jobs = n, "jobs js après livraison iframe dyn");
            Self::sync_for_apply(eng.as_mut())
        };
        if let Some(s) = mutated {
            self.queue_apply_sync(tab, s);
        }
        self.after_script(tab);
    }

    /// Résolution de l'origine réseau d'un contexte (étape
    /// 134, extraite de `spawn_script_fetch` — partagée par
    /// les `<script>` dynamiques CAP-169) : document
    /// émetteur (page effective + base `<base href>`) +
    /// politique Referer + CSP + origine opaque sandboxée.
    /// `ctx` inconnu (cadre mort) → `None`.
    #[allow(clippy::type_complexity)]
    fn ctx_net_origin(
        &self,
        tab: TabId,
        ctx: explorer_js::JsCtxId,
        page: &Url,
        base: &Url,
    ) -> Option<(
        Url,
        Url,
        Option<explorer_net::http::ReferrerPolicy>,
        Vec<explorer_net::csp::CspPolicy>,
        bool,
    )> {
        if ctx == explorer_js::JS_CTX_TOP {
            let e = self.tabs.get(&tab)?;
            return Some((
                page.clone(),
                e.nav_base().cloned().unwrap_or_else(|| base.clone()),
                e.referrer_policy,
                e.csp.clone(),
                false,
            ));
        }
        let (f_url, f_opaque, sf) = self
            .tabs
            .get(&tab)
            .and_then(|e| e.frame_ctxs.iter().find(|f| f.ctx == ctx))
            .and_then(|f| {
                self.tabs
                    .get(&tab)
                    .and_then(|e| e.frames.get(f.sub))
                    .map(|sf| (f.url.clone(), f.opaque, sf))
            })?;
        let req_base = if sf.srcdoc.is_some() {
            base.clone()
        } else {
            document_base(&sf.dom, &f_url)
                .0
                .unwrap_or_else(|| f_url.clone())
        };
        Some((
            f_url.clone(),
            req_base,
            meta_referrer_policy(&sf.dom),
            collect_meta_csp(&sf.dom)
                .iter()
                .map(|v| explorer_net::csp::CspPolicy::parse(v, true))
                .collect(),
            f_opaque,
        ))
    }

    /// Branche `Fetch` de [`Self::drain_script_net`] (étape
    /// 134 — extraite pour rester lisible) : la requête d'un
    /// cadre est émise sous **son** initiateur — base de
    /// résolution, `Origin`/`Referer` et site top-level
    /// (cookies `Partitioned`) sont ceux du document enfant,
    /// pas du parent. `ctx` inconnu (cadre mort entre
    /// l'émission et le drain) → `Err` (échec livré, jamais
    /// de requête sous la mauvaise identité).
    #[allow(clippy::too_many_arguments)]
    fn spawn_script_fetch(
        &mut self,
        tab: TabId,
        page: &Url,
        base: &Url,
        handle: u64,
        method: &str,
        url: &str,
        headers: &[(String, String)],
        body: Option<&str>,
        mode: explorer_js::FetchMode,
        credentials: explorer_js::FetchCredentials,
        ctx: explorer_js::JsCtxId,
    ) -> Result<(), ()> {
        // Document émetteur (étape 134) + sa base effective
        // (étape 135) + ses politiques Referer (132) et CSP
        // (CAP-145) — un `fetch` de cadre vit sous **son**
        // document, pas sous la page top. `opaque` = origine
        // opaque sandboxée (CAP-149).
        // Document émetteur (étape 134) + sa base effective
        // (étape 135) + ses politiques Referer (132) et CSP
        // (CAP-145) — un `fetch` de cadre vit sous **son**
        // document, pas sous la page top. `opaque` = origine
        // opaque sandboxée (CAP-149).
        let Some((req_page, req_base, policy, csp, opaque)) =
            self.ctx_net_origin(tab, ctx, page, base)
        else {
            tracing::trace!(tab, handle, ctx, "fetch d'un contexte mort");
            return Err(());
        };
        let inflight = self.net_tasks.keys().filter(|(t, _)| *t == tab).count();
        if inflight >= self.max_script_net {
            tracing::trace!(tab, handle, "quota requêtes script atteint");
            return Err(());
        }
        let Some((req, cross)) = self.script_request(
            &req_page,
            &req_base,
            method,
            url,
            headers,
            body,
            policy,
            &csp,
            opaque,
            mode,
            credentials,
        ) else {
            return Err(());
        };
        let tr = self.transport.clone();
        let tx = self.net_tx.clone();
        // Origine opaque : `null` (sérialisation spec) — ACAO
        // `*` reste admis, un `ACAO: <origine>` ne l'est pas.
        let page_origin = if opaque {
            "null".to_string()
        } else {
            origin_header_value(&req_page)
        };
        let req_url = req.url.clone();
        // `credentials` du script (JS-C77) — porté hors
        // `req.cors` : `no-cors`/`include` (`sendBeacon`
        // cross) capture aussi `Set-Cookie`.
        let credentials = match credentials {
            explorer_js::FetchCredentials::Omit => explorer_net::http::FetchCredentials::Omit,
            explorer_js::FetchCredentials::SameOrigin => {
                explorer_net::http::FetchCredentials::SameOrigin
            }
            explorer_js::FetchCredentials::Include => explorer_net::http::FetchCredentials::Include,
        };
        let partition = explorer_net::cookies::request_partition(&req);
        let fetch_mode = req.fetch_mode;
        let j = tokio::spawn(async move {
            let out = tr.fetch(&req).await;
            let _ = tx.send(NetDone {
                tab,
                handle,
                cross,
                mode: fetch_mode,
                credentials,
                page_origin,
                req_url,
                partition,
                out,
                script: None,
                frame: None,
                img: None,
                link: None,
                dyn_module: None,
                import: None,
            });
        });
        self.net_tasks.insert((tab, handle), j);
        Ok(())
    }

    /// Ouverture d'un flux SSE `EventSource` (JS-C92) —
    /// `fetch_stream` : `open` livré aux en-têtes si le
    /// serveur sert bien `text/event-stream` en 200, un
    /// `message` par événement parsé au fil des chunks,
    /// `error` à l'échec ou à la fin de flux (borne
    /// documentée : pas de reconnexion `Last-Event-ID` à
    /// cette tranche — le script ré-ouvre s'il veut).
    /// Ouverture `WebSocket` (JS-C94) — `ctx_net_origin`
    /// porte l'initiateur (origine `Origin`, base, CSP
    /// `connect-src` via `FetchDest::Empty`, cookies du
    /// cadre) ; la socket dédiée est possédée par la tâche
    /// `WsDial` (jamais dans le pool HTTP). `Sec-WebSocket-
    /// Protocol` = liste script jointe par virgule (spec).
    fn spawn_ws(&mut self, spec: &WsSpawnArgs<'_>) -> Result<(), ()> {
        let &WsSpawnArgs {
            tab,
            page,
            base,
            handle,
            url,
            protocols,
            ctx,
        } = spec;
        let Some((req_page, _req_base, _policy, csp, _opaque)) =
            self.ctx_net_origin(tab, ctx, page, base)
        else {
            return Err(());
        };
        let inflight = self.net_tasks.keys().filter(|(t, _)| *t == tab).count();
        if inflight >= self.max_script_net {
            return Err(());
        }
        let secure = url.starts_with("wss:");
        // `connect-src` : l'URL HTTP correspondante sert à la
        // vérification (`ws:`/`wss:` mappés `http:`/`https:` —
        // spec Fetch « WebSocket connections »). On parse
        // l'URL **mappée** — `Url` ne connaît pas `ws:` comme
        // schéma hiérarchique (`host()` serait vide).
        let http_url = Url::parse(&format!(
            "{}{}",
            if secure { "https" } else { "http" },
            &url[url.find(':').unwrap_or(2)..]
        ))
        .map_err(|_| ())?;
        let ws_url = &http_url;
        if !explorer_net::csp::csp_allows(
            &csp,
            &req_page,
            explorer_net::http::FetchDest::Empty,
            &http_url,
        ) {
            return Err(());
        }
        let mut headers = vec![("Origin".into(), origin_header_value(&req_page))];
        if !protocols.is_empty() {
            headers.push(("Sec-WebSocket-Protocol".into(), protocols.join(", ")));
        }
        // Cookies : la spec WS les envoie comme une requête
        // `same-origin`/`include` selon l'origine — borne
        // v1 : header du jar si same-origin (cross-origin
        // sans `SameSite=None` ne passe de toute façon pas).
        if req_page.origin() == http_url.origin() {
            let c = self.cookies.header_for(&http_url, false);
            if !c.is_empty() {
                headers.push(("Cookie".into(), c));
            }
        }
        let ws_spec = explorer_net::ws::WsSpec {
            secure,
            host: ws_url.host(),
            port: ws_url.port().unwrap_or(if secure { 443 } else { 80 }),
            resource: ws_url.path_query(),
            headers,
        };
        let Some(dial) = self.ws_dial.clone() else {
            return Err(());
        };
        let (send_tx, send_rx) = tokio::sync::mpsc::channel::<explorer_net::ws::WsSend>(64);
        self.ws_sends.insert((tab, handle), send_tx);
        let tx = self.ws_tx.clone();
        let j = tokio::spawn(async move {
            // Adaptateur événements → canal session (même
            // discipline `EsDone` : intentions drainées).
            // `WsDial` veut du tokio mpsc ; `ws_tx` est le
            // `std::mpsc` session (`send` non bloquant).
            let (ev_tx, mut ev_rx) = tokio::sync::mpsc::channel::<explorer_net::ws::WsEvent>(64);
            let tx_fwd = tx.clone();
            let fwd = tokio::spawn(async move {
                while let Some(ev) = ev_rx.recv().await {
                    let _ = tx_fwd.send(WsDone {
                        tab,
                        ctx,
                        handle,
                        ev,
                    });
                }
            });
            let r = dial.open(ws_spec, ev_tx, send_rx).await;
            if let Err(e) = r {
                // Échec dial/handshake → `Error` (la session
                // normalise `error` + `close 1006`).
                let _ = tx.send(WsDone {
                    tab,
                    ctx,
                    handle,
                    ev: explorer_net::ws::WsEvent::Error(e.to_string()),
                });
            }
            let _ = fwd.await;
        });
        self.net_tasks.insert((tab, handle), j);
        Ok(())
    }

    /// Livraison d'un événement WS : traduction net→moteur,
    /// `Error` → `error` + `close {1006, wasClean:false}`
    /// (spec « fail the WebSocket connection » — les
    /// fallbacks ne voient jamais un faux `open`).
    fn deliver_ws(
        &mut self,
        tab: TabId,
        ctx: explorer_js::JsCtxId,
        handle: u64,
        ev: explorer_net::ws::WsEvent,
    ) {
        use explorer_js::WsEventKind as K;
        let kinds: Vec<K> = match ev {
            explorer_net::ws::WsEvent::Open { protocol } => {
                vec![K::Open(protocol)]
            }
            explorer_net::ws::WsEvent::Text(s) => vec![K::Text(s)],
            explorer_net::ws::WsEvent::Binary(b) => vec![K::Binary(b)],
            explorer_net::ws::WsEvent::Close { code, reason } => vec![K::Close {
                code,
                reason,
                clean: true,
            }],
            explorer_net::ws::WsEvent::Error(_) => vec![
                K::Error,
                K::Close {
                    code: 1006,
                    reason: String::new(),
                    clean: false,
                },
            ],
        };
        let mutated = {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return;
            };
            if entry.js_detached.is_some() {
                entry
                    .js_pending_ws
                    .extend(kinds.into_iter().map(|k| (ctx, handle, k)));
                return;
            }
            let Some(eng) = entry.js.as_mut() else {
                return;
            };
            for k in kinds {
                if let Err(e) = eng.deliver_ws_event(ctx, handle, k) {
                    tracing::debug!(tab, error = %e, "livraison websocket js");
                    let _ = self.events.send(CoreEvent::PipelineError {
                        tab: Some(tab),
                        message: e.to_string(),
                    });
                }
            }
            let n = eng.run_pending();
            tracing::trace!(tab, jobs = n, "jobs js après événement ws");
            Self::sync_for_apply(eng.as_mut())
        };
        if let Some(s) = mutated {
            self.queue_apply_sync(tab, s);
        }
        self.after_script(tab);
    }

    /// `ctx_net_origin` porte le document **émetteur** (base
    /// résolue côté moteur, CSP `connect-src` via
    /// `FetchDest::Empty`, politique Referer du cadre) ;
    /// `withCredentials` → `credentials: include`.
    fn spawn_es(&mut self, spec: &EsSpawnArgs<'_>) -> Result<(), ()> {
        let &EsSpawnArgs {
            tab,
            page,
            base,
            handle,
            url,
            with_credentials,
            ctx,
        } = spec;
        let Some((req_page, req_base, policy, csp, opaque)) =
            self.ctx_net_origin(tab, ctx, page, base)
        else {
            tracing::trace!(tab, handle, ctx, "eventsource d'un contexte mort");
            return Err(());
        };
        let inflight = self.net_tasks.keys().filter(|(t, _)| *t == tab).count();
        if inflight >= self.max_script_net {
            tracing::trace!(tab, handle, "quota requêtes script atteint (eventsource)");
            return Err(());
        }
        let creds = if with_credentials {
            explorer_js::FetchCredentials::Include
        } else {
            explorer_js::FetchCredentials::SameOrigin
        };
        // Spec : `Accept: text/event-stream`, mode `cors` —
        // `connect-src` consulté comme un XHR.
        let Some((req, _cross)) = self.script_request(
            &req_page,
            &req_base,
            "GET",
            url,
            &[("Accept".into(), "text/event-stream".into())],
            None,
            policy,
            &csp,
            opaque,
            explorer_js::FetchMode::Cors,
            creds,
        ) else {
            return Err(());
        };
        let tr = self.transport.clone();
        let tx = self.es_tx.clone();
        let j = tokio::spawn(async move {
            let send = |ev: explorer_js::EsEvent| {
                let _ = tx.send(EsDone {
                    tab,
                    ctx,
                    handle,
                    ev,
                });
            };
            let ok = tr.fetch_stream(&req).await.ok().filter(|r| {
                r.status == 200
                    && r.header("content-type").is_some_and(|c| {
                        c.split(';')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .eq_ignore_ascii_case("text/event-stream")
                    })
            });
            let Some(resp) = ok else {
                send(explorer_js::EsEvent::Error);
                return;
            };
            send(explorer_js::EsEvent::Open);
            let mut parser = explorer_net::sse::SseParser::new();
            let mut body = resp.body;
            while let Some(chunk) = body.recv().await {
                let Ok(bytes) = chunk else { break };
                for ev in parser.feed(&bytes) {
                    send(explorer_js::EsEvent::Message {
                        data: ev.data,
                        event: ev.event,
                        id: ev.id,
                    });
                }
            }
            send(explorer_js::EsEvent::Error);
        });
        self.net_tasks.insert((tab, handle), j);
        Ok(())
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

    /// Livre un résultat `crypto.subtle` au moteur (JS-C79) —
    /// même discipline que [`Self::deliver_net_response`] :
    /// buffer si le moteur est détaché (réinséré au flush),
    /// tour post-script complet (microtasks + resynchro +
    /// drains — le handler `then` peut émettre de nouvelles
    /// requêtes, crypto ou réseau).
    fn deliver_crypto_response(&mut self, tab: TabId, resp: explorer_js::CryptoResponse) {
        let mutated = {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return;
            };
            if entry.js_detached.is_some() {
                entry.js_pending_crypto.push(resp);
                return;
            }
            let Some(eng) = entry.js.as_mut() else {
                return;
            };
            if let Err(e) = eng.deliver_crypto_response(resp) {
                tracing::debug!(tab, error = %e, "livraison crypto js");
                let _ = self.events.send(CoreEvent::PipelineError {
                    tab: Some(tab),
                    message: e.to_string(),
                });
            }
            let n = eng.run_pending();
            tracing::trace!(tab, jobs = n, "jobs js après réponse crypto");
            Self::sync_for_apply(eng.as_mut())
        };
        if let Some(s) = mutated {
            self.queue_apply_sync(tab, s);
        }
        self.after_script(tab);
    }

    /// Livre un `EsEvent` au flux `handle` du contexte `ctx`
    /// (JS-C92) — même discipline que
    /// [`Self::deliver_crypto_response`] : buffer si le moteur
    /// est détaché (rejoué à la réinsertion), tour post-script
    /// complet — un handler `message` peut muter le DOM et
    /// émettre fetch/XHR/`postMessage`.
    fn deliver_es(
        &mut self,
        tab: TabId,
        ctx: explorer_js::JsCtxId,
        handle: u64,
        ev: explorer_js::EsEvent,
    ) {
        let mutated = {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return;
            };
            if entry.js_detached.is_some() {
                entry.js_pending_es.push((ctx, handle, ev));
                return;
            }
            let Some(eng) = entry.js.as_mut() else {
                return;
            };
            if let Err(e) = eng.deliver_es_event(ctx, handle, ev) {
                tracing::debug!(tab, error = %e, "livraison eventsource js");
                let _ = self.events.send(CoreEvent::PipelineError {
                    tab: Some(tab),
                    message: e.to_string(),
                });
            }
            let n = eng.run_pending();
            tracing::trace!(tab, jobs = n, "jobs js après événement sse");
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
        if self.js_stalled(tab) {
            return;
        }
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
            // `document.cookie` écrit dans le contexte de la
            // page — un `Partitioned` posé par script se range
            // sous le site de la page (étape 127).
            let partition = explorer_net::cookies::partition_key(&page);
            for w in &writes {
                self.cookies
                    .set_from_header_ctx(&page, &strip_http_only(w), partition.clone());
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
        // Étape 134 : surfaces script des contextes enfants —
        // cookies rangés sous l'URL **de l'enfant** (partition
        // du site top-level, spec `Partitioned`) et
        // `localStorage` sous l'origine enfant. Jamais sous
        // l'identité du parent. **Avant** `drain_script_net` :
        // un `document.cookie` suivi d'un `fetch` dans le même
        // script doit émettre le cookie — comme le top-level.
        let sinks: Vec<FrameSink> = self
            .tabs
            .get_mut(&tab)
            .map(|e| {
                e.js.as_mut().map_or_else(Vec::new, |eng| {
                    e.frame_ctxs
                        .iter()
                        .map(|f| {
                            let (writes, map) = eng.drain_sinks_ctx(f.ctx);
                            (f.url.clone(), f.opaque, f.ctx, writes, map)
                        })
                        .collect()
                })
            })
            .unwrap_or_default();
        if let Some(page) = self.tabs.get(&tab).and_then(|e| e.url.clone()) {
            for (furl, opaque, ctx, writes, map) in sinks {
                // Origine opaque (CAP-149) : `document.cookie`
                // est muet (écritures jetées — pas de fuite
                // vers le vrai jar) et `localStorage` vit
                // sous une clé éphémère par contexte, jamais
                // persistée (une origine opaque n'a pas
                // d'identité durable — spec HTML).
                if opaque {
                    let key = format!("opaque:{ctx}");
                    let changed = match self.local_storage.get(&key) {
                        Some(prev) => *prev != map,
                        None => !map.is_empty(),
                    };
                    if changed {
                        self.local_storage.insert(key, map);
                    }
                    continue;
                }
                let partition = explorer_net::cookies::partition_key(&page);
                for w in &writes {
                    self.cookies
                        .set_from_header_ctx(&furl, &strip_http_only(w), partition.clone());
                }
                if !writes.is_empty() {
                    self.persist_cookies();
                }
                let key = furl.origin().to_string();
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
        }
        self.drain_script_net(tab);
        // Étape 134 : les miroirs des contextes `<iframe>`
        // mutés par leurs scripts reviennent en
        // `SubFrame.dom` (le composite re-rend au relayout).
        self.sync_frame_mirrors(tab);
        self.drain_media_commands(tab);
    }

    /// Livre les réponses arrivées — synchrone, sans yield :
    /// appelée aux points de synchro internes (événements)
    /// **et** par les hôtes partagés — les resynchros DOM
    /// rapportées sont enfilées (étape 55h), à drainer via
    /// `drain_applies_shared` hors verrou ou `drain_applies`
    /// sous `&mut self`.
    #[allow(clippy::too_many_lines)] // dispatch séquentiel des NetDone — chaque variante documentée in situ
    pub fn poll_net(&mut self) -> usize {
        let mut done = Vec::new();
        while let Ok(d) = self.net_rx.try_recv() {
            done.push(d);
        }
        // Résultats `crypto.subtle` (JS-C79) — même pompe :
        // la livraison passe par `deliver_crypto_response`
        // (buffer détaché + tour post-script inclus).
        let mut crypto_done = Vec::new();
        while let Ok(d) = self.crypto_rx.try_recv() {
            crypto_done.push(d);
        }
        for d in crypto_done {
            self.deliver_crypto_response(
                d.tab,
                explorer_js::CryptoResponse {
                    handle: d.handle,
                    result: d.result,
                },
            );
        }
        self.drain_es_events();
        self.drain_ws_events();
        let n = done.len();
        self.drain_net_done(done);
        // Des `Set-Cookie` XHR ont pu enrichir le jar —
        // persistance au même point de synchro.
        if n > 0 {
            self.persist_cookies();
        }
        n
    }

    /// Livraison d'un `<script>` dynamique arrivé
    /// (CAP-169/186) — sécurité sous-ressource + SRI,
    /// décodage, puis `deliver_dyn_script` ou ensemence
    /// `spawn_dyn_module_graph` selon `meta.module`.
    fn deliver_dyn_done(&mut self, d: &NetDone, meta: &DynScriptMeta) {
        self.dyn_tasks.remove(&(d.tab, meta.seq));
        let body = match &d.out {
            Ok(resp)
                if (200..300).contains(&resp.status)
                    && subresp_security_allows(
                        &meta.page,
                        &d.req_url,
                        explorer_net::http::FetchDest::Script,
                        resp,
                    )
                    && sri_allows(meta.integrity.as_deref(), &resp.body) =>
            {
                Some(decode_subresource(&resp.body, meta.charset.as_deref()))
            }
            _ => None,
        };
        // CAP-186 : le corps vérifié ensemence le
        // graphe de modules (deps fetchées hors
        // session) — `error` si l'entrée est refusée.
        if meta.module {
            match body {
                Some(text) => {
                    let Some((req_page, _rb, doc_policy, csp, _o)) =
                        self.ctx_net_origin(d.tab, meta.ctx, &meta.page, &meta.page)
                    else {
                        return;
                    };
                    self.spawn_dyn_module_graph(
                        d.tab,
                        meta.ctx,
                        meta.seq,
                        d.req_url.to_string(),
                        text,
                        d.req_url.clone(),
                        req_page,
                        doc_policy,
                        csp,
                    );
                }
                None => self.deliver_dyn_module(
                    d.tab,
                    DynModuleDone {
                        ctx: meta.ctx,
                        seq: meta.seq,
                        name: String::new(),
                        graph: None,
                    },
                ),
            }
        } else {
            self.deliver_dyn_script(d.tab, meta.ctx, meta.seq, body);
        }
    }

    /// Dispatch des réponses `NetDone` collectées —
    /// extrait de `poll_net` (étape 51 + CAP-169/186/JS-C94
    /// ont fait passer la fonction au-delà de 100 lignes) :
    /// `<script>` dynamique, graphe module, `<img>`, `<link>`,
    /// WS et fetch/XHR y sont livrés dans l'ordre d'arrivée.
    fn drain_net_done(&mut self, done: Vec<NetDone>) {
        for mut d in done {
            // `<script>` dynamique (CAP-169) — la tâche vit
            // dans `dyn_tasks` (son `handle` est le `seq`
            // moteur, espace distinct des handles fetch) :
            // sécurité sous-ressource + SRI, décodage puis
            // livraison `load`/`error` sur l'élément.
            if let Some(meta) = d.script.take() {
                self.deliver_dyn_done(&d, &meta);
                continue;
            }
            // `<script type="module">` dynamique (CAP-186) —
            // graphe rempli hors session, `eval_module_graph`
            // puis `load`/`error` sur l'élément.
            if let Some(dm) = d.dyn_module {
                self.dyn_tasks.remove(&(d.tab, dm.seq));
                self.deliver_dyn_module(d.tab, dm);
                continue;
            }
            // `<iframe>` dynamique (CAP-171) — `handle` = `seq`
            // moteur, espace distinct des handles fetch.
            if let Some(meta) = d.frame {
                self.dyn_tasks.remove(&(d.tab, meta.seq));
                self.commit_dyn_frame_resp(d.tab, &meta, &d.out);
                continue;
            }
            // `<img>` dynamique (CAP-172) — même convention
            // `handle` = `seq` moteur.
            if let Some(meta) = d.img {
                self.dyn_tasks.remove(&(d.tab, meta.seq));
                self.commit_dyn_img(d.tab, meta, &d.out);
                continue;
            }
            // `<link>` dynamique (CAP-173) — même convention.
            if let Some(meta) = d.link {
                self.dyn_tasks.remove(&(d.tab, meta.seq));
                self.commit_dyn_link(d.tab, &meta, &d.out);
                continue;
            }
            // `import()` dynamique (JS-C100) — corps livré au
            // moteur (instancier+évaluer+résoudre la promesse).
            if let Some(meta) = d.import {
                self.dyn_tasks.remove(&(d.tab, meta.seq));
                self.commit_dyn_import(d.tab, &meta);
                continue;
            }
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
                        self.cookies
                            .set_from_header_ctx(&d.req_url, v, d.partition.clone());
                    }
                }
            }
            let resp = match &d.out {
                // `no-cors` cross : livrée opaque par le moteur —
                // `ACAO` n'est pas exigé (JS-C77 ; la lisibilité
                // CORS ne s'applique qu'au mode `cors`).
                Ok(resp)
                    if !d.cross
                        || d.mode == explorer_net::http::FetchMode::NoCors
                        || cors_readable(&d.page_origin, resp) =>
                {
                    NetResponse {
                        handle: d.handle,
                        status: resp.status,
                        status_text: resp.reason.clone(),
                        headers: resp.headers.clone(),
                        body: resp.body.clone(),
                    }
                }
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
        self.net_tasks.len() + self.dyn_tasks.len()
    }

    /// Collecte des sous-ressources du document (étape 50) :
    /// URLs résolues ici (émission propre), mixed-content
    /// filtré, images dédupliquées — plan borné par la
    /// config (étape 128 : les `<iframe>` le rejoignent).
    /// `page` = URL du document (ancre mixed-content,
    /// initiateur) ; `base` = base de résolution effective
    /// (`<base href>` ou `page`, étape 135) — seule la
    /// résolution des `src`/`href` suit la base. `csp` =
    /// politiques du document (CAP-145) : chaque cible doit
    /// être admise par sa directive (`upgrade-insecure-
    /// requests` réécrit `http:` → `https:` en amont).
    pub(crate) fn plan_subresources(
        &self,
        dom: &Dom,
        page: &Url,
        base: &Url,
        scripting: bool,
        csp: &[explorer_net::csp::CspPolicy],
    ) -> SubPlan {
        // Étape 96 : `media=` évalué contre l'env viewport
        // AVANT la borne — une feuille hors-media (variante
        // `print`, `(prefers-color-scheme: …)`) n'est ni
        // fetchée ni comptée dans `max_sheets`. Dédup sur
        // l'URL résolue (ordre conservé — les pages à blocs
        // de liens générés répètent le même `href`, GitHub
        // ~7 doublons : un doublon brûle quota + fetch pour
        // un contenu identique en cascade).
        let env = super::apply::viewport_env(self.viewport, self.color_scheme, scripting);
        // CSP `upgrade-insecure-requests` (CAP-145) —
        // réécriture en amont des filtres : une cible
        // upgradée devient admissible mixed-content.
        let upg = |u: Url| explorer_net::csp::csp_upgrade(csp, &u).unwrap_or(u);
        let allow = |dest: explorer_net::http::FetchDest, u: &Url| {
            explorer_net::csp::csp_allows(csp, page, dest, u)
        };
        let mut seen_sheets = std::collections::HashSet::new();
        // `(url, integrity, crossorigin)` — SRI (CAP-163) et
        // mode CORS (CAP-167) voyagent jusqu'au commit de la
        // réponse.
        let sheets: Vec<SheetFetch> = collect_stylesheet_links(dom)
            .into_iter()
            .filter(|(_, m, ..)| media_attr_matches(&env, m.as_deref().unwrap_or_default()))
            .filter_map(|(h, _, i, c, lord)| Url::resolve(base, &h).ok().map(|u| (u, i, c, lord)))
            .map(|(u, i, c, lord)| (upg(u), i, c, lord))
            .filter(|(u, ..)| {
                !is_mixed_content(page, u)
                    && allow(explorer_net::http::FetchDest::Style, u)
                    && seen_sheets.insert(u.to_string())
            })
            .take(self.max_sheets)
            .collect();
        let scripts = collect_scripts(dom, page, base, scripting, self.max_scripts, csp);
        let mut seen_img = std::collections::HashSet::new();
        let imgs: Vec<Url> = collect_images(dom, self.viewport.0, self.scale)
            .into_iter()
            .filter_map(|(_, src)| Url::resolve(base, &src).ok())
            .map(&upg)
            // `data:` (et autres non-http) ne se fetch pas —
            // peints inline par `collect_data_images` (56f).
            .filter(|u| matches!(u.scheme(), "http" | "https"))
            .filter(|u| {
                !is_mixed_content(page, u)
                    && allow(explorer_net::http::FetchDest::Image, u)
                    && seen_img.insert(u.as_str())
            })
            .take(self.max_images)
            .collect();
        // `<iframe>` (étape 128 + CAP-142) — un sous-contexte
        // est un document complet : `src` = fetch http(s),
        // mixed-content filtré, dédup par URL résolue ;
        // `srcdoc` = document inline (prioritaire sur `src`,
        // aucun fetch — spec HTML) dédupliqué par contenu.
        // Les deux partagent la borne `max_frames`
        // (`0` = désactivé — vide) dans l'ordre document.
        let mut seen_frames = std::collections::HashSet::new();
        let mut seen_srcdoc = std::collections::HashSet::new();
        let mut frames: Vec<(
            Url,
            Option<explorer_net::http::ReferrerPolicy>,
            Option<SandboxFlags>,
        )> = Vec::new();
        let mut srcdoc_frames: Vec<(String, Option<SandboxFlags>)> = Vec::new();
        for (kind, policy, sandbox) in collect_iframes(dom) {
            if frames.len() + srcdoc_frames.len() >= self.max_frames {
                break;
            }
            match kind {
                IframeKind::Src(src) => {
                    if let Ok(u) = Url::resolve(base, &src).map(upg) {
                        if matches!(u.scheme(), "http" | "https")
                            && !is_mixed_content(page, &u)
                            && allow(explorer_net::http::FetchDest::Iframe, &u)
                            && seen_frames.insert(u.as_str())
                        {
                            frames.push((u, policy, sandbox));
                        }
                    }
                }
                IframeKind::Srcdoc(html) => {
                    // `frame-src` s'applique à `srcdoc`
                    // (CAP-145) : le contexte inline hérite
                    // l'origine du parent — `'self'` l'admet,
                    // `'none'` le bloque. La cible testée est
                    // l'URL de la page (origine héritée).
                    if allow(explorer_net::http::FetchDest::Iframe, page)
                        && seen_srcdoc.insert(html.clone())
                    {
                        srcdoc_frames.push((html, sandbox));
                    }
                }
            }
        }
        SubPlan {
            sheets,
            scripts,
            imgs,
            frames,
            srcdoc_frames,
        }
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
        page_policy: Option<explorer_net::http::ReferrerPolicy>,
        csp: &[explorer_net::csp::CspPolicy],
        entry_base: &Url,
        entry_src: &str,
        import_specs: &[(String, String)],
        graph: &mut ModuleGraph,
    ) {
        let mut queued: std::collections::HashSet<String> =
            std::iter::once(entry_base.as_str()).collect();
        let mut frontier: Vec<(String, Url)> = Vec::new();
        for (spec, u) in module_deps(entry_src, entry_base, import_specs) {
            let u = explorer_net::csp::csp_upgrade(csp, &u).unwrap_or(u);
            graph
                .alias
                .entry(spec.clone())
                .or_insert_with(|| u.as_str());
            let allowed = !is_mixed_content(page, &u)
                && explorer_net::csp::csp_allows(
                    csp,
                    page,
                    explorer_net::http::FetchDest::Script,
                    &u,
                );
            if allowed && queued.insert(u.as_str()) {
                frontier.push((spec, u));
            }
        }
        while !frontier.is_empty() && graph.sources.len() < max_modules {
            let room = (max_modules - graph.sources.len()).min(frontier.len());
            let mut jobs = VecDeque::new();
            for (_, u) in frontier.drain(..room) {
                jobs.push_back((
                    (u.as_str(), u.clone()),
                    sub_request(
                        page,
                        &u,
                        explorer_net::http::FetchDest::Script,
                        page_policy,
                        None,
                    ),
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
                // En-têtes de sécurité de réponse (étape 134) —
                // dest `script` : nosniff + CORP.
                if !subresp_security_allows(page, &u, explorer_net::http::FetchDest::Script, &resp)
                {
                    continue;
                }
                let text = decode_subresource(&resp.body, None);
                graph.sources.insert(key, text.clone());
                for (spec2, u2) in module_deps(&text, &u, import_specs) {
                    let u2 = explorer_net::csp::csp_upgrade(csp, &u2).unwrap_or(u2);
                    graph
                        .alias
                        .entry(spec2.clone())
                        .or_insert_with(|| u2.as_str());
                    let allowed = !is_mixed_content(page, &u2)
                        && explorer_net::csp::csp_allows(
                            csp,
                            page,
                            explorer_net::http::FetchDest::Script,
                            &u2,
                        );
                    if allowed && queued.insert(u2.as_str()) {
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
        let img_w = self.viewport.0;
        let dpr = self.scale;
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
            &mut entry.img_bodies,
            img_w,
            dpr,
        );
    }

    /// Étape 133 — retour bfcache : les bitmaps de la page
    /// gelée ont été libérés au `freeze` (`images.*` vidées,
    /// `list` lâchée) ; on les re-décode ici depuis les corps
    /// compressés retenus (`img_bodies`) + attributs/styles
    /// restaurés — restauration toujours **sans réseau**.
    /// `data:` est auto-porteur (payload dans l'attribut),
    /// les `svg` inline re-rasterisés depuis le DOM.
    pub(crate) fn restore_images(&mut self, tab: TabId) {
        let max_px = self.image_max_px;
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        let Some(dom) = entry.dom.as_ref() else {
            return;
        };
        let Some(base) = entry.url.clone() else {
            return;
        };
        // Phase lecture : items `<img>`/`poster`, table
        // ordinal→nœud (`prev_styles` est indexée ordinal), et
        // racine pour la collecte svg.
        let items = collect_images(dom, self.viewport.0, self.scale);
        let ord_to_node: HashMap<usize, explorer_dom::NodeId> = element_ordinals(dom)
            .into_iter()
            .map(|(n, o)| (o, n))
            .collect();
        let root = dom.document_element();
        // Travaux styles (lecture seule de `prev_styles`) —
        // même surface que `insert_fetched_images` : fonds via
        // `background-image`, masques via les deux propriétés.
        let mut bg_work = Vec::new();
        let mut mask_work = Vec::new();
        for (ord, st) in &entry.prev_styles {
            if let Some(raw) = st.get("background-image").and_then(css_url_arg) {
                if let Some(node) = ord_to_node.get(ord) {
                    bg_work.push((*node, raw));
                }
            }
            for prop in ["mask-image", "-webkit-mask-image"] {
                if let Some(raw) = st.get(prop).and_then(css_url_arg) {
                    mask_work.push(raw);
                }
            }
        }

        // Corps compressé → bitmap décodé ; `data:` se décode
        // depuis l'URL elle-même, sinon l'alias résout la clé
        // du magasin retenu. Renvoie `(octets, clé_résolue)`.
        let body_of = |raw: &str,
                       aliases: &HashMap<String, String>,
                       store: &HashMap<String, Vec<u8>>|
         -> Option<(Vec<u8>, Option<String>)> {
            if let Some(b) = data_url_bytes(raw) {
                return Some((b, None));
            }
            let resolved = aliases.get(raw).cloned()?;
            store.get(&resolved).cloned().map(|b| (b, Some(resolved)))
        };

        // (a) `<img>`/`poster` — `src` relu du DOM restauré.
        for (node, src) in items {
            let Some((bytes, key)) = (if let Some(b) = data_url_bytes(&src) {
                Some((b, None))
            } else {
                let Ok(u) = Url::resolve(&base, &src) else {
                    continue;
                };
                entry
                    .img_bodies
                    .get(&u.as_str())
                    .cloned()
                    .map(|b| (b, Some(u.as_str())))
            }) else {
                continue;
            };
            let Some(img) = decode_image(&bytes, max_px).map(Arc::new) else {
                continue;
            };
            entry.images.replaced.insert(node, img.clone());
            entry.img_decoded.insert(src, img.clone());
            if let Some(k) = key {
                entry.bg_decoded.insert(k, Some(img));
            }
        }
        // (b) fonds `url(...)` — alias → corps → bitmap, le
        // cache `bg_decoded` est re-alimenté sous la clé résolue.
        for (node, raw) in bg_work {
            let Some((bytes, key)) = body_of(&raw, &entry.bg_aliases, &entry.img_bodies) else {
                continue;
            };
            let Some(img) = decode_image(&bytes, max_px).map(Arc::new) else {
                continue;
            };
            entry.images.backgrounds.insert(node, img.clone());
            if let Some(k) = key {
                entry.bg_decoded.insert(k, Some(img));
            }
        }
        // (c) masques — table cléée par le texte `url(...)` brut
        // (couvre les pseudo-éléments sans `NodeId`).
        for raw in mask_work {
            let Some((bytes, key)) = body_of(&raw, &entry.mask_aliases, &entry.img_bodies) else {
                continue;
            };
            let Some(img) = decode_image(&bytes, max_px).map(Arc::new) else {
                continue;
            };
            if let Some(k) = key {
                entry.bg_decoded.insert(k, Some(img.clone()));
            }
            entry.images.register_mask(&raw, img);
        }
        // (d) `<svg>` inline — re-rasterisés depuis le DOM
        // restauré (collecte pure, pas de corps nécessaire).
        if let Some(root_el) = root {
            entry.images.svgs = explorer_render::svg::collect_svg(dom, root_el, max_px);
        }
    }
}

/// Plafond du magasin de corps compressés retenus par page
/// (étape 133) — 32 Mio de compressé ≈ plusieurs centaines
/// de Mio de bitmaps. Au-delà, les corps excédentaires ne
/// sont pas retenus : ces images ne survivront pas au gel
/// bfcache (dégradation honnête à la restauration, pas de
/// réservoir non borné).
const MAX_IMG_BODY_STORE: usize = 32 << 20;

/// Retient le corps compressé `body` sous `key` (URL
/// résolue) dans le magasin de l'onglet — no-op si déjà
/// présent ou si le plafond serait dépassé.
pub(crate) fn retain_img_body(store: &mut HashMap<String, Vec<u8>>, key: &str, body: Vec<u8>) {
    if store.contains_key(key) {
        return;
    }
    let total: usize = store.values().map(Vec::len).sum();
    if total + body.len() > MAX_IMG_BODY_STORE {
        return;
    }
    store.insert(key.to_string(), body);
}

/// Boucle `insert_images` partagée — `dom` et `images`
/// proviennent de l'onglet (chemin synchrone) ou de la
/// payload détachée (étape 90). `decoded_cache` = cache
/// persistant `src`/`poster` → image (étape 108) : la table
/// `replaced` indexée `NodeId` meurt à chaque re-parse, le
/// cache survit et permet la ré-insertion sans re-fetch ni
/// re-décodage.
#[allow(clippy::too_many_arguments)]
fn insert_images_into(
    dom: &Dom,
    images: &mut PaintImages,
    base: &Url,
    bodies: &HashMap<String, Vec<u8>>,
    max_px: u32,
    decoded_cache: &mut HashMap<String, Arc<DecodedImage>>,
    bodies_store: &mut HashMap<String, Vec<u8>>,
    img_w: f32,
    dpr: f32,
) {
    let items = collect_images(dom, img_w, dpr);
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
            // Corps compressé retenu (étape 133) — source du
            // re-décodage au retour bfcache.
            retain_img_body(bodies_store, &u.as_str(), body.clone());
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
    img_w: f32,
    dpr: f32,
) {
    if decoded_cache.is_empty() {
        return;
    }
    for (node, src) in collect_images(dom, img_w, dpr) {
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
    img_w: f32,
    dpr: f32,
) {
    if bodies.is_empty() {
        return;
    }
    let PaintPayload {
        dom,
        images,
        img_decoded,
        img_bodies,
        ..
    } = p;
    let Some(dom) = dom.as_ref() else {
        return;
    };
    insert_images_into(
        dom,
        images,
        base,
        bodies,
        max_px,
        img_decoded,
        img_bodies,
        img_w,
        dpr,
    );
}

/// Drain des surfaces script d'un contexte enfant (étape
/// 134 + CAP-149) : (URL de l'enfant, origine opaque,
/// `JsCtxId`, écritures `document.cookie`, map
/// `localStorage` complète).
type FrameSink = (
    Url,
    bool,
    u32,
    Vec<String>,
    std::collections::BTreeMap<String, String>,
);

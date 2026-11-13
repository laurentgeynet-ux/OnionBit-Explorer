// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! API partagée async — points d'entrée prenant
//! `&Mutex<Session>` sans détenir le verrou.
use super::*;
use apply::NavPaint;
use std::time::{Duration, Instant};

use explorer_dom::Dom;
use explorer_net::http::Request;

/// Conduit une navigation sur une session partagée
/// (`Mutex<Session>`) **sans tenir le verrou pendant
/// l'I/O** (étape 53d — le défaut produit était un gel de
/// toute l'API pendant `navigate`) : `nav_begin` arme
/// l'epoch sous verrou court, `NavJob::run` télécharge et
/// parse hors verrou (chaque snapshot de peinture
/// progressive est appliqué dans un re-lock minimal),
/// `nav_apply` puis `sub_commit` n'écrivent qu'en verrou
/// court. Pendant tout le parcours, `status`, `tabs`,
/// `frame`, l'input et les autres onglets restent
/// serviables.
///
/// Une navigation supplantée (epoch périmée — double-clic,
/// redirection script plus récente) est abandonnée
/// silencieusement : ses fetches ont tourné mais rien
/// d'obsolète n'est commis.
///
/// Ne draine pas les intentions produites : l'appelant
/// enchaîne [`drain_navs_shared`] — même contrat que
/// [`Session::navigate`] + [`Session::drain_script_nav`].
///
/// # Errors
/// B-30 : future qui s'achève quand l'onglet meurt — les
/// phases détachées (`job.run`/`sub.run`) s'interrompent au
/// lieu d'allouer une page pour un onglet fermé.
async fn wait_tab_dead(flag: &std::sync::atomic::AtomicBool) {
    use std::sync::atomic::Ordering;
    while flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Clone le drapeau `nav_alive` de l'onglet (B-30) — `None`
/// si l'onglet est déjà mort : la phase ne démarre même pas.
async fn tab_alive(
    sess: &tokio::sync::Mutex<Session>,
    tab: TabId,
) -> Option<std::sync::Arc<std::sync::atomic::AtomicBool>> {
    sess.lock()
        .await
        .tabs
        .get(&tab)
        .map(|e| e.nav_alive.clone())
}

/// Mêmes erreurs que [`Session::navigate`]
/// (`Url`, `Fetch`, `HttpStatus`, `EmptyDocument`,
/// `NothingToPaint`, `FrameTooBig`, `UnknownTab`) —
/// `PipelineError` est émis sur le bus pour les erreurs de
/// pipeline, comme le chemin `&mut self`.
///
/// # Errors
/// Voir ci-dessus — propagées via `emit_error` après peinture
/// de la page d'erreur.
pub async fn navigate_shared(
    sess: &tokio::sync::Mutex<Session>,
    tab: TabId,
    req: Request,
    push: bool,
) -> Result<(), CoreError> {
    let mut job = {
        let mut s = sess.lock().await;
        // `navigate` vers l'entrée forward = restauration
        // bfcache (56c) — jamais une peinture non commise.
        if push && s.try_forward_nav(tab, &req) {
            return Ok(());
        }
        // `#fragment` même document = scroll, pas re-fetch
        // (56d).
        if s.try_fragment_nav(tab, &req) {
            return Ok(());
        }
        // Étape 209 — refus pendant épisode shed **après** les
        // chemins gratuits (restauration bfcache/fragment ne
        // fetch pas — les autoriser ne coûte rien et garde
        // l'historique fluide).
        s.maybe_rss_shed();
        if let Some(retry) = s.shed_refusal() {
            return Err(CoreError::Overloaded(retry));
        }
        s.nav_begin(tab, req, push)?
    };
    let epoch = job.epoch;
    let alive = tab_alive(sess, tab).await;
    let Some(alive) = alive else {
        return Ok(()); // onglet mort entre nav_begin et run
    };
    let job_run = job.run(|dom: &Dom| -> PaintFut<'_> {
        // `try_lock` : la peinture progressive ne bloque
        // jamais le téléchargement — un tick manqué (lock
        // occupé par un `/frame` ou une autre phase) est
        // repeint au suivant, le commit final repeint de
        // toute façon.
        if let Ok(mut s) = sess.try_lock() {
            s.nav_paint(tab, epoch, dom);
        }
        Box::pin(async {})
    });
    let (done, tail) = match tokio::select! {
        r = job_run => Some(r),
        () = wait_tab_dead(&alive) => None,
    } {
        Some(r) => match r {
            // Téléchargement (étape 70) : enregistré en attente
            // — rien de committé, la page courante reste.
            Ok(NavOutcome::Download(dl)) => {
                let mut s = sess.lock().await;
                s.offer_download(tab, *dl);
                return Ok(());
            }
            Ok(NavOutcome::Document(d, t)) => (d, t),
            Err(e) => {
                let mut s = sess.lock().await;
                s.unspec(tab); // peinture spéculative → réverte (56c)
                               // Page d'erreur commise (étape 69) — l'URL tentée
                               // s'affiche ; étape 76 : entrée `failed` poussée.
                s.paint_error_page(tab, &job.req, job.push, &e);
                return Err(s.emit_error(tab, e));
            }
        },
        // Onglet mort en vol (B-30) : abandon propre — la
        // page en cours n'a plus de réceptacle.
        None => return Ok(()),
    };
    // Requête/page d'erreur éventuelle : l'URL effectivement
    // naviguée (post-redirections) — les erreurs post-fetch
    // sont couvertes aussi (étapes 75/76).
    let err_req = {
        let mut r = tail.req.clone();
        r.url = done.url.clone();
        r
    };
    let push = tail.push;
    let sub = {
        let mut s = sess.lock().await;
        match s.nav_apply(tab, *tail, *done) {
            Ok(sub) => sub,
            Err(e) => {
                s.paint_error_page(tab, &err_req, push, &e);
                return Err(s.emit_error(tab, e));
            }
        }
    };
    // Epoch supplantée : la navigation la plus récente a
    // gagné — celle-ci est jetée proprement (rien à
    // commettre, aucune erreur n'est observable).
    let Some(mut sub) = sub else {
        return Ok(());
    };
    // Phase d'allocation sous-ressources — stoppée si
    // l'onglet meurt en vol (B-30 : l'orphelin allouait
    // une page entière pour un onglet fermé, +28 Mo/s).
    tokio::select! {
        () = sub.run() => {},
        () = wait_tab_dead(&alive) => {
            tracing::warn!(tab, "nav orpheline stoppée — onglet fermé en fetch (B-30)");
        }
    }
    nav_commit_shared(sess, tab, epoch, sub, &err_req, push).await
}

/// Commit partagé d'une navigation — trois temps (étape 90) :
/// la cascade+layout d'un document de terrain peut coûter des
/// dizaines de secondes — sous le verrou elle affamait toute
/// l'API (gel ~85 s mesuré sur github). Tête sous verrou
/// court (epoch, CSS, JS, drains), rendu détaché en
/// `spawn_blocking` sur la payload (même mécanique que
/// l'apply 55h), queue sous verrou (historique, `Navigated`,
/// fragment, `mark_hist_ok`).
#[allow(clippy::too_many_lines)] // trois temps séquencés — découper fragmenterait le contrat epoch
async fn nav_commit_shared(
    sess: &tokio::sync::Mutex<Session>,
    tab: TabId,
    epoch: u64,
    sub: SubJob,
    err_req: &Request,
    push: bool,
) -> Result<(), CoreError> {
    let head = {
        let mut s = sess.lock().await;
        s.sub_commit_head(tab, sub).await
    };
    let mut tail = match head {
        Ok(Some(t)) => t,
        // Epoch supplantée en cours de route — silencieux.
        Ok(None) => return Ok(()),
        Err(e) => {
            let mut s = sess.lock().await;
            s.paint_error_page(tab, err_req, push, &e);
            return Err(s.emit_error(tab, e));
        }
    };
    // Un apply détaché peut tenir la payload (navigation
    // pendant une mutation JS) : attente pollée — le
    // détenteur la restitue toujours à son commit, l'epoch
    // est revérifiée à chaque tour (une nav plus récente
    // coupe court). Le verrou n'est jamais tenu ici.
    let wait_start = std::time::Instant::now();
    let (mut g, ctx, canvas_ops) = loop {
        let take = {
            let mut s = sess.lock().await;
            s.take_nav_paint(tab, epoch)
        };
        match take {
            NavPaint::Ready(p, c, o) => break (p, c, o),
            NavPaint::Superseded => return Ok(()),
            NavPaint::Busy => {
                // B-33 (ici/20minutes) : le détenteur peut
                // mourir en vol sans restituer (B-26 — apply
                // mort entre layout et repaint) : sans borne
                // la nav attend à vie, page jamais servie.
                // Timeout → réarmement + erreur nommée.
                if wait_start.elapsed() > Duration::from_secs(30) {
                    let mut s = sess.lock().await;
                    s.rearm_nav_paint(tab);
                    let age = s.paint_in_flight_age_ms(tab);
                    tracing::warn!(
                        tab,
                        age_ms = ?age,
                        "nav_paint wedge — détenteur mort, réarmé (B-33)"
                    );
                    return Err(s.emit_error(
                        tab,
                        CoreError::Timeout(format!(
                            "nav_paint en attente >30 s (in-flight {age:?} ms)"
                        )),
                    ));
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    };
    // `p.base_url` porte la base effective (`<base href>`
    // ou l'URL de la page, étape 135) — l'URL de la
    // réponse reste en repli.
    let base = g
        .payload()
        .and_then(|p| p.base_url.clone())
        .unwrap_or_else(|| tail.url.clone());
    let bodies = std::mem::take(&mut tail.img_bodies);
    // Fonds `url(http…)` (étape 98) et masques `mask-image`
    // (étape 121) : posés sur la payload, insérés à la
    // cascade dans `relayout_dom_payload`.
    let bg_aliases = std::mem::take(&mut tail.bg_aliases);
    let bg_bodies = std::mem::take(&mut tail.bg_bodies);
    let mask_aliases = std::mem::take(&mut tail.mask_aliases);
    // La garde traverse le worker : si ce futur est droppé
    // (client HTTP tombé, shutdown), la payload est parquée
    // puis réinsérée à la prochaine prise de verrou —
    // jamais perdue avec `paint_detached` collé (issue #23).
    let work = tokio::task::spawn_blocking(move || {
        let p = g.payload_mut();
        net::insert_images_payload(
            p,
            &base,
            &bodies,
            ctx.image_max_px,
            ctx.image_max_bytes,
            &apply::viewport_env(ctx.viewport, ctx.color_scheme, ctx.scripting, ctx.scale),
        );
        p.bg_aliases = bg_aliases;
        p.bg_bodies = bg_bodies;
        p.mask_aliases = mask_aliases;
        let Some(dom) = p.dom.take() else {
            return (g, Err(CoreError::EmptyDocument));
        };
        let r = apply::relayout_dom_payload(p, &ctx, tab, &dom, &canvas_ops);
        p.dom = Some(dom);
        (g, r)
    });
    // B-46 (frandroid vol70) : le worker relayout/insert_images
    // n'avait aucune borne — un coût quadratique (B-44) ou une
    // image pathologique laissait le navigate muet à vie après
    // `apply fin` (CPU 200 %, API morte). Borné : au-delà, le
    // futur est abandonné — le payload se perd avec le bloc
    // (rearm + page d'erreur honnête, jamais muet).
    let work = tokio::time::timeout(Duration::from_secs(90), work).await;
    let mut s = sess.lock().await;
    match work {
        Ok(Ok((g, r))) => {
            let live = s.commit_nav_paint(tab, epoch, (*g).into_payload());
            tracing::info!(target: "dom_probe", tab,
                ok = r.is_ok(), live,
                site = "commit.nav", "bras commit paint");
            match (r, live) {
                (Ok(()), true) => {
                    s.sub_commit_tail(tab, tail);
                    // Rejeu/reload réussi : la position courante
                    // n'est plus en échec (étape 76).
                    let idx = s.tabs.get(&tab).map(|e| e.hist_idx);
                    if let Some(i) = idx {
                        s.mark_hist_ok(tab, i);
                    }
                    Ok(())
                }
                (Err(e), true) => {
                    s.paint_error_page(tab, err_req, push, &e);
                    Err(s.emit_error(tab, e))
                }
                // Epoch supplantée pendant le rendu — la
                // payload a été jetée, mais le document reste
                // celui installé : son cycle de vie
                // (`interactive`/`DOMContentLoaded`/`complete`/
                // `load`) n'est pas la propriété du payload —
                // `sub_commit_tail` sauté = `readyState` figé
                // `loading`, polyfills d'upgrade gelés
                // (JS-C471, youtube `[[ ]]` non hydratés).
                _ => {
                    s.fire_top_doc_lifecycle(tab);
                    Ok(())
                }
            }
        }
        Ok(Err(e)) => {
            s.rearm_nav_paint(tab);
            let err = CoreError::Fetch(format!("worker rendu de navigation paniqué : {e}"));
            s.paint_error_page(tab, err_req, push, &err);
            Err(s.emit_error(tab, err))
        }
        Err(_) => {
            // B-46 : le bloc relayout/images a dépassé sa
            // borne — la payload part avec lui, `rearm`
            // rend l'état utilisable et la page d'erreur
            // est peinte (timeout nommé, jamais muet).
            s.rearm_nav_paint(tab);
            let err = CoreError::Timeout("worker relayout/images > 90 s (B-46)".to_string());
            s.paint_error_page(tab, err_req, push, &err);
            Err(s.emit_error(tab, err))
        }
    }
}

/// `history_move` partagé (étape 53d) : la cible est lue
/// sous verrou court, la navigation conduite hors verrou,
/// l'index commis seulement si la navigation a abouti.
///
/// # Errors
/// [`CoreError::UnknownTab`], [`CoreError::EmptyDocument`]
/// (pas d'entrée dans cette direction) ou erreur de
/// navigation.
pub async fn history_move_shared(
    sess: &tokio::sync::Mutex<Session>,
    tab: TabId,
    delta: isize,
) -> Result<(), CoreError> {
    let (idx, req) = {
        let mut s = sess.lock().await;
        let t = s.hist_target(tab, delta)?;
        // Hit bfcache (étape 56) : restauration instantanée
        // sous verrou court — aucune navigation n'est lancée.
        if s.try_bfcache(tab, t.0) {
            return Ok(());
        }
        t
    };
    // Miss → la requête d'origine est rejouée (POST rejoué
    // tel quel — étape 56).
    match Box::pin(navigate_shared(sess, tab, req, false)).await {
        Ok(()) => {
            let mut s = sess.lock().await;
            s.set_hist_idx(tab, idx);
            // Rejeu d'une entrée `failed` ayant abouti : la
            // cible héberge une vraie page (étape 76).
            s.mark_hist_ok(tab, idx);
            Ok(())
        }
        Err(e) => {
            let mut s = sess.lock().await;
            // Page d'erreur commise à la position de l'entrée
            // (étape 76) — la traversée a atterri malgré
            // l'échec du rejeu.
            s.set_hist_idx(tab, idx);
            Err(s.emit_error(tab, e))
        }
    }
}

/// Drain partagé des intentions de navigation (étape 53d) —
/// même contrat que [`Session::drain_script_nav`] mais
/// chaque navigation est conduite par [`navigate_shared`]
/// **hors verrou** : la boucle bornée `MAX_NAV_HOPS`
/// alterne collecte sous verrou court et conduite libre —
/// `form.submit()`, `location=` et les clics de liens n'en
/// gèlent plus l'API non plus.
/// Renvoie le nombre de navigations **commises** (étape
/// 64 : les appelants distinguent intention et commit
/// effectif — un fetch mort ne rapporte pas « navigated »).
pub async fn drain_navs_shared(sess: &tokio::sync::Mutex<Session>, tab: TabId) -> usize {
    const MAX_NAV_HOPS: u8 = 8;
    let mut committed = 0usize;
    for _ in 0..MAX_NAV_HOPS {
        let act = {
            let mut s = sess.lock().await;
            s.collect_script_navs(tab);
            s.pop_pending_nav(tab)
        };
        let t0 = std::time::Instant::now();
        let ok = match act {
            None => break,
            Some(DrainAct::Req(req, push)) => {
                // `javascript:` (CAP-154) — évalué sous
                // verrou court dans le contexte top, jamais
                // fetché (même règle que le drain exclusif).
                if req.url.scheme() == "javascript" {
                    let mut s = sess.lock().await;
                    s.run_javascript_url(tab, &req.url)
                } else {
                    Box::pin(navigate_shared(sess, tab, *req, push))
                        .await
                        .is_ok()
                }
            }
            Some(DrainAct::NewTab(req, name)) => {
                // `target=_blank`/nommé (étape 131,
                // CAP-159) — sélection du contexte
                // (`named_or_new_tab`) sous verrou court,
                // navigation conduite hors verrou.
                let t2 = {
                    let mut s = sess.lock().await;
                    s.named_or_new_tab(tab, &name)
                };
                let ok = Box::pin(navigate_shared(sess, t2, *req, true))
                    .await
                    .is_ok();
                {
                    let mut s = sess.lock().await;
                    s.close_stillborn_popup(tab, t2);
                }
                ok
            }
            Some(DrainAct::Go(0)) => {
                // `history.go(0)` = rechargement de l'entrée
                // courante (spec HTML).
                let url = {
                    let s = sess.lock().await;
                    s.tabs.get(&tab).and_then(|t| t.url.clone())
                };
                if let Some(u) = url {
                    Box::pin(navigate_shared(sess, tab, Request::get(u), false))
                        .await
                        .is_ok()
                } else {
                    false
                }
            }
            Some(DrainAct::Go(d)) => Box::pin(history_move_shared(sess, tab, d)).await.is_ok(),
            Some(DrainAct::Replace(req)) => {
                // `location.replace` (CAP-152) — même règle
                // que le drain exclusif : fetch sans push,
                // puis l'entrée courante est réécrite par la
                // requête commise (`back` saute la page
                // remplacée). `same_doc` retombe à faux :
                // le document est neuf (tête de run).
                let req = *req;
                let ok = Box::pin(navigate_shared(sess, tab, req.clone(), false))
                    .await
                    .is_ok();
                if ok {
                    let mut s = sess.lock().await;
                    if let Some(e) = s.tabs.get_mut(&tab) {
                        if let Some(h) = e.hist.get_mut(e.hist_idx) {
                            h.req = req;
                            h.same_doc = false;
                        }
                    }
                }
                ok
            }
            Some(DrainAct::Reload) => {
                // `location.reload` (CAP-152) — rejeu de
                // `last_req` (méthode + corps — POST
                // re-POSTe) à la position courante, sans
                // entrée nouvelle.
                let req = {
                    let s = sess.lock().await;
                    s.tabs.get(&tab).and_then(|e| e.last_req.clone())
                };
                let Some(req) = req else {
                    continue;
                };
                let ok = Box::pin(navigate_shared(sess, tab, req, false))
                    .await
                    .is_ok();
                if ok {
                    // Comme `reload` public : une page
                    // d'erreur rechargée avec succès
                    // redevient une vraie position (76).
                    let mut s = sess.lock().await;
                    let idx = s.tabs.get(&tab).map(|e| e.hist_idx);
                    if let Some(i) = idx {
                        s.mark_hist_ok(tab, i);
                    }
                }
                ok
            }
        };
        // Temps passé dans le hop imputé à la borne-temps
        // nav-auto (étape 172, §B-72) — les navs instantanées
        // n'y laissent ~0 ms, seules les navs lentes cumulent.
        {
            let mut s = sess.lock().await;
            s.charge_auto_nav_ms(
                tab,
                u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX),
            );
        }
        committed += usize::from(ok);
    }
    committed
}

/// Pompe JS partagée (étape 53f) — même contrat qu'un tour de
/// [`Session::pump_timers`] + `advance_animations` (55e)
/// mais le travail long (`run_pending` + sérialisation du
/// miroir) s'exécute dans `spawn_blocking` avec le moteur
/// **extrait** de l'onglet : un burst JS (`repros/relayout.html`
/// ~200 `appendChild`) ne fige plus `/status` — le verrou n'est
/// tenu qu'aux phases take/commit.
///
/// Pendant le détachement : réponses `fetch`/XHR et pushes
/// d'attributs (`value`, `checked`…) sont tamponnés et livrés
/// à la réinsertion ; une navigation supplantant l'onglet jette
/// le moteur détaché (garde epoch, jamais de miroir caduc
/// appliqué). Dégradation documentée : un `dispatch_event`
/// arrivant en plein run trouve `js: None` → handler inerte —
/// l'input reste traité côté hôte, comme une page sans script.
///
/// Retourne le nombre de jobs JS exécutés (0 = pas de moteur,
/// pompe déjà en vol, ou epoch supplantée).
pub async fn pump_timers_shared(sess: &tokio::sync::Mutex<Session>, tab: TabId) -> usize {
    let taken = {
        let mut s = sess.lock().await;
        s.take_js_engine(tab)
    };
    let Some((mut eng, max, epoch, sync_ok)) = taken else {
        // Pas de moteur détachable (page sans <script>, pompe
        // déjà en vol, epoch supplantée) — le battement
        // d'animation tourne quand même : `@keyframes` est
        // purement CSS, un document sans JS doit animer
        // (55e rouvert — 6 frames identiques sur anim.html).
        // Étape 3c : le relayout anim tourne hors verrou.
        let _ = advance_animations_detached(sess, tab).await;
        return 0;
    };
    // Hors verrou : le burst JS n'occupe ni le verrou ni le
    // worker tokio (spawn_blocking — `JsEngine: Send`). Sous
    // fenêtre de coalescence `sync_floor` la sérialisation
    // est sautée — le miroir accumule, l'API respire
    // (55g-suite).
    // §B-111 — marqueurs `pump:` (morgue C405) : la mort
    // `0xC00000FD` est post-collect **sur le worker**
    // (`tokio-rt-worker` — le drain moteur vit sur le thread
    // `explorer-jscore` derrière le canal `Cmd`). Chaque
    // étape du chemin de retour est nommée : la dernière
    // ligne vivante brackete le site fautif.
    let tb = Instant::now();
    let out = tokio::task::spawn_blocking(move || {
        dlv_info!("pump: run");
        // §B-104 — l'exception d'un handler (worker `onmessage`,
        // timer, thenable) remontait jusqu'ici et mourait dans
        // `unwrap_or(0)` : promesse pendante sans témoin. Nommée.
        let jobs = match eng.run_pending_bounded(max) {
            Ok(n) => n,
            Err(e) => {
                if !matches!(e, explorer_js::JsError::Timeout) {
                    tracing::warn!(
                        tab,
                        error = %e,
                        "pump: exception dans un handler JS (avant : avalée)"
                    );
                }
                0
            }
        };
        dlv_info!("pump: js jobs={jobs}");
        let mutated = if sync_ok {
            let m = Session::sync_for_apply(&mut *eng);
            dlv_info!("pump: sync mutated={}", m.is_some());
            m
        } else {
            None
        };
        (jobs, mutated, eng)
    })
    .await;
    sync_phase(tab, "pump.offlock", tb);
    // Étape 3a : commit = swap des tampons sous verrou court
    // → livrées sur le moteur détaché **hors verrou** →
    // réinsertion + tail sous verrou. `js_detached` reste
    // armé pendant la fenêtre : une réponse qui arrive en
    // vol retombe dans les Vec de l'onglet, prise à la
    // pompe suivante.
    dlv_info!("pump: commit");
    let tc = Instant::now();
    let stage = {
        let mut s = sess.lock().await;
        s.commit_js_pump_stage(tab, epoch, out.ok(), sync_ok)
    };
    sync_phase(tab, "commit.stage", tc);
    let del = match stage {
        js_glue::PumpStage::Done => None,
        js_glue::PumpStage::Tail { jobs, mutated, eng } => Some((
            jobs,
            mutated,
            js_glue::Delivered {
                eng: Some(eng),
                stall: None,
                jobs: 0,
                mutated: None,
            },
        )),
        js_glue::PumpStage::Deliver(d) => {
            let js_glue::PumpDeliver {
                eng,
                queues,
                jobs,
                mutated,
                sync_ok,
            } = d;
            let res = tokio::task::spawn_blocking(move || {
                dlv_info!("pump: deliver tab={tab}");
                Session::deliver_pending(eng, queues, sync_ok, max, tab)
            })
            .await;
            // Worker mort en vol — moteur perdu (l'onglet
            // continue sans JS ; le finish libère
            // `js_detached` et jette les arrivées).
            let d = res.unwrap_or_else(|_| js_glue::Delivered {
                eng: None,
                stall: None,
                jobs: 0,
                mutated: None,
            });
            Some((jobs, mutated, d))
        }
    };
    let mut s = sess.lock().await;
    let tc = Instant::now();
    let jobs = del.map_or(0, |(j, m, d)| {
        s.commit_js_pump_finish(tab, epoch, d, j, m, tc)
    });
    // 55e + Étape 3c : les animations cadencées par ce
    // battement — la payload est détachée sous verrou court,
    // le relayout tourne hors verrou (`commit.anim` mesure
    // take+worker+commit, dont le relayout est désormais
    // hors `pump.commit_lock`).
    dlv_info!("pump: anim");
    let anim = s.take_anim_tick(tab);
    sync_phase(tab, "pump.commit_lock", tc);
    drop(s);
    let ta = Instant::now();
    if let Some(job) = anim {
        let _ = run_anim_tick_detached(sess, tab, job).await;
    }
    sync_phase(tab, "commit.anim", ta);
    // 55h : la mutation rapportée par le miroir a été
    // enfilée — l'apply (re-parse → cascade → relayout →
    // repaint) tourne en `spawn_blocking` hors verrou.
    dlv_info!("pump: dapply");
    drain_applies_shared(sess, tab).await;
    jobs
}

/// Tick anim détaché (Étape 3c — complément de la gate
/// Étape 222) : le relayout animé quittait `sess.lock()`…
/// ne sortait **pas** — `relayout_animated` tenait tout le
/// pipeline style→layout→raster dans la fenêtre
/// `commit_lock` (33,5 s mesurés github). Ici
/// `take_anim_tick` ne fait sous verrou que gate + smooth
/// scroll + détachement ; [`paint::anim_tick_payload`]
/// tourne en `spawn_blocking` ; [`Session::commit_anim_tick`]
/// réinsère sous verrou court (garde epoch, repaint de
/// repositionnement si scroll en vol). `take_nav_paint`/
/// `take_next_apply` voient `paint_detached` et sautent ou
/// attendent — un tick perdu est rattrapé par la pompe
/// suivante, un apply n'est jamais supplanté. `true` = une
/// frame a été produite et invalidée.
async fn advance_animations_detached(sess: &tokio::sync::Mutex<Session>, tab: TabId) -> bool {
    let job = {
        let mut s = sess.lock().await;
        s.take_anim_tick(tab)
    };
    let Some(job) = job else {
        return false;
    };
    run_anim_tick_detached(sess, tab, job).await
}

/// Corps async du tick détaché : worker `spawn_blocking`
/// puis commit. Un panic worker laisse la payload parquée
/// par [`apply::DetachedPaint`] — le drapeau est libéré,
/// la prochaine opportunité sous verrou la réinsère.
async fn run_anim_tick_detached(
    sess: &tokio::sync::Mutex<Session>,
    tab: TabId,
    job: anim::AnimTickJob,
) -> bool {
    let anim::AnimTickJob {
        mut g,
        ctx,
        ops,
        ords,
    } = job;
    let out = tokio::task::spawn_blocking(move || {
        dlv_info!("pump: animjob tab={tab}");
        let r = paint::anim_tick_payload(g.payload_mut(), &ctx, &ops, &ords, tab);
        dlv_info!("pump: animjob_done tab={tab}");
        (g, r)
    })
    .await;
    let mut s = sess.lock().await;
    if let Ok((g, r)) = out {
        let produced = r.is_ok();
        s.commit_anim_tick(tab, g.epoch(), g.into_payload(), produced);
        produced
    } else {
        // Le worker est mort avec sa payload — `Drop`
        // de la garde l'a parquée : libérer le drapeau
        // pour que la réclamation la réinsère (issue #23).
        s.paint_in_flight.remove(&tab);
        if let Some(e) = s.tabs.get_mut(&tab) {
            e.paint_detached = false;
        }
        false
    }
}

/// Réponse d'une évaluation JS à la demande (sonde `/eval`).
pub enum EvalReply {
    /// Moteur disponible : `value` = résultat sérialisé par
    /// le moteur ou message d'erreur JS ; `jobs` = jobs
    /// (microtasks/timers) drainés après l'éval.
    Done {
        value: Result<String, String>,
        jobs: usize,
    },
    /// Pas de moteur sur l'onglet (page sans `<script>`,
    /// naviguée) ou pompe détachée déjà en vol — réessayer
    /// plus tard (l'éval n'a pas tourné).
    Unavailable,
}

/// Évalue `src` dans le contexte top-level de l'onglet —
/// sonde live (bissection d'hydratation, état du miroir,
/// probes d'attribution — ex. « la var CSS est-elle posée ? »).
///
/// Même contrat de détachement que [`pump_timers_shared`] :
/// l'éval + le drain des jobs qu'elle enfile +
/// la sérialisation miroir tournent en `spawn_blocking` hors
/// verrou ; `commit_js_pump` réinsère (garde epoch, buffers
/// net/attrs, drains de fin) et les applies sont drainés —
/// une éval peut muter le DOM comme n'importe quel script.
///
/// `run_pending_bounded` borne le drain (JS-C103) ; l'éval
/// elle-même reste soumise au budget ops du moteur — une
/// expression hostile est traitée comme un script hostile.
pub async fn eval_shared(sess: &tokio::sync::Mutex<Session>, tab: TabId, src: String) -> EvalReply {
    if let Ok(mut g) = crate::LAST_DRAIN.lock() {
        *g = "eval";
    }
    let taken = {
        let mut s = sess.lock().await;
        s.take_js_engine(tab)
    };
    let Some((mut eng, max, epoch, sync_ok)) = taken else {
        return EvalReply::Unavailable;
    };
    let out = tokio::task::spawn_blocking(move || {
        let value = eng
            .eval_ctx(explorer_js::JS_CTX_TOP, &src, Some("api/eval"))
            .map_err(|e| e.to_string());
        // L'éval a pu armer timers/microtasks (attribution
        // d'hydratation, setTimeout(0)…) — on les draine pour
        // que la valeur rendue reflète l'état stabilisé.
        dlv_info!("pump: evalrun");
        // Même honnêteté que la pompe périodique : l'exception
        // d'un handler drainé ici était avalée (`unwrap_or(0)`).
        let jobs = match eng.run_pending_bounded(max) {
            Ok(n) => n,
            Err(e) => {
                if !matches!(e, explorer_js::JsError::Timeout) {
                    tracing::warn!(
                        tab,
                        error = %e,
                        "pump: exception dans un handler JS après eval (avant : avalée)"
                    );
                }
                0
            }
        };
        dlv_info!("pump: evalret jobs={jobs}");
        let mutated = if sync_ok {
            Session::sync_for_apply(&mut *eng)
        } else {
            None
        };
        (value, jobs, mutated, eng)
    })
    .await;
    let mut s = sess.lock().await;
    if let Ok((value, jobs, mutated, eng)) = out {
        let _ = s.commit_js_pump(tab, epoch, Some((jobs, mutated, eng)), sync_ok);
        drop(s);
        drain_applies_shared(sess, tab).await;
        EvalReply::Done { value, jobs }
    } else {
        // Bloc perdu (panic du spawn) — commit sans
        // moteur : l'onglet continue sans JS, la sonde
        // rend l'échec plutôt que de masquer.
        let _ = s.commit_js_pump(tab, epoch, None, sync_ok);
        EvalReply::Done {
            value: Err("bloc spawn perdu (panic)".to_string()),
            jobs: 0,
        }
    }
}

/// Drain d'applies sur session partagée (étape 55h) :
/// détache l'état de peinture de l'onglet, applique en
/// `spawn_blocking` (re-parse → cascade → remap/relayout →
/// repaint — le mur de ~157 s mesuré sur `storm.html` ne
/// gèle plus l'API), puis réinsère sous verrou court gardé
/// par epoch de navigation. Itère la file — un apply
/// commis peut en dévoiler un enfilé entre-temps.
///
/// Appelée par la pompe (`pump_timers_shared` la suit) et
/// par les handlers d'entrée des hôtes partagés (après
/// `handle_input_now`, avant `drain_navs_shared`).
/// Retourne le nombre d'applies commis.
pub async fn drain_applies_shared(sess: &tokio::sync::Mutex<Session>, tab: TabId) -> usize {
    if let Ok(mut g) = crate::LAST_DRAIN.lock() {
        *g = "applies";
    }
    let mut applied = 0;
    loop {
        let job = {
            let mut s = sess.lock().await;
            s.take_next_apply()
        };
        let Some((job, mut g, ctx, old_title)) = job else {
            break;
        };
        let (tab, epoch) = (job.tab, job.epoch);
        // Hors verrou : les phases lourdes ne gèlent plus
        // ni l'API ni les autres onglets. La garde traverse
        // le worker : un drain droppé en vol parque la
        // payload au lieu de la perdre (issue #23).
        let out = tokio::task::spawn_blocking(move || {
            // §B-111 — brackette le segment worker avant
            // `journal: apply` (morgue C405 : la mort
            // post-collect vit sur `tokio-rt-worker`).
            dlv_info!("pump: applyjob tab={tab}");
            // ADR-0025 : voie journal (rejeu en place) ou
            // re-parse ; le rejet est remonté, la session
            // re-enfile en mode baseline.
            let r = apply::run_apply_job(g.payload_mut(), &ctx, &job, old_title.as_deref());
            dlv_info!("pump: applyjob_done tab={tab}");
            (g, r, job)
        })
        .await;
        let mut s = sess.lock().await;
        match out {
            Ok((g, Ok(o), job)) => {
                if job.journal.is_some() {
                    // JS-C316 — `journal_applies` = rejouées
                    // **réelles** : les journaux vides (drain
                    // propre C293) ne comptent pas — sinon la
                    // métrique inonde à ~1/pompe sur page
                    // saine. Le streak `UnmappedKey`, lui,
                    // retombe sur tout apply abouti.
                    if job.journal.as_ref().is_some_and(|j| !j.ops.is_empty()) {
                        s.journal_applies += 1;
                    }
                    s.unmapped_streaks.insert(tab, 0);
                }
                s.commit_apply_detached(tab, epoch, g.into_payload(), &o);
            }
            Ok((g, Err(why), job)) => {
                s.journal_apply_rejected(tab, epoch, g.into_payload(), job, &why);
            }
            Err(_) => {
                // Panic dans le worker : la payload est
                // perdue avec la tâche — l'onglet redevient
                // « jamais peint » (lazy-heal au prochain
                // `/frame`, étape 56b) plutôt que détaché
                // à vie.
                s.applies_fin += 1;
                s.paint_in_flight.remove(&tab);
                if let Some(e) = s.tabs.get_mut(&tab) {
                    e.paint_detached = false;
                }
            }
        }
        applied += 1;
    }
    // Étape 3b — hôte `events_offlock` : les entrées
    // différées pendant la peinture ou la fenêtre moteur
    // sont rejouées ici, dispatches détachés, plutôt que
    // sous verrou dans les commits. Hôtes `&mut Session` :
    // replay synchrone inchangé dans les commits.
    let replay = {
        let s = sess.lock().await;
        s.events_offlock
            && s.tabs
                .get(&tab)
                .is_some_and(|e| !e.deferred_inputs.is_empty() && !e.paint_detached)
    };
    if replay {
        drain_deferred_inputs_shared(sess, tab).await;
    }
    applied
}

// ————————————————————————————————————————————————————
// Étape 3b — input hors verrou, voie shared uniquement.
//
// `handle_input_shared` est le pendant détaché de
// `Session::handle_input_now` : chaque grappe session
// (hit-test, édition DOM, focus, intentions nav) court
// sous verrou court ; chaque dispatch JS
// (`dispatch_event_props` + `run_pending` + synchro
// miroir) court dans `spawn_blocking` sur le moteur
// détaché — `js_detached` reste armé pour toute la durée
// de l'input, donc les réponses réseau tamponnent au lieu
// de livrer sous verrou, et les entrées concurrentes
// atterrissent dans `deferred_inputs` (ordre préservé).
// ————————————————————————————————————————————————————

/// Entrée sur une session partagée (étape 3b) — même
/// sémantique d'outcome et de drains que
/// [`Session::handle_input_now`] + le spawn de drain de
/// l'appelant, mais **zéro dispatch JS sous le verrou**.
///
/// Le moteur est sorti pour toute la durée de l'input
/// (`Session::take_event_engine`) et réinséré en fin de
/// fenêtre (`Session::finish_event_engine`) — toujours,
/// même sur `Err`.
///
/// # Errors
/// [`CoreError::UnknownTab`] — onglet inconnu ; erreurs de
/// pipeline d'édition/re-layout, comme la voie synchrone.
pub async fn handle_input_shared(
    sess: &tokio::sync::Mutex<Session>,
    tab: TabId,
    ev: InputEvent,
) -> Result<InputOutcome, CoreError> {
    // Pré sous verrou : existence, auto-réparation frame,
    // report différé — même ordre de garde que la voie
    // synchrone (`paint_detached`, file non vide) +
    // `js_detached` (pompe ou input en vol — l'entrée
    // rejoue à la réinsertion plutôt que mourir à
    // `js = None`).
    let mut ee = {
        let mut s = sess.lock().await;
        if !s.tabs.contains_key(&tab) {
            return Err(CoreError::UnknownTab(tab));
        }
        s.ensure_painted(tab);
        if s.tabs.get(&tab).is_some_and(|e| {
            e.paint_detached || e.js_detached.is_some() || !e.deferred_inputs.is_empty()
        }) {
            let is_scroll = matches!(ev, InputEvent::Scroll { .. });
            s.defer_input(tab, ev);
            if is_scroll {
                return Ok(InputOutcome::Scrolled {
                    scroll_y: s.projected_scroll(tab),
                });
            }
            tracing::info!(tab, "input différé — paint/js en vol, rejeté comme ignored");
            return Ok(InputOutcome::Ignored);
        }
        s.take_event_engine(tab)
    };
    let out = input_shared_body(sess, tab, ev, &mut ee).await;
    // Fin de fenêtre — réinsertion epoch-gardée, quoi
    // qu'il soit arrivé (`Err` compris).
    {
        let mut s = sess.lock().await;
        if let Some(e) = ee.take() {
            s.finish_event_engine(tab, e);
        }
    }
    out
}

/// Corps de [`handle_input_shared`] — `ee` emprunté :
/// les sorties `?`/`Err` laissent le moteur à refermer
/// par l'appelant.
#[allow(clippy::too_many_lines)] // même ordonnancement que `handle_input_now` — les bras sont les frontières verrou/dispatch
async fn input_shared_body(
    sess: &tokio::sync::Mutex<Session>,
    tab: TabId,
    ev: InputEvent,
    ee: &mut Option<js_glue::EventEngine>,
) -> Result<InputOutcome, CoreError> {
    match ev {
        InputEvent::Key { key, mods } => {
            // `keydown` — `preventDefault` supprime l'édition
            // et la soumission `Enter` (étape 103/105).
            let (path, props) = {
                let mut s = sess.lock().await;
                (s.key_path(tab), js_glue::key_event_props(&key, mods))
            };
            // `None` = doc-event nu (props vides, comme
            // `dispatch_doc_event` en voie synchrone).
            let spec = js_glue::EventSpec {
                ty: "keydown".to_string(),
                path: path.clone().unwrap_or_default(),
                props: if path.is_some() { props } else { Vec::new() },
                attrs: Vec::new(),
                revert_on_prevent: false,
            };
            let prevented = dispatch_events(sess, tab, ee, vec![spec], true)
                .await
                .first()
                .copied()
                .unwrap_or(false);
            if prevented {
                return Ok(InputOutcome::Ignored);
            }
            if mods.chord() {
                return Ok(InputOutcome::Ignored);
            }
            // Popup `<select>` ouvert — clavier chrome.
            if sess
                .lock()
                .await
                .tabs
                .get(&tab)
                .is_some_and(|e| e.popup.is_some())
            {
                return popup_key_shared(sess, tab, ee, &key).await;
            }
            // Soumission implicite `Enter`.
            if key == "Enter" {
                let paths = {
                    let s = sess.lock().await;
                    s.focused_form_default_paths(tab)
                };
                if let Some((form, sub)) = paths {
                    set_focus_shared(sess, tab, ee, None).await;
                    return Ok(
                        match try_submit_shared(sess, tab, ee, Some(form), sub, false).await? {
                            InputOutcome::Ignored => InputOutcome::Unfocused,
                            o => o,
                        },
                    );
                }
            }
            handle_key_shared(sess, tab, ee, &key).await
        }
        InputEvent::KeyUp { key, mods } => {
            let (path, props) = {
                let mut s = sess.lock().await;
                (s.key_path(tab), js_glue::key_event_props(&key, mods))
            };
            let spec = js_glue::EventSpec {
                ty: "keyup".to_string(),
                path: path.clone().unwrap_or_default(),
                props: if path.is_some() { props } else { Vec::new() },
                attrs: Vec::new(),
                revert_on_prevent: false,
            };
            dispatch_events(sess, tab, ee, vec![spec], true).await;
            Ok(InputOutcome::Ignored)
        }
        InputEvent::Scroll { dy } => {
            let wpath = {
                let mut s = sess.lock().await;
                s.scroll_pre(tab)
            };
            let prevented = dispatch_events(
                sess,
                tab,
                ee,
                vec![js_glue::EventSpec {
                    ty: "wheel".to_string(),
                    path: wpath,
                    props: Vec::new(),
                    attrs: Vec::new(),
                    revert_on_prevent: false,
                }],
                true,
            )
            .await
            .first()
            .copied()
            .unwrap_or(false);
            if !prevented {
                {
                    let mut s = sess.lock().await;
                    s.scroll_by(tab, dy)?;
                }
                // `scroll` niveau document (non annulable).
                dispatch_events(
                    sess,
                    tab,
                    ee,
                    vec![js_glue::EventSpec {
                        ty: "scroll".to_string(),
                        path: Vec::new(),
                        props: Vec::new(),
                        attrs: Vec::new(),
                        revert_on_prevent: false,
                    }],
                    true,
                )
                .await;
            }
            Ok(InputOutcome::Scrolled {
                scroll_y: sess.lock().await.tabs.get(&tab).map_or(0.0, |t| t.scroll_y),
            })
        }
        InputEvent::MouseMove { x, y } => mouse_move_shared(sess, tab, ee, x, y).await,
        InputEvent::Click { x, y } => click_shared(sess, tab, ee, x, y).await,
    }
}

/// Rafale de dispatches détachée (étape 3b) — le moteur
/// navette dans `spawn_blocking` via `ee` ; commit sous
/// verrou court qui **garde** la fenêtre ouverte (sauf
/// fenêtre morte : `finish_event_engine` la referme
/// aussitôt). Retourne `prevented` par event.
async fn dispatch_events(
    sess: &tokio::sync::Mutex<Session>,
    tab: TabId,
    ee: &mut Option<js_glue::EventEngine>,
    events: Vec<js_glue::EventSpec>,
    run_poll_net: bool,
) -> Vec<bool> {
    let n = events.len();
    let Some(e) = ee.take() else {
        // Pas de moteur dans la fenêtre (page sans JS,
        // stall, ou fenêtre morte plus tôt) — mêmes sauts
        // que la voie synchrone, tampon doc-event inclus.
        let mut s = sess.lock().await;
        s.buffer_doc_events(tab, &events);
        return vec![false; n];
    };
    if e.dead || e.eng.is_none() {
        let mut s = sess.lock().await;
        s.buffer_doc_events(tab, &events);
        *ee = Some(e);
        return vec![false; n];
    }
    let epoch = e.epoch;
    let t0 = Instant::now();
    let out = tokio::task::spawn_blocking(move || Session::run_event_dispatch(e, events)).await;
    let (mut e, outcome) = match out {
        Ok(pair) => pair,
        Err(_) => {
            // Worker perdu — fenêtre morte : le finish
            // referme et jette les tampons (page fraîche =
            // miroir frais), comme le commit de pompe.
            (
                js_glue::EventEngine {
                    eng: None,
                    epoch,
                    lifecycle: Vec::new(),
                    fire_doc_load: false,
                    dead: true,
                },
                js_glue::EventOutcome {
                    prevented: vec![false; n],
                    stall: None,
                    emit_err: None,
                    mutated: Vec::new(),
                },
            )
        }
    };
    let prevented = {
        let mut s = sess.lock().await;
        s.commit_event_round(tab, &mut e, outcome, run_poll_net, t0)
    };
    if e.dead {
        let mut s = sess.lock().await;
        s.finish_event_engine(tab, e);
    } else {
        *ee = Some(e);
    }
    prevented
}

/// Construit un [`js_glue::EventSpec`] sans props ni
/// poussées (le cas courant des rafales input).
fn spec(ty: &str, path: Vec<(String, u32)>) -> js_glue::EventSpec {
    js_glue::EventSpec {
        ty: ty.to_string(),
        path,
        props: Vec::new(),
        attrs: Vec::new(),
        revert_on_prevent: false,
    }
}

/// `blur`/`focus` hors verrou (étape 3b) — même ordre que
/// [`Session::set_focus`] : `blur` sur l'ancien chemin,
/// pose d'état sous verrou, `focus` sur le nouveau.
async fn set_focus_shared(
    sess: &tokio::sync::Mutex<Session>,
    tab: TabId,
    ee: &mut Option<js_glue::EventEngine>,
    field: Option<explorer_dom::NodeId>,
) {
    let (old_path, new_path, new_key) = {
        let s = sess.lock().await;
        s.focus_paths(tab, field)
    };
    if old_path == new_path {
        return;
    }
    if let Some(o) = old_path {
        dispatch_events(sess, tab, ee, vec![spec("blur", o)], false).await;
    }
    {
        let mut s = sess.lock().await;
        s.set_focus_state(tab, new_path.clone(), new_key);
    }
    if let Some(p) = new_path {
        dispatch_events(sess, tab, ee, vec![spec("focus", p)], false).await;
    }
}

/// Soumission hors verrou (étape 3b) — même séquence que
/// [`Session::try_submit`] : contrôles invalides →
/// `invalid` puis blocage ; `submit` puis action par
/// défaut (nav script prioritaire, `submit_form` sinon).
async fn try_submit_shared(
    sess: &tokio::sync::Mutex<Session>,
    tab: TabId,
    ee: &mut Option<js_glue::EventEngine>,
    form_path: Option<Vec<(String, u32)>>,
    sub_path: Option<Vec<(String, u32)>>,
    click_prevented: bool,
) -> Result<InputOutcome, CoreError> {
    let (novalidate, invalid_paths) = {
        let s = sess.lock().await;
        s.submit_pre(tab, form_path.as_deref(), sub_path.as_deref())
    };
    if !click_prevented && !novalidate && !invalid_paths.is_empty() {
        let specs = invalid_paths
            .into_iter()
            .map(|p| spec("invalid", p))
            .collect();
        dispatch_events(sess, tab, ee, specs, true).await;
        return Ok(InputOutcome::Ignored);
    }
    if !click_prevented {
        let prevented = match &form_path {
            Some(p) => dispatch_events(sess, tab, ee, vec![spec("submit", p.clone())], true)
                .await
                .first()
                .copied()
                .unwrap_or(false),
            None => false,
        };
        if !prevented {
            let nav = {
                let mut s = sess.lock().await;
                s.collect_script_navs(tab);
                s.nav_pending(tab)
            };
            if nav {
                return Ok(InputOutcome::Navigated);
            }
            let enqueued = {
                let mut s = sess.lock().await;
                let f = s.resolve_path(tab, form_path.as_deref());
                let sub = s.resolve_path(tab, sub_path.as_deref());
                match f {
                    Some(f) => {
                        s.submit_form(tab, f, sub)?;
                        true
                    }
                    None => false,
                }
            };
            if enqueued {
                return Ok(InputOutcome::Navigated);
            }
        }
    }
    Ok(InputOutcome::Ignored)
}

/// Édition clavier hors verrou (étape 3b) — même
/// séquence que [`Session::handle_key`] : `Enter`/`Escape`
/// sortent du champ, sinon édition DOM + `input` avec la
/// `value` poussée au miroir d'abord.
async fn handle_key_shared(
    sess: &tokio::sync::Mutex<Session>,
    tab: TabId,
    ee: &mut Option<js_glue::EventEngine>,
    key: &str,
) -> Result<InputOutcome, CoreError> {
    let has_field = {
        let mut s = sess.lock().await;
        s.focused_node(tab).is_some()
    };
    if !has_field {
        tracing::trace!(tab, key, "touche sans champ focalisé");
        return Ok(InputOutcome::Unfocused);
    }
    if matches!(key, "Enter" | "Escape") {
        set_focus_shared(sess, tab, ee, None).await;
        {
            let mut s = sess.lock().await;
            // Le caret disparaît avec le focus (M-H1 #1).
            let _ = s.repaint(tab);
            s.emit_invalidated(tab);
        }
        return Ok(InputOutcome::Unfocused);
    }
    let edited = {
        let mut s = sess.lock().await;
        s.key_edit(tab, key)?
    };
    let Some((field_path, value)) = edited else {
        tracing::trace!(tab, key, "touche — champ focalisé mort entre-temps");
        return Ok(InputOutcome::Unfocused);
    };
    let mut input_spec = spec("input", field_path.clone());
    input_spec
        .attrs
        .push((field_path, "value".to_string(), Some(value)));
    dispatch_events(sess, tab, ee, vec![input_spec], true).await;
    {
        let mut s = sess.lock().await;
        s.relayout(tab)?;
        s.emit_invalidated(tab);
    }
    Ok(InputOutcome::Edited)
}

/// Clavier du popup `<select>` ouvert (étape 3b) — même
/// logique que [`Session::popup_key`] : flèches bornées,
/// `Enter` choisit (dispatches détachés via
/// `popup_choose_shared`), autre touche = fermeture.
async fn popup_key_shared(
    sess: &tokio::sync::Mutex<Session>,
    tab: TabId,
    ee: &mut Option<js_glue::EventEngine>,
    key: &str,
) -> Result<InputOutcome, CoreError> {
    enum Pop {
        Choose(Vec<(String, u32)>, Option<usize>),
        Out(InputOutcome),
    }
    let act = {
        let mut s = sess.lock().await;
        // Lecture du popup dans une portée dédiée — les
        // bras appellent `repaint`/`emit_invalidated` sur
        // `s` entier.
        let (path, rows, hovered, first_sel) = {
            let entry = s.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            let Some(pop) = &entry.popup else {
                return Ok(InputOutcome::Ignored);
            };
            (
                pop.path.clone(),
                pop.options.len().min(MAX_POPUP_ROWS),
                pop.hovered,
                pop.options.iter().position(|&(_, sel)| sel),
            )
        };
        match key {
            "ArrowDown" | "ArrowUp" => {
                let next = match hovered {
                    Some(h) if key == "ArrowDown" => (h + 1).min(rows.saturating_sub(1)),
                    Some(h) => h.saturating_sub(1),
                    None if key == "ArrowDown" => 0,
                    None => rows.saturating_sub(1),
                };
                if let Some(pop) = s.tabs.get_mut(&tab).and_then(|e| e.popup.as_mut()) {
                    pop.hovered = Some(next);
                }
                s.repaint(tab)?;
                s.emit_invalidated(tab);
                Pop::Out(InputOutcome::Edited)
            }
            "Enter" => Pop::Choose(path, hovered.or(first_sel)),
            _ => {
                if let Some(e) = s.tabs.get_mut(&tab) {
                    e.popup = None;
                }
                s.repaint(tab)?;
                s.emit_invalidated(tab);
                Pop::Out(InputOutcome::Ignored)
            }
        }
    };
    match act {
        Pop::Out(o) => Ok(o),
        Pop::Choose(path, row) => popup_choose_shared(sess, tab, ee, path, row).await,
    }
}

/// Choix popup hors verrou (étape 3b) — `popup_choose_pre`
/// sous verrou puis `input`/`change` détachés avec les
/// poussées `selected` en `attrs` (le handler lit
/// `option.selected` post-choix).
async fn popup_choose_shared(
    sess: &tokio::sync::Mutex<Session>,
    tab: TabId,
    ee: &mut Option<js_glue::EventEngine>,
    path: Vec<(String, u32)>,
    row: Option<usize>,
) -> Result<InputOutcome, CoreError> {
    let (fire, attrs) = {
        let mut s = sess.lock().await;
        s.popup_choose_pre(tab, &path, row)?
    };
    if fire {
        let mut input = spec("input", path.clone());
        input.attrs = attrs.clone();
        let mut change = spec("change", path);
        change.attrs = attrs;
        dispatch_events(sess, tab, ee, vec![input, change], true).await;
    }
    {
        let mut s = sess.lock().await;
        s.emit_invalidated(tab);
    }
    Ok(InputOutcome::Edited)
}

/// Survol hors verrou (étape 3b) — `mouse_move_pre` sous
/// verrou, rafale `mouseout`/`mouseover`/`mousemove`
/// détachée, `mouse_move_post` sous verrou.
async fn mouse_move_shared(
    sess: &tokio::sync::Mutex<Session>,
    tab: TabId,
    ee: &mut Option<js_glue::EventEngine>,
    x: f32,
    y: f32,
) -> Result<InputOutcome, CoreError> {
    let pre = {
        let mut s = sess.lock().await;
        s.mouse_move_pre(tab, x, y)?
    };
    match pre {
        input::MovePre::Popup(o) => Ok(o),
        input::MovePre::Hover(events) => {
            if !events.is_empty() {
                dispatch_events(sess, tab, ee, events, true).await;
            }
            let mut s = sess.lock().await;
            Ok(s.mouse_move_post(tab, x, y))
        }
    }
}

/// Clic hors verrou (étape 3b) — même chaîne que
/// [`Session::handle_click`] : `click` puis revert-ou-
/// `input`/`change`, focus, popup `<select>`, `submit`,
/// activation `<a>`, `Edited`/`Ignored`.
#[allow(clippy::too_many_lines)] // même séquence que la voie synchrone — les grappes sont les frontières
async fn click_shared(
    sess: &tokio::sync::Mutex<Session>,
    tab: TabId,
    ee: &mut Option<js_glue::EventEngine>,
    x: f32,
    y: f32,
) -> Result<InputOutcome, CoreError> {
    let pre = {
        let mut s = sess.lock().await;
        s.click_pre(tab, x, y)?
    };
    let (node, node_path, toggled, ctrl_path, flip_attr) = match pre {
        input::ClickPre::Popup { path, row } => {
            return popup_choose_shared(sess, tab, ee, path, row).await;
        }
        input::ClickPre::Hit {
            node,
            node_path,
            toggled,
            ctrl_path,
            flip_attr,
        } => (node, node_path, toggled, ctrl_path, flip_attr),
    };
    let prevented = match &node_path {
        Some(p) => {
            let mut click = spec("click", p.clone());
            if let Some(m) = flip_attr {
                click.attrs.push(m);
                // `preventDefault` restaure `checked` —
                // revert miroir dans le worker, journal
                // `set`+`remove` en ordre sur le DOM hôte.
                click.revert_on_prevent = true;
            }
            dispatch_events(sess, tab, ee, vec![click], true)
                .await
                .first()
                .copied()
                .unwrap_or(false)
        }
        None => false,
    };
    if toggled.is_some() {
        if prevented {
            // Revert de l'activation — `flip_checked`
            // repousse `checked` (tamponné, livré à la
            // prochaine synchro).
            let r = {
                let mut s = sess.lock().await;
                match s.resolve_path(tab, ctrl_path.as_deref()) {
                    Some(c) => s.flip_checked(tab, c),
                    None => Ok(()),
                }
            };
            r?;
        } else if let Some(p) = ctrl_path {
            // `input` puis `change` — ordre DOM.
            dispatch_events(
                sess,
                tab,
                ee,
                vec![spec("input", p.clone()), spec("change", p)],
                true,
            )
            .await;
        }
        {
            let mut s = sess.lock().await;
            s.relayout(tab)?;
            s.emit_invalidated(tab);
        }
        return Ok(if prevented {
            InputOutcome::Ignored
        } else {
            InputOutcome::Edited
        });
    }
    // Navigation script (`location`, `history`) : priorité
    // sur l'action par défaut du clic.
    let nav = {
        let mut s = sess.lock().await;
        s.collect_script_navs(tab);
        s.nav_pending(tab)
    };
    if nav {
        return Ok(InputOutcome::Navigated);
    }
    let field = {
        let s = sess.lock().await;
        s.field_at(tab, x, y)
    };
    if let Some(field) = field {
        set_focus_shared(sess, tab, ee, Some(field)).await;
        {
            let mut s = sess.lock().await;
            s.repaint(tab)?;
            s.emit_invalidated(tab);
        }
        return Ok(InputOutcome::Focused);
    }
    let had_focus = sess
        .lock()
        .await
        .tabs
        .get(&tab)
        .is_some_and(|e| e.focused.is_some());
    set_focus_shared(sess, tab, ee, None).await;
    if had_focus {
        let mut s = sess.lock().await;
        s.repaint(tab)?;
        s.emit_invalidated(tab);
    }
    let sel = {
        let s = sess.lock().await;
        s.select_at(tab, x, y)
    };
    if let Some(sel) = sel {
        let mut s = sess.lock().await;
        s.toggle_select_popup(tab, sel)?;
        s.emit_invalidated(tab);
        return Ok(InputOutcome::Edited);
    }
    let sub = {
        let s = sess.lock().await;
        s.submit_at(tab, x, y)
    };
    if let Some((form, sub_id)) = sub {
        // Chemins captés sous verrou — le `submit` est
        // dispatché hors verrou, les `NodeId` mourraient.
        let paths = {
            let s = sess.lock().await;
            s.tabs
                .get(&tab)
                .and_then(|e| e.dom.as_ref())
                .map(|d| (element_path(d, form), element_path(d, sub_id)))
        };
        let (form_path, sub_path) = paths.unwrap_or_default();
        return try_submit_shared(sess, tab, ee, form_path, sub_path, prevented).await;
    }
    let linked = {
        let mut s = sess.lock().await;
        s.link_activate(tab, x, y, prevented)?
    };
    if linked {
        return Ok(InputOutcome::Navigated);
    }
    if node.is_some() && !prevented {
        // B-31 — élément cliqué, handlers possibles.
        tracing::info!(tab, x, y, "click livré au DOM — aucune action par défaut");
        let mut s = sess.lock().await;
        s.relayout(tab)?;
        s.emit_invalidated(tab);
        return Ok(InputOutcome::Edited);
    }
    tracing::info!(
        tab,
        x,
        y,
        hit = node.is_some(),
        prevented,
        "click ignored — aucun contrôle/lien actionnable sous le point"
    );
    Ok(InputOutcome::Ignored)
}

/// Replay des entrées différées, voie shared (étape 3b) —
/// la file est **prise d'un coup** puis chaque entrée est
/// gérée hors verrou : une entrée qui re-diffère
/// (re-détachement entre-temps) réenfile sur la file
/// fraîche sans être rejouée dans cette passe, comme la
/// voie synchrone.
pub async fn drain_deferred_inputs_shared(sess: &tokio::sync::Mutex<Session>, tab: TabId) {
    let mut q = {
        let mut s = sess.lock().await;
        s.take_deferred_queue(tab)
    };
    while let Some(ev) = q.pop_front() {
        let _ = handle_input_shared(sess, tab, ev).await;
    }
}

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
/// Mêmes erreurs que [`Session::navigate`]
/// (`Url`, `Fetch`, `HttpStatus`, `EmptyDocument`,
/// `NothingToPaint`, `FrameTooBig`, `UnknownTab`) —
/// `PipelineError` est émis sur le bus pour les erreurs de
/// pipeline, comme le chemin `&mut self`.
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
        s.nav_begin(tab, req, push)?
    };
    let epoch = job.epoch;
    let (done, tail) = match job
        .run(|dom: &Dom| -> PaintFut<'_> {
            // `try_lock` : la peinture progressive ne bloque
            // jamais le téléchargement — un tick manqué (lock
            // occupé par un `/frame` ou une autre phase) est
            // repeint au suivant, le commit final repeint de
            // toute façon.
            if let Ok(mut s) = sess.try_lock() {
                s.nav_paint(tab, epoch, dom);
            }
            Box::pin(async {})
        })
        .await
    {
        // Téléchargement (étape 70) : enregistré en attente
        // — rien de committé, la page courante reste.
        Ok(NavOutcome::Download(dl)) => {
            let mut s = sess.lock().await;
            s.offer_download(tab, dl);
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
        match s.nav_apply(tab, tail, done) {
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
    sub.run().await;
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
    let (mut p, ctx, canvas_ops) = loop {
        let take = {
            let mut s = sess.lock().await;
            s.take_nav_paint(tab, epoch)
        };
        match take {
            NavPaint::Ready(p, c, o) => break (*p, c, o),
            NavPaint::Superseded => return Ok(()),
            NavPaint::Busy => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    };
    let base = tail.url.clone();
    let bodies = std::mem::take(&mut tail.img_bodies);
    // Fonds `url(http…)` (étape 98) et masques `mask-image`
    // (étape 121) : posés sur la payload, insérés à la
    // cascade dans `relayout_dom_payload`.
    let bg_aliases = std::mem::take(&mut tail.bg_aliases);
    let bg_bodies = std::mem::take(&mut tail.bg_bodies);
    let mask_aliases = std::mem::take(&mut tail.mask_aliases);
    let work = tokio::task::spawn_blocking(move || {
        net::insert_images_payload(&mut p, &base, &bodies, ctx.image_max_px);
        p.bg_aliases = bg_aliases;
        p.bg_bodies = bg_bodies;
        p.mask_aliases = mask_aliases;
        let Some(dom) = p.dom.take() else {
            return (p, Err(CoreError::EmptyDocument));
        };
        let r = apply::relayout_dom_payload(&mut p, &ctx, &dom, &canvas_ops);
        p.dom = Some(dom);
        (p, r)
    })
    .await;
    let mut s = sess.lock().await;
    match work {
        Ok((p, r)) => {
            let live = s.commit_nav_paint(tab, epoch, p);
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
                // payload a été jetée, silencieux.
                _ => Ok(()),
            }
        }
        Err(e) => {
            s.rearm_nav_paint(tab);
            let err = CoreError::Fetch(format!("worker rendu de navigation paniqué : {e}"));
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
        let ok = match act {
            None => break,
            Some(DrainAct::Req(req, push)) => Box::pin(navigate_shared(sess, tab, *req, push))
                .await
                .is_ok(),
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
        };
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
        let mut s = sess.lock().await;
        let _ = s.advance_animations(tab);
        return 0;
    };
    // Hors verrou : le burst JS n'occupe ni le verrou ni le
    // worker tokio (spawn_blocking — `JsEngine: Send`). Sous
    // fenêtre de coalescence `sync_floor` la sérialisation
    // est sautée — le miroir accumule, l'API respire
    // (55g-suite).
    let tb = Instant::now();
    let out = tokio::task::spawn_blocking(move || {
        let jobs = eng.run_pending_bounded(max);
        let mutated = if sync_ok {
            Session::sync_for_apply(&mut *eng)
        } else {
            None
        };
        (jobs, mutated, eng)
    })
    .await;
    sync_phase(tab, "pump.offlock", tb);
    let mut s = sess.lock().await;
    let tc = Instant::now();
    let jobs = s.commit_js_pump(tab, epoch, out.ok(), sync_ok);
    // 55e : les animations cadencées par ce battement.
    let _ = s.advance_animations(tab);
    sync_phase(tab, "pump.commit_lock", tc);
    drop(s);
    // 55h : la mutation rapportée par le miroir a été
    // enfilée — l'apply (re-parse → cascade → relayout →
    // repaint) tourne en `spawn_blocking` hors verrou.
    drain_applies_shared(sess, tab).await;
    jobs
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
pub async fn drain_applies_shared(sess: &tokio::sync::Mutex<Session>, _tab: TabId) -> usize {
    let mut applied = 0;
    loop {
        let job = {
            let mut s = sess.lock().await;
            s.take_next_apply()
        };
        let Some((job, mut payload, ctx, old_title)) = job else {
            break;
        };
        let (tab, epoch) = (job.tab, job.epoch);
        // Hors verrou : les phases lourdes ne gèlent plus
        // ni l'API ni les autres onglets.
        let out = tokio::task::spawn_blocking(move || {
            // ADR-0025 : voie journal (rejeu en place) ou
            // re-parse ; le rejet est remonté, la session
            // re-enfile en mode baseline.
            let r = apply::run_apply_job(&mut payload, &ctx, &job, old_title.as_deref());
            (payload, r, job)
        })
        .await;
        let mut s = sess.lock().await;
        match out {
            Ok((p, Ok(o), job)) => {
                if job.journal.is_some() {
                    s.journal_applies += 1;
                }
                s.commit_apply_detached(tab, epoch, p, &o);
            }
            Ok((p, Err(why), job)) => {
                s.journal_apply_rejected(tab, epoch, p, job, &why);
            }
            Err(_) => {
                // Panic dans le worker : la payload est
                // perdue avec la tâche — l'onglet redevient
                // « jamais peint » (lazy-heal au prochain
                // `/frame`, étape 56b) plutôt que détaché
                // à vie.
                if let Some(e) = s.tabs.get_mut(&tab) {
                    e.paint_detached = false;
                }
            }
        }
        applied += 1;
    }
    applied
}

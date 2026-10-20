// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! API partagée async — points d'entrée prenant
//! `&Mutex<Session>` sans détenir le verrou.
use super::*;
use std::time::Instant;

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
        Ok(out) => out,
        Err(e) => {
            let mut s = sess.lock().await;
            s.unspec(tab); // peinture spéculative → réverte (56c)
            return Err(s.emit_error(tab, e));
        }
    };
    let sub = {
        let mut s = sess.lock().await;
        match s.nav_apply(tab, tail, done) {
            Ok(sub) => sub,
            Err(e) => return Err(s.emit_error(tab, e)),
        }
    };
    // Epoch supplantée : la navigation la plus récente a
    // gagné — celle-ci est jetée proprement (rien à
    // commettre, aucune erreur n'est observable).
    let Some(mut sub) = sub else {
        return Ok(());
    };
    sub.run().await;
    let mut s = sess.lock().await;
    match s.sub_commit(tab, sub).await {
        Ok(_) => Ok(()),
        Err(e) => Err(s.emit_error(tab, e)),
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
    match navigate_shared(sess, tab, req, false).await {
        Ok(()) => {
            let mut s = sess.lock().await;
            s.set_hist_idx(tab, idx);
            Ok(())
        }
        Err(e) => {
            let mut s = sess.lock().await;
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
/// gèlent plus l'API non plus. `true` si au moins une
/// navigation a été appliquée.
pub async fn drain_navs_shared(sess: &tokio::sync::Mutex<Session>, tab: TabId) -> bool {
    const MAX_NAV_HOPS: u8 = 8;
    let mut applied = false;
    for _ in 0..MAX_NAV_HOPS {
        let act = {
            let mut s = sess.lock().await;
            s.collect_script_navs(tab);
            s.pop_pending_nav(tab)
        };
        match act {
            None => break,
            Some(DrainAct::Req(req, push)) => {
                let _ = navigate_shared(sess, tab, *req, push).await;
            }
            Some(DrainAct::Go(0)) => {
                // `history.go(0)` = rechargement de l'entrée
                // courante (spec HTML).
                let url = {
                    let s = sess.lock().await;
                    s.tabs.get(&tab).and_then(|t| t.url.clone())
                };
                if let Some(u) = url {
                    let _ = navigate_shared(sess, tab, Request::get(u), false).await;
                }
            }
            Some(DrainAct::Go(d)) => {
                let _ = history_move_shared(sess, tab, d).await;
            }
        }
        applied = true;
    }
    applied
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
        let mutated = if sync_ok { eng.sync_dom_path() } else { None };
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
    jobs
}

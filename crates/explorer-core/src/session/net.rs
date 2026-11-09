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

/// Échecs `status: 0` consécutifs tolérés sur une même URL
/// avant d'engager le backoff §B-60 — un échec transitoire
/// (réseau qui revient) ne doit pas ralentir le retry
/// suivant : les premiers repartent immédiatement.
pub(crate) const FETCH_BACKOFF_START: u32 = 4;
/// Backoff de base §B-60 : `BASE << (streak - START)`
/// plafonné à [`FETCH_BACKOFF_CAP`] — la tempête de retries
/// applicative (mastodon ×117 / uMap ×9 vols 209) devient
/// une goutte bornée sans casser un retry raisonnable.
pub(crate) const FETCH_BACKOFF_BASE: std::time::Duration = std::time::Duration::from_millis(250);
/// Plafond du délai de retenue §B-60 — ~6 émissions/min/URL
/// au pire, la promesse finit toujours par se livrer.
pub(crate) const FETCH_BACKOFF_CAP: std::time::Duration = std::time::Duration::from_secs(10);
/// File de retenue §B-60 : borne mémoire — au-delà la
/// requête échoue immédiatement en `status: 0` (comme un
/// quota atteint), la tempête reste bornée par le quota
/// `max_script_net` des tâches vivantes.
const MAX_HELD_FETCHES: usize = 512;
/// Borne de la file `pending_script_net` (JS-C367) — un
/// fetch/XHR validé qui dépasse `max_script_net` **attend
/// un slot** (comportement navigateur) au lieu d'être
/// rejeté `status: 0` sec : la rafale `FontFace.load()`
/// de kevlar (~200 fetch/16 slots — vol 668 §B-97) tuait
/// le font-gating et l'hydratation entière. Au-delà de la
/// borne (file saturée — exhaustion réelle), rejet sec
/// `pend_full`, livré `status: 0` comme avant.
pub(crate) const MAX_PENDING_FETCHES: usize = 1_024;
/// URLs suivies par le streak §B-60 : borne mémoire — une
/// page balayant des milliers d'URL distinctes ne fait pas
/// gonfler la map (les nouvelles URLs passent sans streak).
const MAX_STREAK_URLS: usize = 1_024;
/// Borne dedup des sondes `net.drop`/`net.fail` (JS-C366) —
/// couples `(kind, host)` retenus ; au-delà, log direct.
const MAX_NET_PROBE_SEEN: usize = 512;

/// Étape 209 — cadence d'échantillonnage RSS du shed-load :
/// `memory_stats()` est un appel système — une lecture par
/// fenêtre, pas à chaque `poll_net`.
const RSS_SHED_SAMPLE: std::time::Duration = std::time::Duration::from_millis(250);

/// Marge de réarmement du shed (hysteresis) : l'épisode ne se
/// lève que sous `seuil - max(seuil/8, 64 Mio)` — sinon le RSS
/// en clapet à la frontière réarmerait la dégradation à chaque
/// passe (les fetchs droppés ne reviennent pas).
fn shed_margin(shed_mb: u32) -> u64 {
    u64::from(shed_mb / 8).max(64)
}

/// JS-C415 — kind `net.drop` : `badr` si `url` ne résout pas
/// contre `base` (base opaque/vide — le mur « host='' » réel :
/// ctx né sous `about:`/`data:` — la relative n'a nulle part
/// où aller), `blocked` si elle résout puis est refusée
/// (mixed/CSP/méthode/mode same-origin).
pub(crate) fn net_drop_kind(base: &Url, url: &str) -> &'static str {
    if Url::resolve(base, url.trim()).is_err() {
        "badr"
    } else {
        "blocked"
    }
}
/// Borne de passes `after_script` déroulées par entrée
/// (§B-111 — vol 695) : une tempête de livraisons qui
/// re-marque à chaque passe cède la main sous verrou ; les
/// onglets restants dans `after_script_again` sont repris à
/// la prochaine entrée (la file persiste, rien n'est perdu).
const AFTER_SCRIPT_MAX_PASSES: usize = 64;

/// Variante d'un `NetError` en nom de sonde — la cause
/// transport est nommée sans fuiter les détails (cible,
/// source io) dans le journal.
fn net_err_kind(e: &explorer_net::error::NetError) -> &'static str {
    use explorer_net::error::NetError as E;
    match e {
        E::Dns { .. } => "dns",
        E::Connect { .. } => "connect",
        E::Tls { .. } => "tls",
        E::Timeout { .. } => "timeout",
        E::UnsupportedScheme(_) => "scheme",
        E::InvalidResponse(_) => "bad_response",
        E::HeadTooLarge(_) => "head_large",
        E::BodyTooLarge(_) => "body_large",
        E::TruncatedBody { .. } => "truncated",
        E::InvalidChunked(_) => "chunked",
        E::TooManyRedirects(_) => "redirects",
        E::BlockedMixedContent(_) => "mixed",
        E::CorsBlocked(_) => "cors_preflight",
        E::DecodedTooLarge(_) => "decoded_large",
        E::ContentDecode(_) => "decode",
        E::Http2(_) => "h2",
        E::Io(_) => "io",
    }
}

/// Sonde §B-108 — nomme le refus d'une livraison dyn-img
/// (contexte mort / pas de source résoluble / mixed-content
/// ou CSP `img-src`). `enabled!` = coût nul désarmé.
fn img_dskip(reason: &'static str) {
    if tracing::enabled!(target: "media_probe", tracing::Level::INFO) {
        tracing::info!(target: "media_probe", site = "img.dskip", reason);
    }
}

/// Source retenue d'un `DynImgReq` — `srcset` résolu par
/// `sizes`/dpr prime sur `src` (spec), même pile que
/// `collect_images`.
fn dyn_img_src_key(d: &explorer_js::DynImgReq, img_w: f32, dpr: f32) -> Option<String> {
    d.srcset
        .as_deref()
        .and_then(|ss| {
            let eff_w = resolve_sizes(d.sizes.as_deref(), img_w);
            pick_srcset(ss, eff_w, dpr)
        })
        .or_else(|| {
            d.src
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
}

/// Délai de retenue §B-60 pour un streak donné —
/// `250 ms → 10 s` en doublement plafonné.
pub(crate) fn fetch_backoff_delay(streak: u32) -> std::time::Duration {
    let shift = streak.saturating_sub(FETCH_BACKOFF_START).min(16);
    FETCH_BACKOFF_BASE
        .checked_mul(1u32 << shift)
        .unwrap_or(FETCH_BACKOFF_CAP)
        .min(FETCH_BACKOFF_CAP)
}

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
            arrival_classic,
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
            self.run_commit_scripts(
                tab,
                eng.as_mut(),
                &scripts,
                &script_srcs,
                arrival_classic,
                arrival_evals,
                post_evals,
                &import_maps,
            );
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

    /// Bras JS du commit (`sub_commit_head`) : realms des
    /// cadres, import maps, files d'éval (parse → arrival →
    /// post), `run_pending`, resynchro du miroir et compteur
    /// T2 `evals_sans_mutation`. Extrait pour la borne
    /// clippy — pas de changement de sémantique.
    #[allow(clippy::too_many_arguments)]
    fn run_commit_scripts(
        &mut self,
        tab: TabId,
        eng: &mut dyn JsEngine,
        scripts: &[ScriptItem],
        script_srcs: &[Option<String>],
        arrival_classic: Vec<(usize, Option<Url>, String)>,
        arrival_evals: Vec<ModEval>,
        post_evals: Vec<ModEval>,
        import_maps: &[String],
    ) {
        // Étape CMP-4 : scripts réarmés depuis les marqueurs
        // « bloqués par consentement » — compteur exposé au
        // diagnostic §B-4 : un boot muet avec `cmp_rearmed`
        // >0 = mur franchi mais mort ailleurs ; 0 = le
        // consentement n'a jamais été le problème (ou
        // `Honest`, où rien ne réarme).
        let cmp_rearmed = scripts.iter().filter(|i| i.rearmed).count();
        if cmp_rearmed > 0 {
            tracing::info!(tab, cmp_rearmed, "scripts CMP réarmés au commit");
        }
        if let Some(e) = self.tabs.get_mut(&tab) {
            e.cmp_rearmed = u32::try_from(cmp_rearmed).unwrap_or(u32::MAX);
        }
        // Étape 134 : un realm JS par `<iframe>` fetché —
        // créé avant les scripts du parent pour qu'un
        // `frames[n]`/`contentWindow` lu en cours de
        // parse résolve un vrai contexte.
        self.wire_frame_contexts(tab, eng);
        // Issue #6 : les import maps arment la résolution des
        // bare specifiers — la spec les veut enregistrées avant
        // le premier module évalué (le prefetch, lui, a déjà
        // réécrit les spécificateurs via `import_specs`).
        for json in import_maps {
            eng.set_import_map(json);
        }
        let mut evals = 0usize;
        for item in scripts.iter().filter(|i| i.exec == ScriptExec::Parse) {
            let Some(src) = item_source(item, script_srcs) else {
                continue;
            };
            let url = item.src.as_ref().map(Url::as_str);
            self.eval_or_report(tab, eng, src, url.as_deref(), Some(item.el_ord));
            evals += 1;
        }
        // §B-119 — scripts classiques `async` : évalués en
        // ordre d'arrivée **après** les scripts parser-inserted
        // (`Parse` — ordre Chrome : les inlines tournent à leur
        // position de parse, les asyncs ensuite). Un `async`
        // évalué avant un inline qui le suit faisait voir à
        // `document.currentScript` l'inline sous le push-handler
        // → `new URL(currentScript.src)` → `Invalid URL:`
        // (Next.js `getAssetPrefix`, vols 721/733/734).
        for (ord, url, src) in arrival_classic {
            let url = url.as_ref().map(Url::as_str);
            self.eval_or_report(tab, eng, &src, url.as_deref(), Some(ord));
            evals += 1;
        }
        for me in arrival_evals {
            self.eval_module_graph(tab, eng, &me);
            evals += 1;
        }
        let mut post_evals = post_evals.into_iter();
        for item in scripts.iter().filter(|i| i.exec == ScriptExec::Post) {
            if item.module {
                if let Some(me) = post_evals.next() {
                    self.eval_module_graph(tab, eng, &me);
                    evals += 1;
                }
            } else if let Some(src) = item_source(item, script_srcs) {
                let url = item.src.as_ref().map(Url::as_str);
                self.eval_or_report(tab, eng, src, url.as_deref(), Some(item.el_ord));
                evals += 1;
            }
        }
        // §B-111 — paire `net:` (morgue C406) : la ligne
        // d'entrée sans son `ret` nomme le drain fautif.
        dlv_info!("net: scripts");
        let n = eng.run_pending();
        dlv_info!("net: scripts ret n={n}");
        // Miroir muté ? → re-parse : le relayout qui suit
        // rend les modifications (dirty bit consommé côté
        // moteur).
        let mutated = if let Some(k) = eng.sync_dom_keyed() {
            self.apply_dom_sync(tab, &k.html, None, false, Some(k.ids), Some(k.tags));
            true
        } else if let Some((mutated, _)) = eng.sync_dom_path() {
            self.apply_dom_sync(tab, &mutated, None, false, None, None);
            true
        } else {
            false
        };
        // T2 `evals_sans_mutation` (B-4) : ≥8 évals exécutées
        // sans aucune mutation DOM = signature « boot muet »
        // (twitch : 88 ok=true, 0 mutation, shell vide — un
        // lecteur INFO/WARN concluait « tout vert »). Une
        // page à scripts purement passifs reste rare au-
        // delà du seuil.
        if !mutated && evals >= 8 {
            // §B-4(c) : le gap `promises` nomme la thèse —
            // `await` gelé = promesse créée jamais réglée,
            // invisible aux erreurs ; `pending=0` = boot
            // muet *sans* promesse en attente (retour avalé
            // synchrone — autre famille).
            let (promises_created, promises_settled) = eng.promise_stats();
            // §B-4(c) bis — discriminant « pourquoi muet » :
            // `timers_pending`/`microtasks_pending` > 0 = la
            // continuation est garée sur un job jamais drainé
            // (pompe) ; tout à 0 + `ready_state` = `complete`
            // = retour avalé synchrone (gate de feature-
            // detect — autre famille à nommer par bundle).
            let (timers_pending, microtasks_pending, ready_state) = eng.boot_diag();
            tracing::warn!(
                tab,
                evals,
                promises_created,
                promises_settled,
                timers_pending,
                microtasks_pending,
                ready_state,
                cmp_rearmed,
                "evals_sans_mutation — boot possiblement muet (aucune mutation DOM)"
            );
            // Le tir voit le commit en cours — `ready_state`
            // `loading` est attendu ici ; `sub_commit_tail`
            // ré-échantillonne post-`load` pour trancher
            // « cycle de vie jamais livré » vs « échantillon
            // trop tôt ».
            if let Some(e) = self.tabs.get_mut(&tab) {
                e.boot_mute_warned = true;
            }
        }
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
        // §B-4(c) T2-bis : le WARN `evals_sans_mutation` a
        // tiré pendant le commit — ré-échantillonne le
        // diagnostic **après** le cycle de vie : un
        // `ready_state` toujours `loading` ici nomme le bras
        // « cycle jamais livré » ; `complete` confirme le tir
        // mid-commit (famille « retour avalé synchrone » si
        // les compteurs restent à 0).
        let warned = self.tabs.get(&tab).is_some_and(|e| e.boot_mute_warned);
        if warned {
            if let Some(entry) = self.tabs.get_mut(&tab) {
                entry.boot_mute_warned = false;
                let cmp_rearmed = entry.cmp_rearmed;
                if let Some(eng) = entry.js.as_mut() {
                    let (timers_pending, microtasks_pending, ready_state) = eng.boot_diag();
                    tracing::warn!(
                        tab,
                        timers_pending,
                        microtasks_pending,
                        ready_state,
                        cmp_rearmed,
                        "evals_sans_mutation — ré-échantillon post-load (§B-4(c))"
                    );
                }
            }
        }
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
    pub(crate) fn dispatch_net_requests(
        &mut self,
        tab: TabId,
        page: &Url,
        base: &Url,
        reqs: Vec<explorer_js::NetRequest>,
    ) {
        let mut failed = Vec::new();
        self.dispatch_net_reqs_inner(tab, page, base, reqs, &mut failed);
        // Échecs immédiats (ctx mort/bloquée/file quota
        // saturée) : livrés comme un échec réseau —
        // déclenche `error`, jamais de panic. Le dépassement
        // de quota simple n'en fait plus partie (JS-C367 :
        // le fetch attend un slot dans `pending_script_net`).
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
                    // §B-60 — backoff sur tempête de retries :
                    // une URL qui échoue `status: 0` en boucle
                    // voit son émission différée (`fetch_held`
                    // drainée à échéance par `poll_net`) au
                    // lieu de frapper le transport — le rejet
                    // n'est pas accéléré, la promesse attend.
                    let streak = self
                        .fetch_fail_streak
                        .get(&(tab, url.clone()))
                        .copied()
                        .unwrap_or(0);
                    if streak >= FETCH_BACKOFF_START && self.fetch_held.len() < MAX_HELD_FETCHES {
                        self.fetch_held.push_back(net_items::HeldFetch {
                            at: std::time::Instant::now() + fetch_backoff_delay(streak),
                            tab,
                            page: page.clone(),
                            base: base.clone(),
                            handle,
                            method,
                            url,
                            headers,
                            body,
                            mode,
                            credentials,
                            ctx,
                        });
                        continue;
                    }
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
                    // Une retenue backoff annulée ne part
                    // jamais (§B-60) — `xhr.abort()` doit la
                    // trouver hors de `net_tasks`.
                    self.fetch_held
                        .retain(|h| !(h.tab == tab && h.handle == handle));
                    // Même purge pour un fetch garé au quota
                    // (JS-C367) — annulé avant d'avoir pu
                    // partir, jamais émis.
                    self.pending_script_net
                        .retain(|p| !(p.tab == tab && p.handle == handle));
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
                                explorer_js::KeyAlgo::AesGcm => nc::KeyAlgo::AesGcm,
                                explorer_js::KeyAlgo::RsaPkcs1 { hash } => {
                                    nc::KeyAlgo::RsaPkcs1(digest_of(hash))
                                }
                                explorer_js::KeyAlgo::RsaOaep { hash } => {
                                    nc::KeyAlgo::RsaOaep(digest_of(hash))
                                }
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
                                    explorer_js::ImportFormat::Jwk => nc::KeyFormat::Jwk,
                                };
                                nc::import_key(key_of(algo), f, &data)
                            }
                            explorer_js::CryptoOp::Sign { algo } => {
                                nc::sign(key_of(algo), &key, &data)
                            }
                            explorer_js::CryptoOp::Verify { algo, sig } => {
                                nc::verify(key_of(algo), &key, &sig, &data)
                            }
                            // AES-GCM (JS-C160) — nonce/aad dans
                            // l'op, `chiffré‖tag` dans `data`.
                            // RSA-OAEP (JS-C231) — `iv` inutilisé,
                            // `aad` = `label` du padding.
                            explorer_js::CryptoOp::Encrypt { algo, iv, aad } => match algo {
                                explorer_js::KeyAlgo::RsaOaep { hash } => {
                                    nc::rsa_oaep_seal(digest_of(hash), &key, &aad, &data)
                                }
                                _ => nc::aead_seal(key_of(algo), &key, &iv, &aad, &data),
                            },
                            explorer_js::CryptoOp::Decrypt { algo, iv, aad } => match algo {
                                explorer_js::KeyAlgo::RsaOaep { hash } => {
                                    nc::rsa_oaep_open(digest_of(hash), &key, &aad, &data)
                                }
                                _ => nc::aead_open(key_of(algo), &key, &iv, &aad, &data),
                            },
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
                // `fetch`/`XHR` du thread worker (§B-85) — même
                // modèle one-shot que `WorkerLoad`, mais pile
                // `script_request` complète : méthode/en-têtes/
                // corps/CORS/CSP `connect-src` ; la réponse
                // binaire repart par `reply` vers la pompe du
                // worker (qui la livre au miroir, jamais
                // ré-entrant).
                NetRequest::WorkerFetch { req, ctx, reply } => {
                    self.spawn_worker_fetch(&WorkerFetchArgs {
                        tab,
                        page,
                        base,
                        req: &req,
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

    /// `NetRequest::WorkerFetch` (§B-85) — `fetch`/`XHR` émis
    /// dans un thread worker : même chemin que les requêtes
    /// script (`script_request` : méthode/en-têtes/corps,
    /// `connect-src`, CORS, `data:` via `fetch_maybe_data`)
    /// mais la réponse **complète** (statut, en-têtes, octets
    /// bruts) repart sur le canal `reply` — le transmuxer vit
    /// dans le worker, ses segments ne quittent jamais le
    /// domaine binaire. `referrer` = URL du script worker
    /// (base de résolution et `Referer`), contexte hérité de
    /// la page créatrice.
    fn spawn_worker_fetch(&mut self, a: &WorkerFetchArgs<'_>) {
        let Some((req_page, req_base, policy, csp, opaque)) =
            self.ctx_net_origin(a.tab, a.ctx, a.page, a.base)
        else {
            let _ = a.reply.0.send(Err("pas d'origine de contexte".to_string()));
            return;
        };
        // La résolution relative part de l'URL du script worker
        // (spec `fetch` dans `WorkerGlobalScope`), repli base doc.
        let wbase = Url::parse(&a.req.referrer).ok().unwrap_or(req_base);
        let Some((req, _cross)) = self.script_request(
            &req_page,
            &wbase,
            &a.req.method,
            &a.req.url,
            &a.req.headers,
            a.req.body.as_deref(),
            policy,
            &csp,
            opaque,
            a.req.mode,
            a.req.credentials,
        ) else {
            let _ = a.reply.0.send(Err("résolution/CSP refusée".to_string()));
            return;
        };
        let tr = self.transport.clone();
        let reply = a.reply.clone();
        tokio::spawn(async move {
            let out = fetch_maybe_data(&tr, &req)
                .await
                .map(|r| explorer_js::WorkerNetResponse {
                    status: r.status,
                    status_text: r.reason,
                    headers: r.headers,
                    body: r.body,
                })
                .map_err(|e| e.to_string());
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
        // `unhandledrejection` (JS-C154) — les rejets JS non
        // gérés étaient des morts silencieuses (pinterest,
        // airbnb) : le moteur tire l'event au checkpoint, la
        // session verse la description au ring console.
        // Via `emit_pipeline_error` (Étape 153) — l'envoi
        // direct contournait le cap `heap_dead_reported`
        // (§B-12 vol 97 : les erreurs heap qui rejettent des
        // promesses — `unhandledrejection — heap memory
        // limit exceeded` — repassaient ×3 sur bookshop).
        for msg in self
            .tabs
            .get_mut(&tab)
            .and_then(|e| e.js.as_mut())
            .map_or_else(Vec::new, |eng| eng.drain_unhandled_rejections())
        {
            self.emit_pipeline_error(tab, msg);
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
        let mut ordered_ctxs = Vec::new();
        for d in dyns {
            // Étape 207-bis (mur II.4-bis vol 647) : une balise
            // **parse-marquée** (`defer_order`) attend son tour
            // en file par ctx — l'ordre document du sous-
            // document prime `force-async` : l'inline `writeEmbed()`
            // de l'embed `YouTube` ne tire qu'après la livraison
            // de `ytembeds.base`. Les insertions dynamiques
            // (`defer_order = false`) gardent le tir immédiat.
            if d.defer_order {
                ordered_ctxs.push(d.ctx);
                self.ordered_dyn
                    .entry((tab, d.ctx))
                    .or_default()
                    .queue
                    .push_back(d);
            } else {
                self.spawn_dyn_script(tab, page, base, &d);
            }
        }
        ordered_ctxs.dedup();
        for ctx in ordered_ctxs {
            self.pump_ordered_dyn(tab, ctx, page, base);
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
        // Étape 207-bis — settle : les contextes de cadre
        // dont la file ordonnée a vidé (ou jamais existé)
        // tirent leur cycle `readyState`/`load` ici — DCL
        // n'avance qu'après les scripts parseur (spec).
        self.frame_ctx_load_events(tab);
    }

    /// `import()` dynamique (JS-C100) — l'URL est déjà résolue
    /// par le chargeur hôte ; fetch `script-src` + mixed-content
    /// puis `deliver_dyn_import` au moteur (instancier+évaluer+
    /// résoudre la promesse pending).
    #[allow(clippy::too_many_lines)] // spawn + tâche fetch/graphe inline, chaque bras documenté
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
                        &[],
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
                url: url_g.to_string(),
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
                cors_unsafe: req
                    .cors
                    .as_ref()
                    .map(|c| c.unsafe_headers.clone())
                    .unwrap_or_default(),
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
    /// Échec de fetch (`entry: None`) → warn + `PipelineError`
    /// avec status/URL : le rejet côté moteur est un
    /// « cannot load module » générique — la console doit
    /// dire *où* la chaîne a cassé (vol letterboxd).
    fn commit_dyn_import(
        &mut self,
        tab: TabId,
        meta: &crate::session::net_items::DynImportMeta,
        out: &NetResult<Response>,
    ) {
        if meta.entry.is_none() {
            let why = match out {
                Ok(r) => format!("statut {}", r.status),
                Err(e) => e.to_string(),
            };
            tracing::warn!(tab, url = %meta.url, seq = meta.seq, error = %why,
                "dyn import : fetch échoué — rejet livré");
            let _ = self.events.send(CoreEvent::PipelineError {
                tab: Some(tab),
                message: format!("import() — fetch échoué : {} ({why})", meta.url),
            });
        }
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
        // Sandbox `!allow-scripts` (CAP-149, Étape 207-bis) :
        // le miroir enfile tout le flux parse du cadre — les
        // balises d'un cadre à scripting désactivé ne doivent
        // ni fetch ni évaluer (`error` honnête, comme un refus
        // CSP — insertion dynamique couverte au passage).
        let scripts_ok = self
            .tabs
            .get(&tab)
            .and_then(|e| {
                e.frame_ctxs
                    .iter()
                    .find(|f| f.ctx == d.ctx)
                    .and_then(|f| e.frames.get(f.sub))
            })
            .is_none_or(|sf| sf.sandbox.is_none_or(|s| s.scripts));
        if !scripts_ok {
            self.deliver_dyn_script(tab, d.ctx, d.seq, None);
            return;
        }
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

    /// Relais de livraison vers [`Self::pump_ordered_dyn`] —
    /// le `src` en vol **vient d'être livré** : `in_flight`
    /// retombe avant la pompe, sinon la file gelerait à
    /// jamais (un seul item en vol par ctx — le flag tombé
    /// est celui de la séquence livrée). `page`/`base`
    /// dérivés de l'onglet (les balises ordonnées vivent dans
    /// un ctx enfant : `ctx_net_origin` résout l'origine **du
    /// cadre** et n'atteint jamais le repli top-level que
    /// `page`/`base` n'alimentent que pour `JS_CTX_TOP`).
    fn pump_ordered_dyn_tail(&mut self, tab: TabId, ctx: explorer_js::JsCtxId) {
        if let Some(q) = self.ordered_dyn.get_mut(&(tab, ctx)) {
            q.in_flight = false;
        }
        let Some((page, base)) = self.tabs.get(&tab).and_then(|e| {
            e.url
                .clone()
                .zip(e.nav_base().cloned().or_else(|| e.url.clone()))
        }) else {
            return;
        };
        self.pump_ordered_dyn(tab, ctx, &page, &base);
    }

    /// Relâche la file ordonnée d'un contexte (Étape 207-bis,
    /// mur II.4-bis) : tant qu'aucun `src` n'est en vol, la
    /// tête part — texte livré en ligne, `src` fetché puis
    /// `in_flight` armé jusqu'à `deliver_dyn_done`. Un `src`
    /// **refusé en ligne** (résolution/mixed-content/CSP ou
    /// ctx mort — `deliver_dyn_script(None)` synchrone, pas
    /// de `dyn_tasks`) ne doit pas bloquer la file : son
    /// `error` a déjà couru, on enchaîne. Appelée après
    /// l'enfile ([`Self::spawn_dyn_subresources`]) et après
    /// chaque livraison ordonnée.
    fn pump_ordered_dyn(&mut self, tab: TabId, ctx: explorer_js::JsCtxId, page: &Url, base: &Url) {
        loop {
            let next = {
                let Some(q) = self.ordered_dyn.get_mut(&(tab, ctx)) else {
                    return;
                };
                if q.in_flight {
                    return;
                }
                let Some(d) = q.queue.pop_front() else {
                    self.ordered_dyn.remove(&(tab, ctx));
                    // File vidée : le cycle `readyState`/`load`
                    // du cadre part (différé depuis le câblage
                    // — spec : DCL après les scripts parseur).
                    self.frame_ctx_load_events(tab);
                    return;
                };
                q.in_flight = d.src.is_some();
                d
            };
            let seq = next.seq;
            self.spawn_dyn_script(tab, page, base, &next);
            if next.src.is_some() && !self.dyn_tasks.contains_key(&(tab, seq)) {
                // Refus synchrone — `error` déjà livré :
                // l'in-flight tombe, la balise suivante part.
                if let Some(q) = self.ordered_dyn.get_mut(&(tab, ctx)) {
                    q.in_flight = false;
                }
            }
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
            let out = fetch_maybe_data(&tr, &req).await;
            let _ = tx.send(NetDone {
                tab,
                handle: seq,
                cross: is_cross,
                mode: fetch_mode,
                credentials,
                page_origin,
                cors_unsafe: req
                    .cors
                    .as_ref()
                    .map(|c| c.unsafe_headers.clone())
                    .unwrap_or_default(),
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
                cors_unsafe: Vec::new(),
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
        dlv_info!("net: module");
        let n = eng.run_pending();
        dlv_info!("net: module ret n={n}");
        let mutated = Self::sync_for_apply(eng.as_mut());
        let mut eng = Some(eng);
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.js = eng.take();
        }
        // Onglet mort pendant l'éval : le moteur est orphelin
        // — destruction sur fil dédié, jamais droppé sous le
        // verrou (§B-30).
        if let Some(eng) = eng {
            self.destroy_engine_offlock(eng);
        }
        if let Some(s) = mutated {
            self.queue_apply_sync(tab, s);
        }
        self.after_script(tab);
        // Étape 207-bis : un module `src` ordonné vient d'être
        // évalué — la balise parse suivante du ctx part.
        self.pump_ordered_dyn_tail(tab, ctx);
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
        // §B-1 (vol 221 : la rafale d'évals lentes passait
        // ici — livraisons dyn, hors des deux sites couverts
        // par Étape 161). Livrer `error` plutôt qu'évaluer :
        // le script saute, le tour moteur continue. Un corps
        // ≤ `SLOW_EVAL_PROBE_BYTES` peut sonder une fois par
        // fenêtre (demi-ouvert, C397) ; `None` (fetch
        // échoué — pas une éval) reste une erreur livrée.
        let bytes = body.as_ref().map_or(usize::MAX, String::len);
        if self.slow_eval_skip(tab, bytes) {
            let entry = self.tabs.get_mut(&tab);
            if let Some(eng) = entry.and_then(|e| e.js.as_mut()) {
                eng.deliver_dyn_script(ctx, seq, None);
            }
            return;
        }
        let (mutated, eval_ms) = {
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
            let t = std::time::Instant::now();
            eng.deliver_dyn_script(ctx, seq, body);
            let eval_ms = t.elapsed().as_millis();
            dlv_info!("net: script");
            let n = eng.run_pending();
            dlv_info!("net: script ret n={n}");
            (Self::sync_for_apply(eng.as_mut()), eval_ms)
        };
        self.note_eval_ms(tab, eval_ms);
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
            let out = fetch_maybe_data(&tr, &req).await;
            let _ = tx.send(NetDone {
                tab,
                handle: seq,
                cross: is_cross,
                mode: req.fetch_mode,
                credentials: explorer_net::http::FetchCredentials::SameOrigin,
                page_origin,
                cors_unsafe: req
                    .cors
                    .as_ref()
                    .map(|c| c.unsafe_headers.clone())
                    .unwrap_or_default(),
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
            img_dskip("ctx");
            self.deliver_dyn_img_el(tab, d.ctx, d.seq, false);
            return;
        };
        // Choix de la source — `srcset` prime sur `src`
        // (spec) ; la clé est la chaîne telle que
        // `collect_images` la relira au prochain apply.
        let Some(src_key) = dyn_img_src_key(d, self.viewport.0, self.scale) else {
            img_dskip("nosrc");
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
            img_dskip("blocked");
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
            let out = fetch_maybe_data(&tr, &req).await;
            let _ = tx.send(NetDone {
                tab,
                handle: seq,
                cross: is_cross,
                mode: fetch_mode,
                credentials,
                page_origin,
                cors_unsafe: req
                    .cors
                    .as_ref()
                    .map(|c| c.unsafe_headers.clone())
                    .unwrap_or_default(),
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
            retain_img_body(&mut entry.img_bodies, &url.as_str(), body);
            // Budget octets global sur les bitmaps retenus
            // (§B-59) — au-delà, le bitmap n'entre pas en cache
            // (repli `alt` peint) ; le corps compressé reste
            // disponible pour un re-décodage ultérieur.
            let held = decoded_cache_bytes(&entry.img_decoded, &entry.bg_decoded);
            if held.saturating_add(img.rgba.len()) <= self.image_max_bytes {
                entry.img_decoded.insert(src_key, img.clone());
                entry.bg_decoded.insert(url.as_str(), Some(img));
            }
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
            let out = fetch_maybe_data(&tr, &req).await;
            let _ = tx.send(NetDone {
                tab,
                handle: seq,
                cross: is_cross,
                mode: fetch_mode,
                credentials,
                page_origin,
                cors_unsafe: req
                    .cors
                    .as_ref()
                    .map(|c| c.unsafe_headers.clone())
                    .unwrap_or_default(),
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
            dlv_info!("net: link");
            let n = eng.run_pending();
            dlv_info!("net: link ret n={n}");
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
            dlv_info!("net: img");
            let n = eng.run_pending();
            dlv_info!("net: img ret n={n}");
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
        // Phase 1 : `SubFrame` rangé, realm créé —
        // `frame_ord` relie l'élément pour `contentWindow`.
        let Some((sub, ctx)) = self.prepare_dyn_frame(tab, meta, sf, &url) else {
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
        // Phase 3 : miroir installé, « loading » posé — le
        // cycle `readyState` complet (interactive → DCL →
        // complete → `load`) puis le `load` de l'élément sont
        // **différés** au settle : `after_script` →
        // `drain_script_net` enfile/pompe le flux parse
        // ordonné, `frame_ctx_load_events` tire le cycle à
        // vidange et livre `el_dyn` (spec : DCL/`load` après
        // les scripts parseur — Étape 207-bis).
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        let Some(eng) = entry.js.as_mut() else {
            return;
        };
        eng.install_dom_ctx(ctx, &html, &url.to_string(), local, &cookie);
        // « loading » retient les `postMessage` entrants
        // pendant les scripts du cadre dynamique (étape 130).
        eng.set_ready_state(ctx, "loading");
        entry.frame_ctxs.push(FrameCtx {
            ctx,
            sub,
            url,
            node: explorer_dom::NodeId::from_parts(u32::MAX, u32::MAX),
            opaque,
            load_fired: false,
            el_dyn: Some((meta.ctx, meta.seq)),
        });
        self.after_script(tab);
    }

    /// Phase 1 de [`Self::install_dyn_frame_ctx`] (CAP-171) :
    /// pousse le `SubFrame` et crée le realm **sous le ctx
    /// émetteur** (`context_new_in` — `frame_ord` résolu dans
    /// son miroir). `(index frames, ctx)` — `None` = realm
    /// indisponible (le `SubFrame` reste rendu). Les scripts
    /// du sous-document ne sont plus collectés ici : depuis
    /// l'Étape 207-bis `install_dom_ctx` enfile tout le flux
    /// parse dans la file ordonnée dyn-script (II.4-bis) ;
    /// sandbox/CSP restent filtrés à `spawn_dyn_script`.
    fn prepare_dyn_frame(
        &mut self,
        tab: TabId,
        meta: &DynFrameMeta,
        sf: SubFrame,
        url: &Url,
    ) -> Option<(usize, explorer_js::JsCtxId)> {
        let entry = self.tabs.get_mut(&tab)?;
        entry.frames.push(sf);
        let sub = entry.frames.len() - 1;
        let ctx = entry
            .js
            .as_mut()
            .and_then(|eng| eng.context_new_in(meta.ctx, &url.to_string(), Some(meta.ord)))?;
        Some((sub, ctx))
    }

    /// `load`/`error` sur l'élément `<iframe>` miroir
    /// (CAP-171) — relais mince de `deliver_dyn_frame` avec
    /// resynchro (la livraison peut muter le DOM du ctx).
    pub(crate) fn deliver_dyn_frame_el(
        &mut self,
        tab: TabId,
        ctx: explorer_js::JsCtxId,
        seq: u64,
        ok: bool,
    ) {
        let mutated = {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return;
            };
            let Some(eng) = entry.js.as_mut() else {
                return;
            };
            eng.deliver_dyn_frame(ctx, seq, ok);
            dlv_info!("net: frame");
            let n = eng.run_pending();
            dlv_info!("net: frame ret n={n}");
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
        body: Option<&[u8]>,
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
            self.note_net_drop("ctx_dead", url);
            return Err(());
        };
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
            // JS-C415 — discrimine le motif : `badr` = l'URL
            // brute n'a pas résolu contre `req_base` (le mur
            // réel « host='' » — base de ctx opaque/vide :
            // une relative n'a nulle part où aller) ;
            // `blocked` = résolue puis refusée (mixed/CSP/
            // méthode/mode same-origin).
            let kind = net_drop_kind(&req_base, url);
            if kind == "badr" {
                self.note_net_badr(&req_base, url, ctx);
            }
            self.note_net_drop(kind, url);
            return Err(());
        };
        // JS-C415 — `net.host` : l'hôte **résolu** (dedup par
        // host) — discrimine l'artefact `fetch.send host=""`
        // (sonde moteur sur l'URL **brute** pré-résolution :
        // toute relative logue "") d'une vraie résolution
        // manquée — celle-ci apparaît en `net.badr`, jamais ici.
        if tracing::enabled!(target: "media_probe", tracing::Level::INFO) {
            let rhost = req.url.host();
            if self.note_net_probe("host.res", &rhost) {
                tracing::info!(target: "media_probe", site = "net.host", host = %rhost, cross, ctx);
            }
        }
        // Origine opaque : `null` (sérialisation spec) — ACAO
        // `*` reste admis, un `ACAO: <origine>` ne l'est pas.
        let page_origin = if opaque {
            "null".to_string()
        } else {
            origin_header_value(&req_page)
        };
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
        let p = net_items::PendingFetch {
            tab,
            handle,
            partition: explorer_net::cookies::request_partition(&req),
            mode: req.fetch_mode,
            req,
            cross,
            page_origin,
            credentials,
        };
        // JS-C367 — le quota borne les tâches **vivantes** :
        // l'excédent attend un slot dans la file (FIFO par
        // onglet) plutôt qu'un `status: 0` sec — la rafale
        // `FontFace.load()` de kevlar (~200 fetch/16 slots)
        // tuait le `Promise.all` de font-gating (vol 668,
        // §B-97). `pending.any(tab)` = un fetch frais ne
        // double jamais un garé de son onglet (FIFO strict).
        let inflight = self.net_tasks.keys().filter(|(t, _)| *t == tab).count();
        if inflight >= self.max_script_net || self.pending_script_net.iter().any(|q| q.tab == tab) {
            // §B-122(e) — sonde `net.pend` : un fetch garé au
            // quota/FIFO n'émet jamais de `NetDone` tant qu'il
            // ne spawn pas — silencieux sans cette trace
            // (dedup par hôte, vol 753 : deux GET omega
            // invisibles pendant que le 3e répondait).
            if tracing::enabled!(target: "media_probe", tracing::Level::INFO) {
                let h = p.req.url.host().clone();
                if self.note_net_probe("pend", &h) {
                    tracing::info!(target: "media_probe", site = "net.pend", host = %h, inflight, ctx);
                }
            }
            if self.pending_script_net.len() >= MAX_PENDING_FETCHES {
                tracing::trace!(tab, handle, "file requêtes script saturée");
                self.note_net_drop("pend_full", url);
                return Err(());
            }
            self.pending_script_net.push_back(p);
            return Ok(());
        }
        self.launch_pending(p);
        Ok(())
    }

    /// Émission effective d'un [`net_items::PendingFetch`]
    /// (JS-C367) — la tâche tokio est créée et rangée dans
    /// `net_tasks`. Partagée par `spawn_script_fetch` (slot
    /// libre à l'émission) et `pump_pending_script_net`
    /// (slot libéré par un `NetDone`).
    fn launch_pending(&mut self, p: net_items::PendingFetch) {
        let tr = self.transport.clone();
        let tx = self.net_tx.clone();
        let req_url = p.req.url.clone();
        let (tab, handle) = (p.tab, p.handle);
        let cors_unsafe = p
            .req
            .cors
            .as_ref()
            .map(|c| c.unsafe_headers.clone())
            .unwrap_or_default();
        // §B-122(e) — `net.go` : le fetch quitte la file/
        // l'émission et devient une tâche vivante — borne le
        // silencieux « jamais spawné » face à « jamais revenu ».
        if tracing::enabled!(target: "media_probe", tracing::Level::INFO) {
            let h = req_url.host().clone();
            let key = format!("{h}|{handle}");
            if self.note_net_probe("go", &key) {
                tracing::info!(target: "media_probe", site = "net.go", host = %h, handle);
            }
        }
        let j = tokio::spawn(async move {
            let out = fetch_maybe_data(&tr, &p.req).await;
            let _ = tx.send(NetDone {
                tab,
                handle,
                cross: p.cross,
                mode: p.mode,
                credentials: p.credentials,
                page_origin: p.page_origin,
                cors_unsafe,
                req_url,
                partition: p.partition,
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
    }

    /// Pompe la file `pending_script_net` (JS-C367) — chaque
    /// `NetDone`/`Abort`/fin de flux a pu libérer un slot :
    /// les requêtes garées partent en FIFO par onglet (une
    /// entrée d'un onglet saturé ne bloque pas les autres).
    fn pump_pending_script_net(&mut self) {
        if self.pending_script_net.is_empty() {
            return;
        }
        let mut inflight: HashMap<TabId, usize> = HashMap::new();
        for (t, _) in self.net_tasks.keys() {
            *inflight.entry(*t).or_default() += 1;
        }
        let max = self.max_script_net;
        let mut still = std::collections::VecDeque::new();
        for p in std::mem::take(&mut self.pending_script_net) {
            let n = inflight.get(&p.tab).copied().unwrap_or(0);
            if n < max {
                inflight.insert(p.tab, n + 1);
                self.launch_pending(p);
            } else {
                still.push_back(p);
            }
        }
        self.pending_script_net = still;
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
            let mut errs = Vec::new();
            for k in kinds {
                if let Err(e) = eng.deliver_ws_event(ctx, handle, k) {
                    tracing::debug!(tab, error = %e, "livraison websocket js");
                    errs.push(e.to_string());
                }
            }
            dlv_info!("net: ws");
            let n = eng.run_pending();
            dlv_info!("net: ws ret n={n}");
            (Self::sync_for_apply(eng.as_mut()), errs)
        };
        let (mutated, errs) = mutated;
        for m in errs {
            // §B-12 vol 100 : les erreurs de livraison passent
            // par la porte (un tas mort rendait chaque
            // callback fautif en `PipelineError` direct).
            self.emit_pipeline_error(tab, m);
        }
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

    /// Sonde `net.drop` (`media_probe`, JS-C366) — requête
    /// script refusée **avant émission** : nomme la cause
    /// (`ctx_dead`/`quota`/`blocked`) + l'hôte ; dedup
    /// `(kind, host)` — le décompte reste côté `fetch.rej`
    /// moteur, un motif est logué une fois par sorte.
    /// Étape 209 / §B-114-b — garde **processus** RSS :
    /// au-delà de `rss_shed_mb` (config, sous les seuils de
    /// kill daemon `EXPLORER_RSS_LIMIT_MB`/`COMMIT_LIMIT_MB` —
    /// opt-in, absents au vol 697 d'où la mort nue), la
    /// session dégrade en gradins au lieu d'attendre le
    /// crash. Échantillonné à `RSS_SHED_SAMPLE` de cadence
    /// depuis `poll_net` et les entrées navigation.
    pub(crate) fn maybe_rss_shed(&mut self) {
        if self.rss_shed_mb == 0 || self.rss_shed_at.elapsed() < RSS_SHED_SAMPLE {
            return;
        }
        self.rss_shed_at = std::time::Instant::now();
        let Some(m) = memory_stats::memory_stats() else {
            return;
        };
        // Même métrique que le watchdog daemon (main.rs :
        // `physical_mem`) — les seuils sont comparables.
        self.shed_evaluate((m.physical_mem / (1024 * 1024)) as u64);
    }

    /// Transition d'état du shed — séparée de l'échantillon
    /// pour que les tests injectent `mb` directement
    /// (`maybe_rss_shed` lit le vrai RSS).
    pub(crate) fn shed_evaluate(&mut self, mb: u64) {
        if self.rss_shed_mb == 0 {
            return; // désarmé par config — jamais de transition
        }
        let over = if self.shed_active {
            mb + shed_margin(self.rss_shed_mb) > u64::from(self.rss_shed_mb)
        } else {
            mb >= u64::from(self.rss_shed_mb)
        };
        if over == self.shed_active {
            return;
        }
        self.shed_active = over;
        if !over {
            tracing::info!(mb, "rss_shed levé (sous marge)");
            return;
        }
        tracing::warn!(
            mb,
            limit = self.rss_shed_mb,
            "rss_shed : dégradation en gradins"
        );
        self.shed_apply();
    }

    /// `true` pendant un épisode shed — les navigations et
    /// onglets neufs sont refusés (`CoreError::Overloaded` →
    /// REST 503 + `Retry-After`).
    #[must_use]
    pub fn shed_active(&self) -> bool {
        self.shed_active
    }

    /// `Some(retry_s)` pendant l'épisode — le refus 503 honnête
    /// de la surface REST ; `None` hors épisode (ou shed
    /// désarmé — `rss_shed_mb == 0` n'arme jamais le drapeau).
    #[must_use]
    pub fn shed_refusal(&self) -> Option<u32> {
        self.shed_active.then_some(self.shed_retry_after_s)
    }

    /// Les gradins, dans l'ordre du moins douloureux :
    /// 1. `fetch_held` + `pending_script_net` → `status:0`
    ///    livrés — les promesses **rejettent proprement**,
    ///    jamais pendantes (un drop muet recréerait le mur
    ///    « resolve jamais invoqué » qu'on forensique
    ///    ailleurs) ;
    /// 2. `eng.gc()` demandé sur chaque onglet — **asynchrone**
    ///    (file de commandes moteur, pas un `collect`
    ///    synchrone) : le shed ne bloque pas la pompe ;
    /// 3. bitmaps décodés évincés — `img_decoded`/`bg_decoded`
    ///    seuls : les corps compressés `img_bodies` restent
    ///    re-décodables (repli `alt` peint, dégradation bornée).
    fn shed_apply(&mut self) {
        // Collecte d'abord : `deliver_net_response` re-emprunte
        // `self` pour le tour post-script.
        let held: Vec<_> = self
            .fetch_held
            .drain(..)
            .map(|h| (h.tab, h.handle))
            .collect();
        let pend: Vec<_> = self
            .pending_script_net
            .drain(..)
            .map(|p| (p.tab, p.handle))
            .collect();
        let dropped = held.len() + pend.len();
        for (tab, handle) in held.into_iter().chain(pend) {
            self.shed_fail(tab, handle);
        }
        for e in self.tabs.values_mut() {
            if let Some(eng) = e.js.as_mut() {
                eng.gc();
            }
            e.img_decoded.clear();
            e.bg_decoded.clear();
        }
        tracing::warn!(
            dropped,
            "rss_shed : fetchs basse priorité abandonnés (status:0), GC demandé, bitmaps évincés"
        );
    }

    /// `status:0` livré à un fetch droppé par le shed — la
    /// promesse moteur rejette au lieu d'attendre à l'infini.
    fn shed_fail(&mut self, tab: TabId, handle: u64) {
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

    /// Sonde `net.badr` (JS-C415) — la résolution
    /// `Url::resolve` elle-même a échoué : logue la **base**
    /// du ctx (suspect n°1 du mur « host='' » — ctx né sous
    /// `about:`/`data:`/base vide : une relative n'a nulle
    /// part où aller) + extrait brut + `ctx`. Dedup par
    /// (`badr`, base) — une ligne par population fautive.
    fn note_net_badr(&mut self, base: &Url, url: &str, ctx: explorer_js::JsCtxId) {
        if !tracing::enabled!(target: "media_probe", tracing::Level::INFO) {
            return;
        }
        let b = base.as_str();
        if self.note_net_probe("badr", &b) {
            let raw = &url[..url.len().min(64)];
            tracing::info!(target: "media_probe", site = "net.badr", base = %b, raw, ctx);
        }
    }

    fn note_net_drop(&mut self, kind: &'static str, url: &str) {
        if !tracing::enabled!(target: "media_probe", tracing::Level::INFO) {
            return;
        }
        let host = Url::parse(url)
            .ok()
            .map(|u| u.host().clone())
            .unwrap_or_default();
        if self.note_net_probe(kind, &host) {
            tracing::info!(target: "media_probe", site = "net.drop", kind, host);
        }
    }

    /// Sonde `net.fail` (`media_probe`, JS-C366) — réponse
    /// arrivée mais livrée `status: 0` : nomme la cause
    /// transport (`NetError` → kind) ou `cors_resp` (corps
    /// reçu mais `ACAO` absent/mismatch). La **raison**
    /// (`why`, vol 731 : un `cors_preflight` couvre six
    /// violations distinctes — statut `OPTIONS`, ACAO
    /// preflight, `ACAM`, `ACAH`, ACAO réponse finale,
    /// `ACAC`) distingue le maillon fautif ; `uh` nomme les
    /// en-têtes auteur non safelistés qui ont armé le
    /// preflight. Dedup `(kind, host, why-tronquée)` —
    /// deux causes distinctes d'un même hôte restent
    /// visibles, chacune une fois.
    fn note_net_fail(&mut self, done: &net_items::NetDone) {
        if !tracing::enabled!(target: "media_probe", tracing::Level::INFO) {
            return;
        }
        let host = done.req_url.host().clone();
        match &done.out {
            Err(e) => {
                let kind = net_err_kind(e);
                let why: String = e.to_string().chars().take(96).collect();
                let key = format!("{host}|{why}");
                if self.note_net_probe(kind, &key) {
                    tracing::info!(target: "media_probe", site = "net.fail", kind, host, why = %why, uh = ?done.cors_unsafe, creds = ?done.credentials);
                }
            }
            Ok(r) => {
                let acao = r
                    .headers_all("access-control-allow-origin")
                    .next()
                    .is_some();
                if self.note_net_probe("cors_resp", &host) {
                    tracing::info!(target: "media_probe", site = "net.fail", kind = "cors_resp", host, status = r.status, acao);
                }
            }
        }
    }

    /// Dedup `(kind, host)` des sondes `net.*` — `true` = le
    /// couple n'a jamais été logué (inséré sous la borne ;
    /// au-delà, non inséré mais signalé comme nouveau).
    fn note_net_probe(&mut self, kind: &'static str, host: &str) -> bool {
        if self.net_probe_seen.len() < MAX_NET_PROBE_SEEN {
            self.net_probe_seen.insert((kind, host.to_string()))
        } else {
            !self.net_probe_seen.contains(&(kind, host.to_string()))
        }
    }

    /// §B-122(e) — `net.arr` : chaque `NetDone` arrivé nomme
    /// hôte+issue — comble le trou entre `fetch.send`/
    /// `net.go` et `net.fail` (un succès livré était
    /// invisible entre deux tallys).
    fn probe_net_arr(&mut self, d: &NetDone) {
        if !tracing::enabled!(target: "media_probe", tracing::Level::INFO) {
            return;
        }
        let h = d.req_url.host().clone();
        let tag = match &d.out {
            Ok(r) => format!("{h}|{}|ok:{}", d.handle, r.status),
            Err(e) => format!("{h}|{}|err:{}", d.handle, net_err_kind(e)),
        };
        if !self.note_net_probe("arr", &tag) {
            return;
        }
        match &d.out {
            Ok(r) => {
                tracing::info!(target: "media_probe", site = "net.arr", host = %h, ok = true, status = r.status, handle = d.handle);
            }
            Err(e) => {
                tracing::info!(target: "media_probe", site = "net.arr", host = %h, ok = false, kind = net_err_kind(e), handle = d.handle);
            }
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
            let mut emit_err = None;
            if let Err(e) = eng.deliver_net_response(resp) {
                tracing::debug!(tab, error = %e, "livraison réseau js");
                emit_err = Some(e.to_string());
            }
            dlv_info!("net: resp");
            let n = eng.run_pending();
            dlv_info!("net: resp ret n={n}");
            (Self::sync_for_apply(eng.as_mut()), emit_err)
        };
        let (mutated, emit_err) = mutated;
        if let Some(m) = emit_err {
            // §B-12 vol 100 : les erreurs de livraison
            // passent par la porte partagée.
            self.emit_pipeline_error(tab, m);
        }
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
            let mut emit_err = None;
            if let Err(e) = eng.deliver_crypto_response(resp) {
                tracing::debug!(tab, error = %e, "livraison crypto js");
                emit_err = Some(e.to_string());
            }
            dlv_info!("net: crypto");
            let n = eng.run_pending();
            dlv_info!("net: crypto ret n={n}");
            (Self::sync_for_apply(eng.as_mut()), emit_err)
        };
        let (mutated, emit_err) = mutated;
        if let Some(m) = emit_err {
            self.emit_pipeline_error(tab, m);
        }
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
            let mut emit_err = None;
            if let Err(e) = eng.deliver_es_event(ctx, handle, ev) {
                tracing::debug!(tab, error = %e, "livraison eventsource js");
                emit_err = Some(e.to_string());
            }
            dlv_info!("net: es");
            let n = eng.run_pending();
            dlv_info!("net: es ret n={n}");
            (Self::sync_for_apply(eng.as_mut()), emit_err)
        };
        let (mutated, emit_err) = mutated;
        if let Some(m) = emit_err {
            self.emit_pipeline_error(tab, m);
        }
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
    ///
    /// §B-111 (vol 695) — **garde de classe, pas de site** :
    /// la morgue C407 a compté 1 768 `net: dsn` non fermés —
    /// `after_script` → `drain_script_net` → livraisons
    /// `deliver_*` → `after_script` imbriquait une passe par
    /// livraison (pile native ∝ file, `0xC00000FD` à charge
    /// égale). Tout appel reçu pendant une passe se re-file
    /// dans `after_script_again` et retourne immédiatement ;
    /// la boucle externe déroule les redemandes à plat, quelle
    /// que soit l'arête de ré-entrance (les voies silencieuses
    /// — `element_path`, `el_dyn`, `frame_ctx_load_events` —
    /// n'ont pas à être nommées : la classe est couverte par
    /// construction).
    pub(crate) fn after_script(&mut self, tab: TabId) {
        if self.in_after_script {
            if self.after_script_again.insert(tab) {
                dlv_info!("net: afs defer tab={tab}");
            }
            return;
        }
        self.in_after_script = true;
        self.after_script_again.insert(tab);
        let mut passes = 0_usize;
        while let Some(t) = self.after_script_again.iter().next().copied() {
            self.after_script_again.remove(&t);
            passes += 1;
            if passes > AFTER_SCRIPT_MAX_PASSES {
                // Une tempête qui re-marque à chaque passe
                // cède la main — le reste de la file persiste
                // pour la prochaine entrée (`HashSet` non
                // vidé), jamais perdu.
                tracing::warn!("net: afs passes>{AFTER_SCRIPT_MAX_PASSES} tab={t}");
                break;
            }
            self.after_script_pass(t);
        }
        self.in_after_script = false;
    }

    /// Corps d'une passe post-script — jamais appelé
    /// directement hors [`Self::after_script`] (la garde
    /// `in_after_script` doit être posée : un appel nu
    /// réintroduirait la ré-entrance §B-111).
    #[allow(clippy::too_many_lines)] // drains séquentiels, chaque bras documenté
    fn after_script_pass(&mut self, tab: TabId) {
        // §B-111 — `net:`/`fmir` (morgue C407) : le drain a
        // retourné (`net: frame ret`), la mort vit dans cette
        // queue post-script — chaque étape nommée.
        dlv_info!("net: afs tab={tab}");
        if self.js_stalled(tab) {
            return;
        }
        let Some((page, writes, map, idb)) = self.tabs.get_mut(&tab).and_then(|e| {
            e.js.as_mut().map(|eng| {
                (
                    e.url.clone(),
                    eng.drain_cookie_writes(),
                    eng.drain_storage(),
                    eng.drain_idb(),
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
            // PR-10 : snapshot IndexedDB persisté — même règle
            // « changement réel » que localStorage (un contexte
            // qui n'a jamais touché IDB n'écrit rien : le
            // snapshot `{}` ne vaut pas une écriture).
            if idb != "{}" && idb != "{\"dbs\":{}}" && !idb.is_empty() {
                let changed = self.idb_snapshots.get(&key).is_none_or(|prev| *prev != idb);
                if changed {
                    self.idb_snapshots.insert(key.clone(), idb.clone());
                    if let Some(s) = &self.store {
                        s.save_idb(&key, &idb);
                    }
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
        dlv_info!("net: sinks");
        let sinks: Vec<FrameSink> = self
            .tabs
            .get_mut(&tab)
            .map(|e| {
                e.js.as_mut().map_or_else(Vec::new, |eng| {
                    e.frame_ctxs
                        .iter()
                        .map(|f| {
                            let (writes, map) = eng.drain_sinks_ctx(f.ctx);
                            let idb = eng.drain_idb_ctx(f.ctx);
                            (f.url.clone(), f.opaque, f.ctx, writes, map, idb)
                        })
                        .collect()
                })
            })
            .unwrap_or_default();
        if let Some(page) = self.tabs.get(&tab).and_then(|e| e.url.clone()) {
            for (furl, opaque, ctx, writes, map, idb) in sinks {
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
                // PR-10d : IndexedDB de l'enfant persisté sous
                // **son** origine (le registre est isolé par
                // realm — plus de pollution du top-level).
                if !idb.is_empty() && idb != "{\"dbs\":{}}" && idb != "{}" {
                    let changed = self.idb_snapshots.get(&key).is_none_or(|prev| *prev != idb);
                    if changed {
                        self.idb_snapshots.insert(key.clone(), idb.clone());
                        if let Some(s) = &self.store {
                            s.save_idb(&key, &idb);
                        }
                    }
                }
            }
        }
        dlv_info!("net: dsn");
        self.drain_script_net(tab);
        dlv_info!("net: dsn done");
        // Étape 134 : les miroirs des contextes `<iframe>`
        // mutés par leurs scripts reviennent en
        // `SubFrame.dom` (le composite re-rend au relayout).
        self.sync_frame_mirrors(tab);
        dlv_info!("net: media");
        self.drain_media_commands(tab);
        dlv_info!("net: afs done");
    }

    /// Livre les réponses arrivées — synchrone, sans yield :
    /// appelée aux points de synchro internes (événements)
    /// **et** par les hôtes partagés — les resynchros DOM
    /// rapportées sont enfilées (étape 55h), à drainer via
    /// `drain_applies_shared` hors verrou ou `drain_applies`
    /// sous `&mut self`.
    #[allow(clippy::too_many_lines)] // dispatch séquentiel des NetDone — chaque variante documentée in situ
    pub fn poll_net(&mut self) -> usize {
        if let Ok(mut g) = crate::LAST_DRAIN.lock() {
            *g = "poll_net";
        }
        // Étape 209 — le shed RSS se jauge à cadence bornée
        // en tête de pompe (le seul rail qui voit tout le
        // trafic) ; les entrées nav le relisent aussi.
        self.maybe_rss_shed();
        // Le budget est mesuré d'ici (Étape 151 / §B-54) : les
        // drains crypto/ES/WS précédant `drain_net_done`
        // comptent dans la même passe.
        let start = std::time::Instant::now();
        // `pending_net_done` d'abord (livraisons différées par
        // la borne — Étape 151) puis les arrivées du canal :
        // l'ordre d'émission global est conservé.
        let mut done: Vec<NetDone> = self.pending_net_done.drain(..).collect();
        while let Ok(d) = self.net_rx.try_recv() {
            done.push(d);
        }
        // Retenues §B-60 échues : émises enfin sur le
        // transport (quota `max_script_net` re-vérifié au
        // spawn — saturé = retenue repoussée d'une base,
        // jamais de `status: 0` accéléré).
        let now = std::time::Instant::now();
        let mut still_held = std::collections::VecDeque::new();
        while let Some(h) = self.fetch_held.pop_front() {
            if h.at > now {
                still_held.push_back(h);
                continue;
            }
            // Quota plein pendant une tempête : repousser
            // plutôt qu'échouer — la boucle applicative reste
            // goutte-à-goutte (sinon elle repartirait en
            // rafale d'échecs immédiats). Les autres échecs
            // du spawn (contexte mort, requête refusée) sont
            // permanents → `status: 0` livré, jamais de
            // promesse pendue à l'infini.
            let inflight = self.net_tasks.keys().filter(|(t, _)| *t == h.tab).count();
            if inflight >= self.max_script_net {
                let mut h = h;
                h.at = now + FETCH_BACKOFF_BASE;
                still_held.push_back(h);
            } else if self
                .spawn_script_fetch(
                    h.tab,
                    &h.page,
                    &h.base,
                    h.handle,
                    &h.method,
                    &h.url,
                    &h.headers,
                    h.body.as_deref(),
                    h.mode,
                    h.credentials,
                    h.ctx,
                )
                .is_err()
            {
                self.deliver_net_response(
                    h.tab,
                    NetResponse {
                        handle: h.handle,
                        status: 0,
                        status_text: String::new(),
                        headers: Vec::new(),
                        body: Vec::new(),
                    },
                );
            }
        }
        self.fetch_held = still_held;
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
        self.drain_net_done(done, start);
        // JS-C367 — les livrées/aborts/fins de flux ont pu
        // libérer des slots : les fetch/XHR garés au quota
        // partent (FIFO par onglet).
        self.pump_pending_script_net();
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
                        // Origine morte : le graphe ne part pas —
                        // relâche la file ordonnée (refus).
                        self.pump_ordered_dyn_tail(d.tab, meta.ctx);
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
            // Étape 207-bis : le `src` ordonné est livré —
            // la balise parse suivante du ctx part.
            self.pump_ordered_dyn_tail(d.tab, meta.ctx);
        }
    }

    /// Dispatch des réponses `NetDone` collectées —
    /// extrait de `poll_net` (étape 51 + CAP-169/186/JS-C94
    /// ont fait passer la fonction au-delà de 100 lignes) :
    /// `<script>` dynamique, graphe module, `<img>`, `<link>`,
    /// WS et fetch/XHR y sont livrés dans l'ordre d'arrivée.
    /// `start` = instant d'entrée de `poll_net` — la borne
    /// `poll_net_budget` (Étape 151 / §B-54) s'y mesure :
    /// au-delà, les `NetDone` restants sont différés en
    /// `pending_net_done` (ordre conservé, relivrés en tête à
    /// la passe suivante) au lieu de tenir le verrou session
    /// sur une rafale de livraisons multi-secondes.
    pub(crate) fn drain_net_done(&mut self, done: Vec<NetDone>, start: std::time::Instant) {
        if let Ok(mut g) = crate::LAST_DRAIN.lock() {
            *g = "net_done";
        }
        let mut it = done.into_iter();
        while let Some(mut d) = it.next() {
            if !self.poll_net_budget.is_zero() && start.elapsed() > self.poll_net_budget {
                let deferred = 1 + it.len();
                self.pending_net_done.push_back(d);
                self.pending_net_done.extend(it);
                tracing::warn!(
                    deferred,
                    budget_ms = self.poll_net_budget.as_millis(),
                    "poll_net : livraisons différées à la prochaine passe (B-54)"
                );
                return;
            }
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
                self.commit_dyn_import(d.tab, &meta, &d.out);
                continue;
            }
            self.net_tasks.remove(&(d.tab, d.handle));
            self.probe_net_arr(&d);
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
                _ => {
                    // JS-C366 — nomme la cause transport/CORS
                    // du `status: 0` (dedup `(kind, host)`).
                    self.note_net_fail(&d);
                    NetResponse {
                        handle: d.handle,
                        status: 0,
                        status_text: String::new(),
                        headers: Vec::new(),
                        body: Vec::new(),
                    }
                }
            };
            // Streak §B-60 : `status: 0` (échec transport ou
            // réponse cross non-lisible livrée opaque) allonge
            // la série de l'URL — un succès la ferme. La map
            // reste bornée (`MAX_STREAK_URLS`) : au-delà les
            // nouvelles URLs ne sont pas suivies.
            let key = (d.tab, d.req_url.to_string());
            if resp.status == 0 {
                let n = self.fetch_fail_streak.get(&key).copied().unwrap_or(0);
                if self.fetch_fail_streak.len() < MAX_STREAK_URLS
                    || self.fetch_fail_streak.contains_key(&key)
                {
                    self.fetch_fail_streak.insert(key, n.saturating_add(1));
                }
            } else {
                self.fetch_fail_streak.remove(&key);
            }
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
        self.net_tasks.len()
            + self.dyn_tasks.len()
            + self.fetch_held.len()
            + self.pending_script_net.len()
    }

    /// Pipeline `<img>` de [`Self::plan_subresources`] —
    /// résolution base, upgrade CSP, mixed-content +
    /// `img-src`, dédup URL, borne `max_images`. Sonde
    /// §B-108 `img.skip` : attribution des drops entre
    /// collecte et fetch (la raison qui tue le candidat).
    fn plan_img_urls(
        &self,
        dom: &Dom,
        page: &Url,
        base: &Url,
        upg: &dyn Fn(Url) -> Url,
        allow: &dyn Fn(explorer_net::http::FetchDest, &Url) -> bool,
    ) -> Vec<Url> {
        let mut seen_img = std::collections::HashSet::new();
        let img_probe = tracing::enabled!(target: "media_probe", tracing::Level::INFO);
        let mut noresolve = 0u64;
        let mut nonhttp = 0u64;
        let mut blocked = 0u64;
        let mut dup = 0u64;
        let img_cands = collect_images(dom, self.viewport.0, self.scale);
        let img_cand_n = img_cands.len();
        let imgs: Vec<Url> = img_cands
            .into_iter()
            .filter_map(|(_, src)| {
                Url::resolve(base, &src).ok().or_else(|| {
                    if img_probe {
                        noresolve += 1;
                    }
                    None
                })
            })
            .map(upg)
            // `data:` (et autres non-http) ne se fetch pas —
            // peints inline par `collect_data_images` (56f).
            .filter(|u| {
                let ok = matches!(u.scheme(), "http" | "https");
                if !ok && img_probe {
                    nonhttp += 1;
                }
                ok
            })
            .filter(|u| {
                if is_mixed_content(page, u) || !allow(explorer_net::http::FetchDest::Image, u) {
                    if img_probe {
                        blocked += 1;
                    }
                    return false;
                }
                let fresh = seen_img.insert(u.as_str());
                if !fresh && img_probe {
                    dup += 1;
                }
                fresh
            })
            .take(self.max_images)
            .collect();
        if img_probe {
            tracing::info!(
                target: "media_probe",
                site = "img.skip",
                cand = img_cand_n,
                noresolve,
                nonhttp,
                blocked,
                dup,
                fetch = imgs.len()
            );
        }
        imgs
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
                // Refus muets nommés (vol 32 — 0 feuille sans
                // aucune trace) : mixed-content et `style-src`
                // refusent en plan, le warn les rend visibles.
                if is_mixed_content(page, u) {
                    tracing::warn!(url = %u, "feuille de style refusée — mixed-content");
                    return false;
                }
                if !allow(explorer_net::http::FetchDest::Style, u) {
                    tracing::warn!(url = %u, "feuille de style refusée par CSP (style-src)");
                    return false;
                }
                seen_sheets.insert(u.to_string())
            })
            .take(self.max_sheets)
            .collect();
        let scripts = self.plan_script_items(dom, page, base, scripting, csp);
        // B-4 (data.gouv/Nuxt) : `<link rel=modulepreload>`
        // et `preload as=script|module` sèment le prefetch
        // des graphes de modules — les chunks déclarés en
        // importmap n'apparaissent dans aucun `import`
        // statique avant eval. Résolus sur `base`, CSP/
        // mixed-content filtrés comme tout `script-src`,
        // dédupliqués, bornés `max_modules` ; rien à semer
        // sans scripting.
        let mut seen_preload = std::collections::HashSet::new();
        let mod_preloads: Vec<Url> = if scripting {
            collect_module_preloads(dom)
                .into_iter()
                .filter_map(|h| Url::resolve(base, &h).ok())
                .map(&upg)
                .filter(|u| {
                    !is_mixed_content(page, u)
                        && allow(explorer_net::http::FetchDest::Script, u)
                        && seen_preload.insert(u.as_str())
                })
                .take(self.max_modules)
                .collect()
        } else {
            Vec::new()
        };
        let imgs = self.plan_img_urls(dom, page, base, &upg, &allow);
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
            mod_preloads,
        }
    }

    /// `<script>` exécutables du document (étape CMP-4) —
    /// `collect_scripts` borné `max_scripts`, marqueurs CMP
    /// réarmés sous politique résolue (`Honest` jamais).
    /// Extrait pour la borne clippy de `plan_subresources`.
    fn plan_script_items(
        &self,
        dom: &Dom,
        page: &Url,
        base: &Url,
        scripting: bool,
        csp: &[explorer_net::csp::CspPolicy],
    ) -> Vec<ScriptItem> {
        collect_scripts(
            dom,
            page,
            base,
            scripting,
            self.max_scripts,
            csp,
            cmp_rearm(self.consent_policy),
        )
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
        mod_preloads: &[Url],
        graph: &mut ModuleGraph,
    ) {
        let mut queued: std::collections::HashSet<String> =
            std::iter::once(entry_base.as_str()).collect();
        let mut frontier: Vec<(String, Url)> = Vec::new();
        for (spec, u) in module_deps(entry_src, entry_base, import_specs) {
            Self::queue_module_dep(
                spec,
                u,
                page,
                csp,
                import_specs,
                graph,
                &mut queued,
                &mut frontier,
            );
        }
        // B-4 : `<link rel=modulepreload>`/`preload
        // as=script|module` ensemencent la frontier — les
        // chunks importmap (Nuxt) hors `import` statique.
        // spec = URL résolue (la spec modulepreload mappe
        // par URL) ; `queued` déduplique avec les deps
        // réelles, `queue_module_dep` refiltre CSP.
        for u in mod_preloads {
            Self::queue_module_dep(
                u.as_str(),
                u.clone(),
                page,
                csp,
                import_specs,
                graph,
                &mut queued,
                &mut frontier,
            );
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
                    Self::queue_module_dep(
                        spec2,
                        u2,
                        page,
                        csp,
                        import_specs,
                        graph,
                        &mut queued,
                        &mut next,
                    );
                }
            }
            frontier = next;
        }
        if !frontier.is_empty() {
            // Plafond `max_modules` atteint avec des deps en
            // attente — troncation nommée (sinon l'échec
            // d'instanciation « module non chargé » arrive sans
            // cause visible — mesuré mastodon, axios en n°65).
            tracing::warn!(
                max = max_modules,
                restants = frontier.len(),
                "graphe de modules tronqué (max_modules)"
            );
        }
    }

    /// Enfile une dépendance de module après CSP/dédup
    /// (`fetch_module_sources`, J3) — `data:` décodé inline
    /// (corps auto-porteur, aucune requête ; ses deps nichées
    /// repassent par ce même bras, récursion bornée par
    /// `queued`), sinon poussé à `out` pour la vague de fetch.
    #[allow(clippy::too_many_arguments)]
    fn queue_module_dep(
        spec: String,
        u: Url,
        page: &Url,
        csp: &[explorer_net::csp::CspPolicy],
        import_specs: &[(String, String)],
        graph: &mut ModuleGraph,
        queued: &mut std::collections::HashSet<String>,
        out: &mut Vec<(String, Url)>,
    ) {
        let u = explorer_net::csp::csp_upgrade(csp, &u).unwrap_or(u);
        graph
            .alias
            .entry(spec.clone())
            .or_insert_with(|| u.as_str());
        let allowed = !is_mixed_content(page, &u)
            && explorer_net::csp::csp_allows(csp, page, explorer_net::http::FetchDest::Script, &u);
        if !(allowed && queued.insert(u.as_str())) {
            return;
        }
        if let Some(body) = data_url_bytes(&u.as_str()) {
            let text = String::from_utf8_lossy(&body).into_owned();
            for (spec2, u2) in module_deps(&text, &u, import_specs) {
                Self::queue_module_dep(spec2, u2, page, csp, import_specs, graph, queued, out);
            }
            graph.sources.insert(u.as_str(), text);
        } else {
            out.push((spec, u));
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
        let max_bytes = self.image_max_bytes;
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        let Some(dom) = entry.dom.as_ref() else {
            return;
        };
        let bg_bytes = entry
            .bg_decoded
            .values()
            .flatten()
            .map(|i| i.rgba.len())
            .sum::<usize>();
        insert_images_into(
            dom,
            &mut entry.images,
            base,
            bodies,
            max_px,
            &mut entry.img_decoded,
            bg_bytes,
            max_bytes,
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
    #[allow(clippy::too_many_lines)] // motif maison : les trois boucles de restauration restent lisibles en bloc.
    pub(crate) fn restore_images(&mut self, tab: TabId) {
        let max_px = self.image_max_px;
        let max_bytes = self.image_max_bytes;
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        // Budget octets global sur les bitmaps retenus (§B-59)
        // — la restauration re-décode depuis `img_bodies` ;
        // au-delà du budget l'image reste absente (repli
        // `alt`), jamais de croissance hors borne.
        let mut running = decoded_cache_bytes(&entry.img_decoded, &entry.bg_decoded);
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
            if running.saturating_add(img.rgba.len()) > max_bytes {
                continue;
            }
            running += img.rgba.len();
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
            if running.saturating_add(img.rgba.len()) > max_bytes {
                continue;
            }
            running += img.rgba.len();
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
            if running.saturating_add(img.rgba.len()) > max_bytes {
                continue;
            }
            running += img.rgba.len();
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
/// Octets décodés actuellement retenus dans les deux caches
/// persistants (`img_decoded` + `bg_decoded`) — référence
/// du budget §B-59 à l'insertion.
pub(crate) fn decoded_cache_bytes(
    img_decoded: &HashMap<String, Arc<DecodedImage>>,
    bg_decoded: &HashMap<String, Option<Arc<DecodedImage>>>,
) -> usize {
    img_decoded.values().map(|i| i.rgba.len()).sum::<usize>()
        + bg_decoded
            .values()
            .flatten()
            .map(|i| i.rgba.len())
            .sum::<usize>()
}

#[allow(clippy::too_many_arguments)]
fn insert_images_into(
    dom: &Dom,
    images: &mut PaintImages,
    base: &Url,
    bodies: &HashMap<String, Vec<u8>>,
    max_px: u32,
    decoded_cache: &mut HashMap<String, Arc<DecodedImage>>,
    bg_bytes: usize,
    max_bytes: usize,
    bodies_store: &mut HashMap<String, Vec<u8>>,
    img_w: f32,
    dpr: f32,
) {
    let items = collect_images(dom, img_w, dpr);
    let mut decoded: HashMap<String, Option<Arc<DecodedImage>>> = HashMap::new();
    // Budget octets global sur les bitmaps retenus (§B-59) :
    // `image_max_px` borne chaque image mais pas leur somme —
    // franceinter accumulait 2,1 Gio de RGBA. Au-delà du
    // budget l'image n'entre ni en table ni en cache (repli
    // `alt` peint) ; le corps compressé reste retenu dans
    // `img_bodies` pour un éventuel re-décodage. L'ordre
    // document de `collect_images` garde le haut de page.
    let mut running = bg_bytes + decoded_cache.values().map(|i| i.rgba.len()).sum::<usize>();
    for (node, src) in items {
        let Ok(u) = Url::resolve(base, &src) else {
            continue;
        };
        let Some(body) = bodies.get(&u.as_str()) else {
            continue;
        };
        let img = decoded.entry(u.as_str()).or_insert_with(|| {
            decode_image(body, max_px).map(Arc::new).filter(|i| {
                let b = i.rgba.len();
                if running.saturating_add(b) > max_bytes {
                    false
                } else {
                    running += b;
                    true
                }
            })
        });
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
    max_bytes: usize,
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
        bg_decoded,
        img_bodies,
        ..
    } = p;
    let Some(dom) = dom.as_ref() else {
        return;
    };
    let bg_bytes = bg_decoded
        .values()
        .flatten()
        .map(|i| i.rgba.len())
        .sum::<usize>();
    insert_images_into(
        dom,
        images,
        base,
        bodies,
        max_px,
        img_decoded,
        bg_bytes,
        max_bytes,
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
    // Snapshot `IndexedDB` du ctx (PR-10d — registre par
    // realm, persisté sous l'origine de l'enfant).
    String,
);

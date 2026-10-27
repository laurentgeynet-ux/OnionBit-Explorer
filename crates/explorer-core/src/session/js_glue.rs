// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Colle JS : moteur extrait/rendu hors verrou, dispatch
//! d'événements, pompe de timers, sync miroir.
use super::*;
use std::sync::Arc;
use std::time::{Duration, Instant};

use explorer_dom::NodeId;
use explorer_js::{EventProp, JsEngine, UrlResolver};
use explorer_net::url::Url;

impl Session {
    /// Pompe les jobs JS dus d'un onglet — timers bornés à
    /// `max_timer_jobs` (ADR-0019) + microtasks +
    /// `MutationObserver` (JS-C26, livré au même checkpoint) —
    /// puis synchronise DOM, storage, cookies et net aux
    /// mêmes points de synchro que les événements. Renvoie le
    /// nombre de jobs exécutés.
    ///
    /// Synchrone : les intentions de navigation émises par un
    /// timer sont à drainer ensuite via
    /// [`Self::drain_script_nav`] (async).
    ///
    /// Tick `<meta http-equiv=refresh>` (CAP-143) : l'échéance
    /// échue devient une intention `DrainAct::Req` — débitée
    /// du budget nav-auto (boucle cdiscount : refresh ré-armé
    /// à chaque commit, 3 812 navs mesurées).
    fn pump_refresh(&mut self, tab: TabId) {
        let refresh_target = self.tabs.get_mut(&tab).and_then(|entry| {
            entry
                .refresh_at
                .as_ref()
                .is_some_and(|(t, _)| Instant::now() >= *t)
                .then(|| entry.refresh_at.take())
                .flatten()
                .map(|(_, target)| target)
        });
        if let Some(target) = refresh_target {
            if self.auto_nav_budget(tab) {
                self.enqueue_nav_act(
                    tab,
                    DrainAct::Req(Box::new(Request::get(target)), true),
                    false,
                );
            }
        }
    }

    /// Fait aussi office de tick `<meta http-equiv=refresh>`
    /// (CAP-143) : l'échéance échue pousse une intention
    /// `DrainAct::Req` — sans moteur JS, au même drain que
    /// les intentions script.
    pub fn pump_timers(&mut self, tab: TabId) -> usize {
        self.pump_refresh(tab);
        let max = self.max_timer_jobs;
        let t0 = Instant::now();
        let (jobs, mutated) = {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return 0;
            };
            // Disjoncteur (vol 25) : un `run_pending` sur
            // worker figé patienterait 30 s sous verrou.
            if entry.js_stall_until.is_some_and(|u| Instant::now() < u) {
                return 0;
            }
            let sync_ok = entry.sync_floor.is_none_or(|f| Instant::now() >= f);
            let Some(eng) = entry.js.as_mut() else {
                return 0;
            };
            // Deadline interne (J1) : une rafale saturante
            // remonte `Timeout` → disjoncteur, même règle
            // qu'une éval figée (allrecipes).
            let jobs = match eng.run_pending_bounded(max) {
                Ok(n) => n,
                Err(explorer_js::JsError::Timeout) => {
                    entry.js_stall_until = Some(Instant::now() + self.js_stall_cooldown);
                    0
                }
                Err(_) => 0,
            };
            let m = if sync_ok {
                Self::sync_for_apply(eng.as_mut())
            } else {
                None
            };
            (jobs, m)
        };
        let mut t = sync_phase(tab, "pump.js_run_sync", t0);
        if let Some(s) = mutated {
            let ta = Instant::now();
            self.queue_apply_sync(tab, s);
            t = sync_phase(tab, "pump.apply_queued", ta);
        }
        if jobs > 0 {
            // Les callbacks ont pu émettre XHR/cookies/storage.
            self.after_script(tab);
            self.poll_net();
            sync_phase(tab, "pump.drains", t);
        }
        // Requêtes émises par la **dernière** rafale de jobs
        // (`fetch`/`import()`/`<img>`/`<link>` dynamiques…) :
        // `drain_script_net` ne tournait que via
        // `after_script` (gate `jobs>0`) — un lien marqué
        // sans jobs suivants n'était jamais fetché, son
        // `load`/`error` jamais livré (mesuré github : les
        // promesses `c.e` des chunks CSS pendent sur
        // `link.onload`). Le drain est bon marché à vide —
        // il court à chaque battement.
        self.drain_script_net(tab);
        self.drain_anim_births(tab);
        // Les applies enfilés par cette pompe sont exécutés
        // ici (hôte `&mut` synchrone — étape 55h).
        self.drain_applies();
        jobs
    }

    /// [`Self::pump_timers`] sur tous les onglets — le
    /// battement de l'hôte (daemon, CLI) itère ici. Fait
    /// aussi avancer les animations en lecture (étape 55e :
    /// `@keyframes` cadencé par ce battement, pas seulement
    /// par le pull de `/frame`).
    pub fn pump_timers_all(&mut self) -> usize {
        let tabs: Vec<TabId> = self.tabs.keys().copied().collect();
        let mut jobs = 0;
        for t in tabs {
            jobs += self.pump_timers(t);
            // Dégradation silencieuse : un onglet sans arbre
            // posé ne produit pas d'erreur de pompe.
            let _ = self.advance_animations(t);
        }
        jobs
    }

    /// Phase « take » de la pompe détachée (étape 53f) : sort
    /// le moteur JS de l'onglet sous verrou court et marque
    /// `js_detached` à l'epoch courante — l'exécution
    /// (`run_pending` + sérialisation miroir) se fera hors
    /// verrou. `None` sans moteur ou si une pompe détachée
    /// est déjà en vol (la suivante reprendra ses jobs). Le
    /// booléen final autorise la resynchro DOM : `false`
    /// sous la fenêtre de coalescence `sync_floor`
    /// (55g-suite — le miroir garde son bit sale et le
    /// prochain apply absorbable absorbe tout d'un coup).
    pub(crate) fn take_js_engine(
        &mut self,
        tab: TabId,
    ) -> Option<(Box<dyn JsEngine>, usize, u64, bool)> {
        let entry = self.tabs.get_mut(&tab)?;
        if entry.js_detached.is_some() {
            return None;
        }
        // Disjoncteur (vol 25) : détacher lancerait un run
        // de 30 s hors verrou puis des livrées à timeout
        // **sous** verrou au commit — autant sauter le tour.
        if entry.js_stall_until.is_some_and(|u| Instant::now() < u) {
            return None;
        }
        let sync_ok = entry.sync_floor.is_none_or(|f| Instant::now() >= f);
        let eng = entry.js.take()?;
        entry.js_detached = Some(entry.nav_epoch);
        Some((eng, self.max_timer_jobs, entry.nav_epoch, sync_ok))
    }

    /// Phase « commit » de la pompe détachée (53f) : réinsère
    /// le moteur si l'onglet n'a pas navigué pendant le run —
    /// une epoch différente jette le moteur et ses tampons
    /// (le miroir parlerait de l'ancienne page, jamais
    /// appliqué par-dessus la neuve). Livre les réponses net
    /// tamponnées, rejoue les pushes d'attributs **avant**
    /// re-sérialisation, applique la synchro DOM puis les
    /// drains de fin de script — même ordre que
    /// [`Self::pump_timers`]. `out: None` = le bloc a paniqué
    /// (moteur perdu : l'onglet continue sans JS).
    #[allow(clippy::too_many_lines)] // phase commit : file de livrées ordonnée, pas de sous-fonction nette
    pub(crate) fn commit_js_pump(
        &mut self,
        tab: TabId,
        epoch: u64,
        out: Option<(usize, Option<apply::ApplySync>, Box<dyn JsEngine>)>,
        sync_ok: bool,
    ) -> usize {
        let t0 = Instant::now();
        let phase = {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return 0;
            };
            entry.js_detached = None;
            let Some((jobs, mutated, mut eng)) = out else {
                entry.js_pending_net.clear();
                entry.js_pending_crypto.clear();
                entry.js_pending_es.clear();
                entry.js_pending_ws.clear();
                entry.js_pending_dyn.clear();
                entry.js_pending_attrs.clear();
                entry.pending_doc_lifecycle.clear();
                return 0;
            };
            if entry.nav_epoch != epoch {
                entry.js_pending_net.clear();
                entry.js_pending_crypto.clear();
                entry.js_pending_es.clear();
                entry.js_pending_ws.clear();
                entry.js_pending_dyn.clear();
                entry.js_pending_attrs.clear();
                entry.pending_doc_lifecycle.clear();
                return 0;
            }
            // Disjoncteur « moteur figé » (vol 25) : chaque
            // livrée/dispatch/eval ci-dessous attend une
            // réponse du worker (`recv_eval`, 30 s) **sous
            // verrou** — sur worker saturé la rafale figeait
            // l'API ~90 s (kayak). Disjoncteur ouvert → on
            // jette les tampons et on réinsère le moteur sans
            // l'appeler : le verrou repasse aux handlers HTTP.
            let mut stall: Option<explorer_js::JsError> = None;
            if entry.js_stall_until.is_some_and(|u| Instant::now() < u) {
                let dropped = entry.js_pending_net.len()
                    + entry.js_pending_crypto.len()
                    + entry.js_pending_es.len()
                    + entry.js_pending_ws.len()
                    + entry.js_pending_dyn.len()
                    + entry.js_pending_mod.len()
                    + entry.pending_doc_lifecycle.len();
                entry.js_pending_net.clear();
                entry.js_pending_crypto.clear();
                entry.js_pending_es.clear();
                entry.js_pending_ws.clear();
                entry.js_pending_dyn.clear();
                entry.js_pending_mod.clear();
                entry.js_pending_attrs.clear();
                entry.pending_doc_lifecycle.clear();
                if dropped > 0 {
                    tracing::warn!(tab, dropped, "moteur figé — livrées tamponnées jetées");
                }
            }
            let mutated = if entry.js_stall_until.is_some_and(|u| Instant::now() < u) {
                mutated
            } else {
                // Cycle de vie document émis pendant le
                // détachement (le tail de commit a pu courir sans
                // moteur) — rejoué **avant** les livraisons
                // tamponnées : une porte `readystatechange` qui se
                // lève enfile ses `import()`/`fetch` avant elles,
                // comme l'aurait fait l'événement à l'heure.
                for step in std::mem::take(&mut entry.pending_doc_lifecycle) {
                    match step {
                        types::DocLifecycle::ReadyState(ctx, s) => {
                            eng.set_ready_state(ctx, &s);
                        }
                        types::DocLifecycle::Event(ctx, ty) => {
                            if let Err(e) = eng.dispatch_event_ctx(ctx, &ty, &[]) {
                                if Self::is_engine_stall(&e) {
                                    stall = Some(e);
                                    break;
                                }
                            }
                        }
                    }
                }
                // Sous-phases `commit_lock` (tâche profil github
                // doc 20,6 s — attribution fine des ~800 ms/round :
                // chaque livraison exécute du moteur sous verrou).
                let mut t = Instant::now();
                // B-39/vol59 : le disjoncteur stall couvre un
                // moteur MORT, pas un worker lent-mais-vivant —
                // ~90 s de livraisons sous verrou ont figé l'API
                // (commit_lock us=89,9 M, tab DELETÉ). Plafond
                // temps de la section : au-delà, tampons jetés +
                // stall marqué (le moteur se réévalue sans verrou).
                let expired = |t: &Instant| t.elapsed() > Duration::from_secs(45);
                if stall.is_none() && !expired(&t) {
                    for resp in std::mem::take(&mut entry.js_pending_net) {
                        if let Err(e) = eng.deliver_net_response(resp) {
                            if Self::is_engine_stall(&e) {
                                stall = Some(e);
                                break;
                            }
                        }
                    }
                }
                t = sync_phase(tab, "commit.dl_net", t);
                if stall.is_none() && !expired(&t) {
                    for resp in std::mem::take(&mut entry.js_pending_crypto) {
                        if let Err(e) = eng.deliver_crypto_response(resp) {
                            if Self::is_engine_stall(&e) {
                                stall = Some(e);
                                break;
                            }
                        }
                    }
                }
                t = sync_phase(tab, "commit.dl_crypto", t);
                if stall.is_none() && !expired(&t) {
                    for (ctx, handle, ev) in std::mem::take(&mut entry.js_pending_es) {
                        if let Err(e) = eng.deliver_es_event(ctx, handle, ev) {
                            if Self::is_engine_stall(&e) {
                                stall = Some(e);
                                break;
                            }
                        }
                    }
                }
                t = sync_phase(tab, "commit.dl_es", t);
                if stall.is_none() && !expired(&t) {
                    for (ctx, handle, ev) in std::mem::take(&mut entry.js_pending_ws) {
                        if let Err(e) = eng.deliver_ws_event(ctx, handle, ev) {
                            if Self::is_engine_stall(&e) {
                                stall = Some(e);
                                break;
                            }
                        }
                    }
                }
                t = sync_phase(tab, "commit.dl_ws", t);
                for (ctx, seq, body) in std::mem::take(&mut entry.js_pending_dyn) {
                    eng.deliver_dyn_script(ctx, seq, body);
                }
                t = sync_phase(tab, "commit.dl_dyn", t);
                if stall.is_none() && !expired(&t) {
                    for (ctx, seq, me) in std::mem::take(&mut entry.js_pending_mod) {
                        let ok = me.is_some_and(|me| Self::eval_mod(eng.as_mut(), &me).is_ok());
                        eng.deliver_dyn_module(ctx, seq, ok);
                    }
                }
                t = sync_phase(tab, "commit.eval_mod", t);
                let attrs = std::mem::take(&mut entry.js_pending_attrs);
                // Les pushes sont **toujours** rejoués dans le
                // miroir (la saisie ne se perd pas) ; la
                // re-sérialisation attend la fin de la fenêtre de
                // coalescence — le bit sale persiste jusqu'au
                // prochain apply autorisé (55g-suite).
                let mutated = if attrs.is_empty() {
                    mutated
                } else {
                    for (path, attr, value) in attrs {
                        eng.sync_mirror_attr(&path, &attr, value.as_deref());
                    }
                    if sync_ok {
                        Self::sync_for_apply(eng.as_mut())
                    } else {
                        None
                    }
                };
                sync_phase(tab, "commit.attrs_sync", t);
                if stall.is_none() && expired(&t) {
                    stall = Some(explorer_js::JsError::Timeout);
                }
                mutated
            };
            if let Some(e) = stall {
                tracing::warn!(tab, error = %e,
                    "livrée moteur en timeout — disjoncteur armé");
                entry.js_stall_until = Some(Instant::now() + self.js_stall_cooldown);
            }
            entry.js = Some(eng);
            (jobs, mutated)
        };
        let (jobs, mutated) = phase;
        let mut t = sync_phase(tab, "commit.deliver", t0);
        if let Some(s) = mutated {
            let ta = Instant::now();
            // La mutation est enfilée — l'apply tournera hors
            // verrou via `drain_applies_shared` (étape 55h,
            // l'appelant pompe partagée draine juste après).
            self.queue_apply_sync(tab, s);
            t = sync_phase(tab, "commit.apply_queued", ta);
        }
        // B-41 (credit-agricole vol63) : les drains finaux
        // sont aussi des attentes worker sous verrou —
        // `commit.drains` 185 s a affamé l'API alors que le
        // plafond des livraisons était déjà posé. Même
        // budget : au-delà de 45 s de commit, les drains
        // non essentiels sautent — warn nommé.
        let drain_expired = t0.elapsed() > Duration::from_secs(45);
        if drain_expired {
            tracing::warn!(
                tab,
                elapsed_ms = t0.elapsed().as_millis() as u64,
                "commit borné — drains finaux sautés (B-41)"
            );
        }
        if jobs > 0 && !drain_expired {
            self.after_script(tab);
            self.poll_net();
            sync_phase(tab, "commit.drains", t);
        }
        // Même liveness que `pump_timers` : les requêtes
        // émises par la dernière rafale (liens/images/
        // scripts/`import()` dynamiques) doivent partir
        // même si plus aucun job n'est en file.
        if !drain_expired {
            self.drain_script_net(tab);
            self.drain_anim_births(tab);
        }
        // B-41 (flixbus) : une rafale `UnmappedKey` →
        // re-parse → éval produisait des jobs à chaque
        // pompe — la pompe repartait sans fin (pump_
        // js_run_sync persistant, API affamée). Série de
        // pompes productives bornée : au-delà de 64,
        // disjoncteur armé + warn — la page tombe en
        // « moteur figé » nommé plutôt que starvation.
        if jobs > 0 {
            let streak = self.pump_streaks.entry(tab).or_insert(0);
            *streak += 1;
            if *streak == 64 {
                if let Some(e) = self.tabs.get_mut(&tab) {
                    e.js_stall_until = Some(Instant::now() + self.js_stall_cooldown);
                }
                tracing::warn!(
                    tab,
                    streak = *streak,
                    "tempête de pompes — disjoncteur armé (B-41)"
                );
            }
        } else {
            self.pump_streaks.insert(tab, 0);
        }
        jobs
    }

    /// Livraison des `animationstart` collectés à la
    /// cascade (étape 141b) — chaque naissance `(nœud réel,
    /// nom)` devient un événement bullant sur l'élément
    /// miroir, props `animationName`/`elapsedTime`/
    /// `pseudoElement` (spec `AnimationEvent`). Moteur
    /// absent (détachement) → la naissance est
    /// **conservée** en queue, jamais perdue : la
    /// découverte lazy des registries CE (`x(e)` catalyst
    /// → `m.get(e.animationName)`) dépend de chaque
    /// dispatch — un drop muet laissait l'entrée lazy
    /// invisible à jamais. Nœud reparti entre passe et
    /// drain → sauté (dispatcher vers un mort est sans
    /// sens, son miroir n'existe plus non plus).
    fn drain_anim_births(&mut self, tab: TabId) {
        loop {
            let Some(entry) = self.tabs.get(&tab) else {
                return;
            };
            if entry.js.is_none() {
                return;
            }
            let Some((node, name)) = entry.pending_anim_births.front().cloned() else {
                return;
            };
            // Le DOM commis fait foi : le nœud d'une
            // naissance peut être reparti entre la passe et
            // le drain (re-parse, detach) → sauté.
            let path = entry.dom.as_ref().and_then(|d| element_path(d, node));
            self.tabs
                .get_mut(&tab)
                .map(|e| e.pending_anim_births.pop_front());
            let Some(path) = path else {
                continue;
            };
            let props = [
                (
                    "animationName".to_string(),
                    explorer_js::EventProp::Str(name),
                ),
                ("elapsedTime".to_string(), explorer_js::EventProp::Num(0.0)),
                (
                    "pseudoElement".to_string(),
                    explorer_js::EventProp::Str(String::new()),
                ),
            ];
            self.dispatch_path_event_props(tab, "animationstart", &path, &props);
        }
    }

    /// Plus proche échéance de timer JS tous onglets — délai
    /// relatif recalculé par chaque moteur (`None` = aucun
    /// timer armé). L'hôte dort jusqu'à `min(délai,
    /// battement)`.
    #[must_use]
    pub fn next_timer_deadline(&mut self) -> Option<std::time::Duration> {
        self.tabs
            .values_mut()
            .filter_map(|e| e.js.as_mut())
            .filter_map(|eng| eng.next_timer_deadline())
            .min()
    }

    /// Évalue un `<script type="module">` dont le graphe est
    /// déjà rempli (prefetch détaché dans [`SubJob::run`]) —
    /// `eval_module` + `PipelineError` sans faire échouer la
    /// `PipelineError` dédupliqué (B-12) — une erreur en
    /// rafale (`heap memory limit` ×14 bookshop, `X is
    /// not defined` ×512 genius) noyait le ring console :
    /// 1ère occurrence émise, répétitions *consécutives*
    /// identiques tues et re-émises toutes les 16 avec
    /// `×N`. Un message différent réarme la série.
    fn emit_pipeline_error(&mut self, tab: TabId, message: String) {
        let n = match self.last_pipeline_error.as_mut() {
            Some((t, m, n)) if *t == tab && *m == message => {
                *n += 1;
                *n
            }
            _ => {
                self.last_pipeline_error = Some((tab, message.clone(), 1));
                1
            }
        };
        if n == 1 || n % 16 == 0 {
            let msg = if n > 1 {
                format!("{message} — ×{n}")
            } else {
                message
            };
            self.emit_pipeline_error(tab, msg);
        }
    }

    /// page.
    pub(crate) fn eval_module_graph(&mut self, tab: TabId, eng: &mut dyn JsEngine, me: &ModEval) {
        tracing::debug!(tab, name = %me.name, "module eval…");
        let t0 = std::time::Instant::now();
        if let Err(e) = Self::eval_mod(eng, me) {
            tracing::debug!(tab, error = %e, "module en échec");
            self.emit_pipeline_error(tab, format!("{e} — module «{}»", me.name));
        } else {
            tracing::debug!(tab, ms = t0.elapsed().as_millis(), "module eval fin");
        }
    }

    /// `eval_module` d'un [`ModEval`] sans `&self` (CAP-186) —
    /// appelable sous emprunt `entry` (replay `js_pending_mod`).
    /// Résolution identique : bare hors import map =
    /// « unresolvable » (issue #6), relatif via `alias`.
    fn eval_mod(eng: &mut dyn JsEngine, me: &ModEval) -> Result<(), explorer_js::JsError> {
        let ModuleGraph { sources, alias } = &me.graph;
        let alias = alias.clone();
        let resolve: UrlResolver = Arc::new(move |spec, referrer| {
            if explorer_js::is_bare_specifier(spec) {
                return None;
            }
            let b = alias.get(referrer).map_or(referrer, String::as_str);
            Url::parse(b)
                .ok()
                .and_then(|u| Url::resolve(&u, spec).ok())
                .map(|u| u.as_str())
        });
        eng.eval_module(&me.name, sources.clone(), resolve)
            .map(|_| ())
    }

    /// Contexte JS de chargement : moteur neuf + miroir posé —
    /// `want = false` (aucun script collecté) → `None`, pas de
    /// moteur créé (règle budgets).
    /// `referrer` (CAP-156) : le `Referer` effectivement
    /// envoyé par la navigation du document — servi à
    /// `document.referrer` (`""` hors navigation référée).
    pub(crate) fn spawn_engine(
        &self,
        tab: TabId,
        html: &str,
        want: bool,
        referrer: &str,
    ) -> Option<Box<dyn JsEngine>> {
        if !want {
            return None;
        }
        self.js_factory.clone().map(|f| {
            let (page_url, hist_len) = self.tabs.get(&tab).map_or_else(
                || (String::new(), 1),
                |t| {
                    (
                        t.url.as_ref().map_or_else(String::new, Url::to_string),
                        u32::try_from(t.hist.len()).unwrap_or(u32::MAX),
                    )
                },
            );
            let mut e = f();
            e.set_budget(self.js_ops);
            e.set_mem_limit(self.js_mem_bytes);
            // Viewport session (CAP-161) — avant `install_dom`
            // pour que `innerWidth`/`screen.*`/`matchMedia`
            // voient la taille réelle (sinon `(0,0)`).
            // px CSS ≪ 2^32 en pratique — `as` sature déjà.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            e.set_viewport(
                self.viewport.0.max(0.0).round() as u32,
                self.viewport.1.max(0.0).round() as u32,
            );
            e.install_dom(html, &page_url, hist_len);
            // « loading » jusqu'au DCL de commit (net.rs) —
            // retient les `postMessage` entrants tant que les
            // scripts du document n'ont pas tourné (étape 130 :
            // un cadre qui poste pendant son propre parse ne
            // devance pas les listeners du parent).
            e.set_ready_state(explorer_js::JS_CTX_TOP, "loading");
            e.set_page_referrer(referrer);
            // `window.name` (CAP-159) — nom du contexte posé
            // par l'ouverture nommée (`target="x"`/
            // `window.open(u,"x")`), `""` sinon.
            if let Some(n) = self.tabs.get(&tab).and_then(|t| t.win_name.as_deref()) {
                e.set_ctx_name(explorer_js::JS_CTX_TOP, n);
            }
            // `matchMedia` (CAP-161) — snapshot de l'env de
            // session : `prefers-color-scheme` ; desktop →
            // `hover`/`pointer: fine`, pas de reduced-motion
            // (borné — pas de `change`). Le viewport est
            // lu en direct (`hooks.viewport`, posé par
            // `install_dom`/`set_viewport`).
            e.set_media_env(explorer_js::MediaEnv {
                dark: self.color_scheme == explorer_css::ColorScheme::Dark,
                reduced_motion: false,
                hover: true,
                pointer_fine: true,
            });
            // Issue #19 — cellule de géométrie publiée, posée
            // sur le document du contexte TOP (`Dom::geometry`
            // — document-local, swap/bfcache sain ; journal
            // interface). `Tab::geom` survit aux navigations
            // (un moteur par document, la même cellule repasse) ;
            // la session la remplit à chaque commit paint,
            // `getBoundingClientRect`/`offset*` la lisent en
            // direct sur le worker (pas d'appel synchrone).
            if let Some(t) = self.tabs.get(&tab) {
                e.set_geometry_ctx(explorer_js::JS_CTX_TOP, t.geom.clone());
            }
            // Étape 51 : état d'origine injecté au contexte —
            // `localStorage` (map session > profil persisté)
            // et snapshot cookie non-HttpOnly
            // (`document.cookie` en lecture).
            if let Ok(u) = Url::parse(&page_url) {
                let key = u.origin().to_string();
                let local = self
                    .local_storage
                    .get(&key)
                    .cloned()
                    .or_else(|| self.store.as_ref().map(|s| s.load_storage(&key)))
                    .unwrap_or_default();
                e.install_storage(local);
                e.set_cookie_snapshot(&self.cookies.header_for(&u, false));
            }
            e
        })
    }

    /// Étape 134 — câblage `SubFrame` → contexte JS : un
    /// realm par `<iframe>` rattaché à un sous-document
    /// fetché, créé **avant** les scripts du parent (un
    /// `frames[n]`/`contentWindow` lu en cours de parse est
    /// alors réel). Pour chaque cadre : `context_new` →
    /// `install_dom_ctx` (miroir + timers + transports de
    /// l'origine enfant — hooks nav absents par design) →
    /// eval des `<script>` **inline** du sous-document (les
    /// `src=` externes ne sont pas fetchés à cette tranche)
    /// → `DOMContentLoaded`. `load` est tiré au tail par
    /// [`Self::frame_ctx_load_events`]. Remplit
    /// `entry.frame_ctxs`.
    pub(crate) fn wire_frame_contexts(&mut self, tab: TabId, eng: &mut dyn JsEngine) {
        let Some(entry) = self.tabs.get(&tab) else {
            return;
        };
        let plan = frame_context_plan(entry);
        if plan.is_empty() {
            return;
        }
        let mut fcs = Vec::with_capacity(plan.len());
        for fp in plan {
            // Realm de l'enfant — `frame_ord` le relie à son
            // élément `<iframe>` pour `contentWindow`.
            let Some(ctx) = eng.context_new(&fp.url.to_string(), Some(fp.ord)) else {
                continue;
            };
            // Origine opaque (CAP-149) : storage éphémère
            // propre au contexte (`opaque:{ctx}` — jamais
            // chargé ni persisté) et `document.cookie`
            // vide/muet ; sinon l'état réel de l'origine.
            let (local, cookie) = if fp.opaque {
                (
                    self.local_storage
                        .get(&format!("opaque:{ctx}"))
                        .cloned()
                        .unwrap_or_default(),
                    String::new(),
                )
            } else {
                self.origin_state(&fp.url)
            };
            eng.install_dom_ctx(ctx, &fp.html, &fp.url.to_string(), local, &cookie);
            // « loading » jusqu'au cycle interactive/complete
            // ci-dessous — retient les `postMessage` entrants
            // pendant les scripts du cadre (étape 130).
            eng.set_ready_state(ctx, "loading");
            for src in &fp.scripts {
                self.eval_ctx_or_report(tab, eng, ctx, src);
            }
            // Cycle de vie du sous-document : `readyState`
            // « interactive » + `readystatechange` puis DCL
            // après ses scripts (CAP-170 ; le `load` enfant
            // est tiré au tail).
            eng.set_ready_state(ctx, "interactive");
            let _ = eng.dispatch_event_ctx(ctx, "readystatechange", &[]);
            let _ = eng.dispatch_event_ctx(ctx, "DOMContentLoaded", &[]);
            fcs.push(FrameCtx {
                ctx,
                sub: fp.sub,
                url: fp.url,
                node: fp.node,
                opaque: fp.opaque,
                load_fired: false,
            });
        }
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.frame_ctxs = fcs;
        }
    }

    /// État d'origine pour un contexte enfant (étape 134) —
    /// même logique que la branche storage de
    /// [`Self::spawn_engine`] : `localStorage` persisté de
    /// l'origine **de l'enfant** et vue `document.cookie`
    /// non-HttpOnly pour son URL.
    pub(crate) fn origin_state(
        &self,
        url: &Url,
    ) -> (std::collections::BTreeMap<String, String>, String) {
        let key = url.origin().to_string();
        let local = self
            .local_storage
            .get(&key)
            .cloned()
            .or_else(|| self.store.as_ref().map(|s| s.load_storage(&key)))
            .unwrap_or_default();
        (local, self.cookies.header_for(url, false))
    }

    /// `eval_ctx` avec rapport — même discipline que
    /// [`Self::eval_or_report`] (`PipelineError`, jamais de
    /// page cassée), adapté au realm `ctx`.
    fn eval_ctx_or_report(&mut self, tab: TabId, eng: &mut dyn JsEngine, ctx: u32, src: &str) {
        // Disjoncteur (vol 29 doctolib) : une éval déjà
        // tombée en timeout fige le worker 30 s — relancer
        // les scripts suivants dans le même commit = N×30 s
        // de verrou → daemon entier mort. On saute l'appel.
        if self.js_stalled(tab) {
            tracing::debug!(
                tab,
                ctx,
                bytes = src.len(),
                "script iframe sauté — disjoncteur moteur ouvert"
            );
            return;
        }
        if let Err(e) = eng.eval_ctx(ctx, src, None) {
            tracing::debug!(tab, ctx, error = %e, "script iframe en échec");
            if Self::is_engine_stall(&e) {
                self.mark_js_stall(tab, &e);
            }
            self.emit_pipeline_error(tab, format!("{e} — iframe — src «{}»", src_head(src)));
        }
    }

    /// Rejet de promesse non rattrapé → `PipelineError`
    /// (vol : mort silencieuse type pinterest —
    /// `__PWS_ROOT__` vide sans aucune trace).
    ///
    /// **Couture en attente** : le trait `JsEngine`
    /// n'expose pas encore de drain pour les rejets (les
    /// `onunhandledrejection` émis côté jscore restent
    /// internes au moteur — chantier FIX-1). Ce routeur est
    /// prêt : le jour où `drain_unhandled_rejections`
    /// (ou équivalent) arrive, il suffit de l'appeler aux
    /// points de synchro et de repasser chaque message ici —
    /// le `PipelineError` part déjà vers le ring `/console`.
    // Aucun drain ne l'appelle encore (couture jscore
    // attendue) — l'avertissement est donc intentionnel.
    #[allow(dead_code)]
    pub(crate) fn report_unhandled_rejection(&mut self, tab: TabId, message: &str) {
        tracing::warn!(tab, error = %message, "promesse rejetée non rattrapée");
        self.emit_pipeline_error(tab, format!("unhandledrejection — {message}"));
    }

    /// `true` si l'erreur trahit un worker moteur saturé ou
    /// mort : `Dead`, ou éval/drain dépassé — `Timeout` est
    /// le kind typé depuis JS-C156 (FIX-1) ; le test de
    /// chaîne `Uncaught("moteur figé — …")` reste en repli
    /// pour les moteurs sans variante dédiée.
    pub(crate) fn is_engine_stall(e: &explorer_js::JsError) -> bool {
        matches!(
            e,
            explorer_js::JsError::Dead | explorer_js::JsError::Timeout
        ) || matches!(e, explorer_js::JsError::Uncaught(m) if m.starts_with("moteur fig\u{e9}"))
    }

    /// Disjoncteur « moteur figé » actif sur l'onglet ?
    /// (vol 25 : chaque appel bloquant au worker saturé
    /// patiente `EVAL_CMD_TIMEOUT` = 30 s — sous verrou, une
    /// rafale de livrées figeait l'API ~90 s sur kayak).
    /// L'échéance passée rouvre d'elle-même — lecture
    /// paresseuse, aucune tâche de réarmement.
    pub(crate) fn js_stalled(&self, tab: TabId) -> bool {
        self.tabs
            .get(&tab)
            .is_some_and(|e| e.js_stall_until.is_some_and(|u| Instant::now() < u))
    }

    /// Ouvre le disjoncteur pour `js_stall_cooldown` : les
    /// appels moteur **bloquants** de l'onglet sont
    /// court-circuités — le verrou session repasse aux
    /// handlers HTTP entre deux pompes plutôt que d'attendre
    /// un worker inerte. `cooldown = 0` désactive.
    fn mark_js_stall(&mut self, tab: TabId, e: &explorer_js::JsError) {
        if self.js_stall_cooldown.is_zero() {
            return;
        }
        if let Some(entry) = self.tabs.get_mut(&tab) {
            if entry.js_stall_until.is_none_or(|u| Instant::now() >= u) {
                tracing::warn!(tab, error = %e,
                    "moteur figé — disjoncteur ouvert, appels JS court-circuités");
            }
            entry.js_stall_until = Some(Instant::now() + self.js_stall_cooldown);
        }
    }

    /// `load` du document enfant dans son contexte, puis
    /// `load` sur l'élément `<iframe>` côté parent
    /// (`onload=` — étape 134). Appelé au `sub_commit_tail`
    /// **avant** le `load` du document top-level (spec : le
    /// `load` de la page attend ses sous-contextes).
    pub(crate) fn frame_ctx_load_events(&mut self, tab: TabId) {
        // CAP-171 : un `<iframe>` dynamique installé pendant
        // les scripts du commit a déjà tiré son cycle —
        // `load_fired` le saute ici (jamais de double
        // `readystatechange`/`load`).
        let fcs: Vec<(u32, NodeId)> = self.tabs.get_mut(&tab).map_or_else(Vec::new, |e| {
            e.frame_ctxs
                .iter_mut()
                .filter_map(|f| {
                    (!f.load_fired).then(|| {
                        f.load_fired = true;
                        (f.ctx, f.node)
                    })
                })
                .collect()
        });
        for (ctx, node) in fcs {
            // L'événement vit dans le realm enfant (chemin
            // vide = document de l'enfant) ; sa mutation
            // éventuelle est synchronisée par le prochain
            // `sync_frame_mirrors` (appelé via after_script).
            {
                let Some(entry) = self.tabs.get_mut(&tab) else {
                    return;
                };
                let Some(eng) = entry.js.as_mut() else {
                    return;
                };
                // `complete` + `readystatechange` dans le realm
                // enfant **avant** son `load` (CAP-170, spec —
                // `readyState === "complete"` dans les
                // listeners `load`).
                eng.set_ready_state(ctx, "complete");
                let _ = eng.dispatch_event_ctx(ctx, "readystatechange", &[]);
                let _ = eng.dispatch_event_ctx(ctx, "load", &[]);
            }
            // `onload` sur l'élément <iframe> — le chemin est
            // re-résolu sur le DOM courant (le `NodeId` peut
            // être mort après un re-parse de synchro : le
            // dispatch dégrade en `Ok(false)`, jamais de
            // livraison au mauvais nœud).
            self.dispatch_dom_event(tab, "load", node);
        }
    }

    /// Synchro miroir → `SubFrame.dom` (étape 134) : chaque
    /// contexte enfant sale est re-sérialisé et re-parsé en
    /// `entry.frames[sub].dom` — le composite re-rend le
    /// cadre au prochain relayout (bitmap `(url,w,h)` purgée
    /// — même clé, contenu neuf). Appelé aux mêmes points
    /// de synchro que les drains script ([`Self::after_script`]).
    pub(crate) fn sync_frame_mirrors(&mut self, tab: TabId) {
        let mut touched = Vec::new();
        {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return;
            };
            let Some(eng) = entry.js.as_mut() else {
                return;
            };
            for fc in &entry.frame_ctxs {
                if let Some(html) = eng.sync_dom_ctx(fc.ctx) {
                    touched.push((fc.sub, html));
                }
            }
            for (sub, html) in &touched {
                if let Some(f) = entry.frames.get_mut(*sub) {
                    // La surface composite est clée par URL —
                    // le contenu change : purger toutes ses
                    // tailles pour forcer la re-rasterisation.
                    let url = f.url.to_string();
                    entry.frame_bitmaps.retain(|(u, ..), _| *u != url);
                    // CAP-155 : le re-parse du miroir enfant
                    // reprend son scripting (un cadre sans
                    // `allow-scripts` affiche `<noscript>`).
                    let scripting = f.sandbox.is_none_or(|s| s.scripts);
                    f.dom = explorer_html::TreeBuilder::parse_scripting(html, scripting);
                }
            }
        }
        if !touched.is_empty() {
            // Un relayout rejoue la passe composite sur les
            // DOM frais — `relayout` est tolérant à l'absence
            // d'arbre posé (commit encore en vol).
            let _ = self.relayout(tab);
        }
    }
    /// Le message porte l'en-tête de la source fautive —
    /// « Error: b » seul ne localise rien sur du minifié
    /// (triage M-H1 : trouver quel `<script>` jette est la
    /// moitié du diagnostic).
    /// `url` = `src` résolue du script externe — étiquette les
    /// frames `error.stack` (`eval_named`, JS-C56) et préfixe le
    /// rapport d'erreur.
    /// `el_ord` (CAP-168) = ordinal de l'élément `<script>`
    /// parmi tous ceux du document — `document.currentScript`
    /// le reflète pendant l'éval ; `None` = pas d'élément
    /// porteur (`javascript:`, eval direct).
    pub(crate) fn eval_or_report(
        &mut self,
        tab: TabId,
        eng: &mut dyn JsEngine,
        src: &str,
        url: Option<&str>,
        el_ord: Option<usize>,
    ) {
        // Disjoncteur (vol 29 doctolib) : après un timeout,
        // chaque éval suivante attendrait encore 30 s sous
        // verrou — 8 bundles = ~4 min d'API morte. On saute
        // l'appel et on rapporte en PipelineError (visible
        // console, comme un échec d'éval normal).
        if self.js_stalled(tab) {
            tracing::warn!(
                tab,
                bytes = src.len(),
                "script sauté — disjoncteur moteur ouvert"
            );
            let origin = url.map(|u| format!("{u} — ")).unwrap_or_default();
            self.emit_pipeline_error(
                tab,
                format!(
                    "moteur figé — éval sautée — {origin}src «{}»",
                    src_head(src)
                ),
            );
            return;
        }
        let t_eval = std::time::Instant::now();
        tracing::info!(tab, bytes = src.len(), "script: eval…");
        let r = match el_ord {
            Some(ord) => eng.eval_script_el(explorer_js::JS_CTX_TOP, src, url, ord),
            None => match url {
                Some(u) => eng.eval_named(src, u),
                None => eng.eval(src),
            },
        };
        tracing::info!(
            tab,
            ms = u64::try_from(t_eval.elapsed().as_millis()).unwrap_or(u64::MAX),
            ok = r.is_ok(),
            "script: eval fin"
        );
        if let Err(e) = r {
            tracing::debug!(tab, error = %e, "script en échec");
            if Self::is_engine_stall(&e) {
                self.mark_js_stall(tab, &e);
            }
            let origin = url.map(|u| format!("{u} — ")).unwrap_or_default();
            self.emit_pipeline_error(tab, format!("{e} — {origin}src «{}»", src_head(src)));
        }
    }

    /// Dispatche l'événement `ty` sur `node` dans le miroir JS
    /// puis resynchronise les mutations DOM. Retourne
    /// `defaultPrevented`.
    pub(crate) fn dispatch_dom_event(&mut self, tab: TabId, ty: &str, node: NodeId) -> bool {
        let path = self
            .tabs
            .get(&tab)
            .and_then(|entry| entry.dom.as_ref().and_then(|d| element_path(d, node)));
        let Some(path) = path else { return false };
        self.dispatch_path_event(tab, ty, &path)
    }

    /// Événement niveau document (étape 52) — cible `<body>`
    /// côté miroir : `scroll`, `keydown`/`keyup`/`wheel` sans
    /// nœud porteur.
    pub(crate) fn dispatch_doc_event(&mut self, tab: TabId, ty: &str) -> bool {
        self.dispatch_path_event(tab, ty, &[])
    }

    /// `document.readyState` pour le contexte `ctx` — pendant
    /// le détachement (53f) l'étape est tamponnée puis
    /// rejouée à la réinsertion, dans l'ordre : le tail de
    /// commit peut courir pendant une pompe détachée, et le
    /// drop muet laissait `readyState` à `"loading"` pour
    /// toujours (les portes `readystatechange` des pages —
    /// github `element-registry` — pendaient sans erreur).
    pub(crate) fn set_doc_ready_state(
        &mut self,
        tab: TabId,
        ctx: explorer_js::JsCtxId,
        state: &'static str,
    ) {
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        if let Some(eng) = entry.js.as_mut() {
            eng.set_ready_state(ctx, state);
        } else if entry.js_detached.is_some()
            && entry.pending_doc_lifecycle.len() < MAX_PENDING_LIFECYCLE
        {
            entry
                .pending_doc_lifecycle
                .push(types::DocLifecycle::ReadyState(ctx, state.to_string()));
        }
    }

    /// Cœur du dispatch : `path` vers le miroir (vide =
    /// document), resynchronisation + drains des surfaces
    /// script (étapes 50-51) — point de synchro partagé par
    /// les événements utilisateur et le tick timers.
    pub(crate) fn dispatch_path_event(
        &mut self,
        tab: TabId,
        ty: &str,
        path: &[(String, u32)],
    ) -> bool {
        self.dispatch_path_event_props(tab, ty, path, &[])
    }

    /// Variante à props de [`Self::dispatch_path_event`]
    /// (étape 103) — `props` devient les propriétés de
    /// l'événement (`KeyboardEvent.key`/`code`/`keyCode`…).
    pub(crate) fn dispatch_path_event_props(
        &mut self,
        tab: TabId,
        ty: &str,
        path: &[(String, u32)],
        props: &[(String, EventProp)],
    ) -> bool {
        let t0 = Instant::now();
        let cooldown = self.js_stall_cooldown;
        let mut emit_err = None;
        let (prevented, mut t, mutated) = {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return false;
            };
            // Disjoncteur (vol 25) : dispatch vers un worker
            // figé = 30 s de verrou pour rien.
            if entry.js_stall_until.is_some_and(|u| Instant::now() < u) {
                return false;
            }
            let Some(eng) = entry.js.as_mut() else {
                // Moteur détaché (53f) : un événement **cible
                // document** est tamponné puis rejoué à la
                // réinsertion — sinon `readystatechange`/
                // `DOMContentLoaded`/`load` du tail de
                // commit mouraient sans trace quand une
                // pompe tenait le moteur (readyState figé à
                // `"loading"`, portes des pages pendantes).
                // Les événements à `path`/`props` gardent la
                // voie existante (entrées → `deferred_inputs`).
                if entry.js_detached.is_some()
                    && path.is_empty()
                    && props.is_empty()
                    && entry.pending_doc_lifecycle.len() < MAX_PENDING_LIFECYCLE
                {
                    entry.pending_doc_lifecycle.push(types::DocLifecycle::Event(
                        explorer_js::JS_CTX_TOP,
                        ty.to_string(),
                    ));
                }
                return false;
            };
            let p = match eng.dispatch_event_props(ty, path, props) {
                Ok(p) => p,
                Err(e) => {
                    if Self::is_engine_stall(&e) && !cooldown.is_zero() {
                        entry.js_stall_until = Some(Instant::now() + cooldown);
                    }
                    tracing::debug!(tab, ty, error = %e, "listener dom en échec");
                    emit_err = Some(e.to_string());
                    false
                }
            };
            let t1 = sync_phase(tab, "event.dispatch", t0);
            let n = eng.run_pending();
            tracing::trace!(tab, jobs = n, "jobs js après événement");
            let m = Self::sync_for_apply(eng.as_mut());
            (p, sync_phase(tab, "event.mirror_sync", t1), m)
        };
        if let Some(m) = emit_err {
            self.emit_pipeline_error(tab, m);
        }
        if let Some(s) = mutated {
            let ta = Instant::now();
            // Mutation enfilée — l'apply tourne hors verrou
            // chez les hôtes partagés (étape 55h) ; le drain
            // de compat `handle_input`/`drain_applies` garde
            // la sémantique synchrone.
            self.queue_apply_sync(tab, s);
            t = sync_phase(tab, "event.apply_queued", ta);
        }
        // Les listeners ont pu émettre XHR/cookies/storage
        // (étape 51) — drains + livraison de ce qui est déjà
        // arrivé, sans yield (point de synchro interne).
        self.after_script(tab);
        let t = sync_phase(tab, "event.after_script", t);
        self.poll_net();
        sync_phase(tab, "event.poll_net", t);
        prevented
    }

    /// `keydown`/`keyup` : cible = champ focalisé s'il vit
    /// encore dans `dom`, document sinon. `key` devient les
    /// props `KeyboardEvent` (`key`/`code`/`keyCode`/`which`,
    /// étape 103 ; `ctrlKey`/`shiftKey`/`altKey`/`metaKey`
    /// depuis `mods`, étape 105). Retourne `defaultPrevented`.
    pub(crate) fn dispatch_key(&mut self, tab: TabId, ty: &str, key: &str, mods: KeyMods) -> bool {
        let props = key_event_props(key, mods);
        let path = match self.focused_node(tab) {
            Some(n) => self
                .tabs
                .get(&tab)
                .and_then(|entry| entry.dom.as_ref().and_then(|d| element_path(d, n))),
            // Rien de focalisé → `document.activeElement`
            // spec = `<body>` : les écouteurs posés sur
            // `document.body` (jeux clavier type wordle,
            // vol 28) doivent recevoir la frappe — le
            // document nu reste le repli sans body.
            None => self
                .tabs
                .get(&tab)
                .and_then(|entry| entry.dom.as_ref())
                .map(|d| {
                    descendant_elements(d, d.document(), "body")
                        .first()
                        .and_then(|b| element_path(d, *b))
                        .unwrap_or_default()
                }),
        };
        match path {
            Some(p) => self.dispatch_path_event_props(tab, ty, &p, &props),
            None => false,
        }
    }

    /// `wheel` : cible = nœud survolé s'il vit encore,
    /// document sinon. Retourne `defaultPrevented` — le
    /// scroll est alors supprimé.
    pub(crate) fn dispatch_wheel(&mut self, tab: TabId) -> bool {
        let path = self
            .tabs
            .get(&tab)
            .and_then(|t| t.hovered.clone())
            .filter(|p| {
                self.tabs
                    .get(&tab)
                    .and_then(|t| t.dom.as_ref())
                    .is_some_and(|d| dom_path_node(d, p).is_some())
            });
        match path {
            Some(p) => self.dispatch_path_event(tab, "wheel", &p),
            None => self.dispatch_doc_event(tab, "wheel"),
        }
    }

    /// Résout `path` vers le `NodeId` courant de `dom` —
    /// re-résolution après re-parse de synchro (étape 52).
    pub(crate) fn resolve_path(
        &self,
        tab: TabId,
        path: Option<&[(String, u32)]>,
    ) -> Option<NodeId> {
        path.and_then(|p| {
            self.tabs
                .get(&tab)
                .and_then(|e| e.dom.as_ref())
                .and_then(|d| dom_path_node(d, p))
        })
    }

    /// Pousse l'état `attr` d'un nœud hôte vers le miroir JS —
    /// les handlers d'événements lisent l'état courant quand
    /// la session a muté le document hors script (champ
    /// édité → `value`, case basculée → `checked`).
    pub(crate) fn sync_mirror_attr(
        &mut self,
        tab: TabId,
        node: NodeId,
        attr: &str,
        value: Option<&str>,
    ) {
        let path = self
            .tabs
            .get(&tab)
            .and_then(|e| e.dom.as_ref())
            .and_then(|d| element_path(d, node));
        let Some(path) = path else { return };
        let Some(entry) = self.tabs.get_mut(&tab) else {
            return;
        };
        // Moteur détaché (53f) : le push est rejoué à la
        // réinsertion **avant** la re-sérialisation miroir —
        // sinon la saisie utilisateur pendant le burst serait
        // écrasée par l'ancien miroir.
        if entry.js_detached.is_some() {
            entry
                .js_pending_attrs
                .push((path, attr.to_string(), value.map(str::to_string)));
            return;
        }
        if let Some(eng) = entry.js.as_mut() {
            eng.sync_mirror_attr(&path, attr, value);
        }
    }
}

/// Plan d'un realm enfant (étape 134) — collecte immuable
/// avant la création des contextes : rang document,
/// `NodeId` de l'élément `<iframe>`, index dans
/// `Tab::frames`, URL, HTML source et scripts inline à
/// évaluer.
struct FramePlan {
    ord: u32,
    node: NodeId,
    sub: usize,
    url: Url,
    html: String,
    scripts: Vec<String>,
    /// Origine opaque (`sandbox` sans
    /// `allow-same-origin`, CAP-149).
    opaque: bool,
}

/// Phase collecte de [`Session::wire_frame_contexts`] :
/// `<iframe>` en ordre document → `FramePlan`. Même
/// correspondance par URL résolue que le composite (étape
/// 129) : deux cadres homonymes partagent le `SubFrame`
/// mais reçoivent chacun un realm propre (spec : realms
/// distincts).
fn frame_context_plan(entry: &Tab) -> Vec<FramePlan> {
    let mut plan = Vec::new();
    let (Some(dom), Some(base)) = (entry.dom.as_ref(), entry.url.as_ref()) else {
        return plan;
    };
    for (ord, node) in composite::iframe_nodes(dom).into_iter().enumerate() {
        // `srcdoc` prime sur `src` (spec, CAP-142) —
        // association par **contenu** comme le composite
        // (tous les `about:srcdoc` partagent leur URL) ;
        // `src` = par URL.
        let srcdoc = dom.attribute(node, "srcdoc").ok().flatten();
        let sub = if let Some(sd) = srcdoc.as_deref() {
            let Some(sub) = entry
                .frames
                .iter()
                .position(|f| f.srcdoc.as_deref() == Some(sd))
            else {
                continue;
            };
            sub
        } else {
            let Ok(Some(src)) = dom.attribute(node, "src") else {
                continue;
            };
            let Ok(url) = Url::resolve(base, &src) else {
                continue;
            };
            let Some(sub) = entry
                .frames
                .iter()
                .position(|f| f.srcdoc.is_none() && f.url == url)
            else {
                continue;
            };
            sub
        };
        let f = &entry.frames[sub];
        // Politiques du document enfant (132/CAP-145) —
        // `srcdoc` hérite base + CSP du parent ; un cadre
        // fetché suit sa `<base href>` propre et ses
        // `<meta>` CSP.
        let (f_base, f_csp) = if f.srcdoc.is_some() {
            (
                entry.nav_base().cloned().unwrap_or_else(|| base.clone()),
                entry.csp.clone(),
            )
        } else {
            (
                document_base(&f.dom, &f.url)
                    .0
                    .unwrap_or_else(|| f.url.clone()),
                collect_meta_csp(&f.dom)
                    .iter()
                    .map(|v| explorer_net::csp::CspPolicy::parse(v, true))
                    .collect::<Vec<_>>(),
            )
        };
        // Scripts **inline** de l'enfant, ordre document —
        // `max_src = 0` écarte les `src=` externes (sous-
        // ressources des cadres non fetchées à cette
        // tranche — journal) ; les modules n'ont pas de
        // chargeur en contexte. `sandbox` sans
        // `allow-scripts` (CAP-149) : le realm existe mais
        // aucun script ne s'exécute — collecte sautée.
        let sb = f.sandbox;
        let scripts = if sb.is_some_and(|s| !s.scripts) {
            Vec::new()
        } else {
            collect_scripts(&f.dom, &f.url, &f_base, true, 0, &f_csp)
                .into_iter()
                .filter_map(|i| (!i.module).then_some(i.inline).flatten())
                .collect()
        };
        plan.push(FramePlan {
            ord: u32::try_from(ord).unwrap_or(u32::MAX),
            node,
            sub,
            url: f.url.clone(),
            html: f.html.clone(),
            scripts,
            opaque: sb.is_some_and(|s| !s.same_origin),
        });
    }
    plan
}

/// Props `KeyboardEvent` d'une touche DOM `key` (étape 103) :
/// `key` tel quel, `code` = code physique UI Events
/// (`KeyA`/`Digit1`/`Space`…), `keyCode`/`which` = codes hérités
/// (lettres → ASCII majuscule, `Enter` 13, flèches 37-40…).
/// Touche hors table → `code` "" / `keyCode` 0 — tolérant,
/// jamais de panic.
fn key_event_props(key: &str, mods: KeyMods) -> Vec<(String, EventProp)> {
    let (code, key_code) = key_code_of(key);
    vec![
        ("key".to_string(), EventProp::Str(key.to_string())),
        ("code".to_string(), EventProp::Str(code)),
        ("keyCode".to_string(), EventProp::Num(f64::from(key_code))),
        ("which".to_string(), EventProp::Num(f64::from(key_code))),
        ("ctrlKey".to_string(), EventProp::Bool(mods.ctrl)),
        ("shiftKey".to_string(), EventProp::Bool(mods.shift)),
        ("altKey".to_string(), EventProp::Bool(mods.alt)),
        ("metaKey".to_string(), EventProp::Bool(mods.meta)),
    ]
}

/// `(code, keyCode)` d'une valeur DOM `key` — table UI Events
/// réduite aux touches livrées par le client (lettre ASCII,
/// chiffre, ponctuation US, touches nommées courantes, F1-F12).
/// `""` en code = touche hors table (comportement navigateur
/// pour les touches non identifiées).
fn key_code_of(key: &str) -> (String, u16) {
    match key {
        "Enter" => ("Enter".into(), 13),
        "Escape" | "Esc" => ("Escape".into(), 27),
        " " | "Spacebar" => ("Space".into(), 32),
        "Tab" => ("Tab".into(), 9),
        "Backspace" => ("Backspace".into(), 8),
        "Delete" => ("Delete".into(), 46),
        "Insert" => ("Insert".into(), 45),
        "Home" => ("Home".into(), 36),
        "End" => ("End".into(), 35),
        "PageUp" => ("PageUp".into(), 33),
        "PageDown" => ("PageDown".into(), 34),
        "ArrowUp" => ("ArrowUp".into(), 38),
        "ArrowDown" => ("ArrowDown".into(), 40),
        "ArrowLeft" => ("ArrowLeft".into(), 37),
        "ArrowRight" => ("ArrowRight".into(), 39),
        "Shift" => ("ShiftLeft".into(), 16),
        "Control" => ("ControlLeft".into(), 17),
        "Alt" => ("AltLeft".into(), 18),
        "Meta" => ("MetaLeft".into(), 91),
        "CapsLock" => ("CapsLock".into(), 20),
        "," => ("Comma".into(), 188),
        "." => ("Period".into(), 190),
        "/" => ("Slash".into(), 191),
        ";" => ("Semicolon".into(), 186),
        "=" => ("Equal".into(), 187),
        "-" => ("Minus".into(), 189),
        "[" => ("BracketLeft".into(), 219),
        "]" => ("BracketRight".into(), 221),
        "\\" => ("Backslash".into(), 220),
        "'" => ("Quote".into(), 222),
        "`" => ("Backquote".into(), 192),
        "F1" => ("F1".into(), 112),
        "F2" => ("F2".into(), 113),
        "F3" => ("F3".into(), 114),
        "F4" => ("F4".into(), 115),
        "F5" => ("F5".into(), 116),
        "F6" => ("F6".into(), 117),
        "F7" => ("F7".into(), 118),
        "F8" => ("F8".into(), 119),
        "F9" => ("F9".into(), 120),
        "F10" => ("F10".into(), 121),
        "F11" => ("F11".into(), 122),
        "F12" => ("F12".into(), 123),
        k if k.len() == 1 => match k.chars().next() {
            Some(c) if c.is_ascii_alphabetic() => {
                let u = c.to_ascii_uppercase();
                (format!("Key{u}"), u16::from(u as u8))
            }
            Some(c) if c.is_ascii_digit() => (format!("Digit{c}"), u16::from(c as u8)),
            // Ponctuation hors table OEM → repli `key`, keyCode 0.
            _ => (String::new(), 0),
        },
        _ => (String::new(), 0),
    }
}

/// En-tête de source fautive pour le diagnostic — espaces
/// compactés, 96 caractères (assez pour reconnaître le script
/// minifié coupable, assez court pour un event SSE).
fn src_head(src: &str) -> String {
    let collapsed: String = src.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut head: String = collapsed.chars().take(96).collect();
    if collapsed.chars().count() > 96 {
        head.push('…');
    }
    head
}

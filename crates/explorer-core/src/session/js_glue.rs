// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Colle JS : moteur extrait/rendu hors verrou, dispatch
//! d'événements, pompe de timers, sync miroir.
use super::*;
use std::sync::Arc;
use std::time::Instant;

use explorer_dom::NodeId;
use explorer_js::{DomSyncPayload, JsEngine, UrlResolver};
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
    pub fn pump_timers(&mut self, tab: TabId) -> usize {
        let max = self.max_timer_jobs;
        let t0 = Instant::now();
        let (jobs, mutated) = {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return 0;
            };
            let sync_ok = entry.sync_floor.is_none_or(|f| Instant::now() >= f);
            let Some(eng) = entry.js.as_mut() else {
                return 0;
            };
            let jobs = eng.run_pending_bounded(max);
            let m = if sync_ok { eng.sync_dom_path() } else { None };
            (jobs, m)
        };
        let mut t = sync_phase(tab, "pump.js_run_sync", t0);
        if let Some((html, dirty)) = mutated {
            let ta = Instant::now();
            self.apply_dom_sync(tab, &html, dirty.as_deref(), true);
            t = sync_phase(tab, "pump.apply_dom_sync", ta);
        }
        if jobs > 0 {
            // Les callbacks ont pu émettre XHR/cookies/storage.
            self.after_script(tab);
            self.poll_net();
            sync_phase(tab, "pump.drains", t);
        }
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
    pub(crate) fn commit_js_pump(
        &mut self,
        tab: TabId,
        epoch: u64,
        out: Option<(usize, Option<DomSyncPayload>, Box<dyn JsEngine>)>,
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
                entry.js_pending_attrs.clear();
                return 0;
            };
            if entry.nav_epoch != epoch {
                entry.js_pending_net.clear();
                entry.js_pending_attrs.clear();
                return 0;
            }
            for resp in std::mem::take(&mut entry.js_pending_net) {
                let _ = eng.deliver_net_response(resp);
            }
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
                    eng.sync_dom_path()
                } else {
                    None
                }
            };
            entry.js = Some(eng);
            (jobs, mutated)
        };
        let (jobs, mutated) = phase;
        let mut t = sync_phase(tab, "commit.deliver", t0);
        if let Some((html, dirty)) = mutated {
            let ta = Instant::now();
            self.apply_dom_sync(tab, &html, dirty.as_deref(), true);
            t = sync_phase(tab, "commit.apply_dom_sync", ta);
        }
        if jobs > 0 {
            self.after_script(tab);
            self.poll_net();
            sync_phase(tab, "commit.drains", t);
        }
        jobs
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
    /// page.
    pub(crate) fn eval_module_graph(&mut self, tab: TabId, eng: &mut dyn JsEngine, me: ModEval) {
        let ModuleGraph { sources, alias } = me.graph;
        let resolve: UrlResolver = Arc::new(move |spec, referrer| {
            let b = alias.get(referrer).map_or(referrer, String::as_str);
            Url::parse(b)
                .ok()
                .and_then(|u| Url::resolve(&u, spec).ok())
                .map(|u| u.as_str())
        });
        if let Err(e) = eng.eval_module(&me.name, sources, resolve) {
            tracing::debug!(tab, error = %e, "module en échec");
            let _ = self.events.send(CoreEvent::PipelineError {
                tab: Some(tab),
                message: e.to_string(),
            });
        }
    }

    /// Contexte JS de chargement : moteur neuf + miroir posé —
    /// `want = false` (aucun script collecté) → `None`, pas de
    /// moteur créé (règle budgets).
    pub(crate) fn spawn_engine(
        &self,
        tab: TabId,
        html: &str,
        want: bool,
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
            e.install_dom(html, &page_url, hist_len);
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

    /// `eval` avec rapport : un échec émet `PipelineError`
    /// sans faire échouer la page (dégradation, règle 3).
    pub(crate) fn eval_or_report(&mut self, tab: TabId, eng: &mut dyn JsEngine, src: &str) {
        if let Err(e) = eng.eval(src) {
            tracing::debug!(tab, error = %e, "script en échec");
            let _ = self.events.send(CoreEvent::PipelineError {
                tab: Some(tab),
                message: e.to_string(),
            });
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
        let t0 = Instant::now();
        let (prevented, mut t, mutated) = {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return false;
            };
            let Some(eng) = entry.js.as_mut() else {
                return false;
            };
            let p = match eng.dispatch_event(ty, path) {
                Ok(p) => p,
                Err(e) => {
                    tracing::debug!(tab, ty, error = %e, "listener dom en échec");
                    let _ = self.events.send(CoreEvent::PipelineError {
                        tab: Some(tab),
                        message: e.to_string(),
                    });
                    false
                }
            };
            let t1 = sync_phase(tab, "event.dispatch", t0);
            let n = eng.run_pending();
            tracing::trace!(tab, jobs = n, "jobs js après événement");
            let m = eng.sync_dom_path();
            (p, sync_phase(tab, "event.mirror_sync", t1), m)
        };
        if let Some((html, dirty)) = mutated {
            let ta = Instant::now();
            self.apply_dom_sync(tab, &html, dirty.as_deref(), true);
            t = sync_phase(tab, "event.apply_dom_sync", ta);
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
    /// encore dans `dom`, document sinon. Retourne
    /// `defaultPrevented`.
    pub(crate) fn dispatch_key(&mut self, tab: TabId, ty: &str) -> bool {
        match self.focused_node(tab) {
            Some(n) => self.dispatch_dom_event(tab, ty, n),
            None => self.dispatch_doc_event(tab, ty),
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

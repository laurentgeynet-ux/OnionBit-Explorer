// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Colle JS : moteur extrait/rendu hors verrou, dispatch
//! d'événements, pompe de timers, sync miroir.
use super::*;
use std::sync::Arc;
use std::time::Instant;

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
                    Self::sync_for_apply(eng.as_mut())
                } else {
                    None
                }
            };
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
            // Un bare arrivant ici est resté hors import map
            // (le moteur réécrit d'abord) — « unresolvable » par
            // spec, jamais un chemin relatif déguisé (issue #6).
            if explorer_js::is_bare_specifier(spec) {
                return None;
            }
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
                message: format!("{e} — module «{}»", me.name),
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
    /// Le message porte l'en-tête de la source fautive —
    /// « Error: b » seul ne localise rien sur du minifié
    /// (triage M-H1 : trouver quel `<script>` jette est la
    /// moitié du diagnostic).
    /// `url` = `src` résolue du script externe — étiquette les
    /// frames `error.stack` (`eval_named`, JS-C56) et préfixe le
    /// rapport d'erreur.
    pub(crate) fn eval_or_report(
        &mut self,
        tab: TabId,
        eng: &mut dyn JsEngine,
        src: &str,
        url: Option<&str>,
    ) {
        let t_eval = std::time::Instant::now();
        tracing::info!(tab, bytes = src.len(), "script: eval…");
        let r = match url {
            Some(u) => eng.eval_named(src, u),
            None => eng.eval(src),
        };
        tracing::info!(
            tab,
            ms = u64::try_from(t_eval.elapsed().as_millis()).unwrap_or(u64::MAX),
            ok = r.is_ok(),
            "script: eval fin"
        );
        if let Err(e) = r {
            tracing::debug!(tab, error = %e, "script en échec");
            let origin = url.map(|u| format!("{u} — ")).unwrap_or_default();
            let _ = self.events.send(CoreEvent::PipelineError {
                tab: Some(tab),
                message: format!("{e} — {origin}src «{}»", src_head(src)),
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
        let (prevented, mut t, mutated) = {
            let Some(entry) = self.tabs.get_mut(&tab) else {
                return false;
            };
            let Some(eng) = entry.js.as_mut() else {
                return false;
            };
            let p = match eng.dispatch_event_props(ty, path, props) {
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
            let m = Self::sync_for_apply(eng.as_mut());
            (p, sync_phase(tab, "event.mirror_sync", t1), m)
        };
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
            None => Some(Vec::new()),
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

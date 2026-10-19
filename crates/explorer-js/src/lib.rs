// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-js` — couture JavaScript (ADR-0007) : trait
//! [`JsEngine`], moteur nul par défaut et adaptateur
//! [`DilloEngine`] vers `dillo-js`.
//!
//! `dillo_js::Engine` est `!Send` (`Rc` dans le tas, les envs et les
//! hooks hôtes) alors que la session vit derrière un `tokio` mutex
//! exigeant `Send`. [`DilloEngine`] est donc un **proxy canal** : un
//! thread dédié par contexte possède le moteur (single-writer — le
//! modèle navigateur : un contexte JS par document, isolé), les
//! appels (`eval`, jobs dus, budget) voyagent par message. Un script
//! hostile ne peut pas suspendre le pipeline : le budget d'ops est
//! borné côté moteur (`set_op_limit`), et la couche appelante peut
//! borner l'attente si besoin.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;

/// Erreur JavaScript côté hôte — sérialisable, sans handle de tas.
#[derive(Debug, thiserror::Error)]
pub enum JsError {
    /// Source impossible à parser.
    #[error("syntaxe js {line}:{col} : {message}")]
    Syntax {
        /// Ligne (1-base).
        line: usize,
        /// Colonne (1-base).
        col: usize,
        /// Détail du parseur.
        message: String,
    },
    /// Le script a lancé une exception non rattrapée.
    #[error("exception js : {0}")]
    Uncaught(String),
    /// Le thread moteur est mort (panic interne, poison).
    #[error("moteur js inerte")]
    Dead,
}

impl From<dillo_js::Error> for JsError {
    fn from(e: dillo_js::Error) -> Self {
        match e {
            dillo_js::Error::Syntax { line, col, message } => Self::Syntax { line, col, message },
            dillo_js::Error::Uncaught(m) => Self::Uncaught(m),
        }
    }
}

/// Intention de navigation émise par un script
/// (`location.assign`/`href=`, `history.go`…) — drainée par
/// le hôte après l'appel JS (jamais de navigation re-entrante
/// pendant l'évaluation).
#[derive(Debug, Clone, PartialEq)]
pub enum NavRequest {
    /// `location.href =`/`assign` — cible brute à résoudre
    /// contre l'URL de la page.
    Navigate(String),
    /// `history.go(n)`/`back()`/`forward()` — déplacement
    /// relatif dans l'historique de session.
    HistoryGo(i32),
}

/// Résolveur de spécificateurs de module fourni par l'hôte :
/// `(specifier, referrer) → url absolue`. La session y injecte
/// sa politique `Url::resolve` — le moteur ne connaît pas les
/// URLs (étape 50).
pub type UrlResolver = Arc<dyn Fn(&str, &str) -> Option<String> + Send + Sync>;

/// Requête réseau émise par un script — `XMLHttpRequest` ou
/// `fetch()` (JS-C27). Le moteur ne touche jamais le réseau :
/// la session draine aux points de synchro, exécute via
/// `HttpTransport` (sa politique : origine, CORS, quotas —
/// ADR-0019) puis livre par [`JsEngine::deliver_net_response`].
#[derive(Debug, Clone, PartialEq)]
pub enum NetRequest {
    /// À exécuter — `handle` à répercuter dans [`NetResponse`].
    Fetch {
        /// Identifiant alloué par le moteur (unique par contexte).
        handle: u64,
        /// Méthode HTTP telle qu'écrite.
        method: String,
        /// URL telle qu'écrite — résolution à l'URL de page et
        /// politique = affaire de la session.
        url: String,
        /// En-têtes `setRequestHeader`/opts, dans l'ordre.
        headers: Vec<(String, String)>,
        /// Corps `send(body)`/`fetch({body})`, si présent.
        body: Option<String>,
    },
    /// `xhr.abort()` — annuler la tâche `handle` si encore en vol.
    Abort {
        /// Identifiant de la requête à annuler.
        handle: u64,
    },
}

/// Réponse livrée au moteur pour un `handle` drainé —
/// `status == 0` = échec réseau (event `error`, rejet de
/// promesse `fetch`). Miroir sérialisable de
/// `dillo_js::XhrResponse` (aucun type moteur ne fuit).
#[derive(Debug, Clone)]
pub struct NetResponse {
    /// Identifiant rendu par [`NetRequest::Fetch`].
    pub handle: u64,
    /// Statut HTTP (`0` = échec transport).
    pub status: u16,
    /// Phrase de raison.
    pub status_text: String,
    /// En-têtes de réponse.
    pub headers: Vec<(String, String)>,
    /// Corps brut — `responseText`/`text()` le décode en lossy.
    pub body: Vec<u8>,
}

/// Moteur JavaScript vu du navigateur — un contexte par document.
/// `Send` exigé : la session est partagée derrière l'API.
pub trait JsEngine: Send {
    /// Évalue un script dans le contexte ; retourne la valeur de
    /// complétion rendue lisible (`describe`).
    ///
    /// # Errors
    /// [`JsError::Syntax`] à l'analyse, [`JsError::Uncaught`] si le
    /// script jette, [`JsError::Dead`] si le moteur est inerte.
    fn eval(&mut self, source: &str) -> Result<String, JsError>;

    /// Vide les jobs dus (microtasks, timers échus) — retourne le
    /// nombre exécuté. Défaut : rien à faire.
    fn run_pending(&mut self) -> usize {
        0
    }

    /// Installe le DOM miroir du document (bindings `document`,
    /// `navigator`, `window`…). `html` = source du document tel
    /// que reçu — le miroir le re-parse côté moteur.
    /// `page_url` alimente `location`/`document.URL` ;
    /// `history_len` alimente `history.length`.
    /// Défaut : pas de bindings (moteur nu).
    fn install_dom(&mut self, _html: &str, _page_url: &str, _history_len: u32) {}

    /// Draine les intentions de navigation accumulées depuis
    /// le dernier appel — `location`/`history` n'agissent
    /// jamais en plein vol, la session décide après retour.
    /// Défaut : rien.
    fn drain_nav_requests(&mut self) -> Vec<NavRequest> {
        Vec::new()
    }

    /// Synchronise le DOM miroir → `Some(html)` si des mutations
    /// JS ont eu lieu depuis la dernière synchro (le hôte
    /// re-parse alors le document), `None` sinon.
    fn sync_dom(&mut self) -> Option<String> {
        None
    }

    /// Évalue un module ES (étape 50) : `entry` est le nom
    /// canonique du module d'entrée (URL résolue — ou nom
    /// synthétique aliasé pour un module inline), `sources` la
    /// map `url → source` **préfetchée** par la session (graphe
    /// statique via `dillo_js::module_specifiers`), `resolve` la
    /// résolution `(specifier, referrer) → url`. Le chargeur ne
    /// fait jamais de fetch : un `import` hors map est une erreur
    /// JS propre — le canal reste à sens unique, jamais de
    /// ré-entrance réseau depuis le thread moteur.
    /// Défaut : `Ok("undefined")` (moteur sans modules).
    ///
    /// # Errors
    /// Mêmes variantes que [`Self::eval`] — `Syntax` à l'analyse
    /// (y compris erreur de liaison), `Uncaught` si un corps de
    /// module jette, `Dead` si le moteur est inerte.
    fn eval_module(
        &mut self,
        _entry: &str,
        _sources: HashMap<String, String>,
        _resolve: UrlResolver,
    ) -> Result<String, JsError> {
        Ok("undefined".to_string())
    }

    /// Dispatche l'événement `ty` au nœud miroir désigné par
    /// `path` — chemin depuis la racine du document, chaque
    /// étape = `(tag, index parmi les frères éléments de même
    /// tag)`. Retourne `defaultPrevented`. Défaut : `Ok(false)`
    /// (pas de miroir → rien à dispatcher).
    ///
    /// Le chemin tolère les divergences de parse : les balises
    /// implicitement synthétisées par un tree-builder
    /// (`html`, `head`, `body`, `tbody`…) sont sautées si
    /// absentes du miroir.
    ///
    /// # Errors
    /// Erreur de listener JS propagée en [`JsError::Uncaught`].
    fn dispatch_event(&mut self, _ty: &str, _path: &[(String, u32)]) -> Result<bool, JsError> {
        Ok(false)
    }

    /// Borne le budget d'opérations de la prochaine évaluation
    /// (protection anti-script-infini — règle « budgets »).
    fn set_budget(&mut self, ops: u64);

    /// Draine les requêtes réseau émises depuis le dernier
    /// appel — `xhr.send()`, `fetch()`, `abort()` (JS-C27).
    /// À appeler aux mêmes points de synchro que
    /// [`Self::drain_nav_requests`] : les handlers livrés par
    /// [`Self::deliver_net_response`] peuvent en émettre de
    /// nouvelles. Défaut : rien.
    fn drain_net_requests(&mut self) -> Vec<NetRequest> {
        Vec::new()
    }

    /// Livre la réponse d'une requête drainée : `readyState` →
    /// `done`, handlers `readystatechange`/`load`/`error` et
    /// résolution de promesse `fetch()` tirent ici, en
    /// synchrone moteur (les microtasks sont vidées au retour —
    /// [`Self::run_pending`] reste le point de pompe).
    /// Défaut : `Ok(())` (moteur sans réseau).
    ///
    /// # Errors
    /// [`JsError::Uncaught`] si un handler jette,
    /// [`JsError::Dead`] si le moteur est inerte.
    fn deliver_net_response(&mut self, _resp: NetResponse) -> Result<(), JsError> {
        Ok(())
    }

    /// Installe `localStorage`/`sessionStorage` : `local` est le
    /// snapshot **par origine** de la session (le moteur y écrit
    /// en direct — relire via [`Self::drain_storage`]),
    /// `sessionStorage` est frais par contexte. Défaut : pas de
    /// storage globals.
    fn install_storage(&mut self, _local: std::collections::BTreeMap<String, String>) {}

    /// Extrait le contenu `localStorage` courant — la session
    /// le persiste par origine. Défaut : map vide.
    fn drain_storage(&mut self) -> std::collections::BTreeMap<String, String> {
        std::collections::BTreeMap::new()
    }

    /// Pose le snapshot `document.cookie` : fragment
    /// `« k=v; k2=v2 »` pré-filtré par la session pour l'origine
    /// de la page (le vrai jar applique HttpOnly/Secure/SameSite
    /// — le moteur ne voit que la vue script). Défaut : noop.
    fn set_cookie_snapshot(&mut self, _header: &str) {}

    /// Draine les écritures `document.cookie =` brutes
    /// (`« k=v; attrs… »`) — la session les soumet à son jar
    /// après avoir retiré `HttpOnly` (non posable par script).
    /// Défaut : rien.
    fn drain_cookie_writes(&mut self) -> Vec<String> {
        Vec::new()
    }
}

/// Fabrique de contextes — une instance par document chargé.
pub type JsFactory = std::sync::Arc<dyn Fn() -> Box<dyn JsEngine> + Send + Sync>;

/// Moteur nul : évalue en succès `"undefined"` — navigation sans
/// JavaScript (défaut, tests, `dillo-js` indisponible).
#[derive(Debug, Default)]
pub struct NullEngine;

impl JsEngine for NullEngine {
    fn eval(&mut self, _source: &str) -> Result<String, JsError> {
        Ok("undefined".to_string())
    }

    fn set_budget(&mut self, _ops: u64) {}
}

/// Fabrique de [`NullEngine`].
#[must_use]
pub fn null_factory() -> JsFactory {
    std::sync::Arc::new(|| Box::new(NullEngine))
}

pub use dillo_js::module_specifiers;

/// Commandes vers le thread moteur — valeurs sérialisables, aucun
/// handle `Rc` ne traverse.
enum Cmd {
    Eval {
        src: String,
        tx: Sender<Result<String, JsError>>,
    },
    RunPending {
        tx: Sender<usize>,
    },
    InstallDom {
        html: String,
        page_url: String,
        history_len: u32,
    },
    DrainNav {
        tx: Sender<Vec<NavRequest>>,
    },
    DomSync {
        tx: Sender<Option<String>>,
    },
    EvalModule {
        entry: String,
        sources: HashMap<String, String>,
        resolve: UrlResolver,
        tx: Sender<Result<String, JsError>>,
    },
    Dispatch {
        ty: String,
        path: Vec<(String, u32)>,
        tx: Sender<Result<bool, JsError>>,
    },
    SetBudget {
        ops: u64,
    },
    DrainNet {
        tx: Sender<Vec<NetRequest>>,
    },
    DeliverNet {
        resp: NetResponse,
        tx: Sender<Result<(), JsError>>,
    },
    InstallStorage {
        local: std::collections::BTreeMap<String, String>,
    },
    DrainStorage {
        tx: Sender<std::collections::BTreeMap<String, String>>,
    },
    CookieSnap {
        header: String,
    },
    DrainCookies {
        tx: Sender<Vec<String>>,
    },
    Shutdown,
}

/// Adaptateur `dillo-js` : un thread par contexte (le `Rc` du tas
/// ne quitte jamais son thread), le `JsEngine` n'est qu'un canal.
pub struct DilloEngine {
    tx: Sender<Cmd>,
    join: Option<JoinHandle<()>>,
}

impl DilloEngine {
    /// Crée un contexte neuf — spawn le thread moteur.
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = channel::<Cmd>();
        let join = std::thread::Builder::new()
            .name("dillo-js".to_string())
            .spawn(move || Self::worker(&rx));
        match join {
            Ok(j) => Self { tx, join: Some(j) },
            // Thread impossible (OS saturé) : moteur inerte —
            // `eval` répondra `Dead`, jamais de panic.
            Err(_) => Self { tx, join: None },
        }
    }

    /// Boucle du thread : possède le `dillo_js::Engine` (`!Send`)
    /// et sert les commandes jusqu'à `Shutdown` ou fermeture du
    /// canal (le `Sender` meurt avec le proxy).
    fn worker(rx: &Receiver<Cmd>) {
        let mut eng = dillo_js::Engine::new();
        // Miroir DOM installé par `install_dom` (`Rc` — vit et
        // meurt sur ce thread). Le `Dom` y reste mutable par les
        // bindings ; `DomSync` lit le dirty bit et sérialise.
        let mut jsd: Option<dillo_js::DomRef> = None;
        // Intentions de navigation script (`location`,
        // `history`) — hooks `Rc` du thread moteur, drainées
        // par `DrainNav` (jamais de navigation re-entrante).
        let nav_q: std::rc::Rc<std::cell::RefCell<Vec<NavRequest>>> =
            std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        // Requêtes XHR/fetch émises par les scripts — `ChanXhr`
        // (`Rc` thread-local) alimente, `DrainNet` vide.
        let net_q: std::rc::Rc<std::cell::RefCell<Vec<NetRequest>>> =
            std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        // Jar `document.cookie` : snapshot session + miroir des
        // écritures — drainé pour le vrai jar (politiques dans
        // `explorer-net`, jamais ici).
        let jar = std::rc::Rc::new(SnapshotJar::default());
        // Backing `localStorage` posé par `InstallStorage` — le
        // moteur y écrit en direct, `DrainStorage` le relit.
        let mut local_store: Option<dillo_js::Storage> = None;
        while let Ok(cmd) = rx.recv() {
            match cmd {
                Cmd::Eval { src, tx } => {
                    let out = match eng.eval(&src) {
                        Ok(v) => Ok(eng.describe(v)),
                        Err(e) => Err(JsError::from(e)),
                    };
                    if tx.send(out).is_err() {
                        return;
                    }
                }
                Cmd::RunPending { tx } => {
                    let n = eng.run_due_timers().unwrap_or(0) + eng.run_microtasks().unwrap_or(0);
                    if tx.send(n).is_err() {
                        return;
                    }
                }
                Cmd::InstallDom {
                    html,
                    page_url,
                    history_len,
                } => {
                    jsd = Some(install_dom(
                        &mut eng,
                        &nav_q,
                        &net_q,
                        &jar,
                        &html,
                        &page_url,
                        history_len,
                    ));
                }
                Cmd::DrainNav { tx } => {
                    let out = std::mem::take(&mut *nav_q.borrow_mut());
                    if tx.send(out).is_err() {
                        return;
                    }
                }
                Cmd::DomSync { tx } => {
                    let out = jsd.as_ref().and_then(|d| {
                        let d = d.borrow();
                        d.take_dirty().then(|| d.serialize_document())
                    });
                    if tx.send(out).is_err() {
                        return;
                    }
                }
                Cmd::EvalModule {
                    entry,
                    sources,
                    resolve,
                    tx,
                } => {
                    let src = sources.get(&entry).cloned().unwrap_or_default();
                    let loader = MapLoader {
                        map: sources,
                        resolve,
                    };
                    eng.set_module_loader(std::rc::Rc::new(std::cell::RefCell::new(loader)));
                    let out = match eng.eval_module(&src, &entry) {
                        Ok(v) => Ok(eng.describe(v)),
                        Err(e) => Err(JsError::from(e)),
                    };
                    if tx.send(out).is_err() {
                        return;
                    }
                }
                Cmd::Dispatch { ty, path, tx } => {
                    let node = jsd.as_ref().and_then(|d| mirror_node(&d.borrow(), &path));
                    let out = match node {
                        Some(n) => match eng.dom_event_on(&ty, n) {
                            Ok(p) => Ok(p),
                            Err(t) => Err(JsError::Uncaught(eng.describe(t.0))),
                        },
                        None => Ok(false),
                    };
                    if tx.send(out).is_err() {
                        return;
                    }
                }
                Cmd::SetBudget { ops } => eng.set_op_limit(ops),
                Cmd::Shutdown => return,
                // Surfaces script-initiées (JS-C27) déléguées.
                net => {
                    if !serve_net(&mut eng, &net_q, &jar, &mut local_store, net) {
                        return;
                    }
                }
            }
        }
    }

    /// Fabrique de [`DilloEngine`] — pour `Session::set_js_factory`.
    #[must_use]
    pub fn factory() -> JsFactory {
        std::sync::Arc::new(|| Box::new(DilloEngine::new()))
    }
}

impl Default for DilloEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl JsEngine for DilloEngine {
    fn eval(&mut self, source: &str) -> Result<String, JsError> {
        let (tx, rx) = channel();
        if self
            .tx
            .send(Cmd::Eval {
                src: source.to_string(),
                tx,
            })
            .is_err()
        {
            return Err(JsError::Dead);
        }
        rx.recv().map_err(|_| JsError::Dead)?
    }

    fn run_pending(&mut self) -> usize {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::RunPending { tx }).is_err() {
            return 0;
        }
        rx.recv().unwrap_or(0)
    }

    fn install_dom(&mut self, html: &str, page_url: &str, history_len: u32) {
        let _ = self.tx.send(Cmd::InstallDom {
            html: html.to_string(),
            page_url: page_url.to_string(),
            history_len,
        });
    }

    fn drain_nav_requests(&mut self) -> Vec<NavRequest> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DrainNav { tx }).is_err() {
            return Vec::new();
        }
        rx.recv().unwrap_or_default()
    }

    fn sync_dom(&mut self) -> Option<String> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DomSync { tx }).is_err() {
            return None;
        }
        rx.recv().ok().flatten()
    }

    fn eval_module(
        &mut self,
        entry: &str,
        sources: HashMap<String, String>,
        resolve: UrlResolver,
    ) -> Result<String, JsError> {
        let (tx, rx) = channel();
        if self
            .tx
            .send(Cmd::EvalModule {
                entry: entry.to_string(),
                sources,
                resolve,
                tx,
            })
            .is_err()
        {
            return Err(JsError::Dead);
        }
        rx.recv().map_err(|_| JsError::Dead)?
    }

    fn dispatch_event(&mut self, ty: &str, path: &[(String, u32)]) -> Result<bool, JsError> {
        let (tx, rx) = channel();
        if self
            .tx
            .send(Cmd::Dispatch {
                ty: ty.to_string(),
                path: path.to_vec(),
                tx,
            })
            .is_err()
        {
            return Err(JsError::Dead);
        }
        rx.recv().map_err(|_| JsError::Dead)?
    }

    fn set_budget(&mut self, ops: u64) {
        // Feu-et-oublie : le budget s'applique au prochain eval.
        let _ = self.tx.send(Cmd::SetBudget { ops });
    }

    fn drain_net_requests(&mut self) -> Vec<NetRequest> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DrainNet { tx }).is_err() {
            return Vec::new();
        }
        rx.recv().unwrap_or_default()
    }

    fn deliver_net_response(&mut self, resp: NetResponse) -> Result<(), JsError> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DeliverNet { resp, tx }).is_err() {
            return Err(JsError::Dead);
        }
        rx.recv().map_err(|_| JsError::Dead)?
    }

    fn install_storage(&mut self, local: std::collections::BTreeMap<String, String>) {
        let _ = self.tx.send(Cmd::InstallStorage { local });
    }

    fn drain_storage(&mut self) -> std::collections::BTreeMap<String, String> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DrainStorage { tx }).is_err() {
            return std::collections::BTreeMap::new();
        }
        rx.recv().unwrap_or_default()
    }

    fn set_cookie_snapshot(&mut self, header: &str) {
        let _ = self.tx.send(Cmd::CookieSnap {
            header: header.to_string(),
        });
    }

    fn drain_cookie_writes(&mut self) -> Vec<String> {
        let (tx, rx) = channel();
        if self.tx.send(Cmd::DrainCookies { tx }).is_err() {
            return Vec::new();
        }
        rx.recv().unwrap_or_default()
    }
}

impl Drop for DilloEngine {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// Bras JS-C27 du worker : drains et livraisons des surfaces
/// net/storage/cookie. `false` = canal de réponse fermé → le
/// worker s'arrête ; tout autre `Cmd` est ignoré (invariant :
/// l'appelant ne délègue que les commandes net).
fn serve_net(
    eng: &mut dillo_js::Engine,
    net_q: &std::rc::Rc<std::cell::RefCell<Vec<NetRequest>>>,
    jar: &std::rc::Rc<SnapshotJar>,
    local_store: &mut Option<dillo_js::Storage>,
    cmd: Cmd,
) -> bool {
    match cmd {
        Cmd::DrainNet { tx } => {
            let out = std::mem::take(&mut *net_q.borrow_mut());
            tx.send(out).is_ok()
        }
        Cmd::DeliverNet { resp, tx } => {
            let out = eng
                .deliver_xhr_response(dillo_js::XhrResponse {
                    handle: resp.handle,
                    status: resp.status,
                    status_text: resp.status_text,
                    headers: resp.headers,
                    body: resp.body,
                })
                .map_err(|t| JsError::Uncaught(eng.describe(t.0)));
            tx.send(out).is_ok()
        }
        Cmd::InstallStorage { local } => {
            let map: dillo_js::Storage = std::rc::Rc::new(std::cell::RefCell::new(local));
            eng.install_storage(&map);
            *local_store = Some(map);
            true
        }
        Cmd::DrainStorage { tx } => {
            let out = local_store
                .as_ref()
                .map_or_else(std::collections::BTreeMap::new, |m| m.borrow().clone());
            tx.send(out).is_ok()
        }
        Cmd::CookieSnap { header } => {
            jar.set_header(&header);
            true
        }
        Cmd::DrainCookies { tx } => {
            let out = std::mem::take(&mut *jar.writes.borrow_mut());
            tx.send(out).is_ok()
        }
        _ => true,
    }
}

/// `Cmd::InstallDom` côté worker : miroir parsé (dirty bit
/// consommé — la construction n'est pas une mutation JS),
/// bindings `document`/`location`/`history`, puis surface
/// script-initiée (JS-C27) : XHR + `fetch` → `net_q`,
/// `document.cookie` → `jar`. Renvoie le `DomRef` installé.
fn install_dom(
    eng: &mut dillo_js::Engine,
    nav_q: &std::rc::Rc<std::cell::RefCell<Vec<NavRequest>>>,
    net_q: &std::rc::Rc<std::cell::RefCell<Vec<NetRequest>>>,
    jar: &std::rc::Rc<SnapshotJar>,
    html: &str,
    page_url: &str,
    history_len: u32,
) -> dillo_js::DomRef {
    let d = std::rc::Rc::new(std::cell::RefCell::new(dillo_js::Dom::from_html(html)));
    d.borrow().take_dirty();
    eng.install_dom(&d);
    eng.set_page_url(page_url);
    {
        let q = nav_q.clone();
        let nav: std::rc::Rc<dyn Fn(&str)> = std::rc::Rc::new(move |u| {
            q.borrow_mut().push(NavRequest::Navigate(u.to_string()));
        });
        let q2 = nav_q.clone();
        let hist: std::rc::Rc<dyn Fn(i32)> = std::rc::Rc::new(move |n| {
            q2.borrow_mut().push(NavRequest::HistoryGo(n));
        });
        eng.set_nav_hooks(nav, hist, history_len);
    }
    eng.install_xhr(std::rc::Rc::new(ChanXhr {
        q: std::rc::Rc::clone(net_q),
        next: std::cell::Cell::new(0),
    }));
    eng.set_cookie_jar(jar.clone());
    d
}

/// Balises que le tree-builder HTML5 synthétise quand la source
/// les omet — le parseur littéral du miroir ne les produit pas.
/// À la descente de `path`, un niveau implicitement absent est
/// sauté sans consommer d'enfant.
const IMPLICIT_TAGS: &[&str] = &["html", "head", "body", "tbody", "thead", "tfoot"];

/// Rejoue `path` (depuis la racine `#document` du miroir) et
/// retourne le `NodeId` miroir — `None` si le chemin ne mène à
/// aucun élément compatible (arbres divergents → dégradation,
/// pas d'event vers le mauvais nœud).
fn mirror_node(d: &dillo_js::Dom, path: &[(String, u32)]) -> Option<dillo_js::NodeId> {
    let mut cur = d.root;
    for (tag, k) in path {
        let kids = d.element_children(cur);
        let found = kids
            .iter()
            .filter(|&&c| d.nodes[c as usize].tag == *tag)
            .nth(*k as usize)
            .copied();
        match found {
            Some(n) => cur = n,
            // Wrapper synthétisé côté navigateur, absent du miroir
            // → ne pas descendre : le niveau suivant retente sur
            // les mêmes enfants.
            None if IMPLICIT_TAGS.contains(&tag.as_str()) => {}
            None => return None,
        }
    }
    Some(cur)
}

/// Chargeur de modules préfetchés (étape 50) : le graphe statique
/// est collecté par la session via `HttpTransport` **avant**
/// l'évaluation — le `load` reste synchrone et le canal
/// `DilloEngine` à sens unique (un fetch depuis le thread moteur
/// bloquerait le hôte qui attend la fin de l'eval : deadlock).
/// Un spécificateur hors map — `import()` dynamique non préfetché,
/// borne `max_modules` atteinte — est une erreur JS propre.
struct MapLoader {
    /// `url absolue → source` — graphe collecté par la session.
    map: HashMap<String, String>,
    /// `(specifier, referrer) → url absolue` (politique hôte).
    resolve: UrlResolver,
}

impl dillo_js::ModuleLoader for MapLoader {
    fn load(&mut self, specifier: &str, referrer: &str) -> Result<String, String> {
        let key = (self.resolve)(specifier, referrer)
            .ok_or_else(|| format!("spécificateur irrésolvable : {specifier}"))?;
        self.map
            .get(&key)
            .cloned()
            .ok_or_else(|| format!("module non chargé : {key}"))
    }
}

/// `XhrTransport` du thread moteur (JS-C27) : `send`/`abort`
/// n'exécutent rien — ils empilent des [`NetRequest`] drainées
/// par la session, qui possède `HttpTransport` (modèle
/// drain-nav généralisé : jamais d'I/O depuis le moteur).
struct ChanXhr {
    /// File partagée avec `DrainNet`.
    q: std::rc::Rc<std::cell::RefCell<Vec<NetRequest>>>,
    /// Prochain handle libre (compteur du contexte).
    next: std::cell::Cell<u64>,
}

impl dillo_js::XhrTransport for ChanXhr {
    fn send(&self, req: &dillo_js::XhrRequest) -> u64 {
        let handle = self.next.get();
        self.next.set(handle + 1);
        self.q.borrow_mut().push(NetRequest::Fetch {
            handle,
            method: req.method.clone(),
            url: req.url.clone(),
            headers: req.headers.clone(),
            body: req.body.clone(),
        });
        handle
    }

    fn abort(&self, handle: u64) {
        self.q.borrow_mut().push(NetRequest::Abort { handle });
    }
}

/// Jar `document.cookie` thread-local (JS-C27) — le vrai jar
/// (`explorer-net`, politiques Secure/SameSite/HttpOnly) vit
/// côté session : le moteur reçoit un snapshot `« k=v »` et
/// miroire les écritures pour une lecture post-écriture
/// cohérente ; la session draine les écritures brutes.
#[derive(Default)]
struct SnapshotJar {
    /// Vue `« nom → valeur »` du snapshot + écritures mises
    /// en miroir (le `set` n'interprète que le premier
    /// segment `k=v` — les attrs relèvent du vrai jar).
    map: std::cell::RefCell<std::collections::BTreeMap<String, String>>,
    /// Écritures brutes `« k=v; attrs… »` — drainées.
    writes: std::cell::RefCell<Vec<String>>,
}

impl SnapshotJar {
    /// Fragment `« k=v; k2=v2 »` pour `document.cookie`.
    fn header(&self) -> String {
        self.map
            .borrow()
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Remplace la vue par le snapshot de la session.
    fn set_header(&self, header: &str) {
        let mut m = self.map.borrow_mut();
        m.clear();
        for pair in header.split(';') {
            if let Some((k, v)) = pair.trim().split_once('=') {
                m.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
}

impl dillo_js::CookieJar for SnapshotJar {
    fn get(&self, _url: &str) -> String {
        self.header()
    }

    fn set(&self, _url: &str, cookie: &str) {
        self.writes.borrow_mut().push(cookie.to_string());
        if let Some((k, v)) = cookie
            .split(';')
            .next()
            .and_then(|p| p.trim().split_once('='))
        {
            self.map
                .borrow_mut()
                .insert(k.trim().to_string(), v.trim().to_string());
        }
    }
}

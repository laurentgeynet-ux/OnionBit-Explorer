// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-ffi` — surface C du mode **in-process**
//! (étape 47, ADR-0018).
//!
//! La bibliothèque dynamique (`explorer_ffi.dll` / `.so` /
//! `.dylib`) embarque la [`Session`] moteur complète dans le
//! processus de l'UI Flutter et sert les frames en **RGBA
//! brut** — zéro encodage/décodage PNG, zéro socket.
//!
//! # Modèle de threading
//!
//! L'appelant DOIT sérialiser tous les appels sur un handle
//! (l'isolate UI Dart le fait naturellement). Aucun verrou
//! interne n'est pris : le handle n'est ni `Send` ni `Sync`.
//!
//! # `unsafe`
//!
//! Seul crate du workspace autorisé à en contenir (ADR-0018) :
//! l'ABI C l'exige. Chaque bloc est confiné à la validation des
//! pointeurs en entrée et à l'écriture bornée des buffers de
//! sortie ; aucun `unsafe` ne touche la logique moteur.
//!
//! # Contrat
//!
//! ```c
//! ExplorerHandle *h = explorer_create();          // NULL = échec
//! uint64_t tab = explorer_open_tab(h);            // 0 = échec
//! explorer_navigate(h, tab, url, url_len);        // 0 = ok
//! uint32_t w, hgt;
//! explorer_frame_size(h, tab, &w, &hgt);          // 0 = ok
//! uint8_t *px = malloc(w * hgt * 4);
//! explorer_frame_copy(h, tab, px, w * hgt * 4);   // >0 = octets écrits
//! explorer_destroy(h);
//! ```
//!
//! Codes retour : `0` succès, `-1` argument/handle invalide,
//! `-2` échec moteur (détail via [`explorer_last_error`]).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use explorer_core::{CoreEvent, InputEvent, InputOutcome as O, KeyMods, Session};
use explorer_net::decode::DecodingTransport;
use explorer_net::h1::HttpConfig;
use explorer_net::h2::AutoClient;
use explorer_net::http::HttpTransport;
use explorer_net::policy::RedirectTransport;
use explorer_net::tls::TlsConnector;
use explorer_render::DirFontSource;

/// Nombre maximal de redirections suivies (cohérent avec
/// `DaemonConfig::default()`).
const MAX_REDIRECTS: usize = 10;
/// Corps de réponse maximal accepté (cohérent avec le démon).
const MAX_BODY: usize = 10 * 1024 * 1024;
/// Longueur maximale d'une URL passée au navigateur.
const MAX_URL: usize = 8 * 1024;
/// Longueur maximale d'une chaîne `key` d'événement clavier.
const MAX_KEY: usize = 64;

thread_local! {
    /// Détail de la dernière erreur du thread appelant.
    static LAST_ERROR: RefCell<String> = const { RefCell::new(String::new()) };
}

fn set_err(msg: impl std::fmt::Display) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = msg.to_string();
    });
}

/// Handle opaque — `struct` possédé par le pointeur retourné
/// de [`explorer_create`], libéré par [`explorer_destroy`].
pub struct ExplorerHandle {
    rt: std::sync::Arc<tokio::runtime::Runtime>,
    /// `Arc<tokio::Mutex>` — partagée entre exports et le
    /// thread pompe (`spawn_pump`) — le daemon prouve
    /// `Session: Send` (même `Arc<Mutex>` dans son
    /// `ApiState`). tokio car la pompe consomme
    /// [`pump_timers_shared`]/[`drain_applies_shared`]
    /// (JS hors verrou via take/commit — les exports ne
    /// patientent jamais derrière un burst JS).
    session: std::sync::Arc<tokio::sync::Mutex<Session>>,
    /// Canal d'événements de session — drainé par
    /// [`explorer_media_events`] (filtré sur le bras
    /// `MediaSurface`, étape 71e ; le pull des frames reste
    /// le modèle de l'ABI).
    events: tokio::sync::broadcast::Receiver<CoreEvent>,
    /// Lignes NDJSON média déjà sérialisées mais non copiées
    /// faute de place dans le buffer appelant — réémises à
    /// l'appel suivant (jamais tronquées à mi-ligne).
    media_pending: String,
    /// Port du loopback média embarqué (étape 177/b2,
    /// ADR-0029) — routes `/api/…/media*` servies par axum
    /// sur `127.0.0.1` depuis le runtime privé ; `0` si le
    /// bind a échoué (`apiBase` Dart reste `''`, honnête).
    api_port: u16,
    /// Verrou d'instance unique du profil (`explorer.lock`) —
    /// gardé jusqu'à la destruction du handle ; le lock OS est
    /// relâché à la mort du processus, pas de résidu après crash.
    _profile_lock: Option<fslock::LockFile>,
    /// Arrêt du thread pompe — posé par [`explorer_destroy`] /
    /// `Drop` (le thread détaché s'arrête à sa prochaine
    /// battement — jamais d'usage post-mortem du mutex).
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for ExplorerHandle {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Verrouille la session (tokio `Mutex` — `lock().await`
/// conduit par `block_on` ; le poison tokio n'existe pas,
/// un panic précédent ne fige pas le mutex).
fn sess(h: &ExplorerHandle) -> tokio::sync::MutexGuard<'_, Session> {
    h.rt.block_on(h.session.lock())
}

/// Répertoire de fontes par défaut — `EXPLORER_FONT_DIR` en
/// priorité, sinon le répertoire système de la plateforme
/// (même logique que `explorer-daemon`).
fn default_fonts_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("EXPLORER_FONT_DIR") {
        return PathBuf::from(dir);
    }
    if cfg!(windows) {
        PathBuf::from("C:\\Windows\\Fonts")
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/System/Library/Fonts")
    } else {
        PathBuf::from("/usr/share/fonts")
    }
}

/// Pile réseau identique au démon : TLS web-roots + ALPN
/// h1/h2 → décodage → HSTS (store du profil, PR-3) →
/// redirections bornées.
fn http_stack(profile: &Path) -> Arc<dyn HttpTransport> {
    // Cache DNS persisté (PR-9) — même pile que le démon.
    let tls = Arc::new(TlsConnector::web_roots_over(Arc::new(
        explorer_net::h1::ResolvingConnector::new(Arc::new(
            explorer_net::resolver::CachingResolver::wrap(
                Arc::new(explorer_net::resolver::SystemResolver),
                Arc::new(explorer_core::JsonDnsStore::open(profile)),
                explorer_net::resolver::DNS_CACHE_TTL,
            ),
        )),
    )));
    let h1: Arc<dyn HttpTransport> = Arc::new(AutoClient::new(tls, HttpConfig::default()));
    let decoded: Arc<dyn HttpTransport> = Arc::new(DecodingTransport::wrap(h1, MAX_BODY));
    let hsts: Arc<dyn HttpTransport> = Arc::new(explorer_net::hsts::HstsTransport::wrap(
        decoded,
        Arc::new(explorer_core::JsonHstsStore::open(profile)),
    ));
    Arc::new(RedirectTransport::wrap(hsts, MAX_REDIRECTS))
}

/// `*mut ExplorerHandle` → `&mut` validé, ou erreur FFI.
///
/// # Safety
///
/// Le pointeur doit provenir de [`explorer_create`] et ne pas
/// avoir été libéré. L'appelant sérialise les accès.
unsafe fn handle<'a>(h: *mut ExplorerHandle) -> Option<&'a mut ExplorerHandle> {
    if h.is_null() {
        set_err("handle nul");
        return None;
    }
    // SAFETY : contrat FFI — pointeur issu de `explorer_create`,
    // vivant, accès sérialisé par l'appelant.
    Some(unsafe { &mut *h })
}

/// `*const u8 + len` → `&str` validé, borné à `max`.
///
/// # Safety
///
/// `ptr` doit désigner `len` octets lisibles.
unsafe fn read_str<'a>(ptr: *const u8, len: usize, max: usize) -> Option<&'a str> {
    if len > max || (len > 0 && ptr.is_null()) {
        set_err("chaîne trop longue ou pointeur nul");
        return None;
    }
    if len == 0 {
        return Some("");
    }
    // SAFETY : contrat FFI — `ptr..ptr+len` lisible côté appelant.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    if let Ok(s) = std::str::from_utf8(bytes) {
        Some(s)
    } else {
        set_err("UTF-8 invalide");
        None
    }
}

/// Battement hôte (miroir du `tokio::spawn` du daemon,
/// main.rs étape 51/52) : sans pompe propre la session
/// n'avance que pendant les appels FFI — fetchs réseau,
/// timers JS et applies gelés entre deux ticks UI de
/// ~50 ms (mesuré : minutes sur une page lourde). La
/// pompe tourne à la cadence `next_timer_deadline`
/// bornée — jamais busy-loop, jamais le sommeil complet
/// quand un timer plus court est en file.
fn spawn_pump(
    rt: &std::sync::Arc<tokio::runtime::Runtime>,
    session: &std::sync::Arc<tokio::sync::Mutex<Session>>,
    stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    const PUMP_MS: std::time::Duration = std::time::Duration::from_millis(10);
    let rt = std::sync::Arc::clone(rt);
    let s = std::sync::Arc::clone(session);
    let stop = std::sync::Arc::clone(stop);
    std::thread::spawn(move || {
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            let wait = rt.block_on(async {
                let (wait, tabs) = {
                    let mut sess = s.lock().await;
                    let _ = sess.poll_net();
                    let w = sess.next_timer_deadline().unwrap_or(PUMP_MS).min(PUMP_MS);
                    (w, sess.tabs())
                };
                // Miroir du daemon (main.rs étape 51/52) :
                // applies + pompe JS par onglet via les
                // helpers partagés — le moteur est extrait
                // (take/commit), le run JS tourne hors
                // verrou — un export n'attend jamais un
                // burst JS sous le mutex.
                for tab in &tabs {
                    explorer_core::drain_applies_shared(&s, *tab).await;
                    explorer_core::pump_timers_shared(&s, *tab).await;
                }
                wait
            });
            std::thread::sleep(wait);
        }
    });
}

/// Corps commun de création : runtime tokio privé, pile
/// réseau réelle, fabrique JS, store de profil dans `dir`
/// (`None` = [`explorer_core::JsonFileStore::default_dir`] — honore
/// `EXPLORER_PROFILE`, étape 61 : le mode in-process
/// persistait dans le vide auparavant).
fn create_inner(profile_dir: Option<PathBuf>) -> *mut ExplorerHandle {
    // `multi_thread`, pas `current_thread` : en mono-thread
    // les tâches `spawn` (fetchs, timers, canaux) ne
    // progressent QUE pendant un `block_on` actif — elles
    // gelaient entre les appels FFI (~50 ms de tick →
    // minutes sur une page lourde). Le multi-thread fait
    // vivre le runtime en continu comme le daemon.
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => std::sync::Arc::new(rt),
        Err(e) => {
            set_err(e);
            return std::ptr::null_mut();
        }
    };
    let fonts = DirFontSource::new(&default_fonts_dir());
    let dir = profile_dir.unwrap_or_else(explorer_core::JsonFileStore::default_dir);
    let profile_lock = match explorer_core::acquire_profile_lock(&dir) {
        Ok(l) => l,
        Err(msg) => {
            set_err(msg);
            return std::ptr::null_mut();
        }
    };
    init_file_logging(&dir);
    init_crash_hook(&dir);
    let mut session = Session::new(http_stack(&dir), Arc::new(fonts));
    session.set_js_factory(explorer_js::JscoreEngine::factory());
    session.set_store(Arc::new(explorer_core::JsonFileStore::open(&dir)));
    let events = session.subscribe();
    let session = std::sync::Arc::new(tokio::sync::Mutex::new(session));
    let api_port = spawn_media_loopback(&rt, &session);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    spawn_pump(&rt, &session, &stop);
    Box::into_raw(Box::new(ExplorerHandle {
        rt,
        session,
        events,
        media_pending: String::new(),
        api_port,
        _profile_lock: profile_lock,
        stop,
    }))
}

/// Loopback média embarqué (étape 177/b2, ADR-0029) — les
/// routes `/api/…/media*` du daemon servies par axum sur
/// `127.0.0.1:0` dans le runtime privé ; `MseSource`
/// `OnionMedia` pull via HTTP comme en mode REST, l'API reste
/// le contrat (seul le transport est in-process). Bind
/// synchrone — le port OS-élu est connu avant le retour ;
/// échec → `0` (`apiBase` `''`, dégradé honnête).
fn spawn_media_loopback(
    rt: &std::sync::Arc<tokio::runtime::Runtime>,
    session: &std::sync::Arc<tokio::sync::Mutex<Session>>,
) -> u16 {
    let Ok(l) = std::net::TcpListener::bind("127.0.0.1:0") else {
        return 0;
    };
    let Ok(port) = l.local_addr().map(|a| a.port()) else {
        return 0;
    };
    if l.set_nonblocking(true).is_err() {
        return 0;
    }
    // `from_std` enregistre la socket sur le réacteur —
    // contexte runtime requis hors `block_on`/`spawn`.
    let _guard = rt.enter();
    let Ok(tl) = tokio::net::TcpListener::from_std(l) else {
        return 0;
    };
    let router = explorer_api::media_router(std::sync::Arc::clone(session));
    rt.spawn(async move {
        let _ = axum::serve(tl, router).await;
    });
    port
}

/// Journal fichier `<profil>/logs/engine.log` — une fois par
/// processus, best-effort (échec silencieux : le mode FFI n'a
/// pas de console, l'absence de journal n'est pas fatale).
fn init_file_logging(profile_dir: &Path) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let logs = profile_dir.join("logs");
        if std::fs::create_dir_all(&logs).is_err() {
            return;
        }
        let file = tracing_appender::rolling::never(logs, "engine.log");
        let _ = tracing_subscriber::fmt()
            .with_writer(file)
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_ansi(false)
            .try_init();
    });
}

/// Crash dumps (PR-17) : un panic dans le moteur écrit
/// `<profil>/crash/panic-<ts_ms>.txt` (payload, site,
/// backtrace) — le diagnostic survit à la mort du processus
/// en bundle portable (miroir de la sentinelle du daemon).
/// Le hook est **chaîné** au précédent (celui de l'hôte
/// Flutter reste exécuté) — jamais de panic dans le hook.
fn init_crash_hook(profile_dir: &Path) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let crash = profile_dir.join("crash");
        if std::fs::create_dir_all(&crash).is_err() {
            return;
        }
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let loc = info
                .location()
                .map_or_else(|| "?".to_string(), ToString::to_string);
            let payload = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string>".to_string());
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis());
            let bt = std::backtrace::Backtrace::capture();
            let _ = std::fs::write(
                crash.join(format!("panic-{stamp}.txt")),
                format!("panic: {payload}\nat: {loc}\n\n{bt}\n"),
            );
            prev(info);
        }));
    });
}

/// Crée une session moteur in-process (runtime tokio privé,
/// pile réseau réelle, fabrique JS branchée). `NULL` si le
/// runtime n'a pas pu être construit.
///
/// Le profil persistant vient de `EXPLORER_PROFILE`, sinon du
/// marqueur `Explorer.portable` (bundle portable → `<racine>/
/// state`), sinon du repli plateforme — voir
/// [`explorer_create_with_profile`] pour un chemin explicite.
#[unsafe(no_mangle)]
pub extern "C" fn explorer_create() -> *mut ExplorerHandle {
    create_inner(None)
}

/// Variante à profil explicite (étape 61, ABI additive) :
/// `dir` pointe sur un chemin UTF-8 de `len` octets ;
/// `NULL`/`0` = repli [`explorer_core::JsonFileStore::default_dir`].
///
/// # Safety
///
/// `dir..dir+len` doit être lisible.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_create_with_profile(
    dir: *const u8,
    len: usize,
) -> *mut ExplorerHandle {
    if len == 0 {
        return create_inner(None);
    }
    // Chemin fourni mais illisible/invalide : échec explicite
    // plutôt que d'écrire le profil au mauvais endroit.
    let Some(profile) = (unsafe { read_str(dir, len, 16384) }) else {
        return std::ptr::null_mut();
    };
    create_inner(Some(PathBuf::from(profile)))
}

/// Port du loopback média embarqué (étape 177/b2, ADR-0029)
/// — `http://127.0.0.1:<port>` sert `GET /api/…/media/*`
/// comme le daemon ; `0` si le bind a échoué (`apiBase`
/// `''`, lecture MSE honnêtement absente).
///
/// Niveau d'ABI de la bibliothèque (étape 187 —
/// déploiement moteur) : la dll s'auto-identifie — une
/// copie périmée se détecte sans la charger à l'aveugle.
/// `0` côté hôte = symbole absent → dll pré-versionnement.
///
/// **Règle** : incrémenter à chaque changement d'ABI
/// (export ajouté, signature modifiée) — `deploy_engine.ps1`
/// relit cette valeur dans le manifest sidecar. ABI 1 =
/// socle jusqu'à l'étape 186 incluse (`explorer_create*`,
/// `open_tab`, `navigate`, `input`, `frame_size`/`copy`/
/// `copy_region`/`pull`, `dirty_rects`, `tab_url`/`title`/
/// `scroll`, `hit_targets`, `media_*`, `downloads_*`,
/// `eval_js`, `api_port`, `media_codecs`,
/// `set_accept_language`, `set_color_scheme`,
/// `set_viewport`, `tick`, `last_error`, `destroy`).
#[unsafe(no_mangle)]
pub extern "C" fn explorer_abi_version() -> u32 {
    1
}

/// # Safety
///
/// `h` doit provenir de [`explorer_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_api_port(h: *mut ExplorerHandle) -> u16 {
    match unsafe { handle(h) } {
        Some(h) => h.api_port,
        None => 0,
    }
}

/// Déclare les capacités média de l'hôte (étape 178 —
/// b-e2e) : corps JSON `{codecs: […]}` identique au
/// `claim` REST (`mediaCodecs` de l'app). À appeler juste
/// après `explorer_create` — les contextes JS spawnés
/// ensuite répondent honnêtement à
/// `MediaSource.isTypeSupported` ; sans déclaration, un
/// player MSE réel n'instancie jamais la `MediaSource`
/// dont le claim apprendrait les codecs (cercle vicieux
/// d'amorçage). `0` succès.
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `json..json+len`
/// lisible UTF-8 (≤ 64 Kio).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_media_codecs(
    h: *mut ExplorerHandle,
    json: *const u8,
    len: usize,
) -> i32 {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    let Some(s) = (unsafe { read_str(json, len, 64 * 1024) }) else {
        return -1;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(s) else {
        return -1;
    };
    let codecs = explorer_core::media_codecs_from_json(&v);
    sess(h).declare_media_codecs(&codecs);
    0
}

/// Libère le handle et sa session.
///
/// # Safety
///
/// `h` doit provenir de [`explorer_create`] et n'être libéré
/// qu'une fois.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_destroy(h: *mut ExplorerHandle) {
    if h.is_null() {
        return;
    }
    // SAFETY : contrat FFI — `h` provient de `Box::into_raw`
    // dans `explorer_create`, libéré une seule fois.
    drop(unsafe { Box::from_raw(h) });
}

/// Copie le dernier message d'erreur dans `buf` (UTF-8, sans
/// NUL final si plein). Retourne la longueur écrite, `0` si
/// vide, `-1` si buffer invalide.
///
/// # Safety
///
/// `buf..buf+cap` doit être inscriptible.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_last_error(buf: *mut u8, cap: usize) -> isize {
    let msg = LAST_ERROR.with(|e| e.borrow().clone());
    write_str(&msg, buf, cap)
}

/// Écrit `s` dans `buf` sans dépasser `cap`.
///
/// # Safety
///
/// `buf..buf+cap` inscriptible si `len > 0`.
unsafe fn write_str(s: &str, buf: *mut u8, cap: usize) -> isize {
    let bytes = s.as_bytes();
    let n = bytes.len().min(cap);
    if n > 0 {
        if buf.is_null() {
            set_err("buffer nul");
            return -1;
        }
        // SAFETY : contrat FFI — `buf..buf+n` inscriptible.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, n) };
    }
    isize::try_from(n).unwrap_or(isize::MAX)
}

/// Ouvre un onglet. Retourne son identifiant (`0` = échec).
///
/// # Safety
///
/// `h` issu de [`explorer_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_open_tab(h: *mut ExplorerHandle) -> u64 {
    match unsafe { handle(h) } {
        Some(h) => sess(h).open_tab(),
        None => 0,
    }
}

/// Ferme un onglet. `0` succès.
///
/// # Safety
///
/// `h` issu de [`explorer_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_close_tab(h: *mut ExplorerHandle, tab: u64) -> i32 {
    match unsafe { handle(h) } {
        Some(h) => match sess(h).close_tab(tab) {
            Ok(()) => 0,
            Err(e) => {
                set_err(e);
                -2
            }
        },
        None => -1,
    }
}

/// Navigue vers `url` (UTF-8, ≤ 8 Kio). `0` succès.
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `url..url+len` lisible.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_navigate(
    h: *mut ExplorerHandle,
    tab: u64,
    url: *const u8,
    len: usize,
) -> i32 {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    let Some(url) = (unsafe { read_str(url, len, MAX_URL) }) else {
        return -1;
    };
    match h.rt.block_on(sess(h).navigate(tab, url)) {
        Ok(()) => 0,
        Err(e) => {
            set_err(e);
            -2
        }
    }
}

/// Recharge la page. `0` succès.
///
/// # Safety
///
/// `h` issu de [`explorer_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_reload(h: *mut ExplorerHandle, tab: u64) -> i32 {
    match unsafe { handle(h) } {
        Some(h) => match h.rt.block_on(sess(h).reload(tab)) {
            Ok(()) => 0,
            Err(e) => {
                set_err(e);
                -2
            }
        },
        None => -1,
    }
}

/// Historique arrière. `0` succès.
///
/// # Safety
///
/// `h` issu de [`explorer_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_go_back(h: *mut ExplorerHandle, tab: u64) -> i32 {
    match unsafe { handle(h) } {
        Some(h) => match h.rt.block_on(sess(h).go_back(tab)) {
            Ok(()) => 0,
            Err(e) => {
                set_err(e);
                -2
            }
        },
        None => -1,
    }
}

/// Historique avant. `0` succès.
///
/// # Safety
///
/// `h` issu de [`explorer_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_go_forward(h: *mut ExplorerHandle, tab: u64) -> i32 {
    match unsafe { handle(h) } {
        Some(h) => match h.rt.block_on(sess(h).go_forward(tab)) {
            Ok(()) => 0,
            Err(e) => {
                set_err(e);
                -2
            }
        },
        None => -1,
    }
}

/// Ajuste le viewport (`w`, `h` CSS px, `scale` DPR).
///
/// # Safety
///
/// `h` issu de [`explorer_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_set_viewport(
    h: *mut ExplorerHandle,
    w: f32,
    hgt: f32,
    scale: f32,
) -> i32 {
    match unsafe { handle(h) } {
        Some(h) => {
            sess(h).set_viewport(w, hgt, scale);
            0
        }
        None => -1,
    }
}

/// Bascule le schéma de couleurs propagé à `StyleEnv`
/// (étape 97) : `0` = `light`, `1` = `dark`. Le re-render
/// des onglets chargés suit le même modèle que
/// [`explorer_set_viewport`] — `explorer_tick` le produit
/// à la prochaine sonde.
///
/// # Safety
///
/// `h` issu de [`explorer_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_set_color_scheme(h: *mut ExplorerHandle, dark: i32) -> i32 {
    match unsafe { handle(h) } {
        Some(h) => {
            sess(h).set_color_scheme(if dark == 0 {
                explorer_core::ColorScheme::Light
            } else {
                explorer_core::ColorScheme::Dark
            });
            0
        }
        None => -1,
    }
}

/// Valeur `Accept-Language` annoncée sur les requêtes
/// suivantes (étape 118) — `len == 0` lève l'en-tête.
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `lang..lang+len` lisible.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_set_accept_language(
    h: *mut ExplorerHandle,
    lang: *const u8,
    len: usize,
) -> i32 {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    let lang = if len == 0 {
        None
    } else {
        match unsafe { read_str(lang, len, MAX_URL) } {
            Some(l) => Some(l.to_string()),
            None => return -1,
        }
    };
    sess(h).set_accept_language(lang);
    0
}

/// Genre d'entrée pour [`explorer_input`].
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum FfiInputKind {
    /// Clic principal à `(x, y)`.
    Click = 0,
    /// Survol à `(x, y)`.
    MouseMove = 1,
    /// Scroll vertical (delta dans `y`).
    Scroll = 2,
    /// Touche clavier (`key` pointe sur son identifiant).
    Key = 3,
}

/// Événement d'entrée. `kind` ∈ `0..=4` (`FfiInputKind`,
/// `4` = `keyup`). `mods` = bitmask modifieurs clavier
/// (`1`=ctrl, `2`=shift, `4`=alt, `8`=meta — étape 105),
/// ignoré hors `key`/`keyup`. Retour : `0` ignoré,
/// `1` scrollé, `2` navigué, `3` hover, `4` champ focalisé,
/// `5` champ édité, `6` aucun champ focalisé (frappe hors
/// champ ou focus perdu) ; négatif = erreur.
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `key..key+key_len`
/// lisible si `kind == Key`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_input(
    h: *mut ExplorerHandle,
    tab: u64,
    kind: u8,
    x: f32,
    y: f32,
    key: *const u8,
    key_len: usize,
    mods: u8,
) -> i32 {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    let ev = match kind {
        0 => InputEvent::Click { x, y },
        1 => InputEvent::MouseMove { x, y },
        2 => InputEvent::Scroll { dy: y },
        3 => match unsafe { read_str(key, key_len, MAX_KEY) } {
            Some(k) => InputEvent::Key {
                key: k.to_owned(),
                mods: KeyMods::from_bits(mods),
            },
            None => return -1,
        },
        4 => match unsafe { read_str(key, key_len, MAX_KEY) } {
            Some(k) => InputEvent::KeyUp {
                key: k.to_owned(),
                mods: KeyMods::from_bits(mods),
            },
            None => return -1,
        },
        _ => {
            set_err("kind inconnu");
            return -1;
        }
    };
    match h.rt.block_on(sess(h).handle_input(tab, ev)) {
        Ok(O::Ignored) => 0,
        Ok(O::Scrolled { .. }) => 1,
        Ok(O::Navigated) => 2,
        Ok(O::Hover { .. }) => 3,
        Ok(O::Focused) => 4,
        Ok(O::Edited) => 5,
        Ok(O::Unfocused) => 6,
        Err(e) => {
            set_err(e);
            -2
        }
    }
}

/// Avance les animations/transitions de l'onglet. `1` si une
/// frame a été invalidée, `0` sinon.
///
/// # Safety
///
/// `h` issu de [`explorer_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_tick(h: *mut ExplorerHandle, tab: u64) -> i32 {
    match unsafe { handle(h) } {
        // `tick_animations` pompe aussi les timers JS (étape
        // 52) — leurs `location.href` sont drainés ici en
        // synchrone (l'hôte FFI n'a pas de battement).
        // `let` (pas scrutinee de `match`) : le `MutexGuard`
        // temporaire doit mourir au `;` — en scrutinee il
        // vivrait jusqu'à la fin du match et le second
        // `sess(h)` ci-dessous s'auto-bloquerait à vie.
        Some(h) => {
            let worked = sess(h).tick_animations(tab);
            match worked {
                Ok(w) => {
                    if h.rt.block_on(sess(h).drain_script_nav(tab)) > 0 {
                        1
                    } else {
                        i32::from(w)
                    }
                }
                Err(e) => {
                    set_err(e);
                    -2
                }
            }
        }
        None => -1,
    }
}

/// Dimensions de la frame courante. `*w`/`*hgt` reçoivent la
/// taille en pixels. `0` succès, `-2` pas de frame.
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `w`, `hgt` inscriptibles.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_frame_size(
    h: *mut ExplorerHandle,
    tab: u64,
    w: *mut u32,
    hgt: *mut u32,
) -> i32 {
    if w.is_null() || hgt.is_null() {
        set_err("sortie nulle");
        return -1;
    }
    match unsafe { handle(h) } {
        Some(h) => {
            if let Some((fw, fh, _)) = sess(h).frame_pixels(tab) {
                // SAFETY : pointeurs de sortie non nuls vérifiés.
                unsafe {
                    *w = fw;
                    *hgt = fh;
                }
                0
            } else {
                set_err("pas de frame");
                -2
            }
        }
        None => -1,
    }
}

/// Copie la frame RGBA dans `out` (capacité `cap` octets).
/// Retourne le nombre d'octets écrits (`w*h*4`), `0` si pas
/// de frame, `-1`/` -2` erreur.
///
/// # Safety
///
/// `out..out+cap` inscriptible.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_frame_copy(
    h: *mut ExplorerHandle,
    tab: u64,
    out: *mut u8,
    cap: usize,
) -> isize {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    match sess(h).frame_pixels(tab) {
        Some((_, _, px)) => {
            if px.len() > cap || out.is_null() {
                set_err("buffer trop petit ou nul");
                return -1;
            }
            // SAFETY : `out..out+px.len()` ⊂ `out..out+cap`
            // inscriptible par contrat FFI.
            unsafe { std::ptr::copy_nonoverlapping(px.as_ptr(), out, px.len()) };
            isize::try_from(px.len()).unwrap_or(isize::MAX)
        }
        None => 0,
    }
}

/// Pull **atomique** de la frame (étape 185, M1) : pixels
/// RGBA dans `out` + métadonnées JSON dans `meta` — un seul
/// verrou `Session` : `frame_size`/`frame_copy` puis une
/// lecture de méta laisseraient deux fenêtres de course.
/// Le JSON est `{"scroll_y":…,"media":[{i,r,v,f,t,c,n,s,o}]}`
/// (contrat `x-explorer-media` REST — `r` document, `v`/`c`
/// viewport). Retourne les octets pixels écrits
/// (`w*h*4`), `0` si pas de frame, `-1` erreur.
/// `*meta_len` reçoit les octets JSON écrits — `0` si
/// `meta` nul, `meta_cap` insuffisante ou `meta_len` nul
/// (les pixels restent servis ; l'hôte retombe alors sur
/// son comportement pré-185).
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `out..out+cap` et
/// `meta..meta+meta_cap` inscriptibles ; `meta_len`
/// inscriptible si non nul.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_frame_pull(
    h: *mut ExplorerHandle,
    tab: u64,
    out: *mut u8,
    cap: usize,
    meta: *mut u8,
    meta_cap: usize,
    meta_len: *mut u32,
) -> isize {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    match sess(h).frame_pull(tab) {
        Some((_, _, px, fm)) => {
            if px.len() > cap || out.is_null() {
                set_err("buffer pixels trop petit ou nul");
                return -1;
            }
            // SAFETY : `out..out+px.len()` ⊂ `out..out+cap`
            // inscriptible par contrat FFI.
            unsafe { std::ptr::copy_nonoverlapping(px.as_ptr(), out, px.len()) };
            if !meta_len.is_null() {
                // SAFETY : pointeur de sortie non nul vérifié.
                unsafe { *meta_len = 0 };
            }
            if !meta.is_null() && !meta_len.is_null() {
                if let Ok(json) = serde_json::to_vec(&fm) {
                    if json.len() <= meta_cap {
                        // SAFETY : `meta..meta+json.len()` ⊂
                        // `meta..meta+meta_cap` inscriptible.
                        unsafe { std::ptr::copy_nonoverlapping(json.as_ptr(), meta, json.len()) };
                        // SAFETY : vérifié non nul ci-dessus.
                        unsafe { *meta_len = u32::try_from(json.len()).unwrap_or(u32::MAX) };
                    }
                }
            }
            isize::try_from(px.len()).unwrap_or(isize::MAX)
        }
        None => 0,
    }
}

/// Pull du **top-slice** de la frame (étape 188 — M2
/// UX-vidéo, réponse `interface-equipes.md`) : RGBA+α des
/// items peints après la plus haute surface média, borné
/// au rect `top` servi dans le JSON de
/// [`explorer_frame_pull`] (`{"e":epoch,"top":{…}}`).
/// `epoch` doit égaler `e` — `-2` si périmée (une autre
/// peinture s'est interposée : l'hôte re-tire la paire),
/// `0` si aucun slice n'existe, `-1` erreur. `u64::MAX` =
/// génération courante acceptée (rafraîchissement du
/// chemin régions, sans méta — l'appairage reste garanti
/// sur les pulls pleins). Succès =
/// `w*h*4` octets ; `out_w`/`out_h` (non nuls) reçoivent
/// les dims **réelles** — `ceil(top.w×scale) ×
/// ceil(top.h×scale) × 4` octets de `cap` suffisent
/// toujours (`round ≤ ceil`).
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `out..out+cap`
/// inscriptible ; `out_w`/`out_h` inscriptibles si non
/// nuls.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_frame_top(
    h: *mut ExplorerHandle,
    tab: u64,
    epoch: u64,
    out: *mut u8,
    cap: usize,
    out_w: *mut u32,
    out_h: *mut u32,
) -> isize {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    let want = if epoch == u64::MAX { None } else { Some(epoch) };
    match sess(h).frame_top(tab, want) {
        explorer_core::FrameTop::Slice(_, frame) => {
            if frame.pixels.len() > cap || out.is_null() {
                set_err("buffer top-slice trop petit ou nul");
                return -1;
            }
            // SAFETY : `out..out+px.len()` ⊂ `out..out+cap`
            // inscriptible par contrat FFI.
            unsafe {
                std::ptr::copy_nonoverlapping(frame.pixels.as_ptr(), out, frame.pixels.len());
            }
            if !out_w.is_null() {
                // SAFETY : pointeur de sortie non nul vérifié.
                unsafe { *out_w = frame.width };
            }
            if !out_h.is_null() {
                // SAFETY : pointeur de sortie non nul vérifié.
                unsafe { *out_h = frame.height };
            }
            isize::try_from(frame.pixels.len()).unwrap_or(isize::MAX)
        }
        explorer_core::FrameTop::Stale => -2,
        explorer_core::FrameTop::None => 0,
    }
}

/// Copie la région `(x, y, w, h)` — pixels physiques — de
/// la frame dans `out` (étape 55 : pull partiel piloté par
/// les dirty-rects de [`explorer_dirty_rects`]). La région
/// est clippée aux bornes de la frame ; le retour est le
/// nombre d'octets écrits (`w2*h2*4` après clip), `0` si
/// pas de frame ou région vide, `-1` erreur.
/// Lit la taille effective via `explorer_frame_size` puis
/// borne `x+w ≤ fw`, `y+h ≤ fh` côté appelant.
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `out..out+cap`
/// inscriptible (`cap ≥ w*h*4` de la région demandée).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_frame_copy_region(
    h: *mut ExplorerHandle,
    tab: u64,
    x: u32,
    y: u32,
    w: u32,
    hgt: u32,
    out: *mut u8,
    cap: usize,
) -> isize {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    match sess(h).frame_region(tab, x, y, w, hgt) {
        Some((_, _, px)) => {
            if px.len() > cap || out.is_null() {
                set_err("buffer trop petit ou nul");
                return -1;
            }
            // SAFETY : `out..out+px.len()` ⊂ `out..out+cap`
            // inscriptible par contrat FFI.
            unsafe { std::ptr::copy_nonoverlapping(px.as_ptr(), out, px.len()) };
            isize::try_from(px.len()).unwrap_or(isize::MAX)
        }
        None => 0,
    }
}

/// Vide et écrit les dirty-rects accumulées dans `out` —
/// quadruplets `(x, y, w, h)` en pixels physiques, `cap`
/// = nombre de `f32` (`4×` le nombre de régions max).
/// Retourne le nombre de régions écrites, `-1` erreur.
/// Étape 55 : le client FFI sans canal d'événements pull
/// les régions puis copie via
/// [`explorer_frame_copy_region`].
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `out..out+cap`
/// inscriptible ; `out` peut être nul si `cap == 0` (sonde
/// du nombre de régions — non implémenté, retourne `-1`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_dirty_rects(
    h: *mut ExplorerHandle,
    tab: u64,
    out: *mut f32,
    cap: usize,
) -> isize {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    if out.is_null() {
        set_err("buffer nul");
        return -1;
    }
    let rects = sess(h).take_dirty(tab);
    let need = rects.len() * 4;
    if need > cap {
        set_err("buffer trop petit");
        return -1;
    }
    for (i, r) in rects.iter().enumerate() {
        // SAFETY : `i*4+4 ≤ need ≤ cap` — `out..out+cap`
        // inscriptible par contrat FFI.
        unsafe {
            *out.add(i * 4) = r.x;
            *out.add(i * 4 + 1) = r.y;
            *out.add(i * 4 + 2) = r.w;
            *out.add(i * 4 + 3) = r.h;
        }
    }
    isize::try_from(rects.len()).unwrap_or(isize::MAX)
}

/// URL courante de l'onglet → `buf` (UTF-8). Retourne la
/// longueur écrite, `0` si aucune URL, `-1` buffer invalide.
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `buf..buf+cap`
/// inscriptible.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_tab_url(
    h: *mut ExplorerHandle,
    tab: u64,
    buf: *mut u8,
    cap: usize,
) -> isize {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    match sess(h).tab_url(tab) {
        Some(url) => unsafe { write_str(&url, buf, cap) },
        None => 0,
    }
}

/// Copie le titre courant de l'onglet dans `buf` (UTF-8,
/// sans NUL final si plein — même contrat que
/// [`explorer_tab_url`]). Retourne la longueur écrite, `0`
/// si l'onglet est inconnu ou sans titre, `-1` si buffer
/// invalide (étape 61 : le `<title>` n'avait aucun canal
/// FFI — l'app devait étiqueter ses onglets à l'URL).
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `buf..buf+cap`
/// inscriptible.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_tab_title(
    h: *mut ExplorerHandle,
    tab: u64,
    buf: *mut u8,
    cap: usize,
) -> isize {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    match sess(h).tab_info(tab).and_then(|t| t.title) {
        Some(title) => unsafe { write_str(&title, buf, cap) },
        None => 0,
    }
}

/// Décalage de scroll de l'onglet en px CSS (étape 61) —
/// `-1.0` si handle/onglet invalide (l'offset est ≥ 0, le
/// code retour est non ambigu).
///
/// # Safety
///
/// `h` issu de [`explorer_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_tab_scroll(h: *mut ExplorerHandle, tab: u64) -> f64 {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1.0;
    };
    sess(h)
        .tab_info(tab)
        .map_or(-1.0, |t| f64::from(t.scroll_y))
}

/// Cibles sous `(x, y)` **viewport** — sérialise
/// `{ "link": …|null, "image": …|null, "media": …|null }`
/// UTF-8 dans `buf` (étape 186 : il n'existait aucun ABI
/// hit-test — le menu contextuel et l'arbitrage pointer
/// média étaient REST-seuls). Retourne la longueur écrite,
/// `0` onglet inconnu, `-1` buffer invalide ou trop petit.
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `buf..buf+cap`
/// inscriptible.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_hit_targets(
    h: *mut ExplorerHandle,
    tab: u64,
    x: f32,
    y: f32,
    buf: *mut u8,
    cap: usize,
) -> isize {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    let Ok(hit) = sess(h).hit_targets_at(tab, x, y) else {
        return 0;
    };
    let json = serde_json::json!({
        "link": hit.link,
        "image": hit.image,
        "media": hit.media,
    })
    .to_string();
    if json.len() > cap {
        set_err("buffer trop petit");
        return -1;
    }
    unsafe { write_str(&json, buf, cap) }
}

/// Taille maximale d'un corps JSON média (`claim`/`state`)
/// passé à l'ABI (étape 71e).
const MAX_MEDIA_JSON: usize = 64 * 1024;

/// `MediaError` → code négatif FFI : `-410` surface morte,
/// `-409` conflit (déjà claimée / `state` sans claim) —
/// mêmes statuts que le REST (`410`/`409`).
fn media_err_code(e: explorer_core::MediaError) -> i32 {
    match e {
        explorer_core::MediaError::Dead => -410,
        explorer_core::MediaError::Taken | explorer_core::MediaError::NotClaimed => -409,
    }
}

/// Draine les événements `MediaSurface` **et**
/// téléchargements accumulés vers `buf` — une ligne NDJSON
/// `{"event": "<nom_sse>", "data": <charge>}` par
/// événement, charges **identiques** au SSE (contrat §
/// `MediaSurface`, variante in-process étape 71e ; les
/// `download_*` rejoignent le drain en étape 176). Seules
/// des lignes complètes sont écrites : si le
/// dernier `\n` excède `cap`, le reste est conservé dans le
/// handle et ressorti à l'appel suivant. Retourne le nombre
/// d'octets écrits (`0` = rien en attente), `-1` argument
/// invalide.
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `buf..buf+cap`
/// inscriptible si `cap > 0` (`buf` peut être nul quand
/// `cap == 0` : les lignes restent en attente).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_media_events(
    h: *mut ExplorerHandle,
    buf: *mut u8,
    cap: usize,
) -> isize {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    // Vide le canal dans `media_pending` (filtré média +
    // téléchargements — étape 176 : les events
    // `download_*` empruntent le même drain NDJSON, sans
    // ABI nouvelle ; les autres variants restent sans
    // canal FFI).
    loop {
        match h.events.try_recv() {
            Ok(ev) => {
                let ser = explorer_core::media_event_json(&ev)
                    .or_else(|| explorer_core::download_event_json(&ev));
                if let Some((name, data)) = ser {
                    use std::fmt::Write;
                    let _ = writeln!(h.media_pending, "{{\"event\":\"{name}\",\"data\":{data}}}");
                }
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
            Err(
                tokio::sync::broadcast::error::TryRecvError::Empty
                | tokio::sync::broadcast::error::TryRecvError::Closed,
            ) => break,
        }
    }
    if cap == 0 {
        return 0;
    }
    if buf.is_null() {
        set_err("buffer nul");
        return -1;
    }
    // Jamais de ligne tronquée : borne au dernier `\n`
    // contenu dans `cap`.
    let Some(end) = h.media_pending[..h.media_pending.len().min(cap)].rfind('\n') else {
        return 0;
    };
    let n = end + 1;
    // SAFETY : `buf..buf+n` inscriptible (`n ≤ cap`), source
    // détenue par le handle.
    unsafe {
        std::ptr::copy_nonoverlapping(h.media_pending.as_ptr(), buf, n);
    }
    h.media_pending.drain(..n);
    isize::try_from(n).unwrap_or(isize::MAX)
}

/// Liste des surfaces média vivantes de l'onglet → `buf`
/// (JSON `{"media": [ … ]}` — même DTO que `GET
/// /api/tabs/{id}/media`, étape 71e). Retourne la longueur
/// écrite, `0` si aucune surface, `-1` buffer invalide ou
/// trop petit.
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `buf..buf+cap`
/// inscriptible.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_media_list(
    h: *mut ExplorerHandle,
    tab: u64,
    buf: *mut u8,
    cap: usize,
) -> isize {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    let list: Vec<serde_json::Value> = sess(h)
        .media_list(tab)
        .iter()
        .map(explorer_core::media_offer_json)
        .collect();
    if list.is_empty() {
        return 0;
    }
    let s = serde_json::json!({ "media": list }).to_string();
    if s.len() > cap {
        set_err("buffer trop petit");
        return -1;
    }
    unsafe { write_str(&s, buf, cap) }
}

/// Claim d'une surface par l'hôte — `json` porte le corps
/// `{"codecs": ["video/mp4", …]}` (UTF-8, ≤ 64 Kio ; corps
/// vide ou non-JSON = `[]` toléré). Retour : `0` claim
/// acquis, `-409` déjà claimée, `-410` surface morte,
/// `-1` argument invalide.
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `json..json+len`
/// lisible si `len > 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_media_claim(
    h: *mut ExplorerHandle,
    tab: u64,
    media: u64,
    json: *const u8,
    len: usize,
) -> i32 {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    let codecs = if len == 0 {
        Vec::new()
    } else {
        let Some(s) = (unsafe { read_str(json, len, MAX_MEDIA_JSON) }) else {
            return -1;
        };
        serde_json::from_str(s)
            .map(|v: serde_json::Value| explorer_core::media_codecs_from_json(&v))
            .unwrap_or_default()
    };
    match sess(h).media_claim(tab, media, codecs) {
        Ok(()) => 0,
        Err(e) => media_err_code(e),
    }
}

/// Release d'une surface claimée — l'hôte se détache, la
/// surface retombe sur les stubs ADR-0023. Retour : `0`,
/// `-410` surface morte, `-1` handle invalide.
///
/// # Safety
///
/// `h` issu de [`explorer_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_media_release(
    h: *mut ExplorerHandle,
    tab: u64,
    media: u64,
) -> i32 {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    match sess(h).media_release(tab, media) {
        Ok(()) => 0,
        Err(e) => media_err_code(e),
    }
}

/// Rapport d'état de lecture hôte → moteur (battement ≤
/// 4 Hz côté hôte — contrat § `MediaSurface`). `json`
/// porte `{position_ms, duration_ms, buffered_ms, state,
/// error?}` avec `state ∈ "playing"|"paused"|"ended"|
/// "error"`. Retour : `0`, `-400` JSON/état invalide,
/// `-409` non claimée, `-410` morte, `-1` argument.
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `json..json+len`
/// lisible.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_media_state(
    h: *mut ExplorerHandle,
    tab: u64,
    media: u64,
    json: *const u8,
    len: usize,
) -> i32 {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    let Some(s) = (unsafe { read_str(json, len, MAX_MEDIA_JSON) }) else {
        return -1;
    };
    let report = serde_json::from_str(s)
        .ok()
        .and_then(|v: serde_json::Value| explorer_core::media_report_from_json(&v));
    let Some(report) = report else {
        set_err("state invalide (∈ playing|paused|ended|error requis)");
        return -400;
    };
    match sess(h).media_report(tab, media, report) {
        Ok(()) => 0,
        Err(e) => media_err_code(e),
    }
}

/// Cap d'un appel [`explorer_media_audio`] — même borne que
/// la route REST (`16 Ko` ≈ 40 ms de flux à 384 Ko/s).
const MAX_MEDIA_AUDIO: usize = 16 * 1024;

/// Chunk PCM de la surface claimée (Étape 204b — tee
/// `AUDIO_MODE_TAP` dépôt, ADR-0032) : `pcm..pcm+len`
/// interleaved non coupé trame ; `rate`/`channels`/`format`
/// = spec du chunk (dépôt : 48000/2/`4`=f32 LE) → ring
/// `pcm` moteur, lu par le graphe Web Audio (204c).
/// Retour : `0`, `-1` argument (null/vide/>16 Ko),
/// `-409` non claimée, `-410` morte.
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `pcm..pcm+len` lisible.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_media_audio(
    h: *mut ExplorerHandle,
    tab: u64,
    media: u64,
    rate: u32,
    channels: u32,
    format: i32,
    pcm: *const u8,
    len: usize,
) -> i32 {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    if pcm.is_null() || len == 0 || len > MAX_MEDIA_AUDIO {
        return -1;
    }
    let chunk = unsafe { std::slice::from_raw_parts(pcm, len) };
    let spec = explorer_core::MediaAudioSpec {
        sample_rate: rate,
        channels: u16::try_from(channels.min(u32::from(u16::MAX))).unwrap_or(u16::MAX),
        format,
    };
    match sess(h).media_audio(tab, media, spec, chunk) {
        Ok(()) => 0,
        Err(e) => media_err_code(e),
    }
}

/// Télécharge `url` directement sous `state/downloads/`
/// (« enregistrer la source » média / « enregistrer sous » —
/// pile transport complète : cookies, cache, HSTS). `0`
/// succès, `-1` handle/arg invalide, `-2` erreur moteur
/// ([`explorer_last_error`]).
///
/// **Compat** — bloque le worker FFI pendant tout le
/// transfert (étape 174 : l'attente se fait par events
/// sous `&mut session`). Préférer
/// [`explorer_download_start`], qui rend l'id immédiatement
/// et suit le cycle en events NDJSON.
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `url..url+len` lisible.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_download_url(
    h: *mut ExplorerHandle,
    tab: u64,
    url: *const u8,
    len: usize,
) -> i32 {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    let Some(u) = (unsafe { read_str(url, len, MAX_URL) }) else {
        return -1;
    };
    match h.rt.block_on(sess(h).download_url(tab, u, None, None)) {
        Ok(_) => 0,
        Err(e) => {
            set_err(e);
            -2
        }
    }
}

/// Démarre un téléchargement direct **non bloquant**
/// (étape 176) : rend l'`id` (> 0) immédiatement — le job
/// détaché stream vers `.part` hors verrou session, puis
/// `download_started`/`download_progress`/`download_done`/
/// `download_failed` arrivent dans le drain NDJSON de
/// [`explorer_media_events`]. `path`/`filename`
/// optionnels (`*_len` `0` = absent — `path` explicite
/// écrit verbatim, `filename` = suggestion avant
/// `Content-Disposition`). `-1` handle/arg invalide, `-2`
/// erreur moteur ([`explorer_last_error`]).
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `url..url+url_len`,
/// `path..path+path_len`, `filename..filename+filename_len`
/// lisibles quand `*_len > 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_download_start(
    h: *mut ExplorerHandle,
    tab: u64,
    url: *const u8,
    url_len: usize,
    path: *const u8,
    path_len: usize,
    filename: *const u8,
    filename_len: usize,
) -> i64 {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    let Some(url_s) = (unsafe { read_str(url, url_len, MAX_URL) }) else {
        return -1;
    };
    let path_opt = unsafe { read_str(path, path_len, MAX_URL) }.map(std::path::PathBuf::from);
    let hint = unsafe { read_str(filename, filename_len, MAX_URL) }.map(str::to_string);
    let mut session = sess(h);
    // `tokio::spawn` du job exige un contexte runtime —
    // `enter()` le pose sans re-bloquer (`sess` a déjà
    // rendu son `block_on`).
    let _rt = h.rt.enter();
    match session.start_download(tab, url_s, path_opt, hint) {
        Ok(id) => i64::try_from(id).unwrap_or(-2),
        Err(e) => {
            set_err(e);
            -2
        }
    }
}

/// Annule un téléchargement direct en cours (étape 176)
/// — la tâche purge son `.part` et émet `download_failed`
/// (`annulé`). `0` succès, `-1` handle invalide, `-2`
/// identifiant inconnu / autre onglet.
///
/// # Safety
///
/// `h` issu de [`explorer_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_download_cancel(
    h: *mut ExplorerHandle,
    tab: u64,
    dl: u64,
) -> i32 {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    match sess(h).cancel_download(tab, dl) {
        Ok(()) => 0,
        Err(e) => {
            set_err(e);
            -2
        }
    }
}

/// Accepte une offre de téléchargement (`dl` = identifiant
/// de `download_offered`) — `path` UTF-8 optionnel
/// (`path_len` `0` = `download_dir` du profil + nom suggéré).
/// Copie le chemin écrit dans `buf` (contrat `cap` UTF-8),
/// longueur rendue ; `-1` handle/arg invalide, `-2` erreur
/// moteur ([`explorer_last_error`]).
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `path..path+len` lisible,
/// `buf..buf+cap` inscriptible.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_download_accept(
    h: *mut ExplorerHandle,
    tab: u64,
    dl: u64,
    path: *const u8,
    path_len: usize,
    buf: *mut u8,
    cap: usize,
) -> isize {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    let opt = unsafe { read_str(path, path_len, MAX_URL) }.map(std::path::PathBuf::from);
    match sess(h).accept_download(tab, dl, opt) {
        Ok(t) => unsafe { write_str(&t.display().to_string(), buf, cap) },
        Err(e) => {
            set_err(e);
            -2
        }
    }
}

/// Rejette une offre de téléchargement — le corps est
/// rejoué comme document (« dump texte »). `0` succès,
/// `-1` handle invalide, `-2` erreur moteur.
///
/// # Safety
///
/// `h` issu de [`explorer_create`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_download_decline(
    h: *mut ExplorerHandle,
    tab: u64,
    dl: u64,
) -> i32 {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    match h.rt.block_on(sess(h).decline_download(tab, dl)) {
        Ok(_) => 0,
        Err(e) => {
            set_err(e);
            -2
        }
    }
}

/// Recherche dans la page — `dir` `0` : nouvelle requête
/// (`find_in_page`) ; `±1` : occurrence suivante/précédente
/// (`find_step`). JSON `{count, index, query}` dans `buf`
/// (même contrat que [`explorer_last_error`] ; miroir du
/// `POST /find` REST). `-1` handle/arg invalide.
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `query..query+len`
/// lisible, `buf..buf+cap` inscriptible.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_find(
    h: *mut ExplorerHandle,
    tab: u64,
    query: *const u8,
    len: usize,
    dir: i64,
    buf: *mut u8,
    cap: usize,
) -> isize {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    let q = unsafe { read_str(query, len, MAX_URL) }.unwrap_or("");
    let (count, index) = if dir == 0 {
        (sess(h).find_in_page(tab, q), 0)
    } else {
        let (i, n) = sess(h).find_step(tab, dir);
        (n, i)
    };
    let s = serde_json::json!({"count": count, "index": index, "query": q}).to_string();
    unsafe { write_str(&s, buf, cap) }
}

/// Journal de navigation du profil — JSON
/// `[{"url":…,"title":…}]` dans `buf` (même contrat
/// UTF-8/`cap` que [`explorer_last_error`]).
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `buf..buf+cap`
/// inscriptible.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_history(
    h: *mut ExplorerHandle,
    buf: *mut u8,
    cap: usize,
) -> isize {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    let list: Vec<serde_json::Value> = sess(h)
        .history()
        .iter()
        .map(|v| serde_json::json!({"url": v.url, "title": v.title}))
        .collect();
    let s = serde_json::to_string(&serde_json::json!({"history": list}))
        .unwrap_or_else(|_| "{\"history\":[]}".to_string());
    unsafe { write_str(&s, buf, cap) }
}

/// Signets du profil — JSON `[{"url":…,"title":…}]` dans `buf`
/// (même contrat UTF-8/`cap` que [`explorer_last_error`]).
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `buf..buf+cap` inscriptible.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_bookmarks(
    h: *mut ExplorerHandle,
    buf: *mut u8,
    cap: usize,
) -> isize {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    let list: Vec<serde_json::Value> = sess(h)
        .bookmarks()
        .iter()
        .map(|b| serde_json::json!({"url": b.url, "title": b.title}))
        .collect();
    let s = serde_json::to_string(&serde_json::json!({"bookmarks": list}))
        .unwrap_or_else(|_| "{\"bookmarks\":[]}".to_string());
    unsafe { write_str(&s, buf, cap) }
}

/// Ajoute un signet (`url`+`title` UTF-8) — `1` ajouté,
/// `0` refusé (doublon/vide), `-1` handle invalide.
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; pointeurs UTF-8 valides
/// sur `*_len` octets.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_bookmark_add(
    h: *mut ExplorerHandle,
    url: *const u8,
    url_len: usize,
    title: *const u8,
    title_len: usize,
) -> i32 {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    let Some(u) = (unsafe { read_str(url, url_len, MAX_URL) }) else {
        return 0;
    };
    let t = unsafe { read_str(title, title_len, MAX_URL) };
    i32::from(sess(h).add_bookmark(u.to_string(), t.map(str::to_string)))
}

/// Retire un signet par URL (`url` UTF-8) — `1` retiré,
/// `0` absent, `-1` handle invalide.
///
/// # Safety
///
/// `h` issu de [`explorer_create`] ; `url..url+len` valide.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn explorer_bookmark_del(
    h: *mut ExplorerHandle,
    url: *const u8,
    url_len: usize,
) -> i32 {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    let Some(u) = (unsafe { read_str(url, url_len, MAX_URL) }) else {
        return 0;
    };
    i32::from(sess(h).remove_bookmark(u))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use explorer_layout::font::{FontRequest, FontSource};
    use explorer_net::error::NetResult;
    use explorer_net::http::{Request, Response};
    use explorer_test_support::fixture_ttf;

    /// Fonte fixture (mêmes métriques que les tests core).
    struct FixtureFont;
    impl FontSource for FixtureFont {
        fn font_data(&self, _req: &FontRequest) -> Option<Vec<u8>> {
            Some(fixture_ttf())
        }
    }

    /// Transport simulé minimal : `path → (status, corps)`.
    struct FakeTransport {
        routes: Mutex<HashMap<String, (u16, Vec<u8>)>>,
    }

    impl FakeTransport {
        fn new() -> Self {
            Self {
                routes: Mutex::new(HashMap::new()),
            }
        }

        fn route(&self, path: &str, status: u16, body: &'static str) {
            if let Ok(mut r) = self.routes.lock() {
                r.insert(path.to_string(), (status, body.as_bytes().to_vec()));
            }
        }
    }

    #[async_trait]
    impl HttpTransport for FakeTransport {
        async fn fetch(&self, req: &Request) -> NetResult<Response> {
            let found = self
                .routes
                .lock()
                .ok()
                .and_then(|r| r.get(req.url.path()).cloned());
            match found {
                Some((status, body)) => Ok(Response {
                    status,
                    reason: "fake".to_string(),
                    headers: vec![],
                    body,
                    reused_connection: false,
                    resolved_url: None,
                }),
                None => Ok(Response {
                    status: 404,
                    reason: "fake".to_string(),
                    headers: vec![],
                    body: b"not found".to_vec(),
                    reused_connection: false,
                    resolved_url: None,
                }),
            }
        }
    }

    /// Handle de test : session sur transport simulé (aucune
    /// socket), même shape que `explorer_create`.
    fn test_handle() -> *mut ExplorerHandle {
        test_handle_with(None)
    }

    /// Variante à store de profil injecté (étape 61).
    fn test_handle_with(profile_dir: Option<&Path>) -> *mut ExplorerHandle {
        let t = Arc::new(FakeTransport::new());
        t.route("/a", 200, "<html><body><h1>bonjour</h1></body></html>");
        t.route(
            "/t",
            200,
            "<html><head><title>ma page</title></head><body><h1>t</h1></body></html>",
        );
        t.route(
            "/v",
            200,
            "<html><body><video src=\"v.mp4\" controls></video></body></html>",
        );
        let tr: Arc<dyn HttpTransport> = t;
        let mut session = Session::new(tr, Arc::new(FixtureFont));
        if let Some(d) = profile_dir {
            session.set_store(Arc::new(explorer_core::JsonFileStore::open(d)));
        }
        session.set_viewport(800.0, 600.0, 1.0);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let events = session.subscribe();
        Box::into_raw(Box::new(ExplorerHandle {
            rt: std::sync::Arc::new(rt),
            session: std::sync::Arc::new(tokio::sync::Mutex::new(session)),
            events,
            media_pending: String::new(),
            api_port: 0,
            _profile_lock: None,
            stop: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }))
    }

    /// Étape 177/b2 — le loopback embarqué sert les routes
    /// `/api/…/media*` : bind `127.0.0.1:0`, un `GET` brut
    /// sur `/media` reçoit du HTTP, `explorer_api_port`
    /// rend le port OS-élu.
    #[test]
    fn api_port_loopback_serves_media() {
        use std::io::{Read, Write};
        let rt = std::sync::Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap(),
        );
        let session = std::sync::Arc::new(tokio::sync::Mutex::new(Session::new(
            Arc::new(FakeTransport::new()),
            Arc::new(FixtureFont),
        )));
        let port = spawn_media_loopback(&rt, &session);
        assert_ne!(port, 0, "le bind loopback doit élire un port");
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(b"GET /api/tabs/1/media HTTP/1.0\r\nHost: loopback\r\n\r\n")
            .unwrap();
        let mut buf = String::new();
        s.read_to_string(&mut buf).unwrap();
        assert!(
            buf.starts_with("HTTP/1.0 200") || buf.starts_with("HTTP/1.1 200"),
            "la route media répond en HTTP : {buf}"
        );
    }

    #[test]
    fn create_destroy() {
        let h = explorer_create();
        assert!(!h.is_null());
        // SAFETY : handle frais.
        unsafe { explorer_destroy(h) };
    }

    #[test]
    fn null_handle_rejete() {
        // SAFETY : les fonctions valident le pointeur nul.
        unsafe {
            assert_eq!(explorer_open_tab(std::ptr::null_mut()), 0);
            assert_eq!(
                explorer_navigate(std::ptr::null_mut(), 1, b"x".as_ptr(), 1),
                -1
            );
            assert_eq!(
                explorer_frame_copy(std::ptr::null_mut(), 1, std::ptr::null_mut(), 0),
                -1
            );
        }
    }

    /// Pipeline complet via l'ABI : open → navigate →
    /// `frame_size` → `frame_copy` → URL → close → destroy.
    #[test]
    fn navigate_frame_rgba() {
        let h = test_handle();
        // SAFETY : handle frais, appels sérialisés.
        unsafe {
            let tab = explorer_open_tab(h);
            assert_ne!(tab, 0);
            let url = b"http://test/a";
            assert_eq!(explorer_navigate(h, tab, url.as_ptr(), url.len()), 0);

            let (mut w, mut hgt) = (0u32, 0u32);
            assert_eq!(
                explorer_frame_size(
                    h,
                    tab,
                    std::ptr::addr_of_mut!(w),
                    std::ptr::addr_of_mut!(hgt)
                ),
                0
            );
            assert_eq!((w, hgt), (800, 600));

            let cap = (w * hgt * 4) as usize;
            let mut buf = vec![0u8; cap];
            let n = explorer_frame_copy(h, tab, buf.as_mut_ptr(), cap);
            assert_eq!(n, isize::try_from(cap).unwrap_or(isize::MAX));
            // Fond blanc : un pixel non blanc doit exister
            // (le texte « bonjour » est rasterisé).
            assert!(buf.chunks(4).any(|px| px[..3] != [255, 255, 255]));

            // Buffer trop petit → rejet borné, pas d'écriture.
            assert_eq!(explorer_frame_copy(h, tab, buf.as_mut_ptr(), 4), -1);

            let mut urlbuf = [0u8; 256];
            let n = explorer_tab_url(h, tab, urlbuf.as_mut_ptr(), 256);
            let n = usize::try_from(n).unwrap_or(0);
            assert_eq!(&urlbuf[..n], b"http://test/a".as_slice());

            assert_eq!(explorer_close_tab(h, tab), 0);
            explorer_destroy(h);
        }
    }

    /// Étape 185 — `explorer_frame_pull` : pixels + JSON
    /// `{scroll_y, w, h, media[]}` de la **même** frame en un
    /// seul appel — parité du contrat REST `x-explorer-*`.
    #[test]
    fn frame_pull_atomic_pixels_and_meta() {
        let h = test_handle();
        // SAFETY : handle frais, appels sérialisés.
        unsafe {
            let tab = explorer_open_tab(h);
            let url = b"http://test/v";
            assert_eq!(explorer_navigate(h, tab, url.as_ptr(), url.len()), 0);

            let (mut w, mut hgt) = (0u32, 0u32);
            assert_eq!(
                explorer_frame_size(
                    h,
                    tab,
                    std::ptr::addr_of_mut!(w),
                    std::ptr::addr_of_mut!(hgt)
                ),
                0
            );
            let cap = (w * hgt * 4) as usize;
            let mut px = vec![0u8; cap];
            let mut meta = vec![0u8; 64 * 1024];
            let mut meta_len = 0u32;
            let n = explorer_frame_pull(
                h,
                tab,
                px.as_mut_ptr(),
                cap,
                meta.as_mut_ptr(),
                meta.len(),
                std::ptr::addr_of_mut!(meta_len),
            );
            assert_eq!(n, isize::try_from(cap).unwrap_or(isize::MAX));
            assert!(meta_len > 0, "méta JSON servie avec les pixels");
            let json: serde_json::Value =
                serde_json::from_slice(&meta[..usize::try_from(meta_len).unwrap_or(0)])
                    .expect("JSON méta valide");
            assert_eq!(json["scroll_y"], 0.0);
            assert_eq!(json["w"], w);
            assert_eq!(json["h"], hgt);
            let arr = json["media"].as_array().expect("media[] tableau");
            assert_eq!(arr.len(), 1, "une surface vidéo : {arr:?}");
            assert_eq!(arr[0]["i"], 1);
            assert!(arr[0]["r"]["w"].as_f64().unwrap() > 0.0);
            assert_eq!(arr[0]["v"], arr[0]["r"], "scroll 0 → view = rect");
            assert_eq!(arr[0]["s"], true);

            // `meta` nul / `meta_cap` insuffisant → pixels
            // servis quand même, `meta_len` reste 0 (repli
            // pré-185 honnête, jamais de JSON tronqué).
            let mut small = [0u8; 8];
            let n2 = explorer_frame_pull(
                h,
                tab,
                px.as_mut_ptr(),
                cap,
                small.as_mut_ptr(),
                small.len(),
                std::ptr::addr_of_mut!(meta_len),
            );
            assert!(n2 > 0);
            assert_eq!(meta_len, 0, "cap insuffisant → meta_len 0");

            explorer_close_tab(h, tab);
            explorer_destroy(h);
        }
    }

    /// Étape 186 — `explorer_hit_targets` : premier ABI
    /// hit-test — `{link, image, media}` JSON. Sur `/v`
    /// (`<video controls>`), le centre du rect rend
    /// `"media":1` ; un point hors surface → `"media":null`
    /// (clé présente — `mediaKnown` vrai côté hôte).
    #[test]
    fn hit_targets_media_json() {
        let h = test_handle();
        // SAFETY : handle frais, appels sérialisés.
        unsafe {
            let tab = explorer_open_tab(h);
            let url = b"http://test/v";
            assert_eq!(explorer_navigate(h, tab, url.as_ptr(), url.len()), 0);

            // Rect de la surface via la méta du pull.
            let (mut w, mut hgt) = (0u32, 0u32);
            assert_eq!(
                explorer_frame_size(
                    h,
                    tab,
                    std::ptr::addr_of_mut!(w),
                    std::ptr::addr_of_mut!(hgt)
                ),
                0
            );
            let cap = (w * hgt * 4) as usize;
            let mut px = vec![0u8; cap];
            let mut meta = vec![0u8; 64 * 1024];
            let mut meta_len = 0u32;
            assert!(
                explorer_frame_pull(
                    h,
                    tab,
                    px.as_mut_ptr(),
                    cap,
                    meta.as_mut_ptr(),
                    meta.len(),
                    std::ptr::addr_of_mut!(meta_len),
                ) > 0
            );
            let mj: serde_json::Value =
                serde_json::from_slice(&meta[..usize::try_from(meta_len).unwrap_or(0)])
                    .expect("méta JSON");
            let r = &mj["media"][0]["r"];
            #[allow(clippy::cast_possible_truncation)]
            let (cx, cy) = (
                (r["x"].as_f64().unwrap() + r["w"].as_f64().unwrap() / 2.0) as f32,
                (r["y"].as_f64().unwrap() + r["h"].as_f64().unwrap() / 2.0) as f32,
            );

            let mut buf = [0u8; 1024];
            let n = explorer_hit_targets(h, tab, cx, cy, buf.as_mut_ptr(), buf.len());
            let n = usize::try_from(n).unwrap_or(0);
            let hit: serde_json::Value = serde_json::from_slice(&buf[..n]).expect("JSON cibles");
            assert_eq!(hit["media"], 1, "vidéo nue → id : {hit}");
            assert!(hit["link"].is_null() && hit["image"].is_null());

            let n = explorer_hit_targets(h, tab, 790.0, 590.0, buf.as_mut_ptr(), buf.len());
            let n = usize::try_from(n).unwrap_or(0);
            let miss: serde_json::Value = serde_json::from_slice(&buf[..n]).expect("JSON cibles");
            assert!(
                miss.get("media").is_some_and(serde_json::Value::is_null),
                "hors média → clé présente à null : {miss}"
            );

            // Handle nul → -1, onglet inconnu → 0.
            assert_eq!(
                explorer_hit_targets(
                    std::ptr::null_mut(),
                    tab,
                    0.0,
                    0.0,
                    buf.as_mut_ptr(),
                    buf.len()
                ),
                -1
            );
            assert_eq!(
                explorer_hit_targets(h, 999, 0.0, 0.0, buf.as_mut_ptr(), buf.len()),
                0
            );

            explorer_close_tab(h, tab);
            explorer_destroy(h);
        }
    }

    /// Étape 188 — `explorer_frame_top` : le pull rend
    /// `e`/`top` dans le JSON ; le slice de la bonne epoch
    /// est servi (dims réelles via `out_w`/`out_h`), epoch
    /// périmée → `-2`, `u64::MAX` → génération courante.
    #[test]
    fn frame_top_export() {
        let h = test_handle();
        // SAFETY : handle frais, appels sérialisés.
        unsafe {
            let tab = explorer_open_tab(h);
            let url = b"http://test/v";
            assert_eq!(explorer_navigate(h, tab, url.as_ptr(), url.len()), 0);

            let (mut w, mut hgt) = (0u32, 0u32);
            assert_eq!(
                explorer_frame_size(
                    h,
                    tab,
                    std::ptr::addr_of_mut!(w),
                    std::ptr::addr_of_mut!(hgt)
                ),
                0
            );
            let cap = (w * hgt * 4) as usize;
            let mut px = vec![0u8; cap];
            let mut meta = vec![0u8; 64 * 1024];
            let mut meta_len = 0u32;
            assert!(
                explorer_frame_pull(
                    h,
                    tab,
                    px.as_mut_ptr(),
                    cap,
                    meta.as_mut_ptr(),
                    meta.len(),
                    std::ptr::addr_of_mut!(meta_len),
                ) > 0
            );
            let mj: serde_json::Value =
                serde_json::from_slice(&meta[..usize::try_from(meta_len).unwrap_or(0)])
                    .expect("méta JSON");
            let epoch = mj["e"].as_u64().expect("e = epoch de la frame");
            assert!(epoch > 0);
            let top = mj["top"].as_object().expect("top = rect du slice");
            let tw = top["w"].as_f64().unwrap();
            let th = top["h"].as_f64().unwrap();
            assert!(tw > 0.0 && th > 0.0);

            // cap = taille frame entière ≥ slice ; dims
            // réelles rendues par out_w/out_h.
            let mut spx = vec![0u8; cap];
            let (mut sw, mut sh) = (0u32, 0u32);
            let n = explorer_frame_top(
                h,
                tab,
                epoch,
                spx.as_mut_ptr(),
                spx.len(),
                std::ptr::addr_of_mut!(sw),
                std::ptr::addr_of_mut!(sh),
            );
            assert!(n > 0, "slice servi à l'epoch courante");
            assert!(sw > 0 && sh > 0);
            assert_eq!(
                usize::try_from(n).unwrap_or(0),
                sw as usize * sh as usize * 4
            );

            // Sans dims de sortie — la variante la plus
            // appelée (probe + comparaison de code retour).
            let top_at = |tab: u64, epoch: u64, px: &mut Vec<u8>| -> isize {
                explorer_frame_top(
                    h,
                    tab,
                    epoch,
                    px.as_mut_ptr(),
                    px.len(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };

            // Epoch périmée → -2 (appairage refusé) ;
            // u64::MAX → génération courante servie.
            assert_eq!(top_at(tab, epoch + 1, &mut spx), -2);
            assert!(top_at(tab, u64::MAX, &mut spx) > 0);
            // Onglet inconnu → 0 (None), handle nul → -1.
            assert_eq!(top_at(999, epoch, &mut spx), 0);
            assert_eq!(
                explorer_frame_top(
                    std::ptr::null_mut(),
                    tab,
                    epoch,
                    spx.as_mut_ptr(),
                    spx.len(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                ),
                -1
            );

            explorer_close_tab(h, tab);
            explorer_destroy(h);
        }
    }

    /// `tick` sans transition → 0 (rien à invalider).
    #[test]
    fn tick_inerte() {
        let h = test_handle();
        // SAFETY : handle frais.
        unsafe {
            let tab = explorer_open_tab(h);
            assert_eq!(explorer_tick(h, tab), 0);
            explorer_destroy(h);
        }
    }

    /// Étape 61 — `tab_title` : le `<title>` atteint l'ABI.
    #[test]
    fn tab_title_export() {
        let h = test_handle();
        // SAFETY : handle frais.
        unsafe {
            let tab = explorer_open_tab(h);
            let url = b"http://test/t";
            assert_eq!(explorer_navigate(h, tab, url.as_ptr(), url.len()), 0);
            let mut buf = [0u8; 64];
            let n = explorer_tab_title(h, tab, buf.as_mut_ptr(), 64);
            let n = usize::try_from(n).unwrap_or(0);
            assert_eq!(&buf[..n], b"ma page".as_slice());
            // Onglet inconnu → 0, handle nul → -1.
            assert_eq!(explorer_tab_title(h, 999, buf.as_mut_ptr(), 64), 0);
            assert_eq!(
                explorer_tab_title(std::ptr::null_mut(), tab, buf.as_mut_ptr(), 64),
                -1
            );
            explorer_destroy(h);
        }
    }

    /// Étape 61 — `tab_scroll` : offset ≥ 0, erreur -1.
    #[test]
    fn tab_scroll_export() {
        let h = test_handle();
        // SAFETY : handle frais.
        unsafe {
            let tab = explorer_open_tab(h);
            assert_eq!(explorer_tab_scroll(h, tab), 0.0);
            assert_eq!(explorer_tab_scroll(h, 999), -1.0);
            assert_eq!(explorer_tab_scroll(std::ptr::null_mut(), tab), -1.0);
            explorer_destroy(h);
        }
    }

    /// Étape 61 — `create_with_profile` : un chemin explicite
    /// produit un profil sur disque ; un pointeur invalide
    /// échoue sans repli silencieux.
    #[test]
    fn create_with_profile_persists() {
        let dir = std::env::temp_dir().join(format!("explorer-ffi-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY : chemin valide.
        unsafe {
            let d = dir.to_string_lossy();
            let h = explorer_create_with_profile(d.as_ptr(), d.len());
            assert!(!h.is_null());
            explorer_destroy(h);
        }
        assert!(dir.is_dir(), "le profil explicite n'a pas été créé (61)");
        // Pointeur non UTF-8 → handle nul + erreur posée.
        let bad = [0xFFu8, 0xFE];
        // SAFETY : le buffer est lisible ; rejet attendu.
        assert!(unsafe { explorer_create_with_profile(bad.as_ptr(), 2) }.is_null());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Étape 204b — `explorer_media_audio` : chunk PCM de la
    /// surface claimée → ring moteur. `-1` argument (null /
    /// vide / > 16 Ko), `-409` non claimée, `-410` morte.
    #[test]
    fn media_audio_ffi_contract() {
        let h = test_handle();
        // SAFETY : handle frais, appels sérialisés.
        unsafe {
            let tab = explorer_open_tab(h);
            let url = b"http://test/v";
            assert_eq!(explorer_navigate(h, tab, url.as_ptr(), url.len()), 0);
            let mut evbuf = vec![0u8; 8192];
            let n = explorer_media_events(h, evbuf.as_mut_ptr(), evbuf.len());
            assert!(n > 0);
            let nd = String::from_utf8_lossy(&evbuf[..usize::try_from(n).unwrap_or(0)]);
            let offered: serde_json::Value = serde_json::from_str(
                nd.lines()
                    .find(|l| l.contains("media_offered"))
                    .expect("media_offered drainé"),
            )
            .unwrap();
            let mid = offered["data"]["media"].as_u64().unwrap();

            // Arguments invalides → -1 (null, vide, oversize).
            let pcm = [0x5Au8; 9600];
            assert_eq!(
                explorer_media_audio(h, tab, mid, 48000, 2, 4, std::ptr::null(), 9),
                -1
            );
            assert_eq!(
                explorer_media_audio(h, tab, mid, 48000, 2, 4, pcm.as_ptr(), 0),
                -1
            );
            let big = vec![0u8; 17 * 1024].into_boxed_slice();
            assert_eq!(
                explorer_media_audio(h, tab, mid, 48000, 2, 4, big.as_ptr(), big.len()),
                -1
            );
            // Non claimée → -409 ; morte → -410.
            assert_eq!(
                explorer_media_audio(h, tab, mid, 48000, 2, 4, pcm.as_ptr(), pcm.len()),
                -409
            );
            assert_eq!(
                explorer_media_audio(h, tab, 999, 48000, 2, 4, pcm.as_ptr(), pcm.len()),
                -410
            );
            // Claimée → 0.
            let claim = br#"{"codecs":["video/mp4"]}"#;
            assert_eq!(
                explorer_media_claim(h, tab, mid, claim.as_ptr(), claim.len()),
                0
            );
            assert_eq!(
                explorer_media_audio(h, tab, mid, 48000, 2, 4, pcm.as_ptr(), pcm.len()),
                0
            );
        }
    }

    /// Étape 71e — variante FFI `MediaSurface` : drain
    /// NDJSON des events, `claim`/`release`/`state`, codes
    /// `-409`/`-410`, `media_gone` à la fermeture.
    #[test]
    fn media_surface_ffi_cycle() {
        let h = test_handle();
        // SAFETY : handle frais, appels sérialisés.
        unsafe {
            let tab = explorer_open_tab(h);
            let url = b"http://test/v";
            assert_eq!(explorer_navigate(h, tab, url.as_ptr(), url.len()), 0);

            // `media_offered` drainé (les events hors média
            // — navigated, invalidated — sont filtrés).
            let mut evbuf = vec![0u8; 8192];
            let n = explorer_media_events(h, evbuf.as_mut_ptr(), evbuf.len());
            assert!(n > 0);
            let nd = String::from_utf8_lossy(&evbuf[..usize::try_from(n).unwrap_or(0)]);
            let offered = nd
                .lines()
                .find(|l| l.contains("media_offered"))
                .expect("media_offered drainé");
            assert!(nd.lines().all(|l| {
                let v: serde_json::Value = serde_json::from_str(l).unwrap();
                v.get("event").is_some() && v.get("data").is_some()
            }));
            let offered: serde_json::Value = serde_json::from_str(offered).unwrap();
            let mid = offered["data"]["media"].as_u64().unwrap();
            assert_eq!(offered["data"]["kind"].as_str(), Some("video"));
            assert_eq!(offered["data"]["url"].as_str(), Some("http://test/v.mp4"));
            assert_eq!(offered["data"]["controls"].as_bool(), Some(true));

            // `media_list` : une surface vivante, même DTO.
            let mut lbuf = vec![0u8; 8192];
            let n = explorer_media_list(h, tab, lbuf.as_mut_ptr(), lbuf.len());
            assert!(n > 0);
            let list: serde_json::Value =
                serde_json::from_slice(&lbuf[..usize::try_from(n).unwrap_or(0)]).unwrap();
            assert_eq!(list["media"][0]["id"].as_u64(), Some(mid));
            assert_eq!(list["media"][0]["claimed"].as_bool(), Some(false));

            // Claim → 0 ; second claim → -409.
            let claim = br#"{"codecs":["video/mp4"]}"#;
            assert_eq!(
                explorer_media_claim(h, tab, mid, claim.as_ptr(), claim.len()),
                0
            );
            assert_eq!(
                explorer_media_claim(h, tab, mid, claim.as_ptr(), claim.len()),
                -409
            );
            // Id inconnu → -410.
            assert_eq!(
                explorer_media_claim(h, tab, 9999, claim.as_ptr(), claim.len()),
                -410
            );

            // `state` → 0, refleté dans la fiche.
            let rep = br#"{"position_ms":0,"duration_ms":1000,"buffered_ms":0,"state":"playing"}"#;
            assert_eq!(
                explorer_media_state(h, tab, mid, rep.as_ptr(), rep.len()),
                0
            );
            // `state` invalide → -400.
            let bad = br#"{"state":"weird"}"#;
            assert_eq!(
                explorer_media_state(h, tab, mid, bad.as_ptr(), bad.len()),
                -400
            );
            let n = explorer_media_list(h, tab, lbuf.as_mut_ptr(), lbuf.len());
            let list: serde_json::Value =
                serde_json::from_slice(&lbuf[..usize::try_from(n).unwrap_or(0)]).unwrap();
            assert_eq!(list["media"][0]["state"]["state"].as_str(), Some("playing"));

            // Release → 0 ; `state` sans claim → -409.
            assert_eq!(explorer_media_release(h, tab, mid), 0);
            assert_eq!(
                explorer_media_state(h, tab, mid, rep.as_ptr(), rep.len()),
                -409
            );

            // Fermeture → `media_gone` drainé.
            assert_eq!(explorer_close_tab(h, tab), 0);
            let n = explorer_media_events(h, evbuf.as_mut_ptr(), evbuf.len());
            assert!(n > 0);
            let nd = String::from_utf8_lossy(&evbuf[..usize::try_from(n).unwrap_or(0)]);
            assert!(nd.contains("media_gone"));
            assert!(nd.contains(&format!("\"media\":{mid}")));
            explorer_destroy(h);
        }
    }

    #[test]
    fn abi_version_declared() {
        // Étape 187 : la dll s'auto-identifie — le manifest
        // de déploiement relit ce niveau dans le sidecar ;
        // `0` côté hôte = dll pré-versionnement (absent).
        // Toute évolution d'ABI incrémente le littéral.
        assert_eq!(explorer_abi_version(), 1);
    }
}

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
use std::path::PathBuf;
use std::sync::Arc;

use explorer_core::{CoreEvent, InputEvent, InputOutcome as O, Session};
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
    rt: tokio::runtime::Runtime,
    session: Session,
    /// Dernier `CoreEvent` lu — non utilisé pour l'instant
    /// (polling de frames côté client), conservé pour la
    /// souscription d'événements d'un prochain incrément.
    _events: tokio::sync::broadcast::Receiver<CoreEvent>,
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
/// h1/h2 → décodage → redirections bornées.
fn http_stack() -> Arc<dyn HttpTransport> {
    let tls = Arc::new(TlsConnector::web_roots());
    let h1: Arc<dyn HttpTransport> = Arc::new(AutoClient::new(tls, HttpConfig::default()));
    let decoded: Arc<dyn HttpTransport> = Arc::new(DecodingTransport::wrap(h1, MAX_BODY));
    Arc::new(RedirectTransport::wrap(decoded, MAX_REDIRECTS))
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

/// Crée une session moteur in-process (runtime tokio privé,
/// pile réseau réelle, fabrique JS branchée). `NULL` si le
/// runtime n'a pas pu être construit.
#[unsafe(no_mangle)]
pub extern "C" fn explorer_create() -> *mut ExplorerHandle {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            set_err(e);
            return std::ptr::null_mut();
        }
    };
    let fonts = DirFontSource::new(&default_fonts_dir());
    let mut session = Session::new(http_stack(), Arc::new(fonts));
    session.set_js_factory(explorer_js::DilloEngine::factory());
    let events = session.subscribe();
    Box::into_raw(Box::new(ExplorerHandle {
        rt,
        session,
        _events: events,
    }))
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
        Some(h) => h.session.open_tab(),
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
        Some(h) => match h.session.close_tab(tab) {
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
    match h.rt.block_on(h.session.navigate(tab, url)) {
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
        Some(h) => match h.rt.block_on(h.session.reload(tab)) {
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
        Some(h) => match h.rt.block_on(h.session.go_back(tab)) {
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
        Some(h) => match h.rt.block_on(h.session.go_forward(tab)) {
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
            h.session.set_viewport(w, hgt, scale);
            0
        }
        None => -1,
    }
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

/// Événement d'entrée. `kind` ∈ `0..=3` (`FfiInputKind`).
/// Retour : `0` ignoré, `1` scrollé, `2` navigué, `3` hover,
/// `4` champ focalisé, `5` champ édité ; négatif = erreur.
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
) -> i32 {
    let Some(h) = (unsafe { handle(h) }) else {
        return -1;
    };
    let ev = match kind {
        0 => InputEvent::Click { x, y },
        1 => InputEvent::MouseMove { x, y },
        2 => InputEvent::Scroll { dy: y },
        3 => match unsafe { read_str(key, key_len, MAX_KEY) } {
            Some(k) => InputEvent::Key { key: k.to_owned() },
            None => return -1,
        },
        _ => {
            set_err("kind inconnu");
            return -1;
        }
    };
    match h.rt.block_on(h.session.handle_input(tab, ev)) {
        Ok(O::Ignored) => 0,
        Ok(O::Scrolled { .. }) => 1,
        Ok(O::Navigated) => 2,
        Ok(O::Hover { .. }) => 3,
        Ok(O::Focused) => 4,
        Ok(O::Edited) => 5,
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
        Some(h) => match h.session.tick_animations(tab) {
            Ok(true) => 1,
            Ok(false) => 0,
            Err(e) => {
                set_err(e);
                -2
            }
        },
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
            if let Some((fw, fh, _)) = h.session.frame_pixels(tab) {
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
    match h.session.frame_pixels(tab) {
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
    match h.session.tab_url(tab) {
        Some(url) => unsafe { write_str(&url, buf, cap) },
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
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
                }),
                None => Ok(Response {
                    status: 404,
                    reason: "fake".to_string(),
                    headers: vec![],
                    body: b"not found".to_vec(),
                    reused_connection: false,
                }),
            }
        }
    }

    /// Handle de test : session sur transport simulé (aucune
    /// socket), même shape que `explorer_create`.
    fn test_handle() -> *mut ExplorerHandle {
        let t = Arc::new(FakeTransport::new());
        t.route("/a", 200, "<html><body><h1>bonjour</h1></body></html>");
        let tr: Arc<dyn HttpTransport> = t;
        let mut session = Session::new(tr, Arc::new(FixtureFont));
        session.set_viewport(800.0, 600.0, 1.0);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let events = session.subscribe();
        Box::into_raw(Box::new(ExplorerHandle {
            rt,
            session,
            _events: events,
        }))
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
}

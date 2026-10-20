// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests internes de la session — helpers et régressions.
use super::*;

use explorer_css::value::Color;
use explorer_html::TreeBuilder;
use explorer_net::error::NetResult;
use explorer_net::http::Response;
use explorer_render::{build_display_list_full, rasterize, DisplayItem};

/// Fonte fixture partagée (TTF synthétique, métriques
/// réelles) — les tests internes peignent pour de vrai.
struct TestFont;

impl FontSource for TestFont {
    fn font_data(&self, _req: &explorer_layout::font::FontRequest) -> Option<Vec<u8>> {
        Some(explorer_test_support::fixture_ttf())
    }
}

/// Transport stub — jamais appelé par ces tests
/// (restauration bfcache = zéro réseau).
struct FailTransport;

#[async_trait::async_trait]
impl HttpTransport for FailTransport {
    async fn fetch(&self, _req: &Request) -> NetResult<Response> {
        Err(explorer_net::error::NetError::InvalidResponse(
            "stub : pas de réseau".to_string(),
        ))
    }
}

/// DOM minimal parsé par le vrai tree-builder.
fn test_dom(body: &str) -> Dom {
    let mut tb = TreeBuilder::new();
    tb.push(&format!("<html><body>{body}</body></html>"));
    tb.finish();
    tb.into_dom()
}

/// Étape 56b — un snapshot gelé pendant la fenêtre
/// `nav_apply` → `sub_commit` (DOM commis, produit
/// jamais peint — `/frame` 204 persistant, hit-test
/// mort, scroll bloqué, terrain mission 11) :
/// (a) `freeze` refuse désormais une page sans produit
/// peint — plus de snapshots empoisonnés ;
/// (b) la restauration d'un snapshot sans produit
/// **repeint** depuis son DOM plutôt que de servir un
/// cadre vide à vie.
#[test]
fn paintless_snapshot_repairs_on_restore() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    let tab = s.open_tab();
    let url_a = Url::parse("http://h.test/a").unwrap();
    let url_b = Url::parse("http://h.test/b").unwrap();

    // (a) Une page commise mais jamais peinte ne gèle pas.
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.dom = Some(test_dom("<p>en vol</p>"));
    }
    s.tabs.get_mut(&tab).unwrap().freeze(4);
    assert!(
        s.tabs.get(&tab).unwrap().bfcache.is_empty(),
        "une page sans produit peint ne doit pas être gelée (56b)"
    );

    // Live = A « peinte » (png factice — l'accesseur ne
    // valide pas le contenu) ; snapshot B gelé sans
    // produit : le poison du bug terrain.
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.url = Some(url_a.clone());
        e.dom = Some(test_dom("<p>page A</p>"));
        e.png = Some(b"png-a".to_vec());
        e.hist = vec![Request::get(url_a), Request::get(url_b.clone())];
        e.hist_idx = 0;
        e.bfcache.push_back(PageSnapshot {
            hist_idx: 1,
            url: Some(url_b),
            title: None,
            scroll_y: 0.0,
            doc_h: 0.0,
            dom: Some(test_dom("<p>page B gelée tôt</p>")),
            tree: None,
            list: None,
            frame: None,
            png: None,
            focused: None,
            hovered: None,
            popup: None,
            js: None,
            images: PaintImages::default(),
            linked_css: String::new(),
            prev_styles: HashMap::new(),
            dirty: Vec::new(),
            emitted: 0,
            last_req: None,
        });
    }

    // (b) Restauration : le snapshot sans produit est
    // repeint depuis son DOM — fin du 204 persistant.
    assert!(s.try_bfcache(tab, 1));
    assert_eq!(s.tab_url(tab).as_deref(), Some("http://h.test/b"));
    assert!(
        s.frame_png(tab).is_some(),
        "un snapshot sans produit doit être repeint à la restauration (56b)"
    );
    assert!(
        s.tabs.get(&tab).unwrap().tree.is_some(),
        "l'arbre repeint restaure aussi le hit-test et le scroll"
    );
}

/// 55g — le cull ne jette que les items-feuilles
/// entièrement hors de la zone raster et conserve les
/// items d'état (`PushClip`/`PopClip`) pour l'équilibre
/// des piles.
#[test]
fn cull_keeps_only_in_frame_items() {
    let rect = |y: f32| DisplayItem::FillRect {
        rect: Rect {
            x: 0.0,
            y,
            width: 10.0,
            height: 10.0,
        },
        color: Color::BLACK,
    };
    let mut list = DisplayList {
        items: vec![
            rect(50.0),  // dedans
            rect(500.0), // hors (h>100)
            DisplayItem::PushClip(Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            }),
            rect(600.0), // hors même sous clip — jeté (bornes fiables)
            DisplayItem::PopClip,
        ],
        width: 100.0,
        height: 100.0,
    };
    cull_display_list(&mut list);
    let fills = list
        .items
        .iter()
        .filter(|i| matches!(i, DisplayItem::FillRect { .. }))
        .count();
    assert_eq!(fills, 1, "seul l'item en cadre survit");
    assert_eq!(list.items.len(), 3, "items d'état conservés");
}

/// Profilage 55g — temps par phase de resynchro sur un
/// document ~3 000 nœuds (repro mission 12 : >5 min au
/// clic). `#[ignore]` : à la demande —
/// `cargo test -p explorer-core --lib profile_ -- --ignored --nocapture`.
#[test]
#[ignore = "profil 55g, à la demande"]
fn profile_resync_phases() {
    use std::fmt::Write;
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    let tab = s.open_tab();
    let mut html = String::from("<html><body>");
    for i in 0..3000 {
        let _ = write!(html, "<div class=\"r{i}\"><p>ligne {i} de texte</p></div>");
    }
    html.push_str("</body></html>");
    let t0 = Instant::now();
    let mut tb = TreeBuilder::new();
    tb.push(&html);
    tb.finish();
    let dom = tb.into_dom();
    eprintln!("parse          : {:?}", t0.elapsed());
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.dom = Some(dom);
        e.url = Some(Url::parse("http://h.test/x").unwrap());
    }
    // `Dom` n'est pas Clone — même pattern take/restore
    // que `relayout`.
    let dom_c = s.tabs.get_mut(&tab).unwrap().dom.take().unwrap();
    let t = Instant::now();
    let _styles = s.compute_styles(tab, &dom_c);
    eprintln!("compute_styles : {:?}", t.elapsed());
    s.tabs.get_mut(&tab).unwrap().dom = Some(dom_c);
    let t = Instant::now();
    let _ = s.relayout(tab);
    eprintln!("relayout total : {:?}", t.elapsed());
    // Sous-phases du repaint : display list → diff →
    // raster → PNG.
    let e = s.tabs.get(&tab).unwrap();
    let bx = e.tree.as_ref().unwrap();
    let (vw, vh) = s.viewport;
    let t = Instant::now();
    let mut list = build_display_list_full(bx, vw, vh, e.scroll_y, &e.images);
    eprintln!(
        "  display list : {:?} ({} items)",
        t.elapsed(),
        list.items.len()
    );
    let prev = s.tabs.get(&tab).unwrap().list.as_ref().unwrap().clone();
    let t = Instant::now();
    if list.items.len() > DIRTY_DIFF_MAX_ITEMS || prev.items.len() > DIRTY_DIFF_MAX_ITEMS {
        eprintln!("  changed_rects: sauté (seuil {DIRTY_DIFF_MAX_ITEMS} items → pleine frame)");
    } else {
        let rects = list.changed_rects(&prev);
        eprintln!("  changed_rects: {:?} ({} rects)", t.elapsed(), rects.len());
    }
    // Même cap que `repaint` puis cull — fenêtre raster.
    if list.height > MAX_FRAME_DIM {
        list.height = MAX_FRAME_DIM;
    }
    cull_display_list(&mut list);
    eprintln!("  après cull   : {} items", list.items.len());
    let t = Instant::now();
    let frame = rasterize(&list, &s.webfonts as &dyn FontSource, s.scale).unwrap();
    eprintln!("  rasterize    : {:?}", t.elapsed());
    let t = Instant::now();
    let _png = frame.encode_png();
    eprintln!("  encode_png   : {:?}", t.elapsed());
}

/// UTF-8 par défaut — la régression 53b : « ’ » (E2 80 99)
/// reste « ’ », pas « â€™ » (repli 1252 du sniffer HTML).
#[test]
fn subresource_utf8_default() {
    assert_eq!(
        decode_subresource("L\u{2019}usage".as_bytes(), None),
        "L\u{2019}usage"
    );
}

/// `charset` déclaré → label appliqué : 0xE9 = « é ».
#[test]
fn subresource_charset_label() {
    assert_eq!(
        decode_subresource(b"caf\xE9", Some("iso-8859-1")),
        "caf\u{E9}"
    );
}

/// Label inconnu → échelon suivant (UTF-8) — 0xE9 nu
/// devient U+FFFD, pas « é » (le label ne gagne pas par
/// défaut).
#[test]
fn subresource_unknown_label_falls_through() {
    assert_eq!(
        decode_subresource(b"caf\xE9", Some("x-bogus")),
        "caf\u{FFFD}"
    );
}

/// BOM UTF-8 + charset Latin-1 : le label déclaré décide
/// de l'encodage, le BOM est sauté du texte.
#[test]
fn subresource_bom_stripped() {
    assert_eq!(
        decode_subresource(b"\xEF\xBB\xBFcaf\xE9", Some("iso-8859-1")),
        "caf\u{E9}"
    );
}

/// `Content-Type: …; charset=` l'emporte sur tout.
#[test]
fn stylesheet_transport_charset() {
    let h = [(
        "content-type".to_string(),
        "text/css; charset=ISO-8859-1".to_string(),
    )];
    assert!(decode_stylesheet(b".x{content:'caf\xE9'}", &h).contains("caf\u{E9}"));
}

/// `@charset` en tête, sans transport → label appliqué.
#[test]
fn stylesheet_at_charset() {
    assert!(
        decode_stylesheet(b"@charset \"iso-8859-1\";.x{content:'caf\xE9'}", &[])
            .contains("caf\u{E9}")
    );
}

/// Rien déclaré → UTF-8 (jamais 1252).
#[test]
fn stylesheet_utf8_default() {
    assert_eq!(
        decode_stylesheet(".x{content:'café'}".as_bytes(), &[]),
        ".x{content:'café'}"
    );
}

/// `@charset` pas en tête → ignoré (spec CSS : la règle
/// doit être absolument première).
#[test]
fn stylesheet_at_charset_late_ignored() {
    let body = b".a{color:red}@charset 'iso-8859-1';.x{content:'caf\xE9'}".as_slice();
    assert_eq!(css_charset(body), None);
}

/// Étape 56b (rouvert, terrain mission 13) — un DOM
/// commis sans produit peint (frame jetée après commit)
/// se répare paresseusement au premier `/frame` au lieu
/// de servir 204 à vie : `ensure_painted` repeint depuis
/// le DOM avant d'encoder.
#[test]
fn committed_dom_repaints_on_frame_pull() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    let tab = s.open_tab();
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.dom = Some(test_dom("<p>commise sans produit</p>"));
        e.url = Some(Url::parse("http://h.test/x").unwrap());
        // `tree`/`list`/`frame`/`png` tous None : l'état
        // empoisonné du 204 persistant.
    }
    assert!(s.frame_pixels(tab).is_none());
    let png = s.frame_png(tab);
    assert!(
        png.as_ref().is_some_and(|b| b.starts_with(b"\x89PNG")),
        "le pull /frame répare la peinture depuis le DOM commis (56b)"
    );
    assert!(s.frame_pixels(tab).is_some());
}

/// Profil mission 13 (55g-suite) — clic-mutation sur
/// ~3 000 nœuds : temps total `dispatch_path_event`
/// (eval + sérialisation + re-parse + incrémental).
/// Mesuré : ~21 s debug, quasi entièrement dans
/// `try_sync_incremental` (remap ~3 s, `relayout_subtree`
/// ~14 s — un append en tête de flux re-pose presque
/// tout le body — repaint ~4 s).
#[test]
#[ignore = "profil mission 13, à la demande"]
fn profile_click_sync_heavy() {
    use std::fmt::Write;
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    s.set_js_factory(explorer_js::DilloEngine::factory());
    let tab = s.open_tab();
    let mut html = String::from("<!doctype html><html><body><div id=\"zone\">ZONE</div>");
    for i in 0..3000 {
        let _ = write!(html, "<p>ligne {i} de texte</p>");
    }
    html.push_str(
        "<script>\
             document.getElementById('zone').addEventListener('click',function(){\
             var d=document.createElement('div');d.textContent='AJOUT';\
             this.appendChild(d);})</script></body></html>",
    );
    let eng = s.spawn_engine(tab, &html, true);
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        let mut tb = TreeBuilder::new();
        tb.push(&html);
        tb.finish();
        e.dom = Some(tb.into_dom());
        e.url = Some(Url::parse("http://h.test/heavy").unwrap());
        e.js = eng;
    }
    // Le <script> n'est évalué que via le pipeline de
    // commit — ici on l'exécute à la main pour armer le
    // listener.
    s.tabs.get_mut(&tab).and_then(|e| e.js.as_mut()).map(|eng| {
        eng.eval(
            "document.getElementById('zone').addEventListener('click',function(){\
                     var d=document.createElement('div');d.textContent='AJOUT';\
                     this.appendChild(d);})",
        )
    });
    let t = Instant::now();
    s.relayout(tab).unwrap();
    eprintln!("peinture initiale : {:?}", t.elapsed());
    // Chemin élément vers #zone : document→html→body→div[0].
    let path = vec![
        ("html".to_string(), 0u32),
        ("body".to_string(), 0),
        ("div".to_string(), 0),
    ];
    let t = Instant::now();
    s.dispatch_path_event(tab, "click", &path);
    eprintln!("DISPATCH TOTAL   : {:?}", t.elapsed());
    eprintln!(
        "frame: {}, incremental_syncs: {}",
        s.tabs.get(&tab).unwrap().frame.is_some(),
        s.incremental_syncs()
    );
}

/// 55g-suite — la coalescence borne les resyncs pilotés
/// par la pompe : après un apply, `sync_floor` saute la
/// sérialisation tant que la fenêtre n'est pas échue —
/// le miroir accumule (le timer tourne à chaque pompe)
/// et le DOM commis n'avance qu'à la cadence autorisée.
/// Un `setTimeout` auto-réarmé imite le motif terrain
/// « site qui mute en continu ».
#[test]
fn pump_resync_coalesced_under_floor() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    s.set_js_factory(explorer_js::DilloEngine::factory());
    let tab = s.open_tab();
    let html = "<!doctype html><html><body><div id=\"x\">init</div></body></html>";
    let eng = s.spawn_engine(tab, html, true);
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        let mut tb = TreeBuilder::new();
        tb.push(html);
        tb.finish();
        e.dom = Some(tb.into_dom());
        e.url = Some(Url::parse("http://h.test/pump").unwrap());
        e.js = eng;
    }
    s.tabs.get_mut(&tab).and_then(|e| e.js.as_mut()).map(|eng| {
        eng.eval(
            "window.n=0;\
                 function tick(){window.n++;\
                   document.getElementById('x').textContent='n'+window.n;\
                   setTimeout(tick,0);}\
                 setTimeout(tick,0);",
        )
    });
    s.relayout(tab).unwrap();
    let dom_text = |s: &Session| {
        s.tabs
            .get(&tab)
            .and_then(|e| e.dom.as_ref())
            .and_then(|d| find_by_id(d, "x"))
            .and_then(|n| {
                s.tabs
                    .get(&tab)
                    .and_then(|e| e.dom.as_ref())
                    .map(|d| d.text_content(n).unwrap_or_default())
            })
            .unwrap_or_default()
    };
    // 1re pompe : le tick mute → apply → floor posé.
    s.pump_timers(tab);
    assert_eq!(dom_text(&s), "n1");
    assert!(s.tabs.get(&tab).unwrap().sync_floor.is_some());
    // Pompe immédiate : le timer a re-muté le miroir mais
    // la synchro est sautée — le DOM commis reste « n1 ».
    s.pump_timers(tab);
    let mirror_n = s
        .tabs
        .get_mut(&tab)
        .and_then(|e| e.js.as_mut())
        .and_then(|eng| eng.eval("window.n").ok())
        .unwrap_or_default();
    assert_eq!(mirror_n.trim(), "2");
    assert_eq!(dom_text(&s), "n1");
    // Fenêtre échue : la pompe suivante absorbe toutes
    // les mutations accumulées en une passe.
    s.tabs.get_mut(&tab).unwrap().sync_floor = None;
    s.pump_timers(tab);
    assert_eq!(dom_text(&s), "n3");
}

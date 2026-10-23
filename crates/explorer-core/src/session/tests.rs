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

/// R-69 en session : `SharedFonts` délègue `font_face` +
/// `fallback_for` sous verrou — sans elles, les défauts du
/// trait re-servaient `font_data`@0 et `None` : le repli
/// par glyphe (mesuré PIPE) restait dormant en session.
#[test]
fn shared_fonts_delegates_face_and_fallback() {
    struct NoFont;
    impl FontSource for NoFont {
        fn font_data(&self, _req: &explorer_layout::font::FontRequest) -> Option<Vec<u8>> {
            None
        }
    }
    let mut w = explorer_render::WebFontSource::new(Arc::new(NoFont), 4);
    assert!(w.register("Fbk", 400, false, explorer_test_support::fixture_ttf()));
    let fonts = crate::session::apply::SharedFonts(Arc::new(std::sync::Mutex::new(w)));
    let req = explorer_layout::font::FontRequest {
        families: vec!["Fbk".to_string()],
        weight: 400,
        italic: false,
    };
    // `font_face` délégué (les faces web = index 0 — la
    // preuve forte porte sur `fallback_for` ci-dessous).
    assert!(fonts.font_face(&req).is_some());
    // Repli par glyphe : la face web « Fbk » couvre 'A' —
    // avant la délégation le défaut du trait rendait `None`.
    let (bytes, idx) = fonts
        .fallback_for('A', &req)
        .expect("repli actif en session");
    assert_eq!(idx, 0);
    assert_eq!(*bytes, explorer_test_support::fixture_ttf());
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
        e.hist = vec![
            HistEntry {
                req: Request::get(url_a),
                failed: false,
            },
            HistEntry {
                req: Request::get(url_b.clone()),
                failed: false,
            },
        ];
        e.hist_idx = 0;
        e.bfcache.push_back(PageSnapshot {
            hist_idx: 1,
            url: Some(url_b),
            title: None,
            scroll_y: 0.0,
            doc_h: 0.0,
            dom: Some(test_dom("<p>page B gelée tôt</p>")),
            tree: None,
            tree_epoch: 0,
            list: None,
            frame: None,
            png: None,
            focused: None,
            focused_key: None,
            hovered: None,
            popup: None,
            find: None,
            js: None,
            images: PaintImages::default(),
            linked_sheets: Vec::new(),
            prev_styles: HashMap::new(),
            dirty: Vec::new(),
            emitted: 0,
            last_req: None,
            frames: Vec::new(),
            frame_nodes: Vec::new(),
            referrer_policy: None,
            base_url: None,
            base_target: None,
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

/// Étape 84 — `back` pendant la fenêtre `nav_apply` →
/// `sub_commit` (URL commise, entrée pas encore poussée —
/// toute la vague de sous-ressources ; mesuré terrain :
/// 502 immédiat sur clic→iana, jusqu'à ~100 s sur github).
/// La page en vol compte pour une place au-delà de
/// `hist_idx` : `back` retombe sur la page quittée (déjà
/// gelée) au lieu d'échouer `EmptyDocument`, et la page
/// en vol n'est PAS gelée sous l'index de l'ancienne
/// (empoisonnement du slot bfcache).
#[tokio::test]
async fn back_during_inflight_nav_restores_previous() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    let tab = s.open_tab();
    let url_a = Url::parse("http://h.test/a").unwrap();
    let url_b = Url::parse("http://h.test/b").unwrap();

    // État « en vol » : A commise et gelée, B affichée
    // (URL + DOM posés par nav_apply) mais son entrée
    // d'historique n'existe pas encore — `hist_pending`.
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.url = Some(url_b.clone());
        e.dom = Some(test_dom("<p>page B en vol</p>"));
        e.png = Some(b"png-b".to_vec());
        e.frame = Some(Frame {
            width: 8,
            height: 8,
            pixels: vec![0; 8 * 8 * 4],
            scale: 1.0,
        });
        e.hist = vec![HistEntry {
            req: Request::get(url_a.clone()),
            failed: false,
        }];
        e.hist_idx = 0;
        e.hist_pending = true;
        e.bfcache.push_back(PageSnapshot {
            hist_idx: 0,
            url: Some(url_a.clone()),
            title: None,
            scroll_y: 0.0,
            doc_h: 0.0,
            dom: Some(test_dom("<p>page A gelée</p>")),
            tree: None,
            tree_epoch: 0,
            list: None,
            frame: Some(Frame {
                width: 8,
                height: 8,
                pixels: vec![0; 8 * 8 * 4],
                scale: 1.0,
            }),
            png: None,
            focused: None,
            focused_key: None,
            hovered: None,
            popup: None,
            find: None,
            js: None,
            images: PaintImages::default(),
            linked_sheets: Vec::new(),
            prev_styles: HashMap::new(),
            dirty: Vec::new(),
            emitted: 0,
            last_req: None,
            frames: Vec::new(),
            frame_nodes: Vec::new(),
            referrer_policy: None,
            base_url: None,
            base_target: None,
        });
    }

    // `can_back` vit pendant le vol — la position logique
    // est hist_idx+1.
    assert!(
        s.tab_info(tab).unwrap().can_back,
        "back possible pendant le vol"
    );
    assert!(
        !s.tab_info(tab).unwrap().can_forward,
        "le futur est mort — tronqué au commit de la nav en vol"
    );

    // `back` restaure la page quittée — pas EmptyDocument.
    s.history_move(tab, -1).await.unwrap();
    assert_eq!(s.tab_url(tab).as_deref(), Some("http://h.test/a"));
    assert!(!s.tabs.get(&tab).unwrap().hist_pending);

    // Le slot bfcache de A a été consommé par la
    // restauration ; la page en vol n'a RIEN gelé sous
    // l'index 0 (sinon un futur back servirait B déguisé
    // en A).
    let e = s.tabs.get(&tab).unwrap();
    assert!(
        e.bfcache.iter().all(|snap| snap.hist_idx != 0),
        "la page en vol ne doit pas empoisonner le slot de A"
    );
}

/// Même fenêtre côté API partagée : `hist_target` compte
/// la page en vol — `delta=-1` vise `hist_idx` au lieu de
/// retourner `EmptyDocument` (le 502 observé).
#[test]
fn hist_target_during_inflight_nav() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    let tab = s.open_tab();
    let url_a = Url::parse("http://h.test/a").unwrap();
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.url = Some(Url::parse("http://h.test/b").unwrap());
        e.hist = vec![HistEntry {
            req: Request::get(url_a.clone()),
            failed: false,
        }];
        e.hist_idx = 0;
        e.hist_pending = true;
    }
    let (idx, req) = s.hist_target(tab, -1).unwrap();
    assert_eq!(idx, 0);
    assert_eq!(req.url, url_a);
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
        spans: explorer_render::SpanMap::default(),
        tree_epoch: 0,
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
    let fonts = crate::session::apply::SharedFonts(Arc::clone(&s.webfonts));
    let frame = rasterize(&list, &fonts, s.scale).unwrap();
    eprintln!("  rasterize    : {:?} (cache neuf)", t.elapsed());
    // R-36 : même liste, cache glyphes persistant — la 2e
    // passe rejoue faces + contours retenus (le chemin réel
    // des repaints session via `p.font_cache`).
    let mut gc = explorer_render::GlyphCache::new();
    let _ = explorer_render::rasterize_cached(&list, &fonts, s.scale, &mut gc);
    let t = Instant::now();
    let warm = explorer_render::rasterize_cached(&list, &fonts, s.scale, &mut gc).unwrap();
    eprintln!("  rasterize_cached : {:?} (chaude)", t.elapsed());
    assert_eq!(frame.pixels, warm.pixels, "équivalence cache");
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

/// Étape 104 — chaque feuille est parsée isolément : un
/// défaut de fermeture (accolade) dans une feuille ne peut
/// plus absorber la suivante. Avant, `<style>` inline et
/// `<link>` étaient concaténés en un blob — la règle
/// survivante était avalée dans le bloc non fermé (R-68 a
/// relevé `MAX_TOKENS`, l'isolation reste nécessaire).
#[test]
fn broken_sheet_does_not_swallow_next() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    let tab = s.open_tab();
    // `<style>` inline : la première feuille n'est pas
    // fermée — son reste avale la seconde en mode blob.
    let dom = test_dom(
        "<style>.x{color:red</style>\
         <style>#victim{position:fixed}</style>\
         <div id=victim></div>",
    );
    let victim = find_by_id(&dom, "victim").unwrap();
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.url = Some(Url::parse("http://h.test/x").unwrap());
        // Feuilles liées : même motif — la première est
        // tronquée, la seconde doit survivre.
        e.linked_sheets = vec![
            ".y{color:blue".to_string(),
            "#victim{visibility:hidden}".to_string(),
        ];
        e.dom = Some(dom);
    }
    // `Dom` n'est pas Clone — take/restore comme `relayout`.
    let dom_c = s.tabs.get_mut(&tab).unwrap().dom.take().unwrap();
    let styles = s.compute_styles(tab, &dom_c);
    s.tabs.get_mut(&tab).unwrap().dom = Some(dom_c);
    let st = styles.get(victim).expect("style du div");
    assert_eq!(st.get("position"), Some("fixed"), "style inline avalé");
    assert_eq!(st.get("visibility"), Some("hidden"), "feuille liée avalée");
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
    s.set_js_factory(explorer_js::JscoreEngine::factory());
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

/// Étape 100 — le `FontCache` de layout persiste entre
/// passes (R-65/R-67 — transplanté dans `PaintPayload`
/// via `layout_ctx`/`restitute_font_cache`) : le second
/// `relayout` ne re-facture **aucune** résolution
/// `font_data` — avant la couture, `BlockCtx::default()`
/// repartait froid et re-shapait tout le document.
#[test]
fn relayout_reuses_persistent_font_cache() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct CountingFont(AtomicUsize);
    impl FontSource for CountingFont {
        fn font_data(&self, _req: &explorer_layout::font::FontRequest) -> Option<Vec<u8>> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Some(explorer_test_support::fixture_ttf())
        }
    }
    let font = Arc::new(CountingFont(AtomicUsize::new(0)));
    let mut s = Session::new(Arc::new(FailTransport), font.clone());
    let tab = s.open_tab();
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.dom = Some(test_dom("<p>du texte à shaper</p>"));
        e.url = Some(Url::parse("http://h.test/fc").unwrap());
    }
    s.relayout(tab).unwrap();
    let first = font.0.load(Ordering::Relaxed);
    assert!(first > 0, "première passe : fontes résolues");
    // Seconde passe : le cache transplante — aucune
    // nouvelle résolution ne doit toucher la source.
    s.relayout(tab).unwrap();
    let delta = font.0.load(Ordering::Relaxed) - first;
    assert_eq!(delta, 0, "relayout : {delta} font_data re-facturées");
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
    s.set_js_factory(explorer_js::JscoreEngine::factory());
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

/// Étape 57bis — find-in-page : occurrences comptées sur
/// le boxtree, navigation cyclique, scroll-vers-courante,
/// surlignage injecté dans la display list.
#[test]
fn find_in_page_counts_steps_and_highlights() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    let tab = s.open_tab();
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.dom = Some(test_dom(
            "<p>alpha lorem un</p><p>deux</p><p>lorem trois lorem</p>",
        ));
        e.url = Some(Url::parse("http://h.test/find").unwrap());
    }
    s.relayout(tab).unwrap();

    // R-35 : granularité **occurrence** — les deux « lorem »
    // du 3e paragraphe comptent chacun (comportement Chrome).
    let n = s.find_in_page(tab, "LOREM"); // insensible à la casse
    assert_eq!(n, 3, "3 occurrences de lorem");
    let (q, i, total) = s.find_state(tab).unwrap();
    assert_eq!((q.as_str(), i, total), ("lorem", 0, 3));

    let (i, n) = s.find_step(tab, 1);
    assert_eq!((i, n), (1, 3));
    // Boucle : après la dernière on repart à 0.
    let _ = s.find_step(tab, 1);
    let (i, _) = s.find_step(tab, 1);
    assert_eq!(i, 0);
    let (i, _) = s.find_step(tab, -1);
    assert_eq!(i, 2, "dir -1 boucle sur la fin");

    // Le surlignage est peint : FillRect translucides en
    // queue de display list (couleur occurrence courante).
    s.repaint(tab).unwrap();
    let list = s.tabs.get(&tab).unwrap().list.clone().unwrap();
    let orange = list
        .items
        .iter()
        .filter(|it| matches!(it, DisplayItem::FillRect { color, .. } if color.r == 255 && color.g == 152 && color.a == 160))
        .count();
    assert_eq!(orange, 1, "une occurrence courante marquée");

    // Requête vide = fermeture — plus d'état ni d'overlay.
    assert_eq!(s.find_in_page(tab, ""), 0);
    assert!(s.find_state(tab).is_none());
}

// ----------------------------------------------------------------
// Étape 71b — seam `MediaSurface` (ADR-0024) : détection,
// cycle de vie des offres, claim/state, honnêteté des stubs.
// ----------------------------------------------------------------

/// Page commise avec `body` puis `refresh_media` — renvoie
/// la session, l'onglet et le receveur d'événements.
fn media_page(body: &str) -> (Session, TabId, broadcast::Receiver<CoreEvent>) {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    let tab = s.open_tab();
    let rx = s.subscribe();
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.dom = Some(test_dom(body));
        e.url = Some(Url::parse("http://h.test/m").unwrap());
    }
    s.relayout(tab).unwrap();
    let mut s = s;
    s.refresh_media(tab);
    (s, tab, rx)
}

/// Événements média accumulés depuis l'abonnement.
fn media_events(rx: &mut broadcast::Receiver<CoreEvent>) -> Vec<CoreEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if matches!(
            ev,
            CoreEvent::MediaOffered { .. }
                | CoreEvent::MediaMoved { .. }
                | CoreEvent::MediaGone { .. }
                | CoreEvent::MediaCommand { .. }
        ) {
            out.push(ev);
        }
    }
    out
}

#[test]
fn media_offered_scan_kinds_and_rects() {
    let (s, tab, mut rx) = media_page(
        "<video src=\"v.mp4\" controls autoplay></video>\
         <video><source src=\"w.webm\" type=\"video/webm\"></video>\
         <audio src=\"a.mp3\"></audio>\
         <audio src=\"b.ogg\" controls></audio>\
         <video></video>",
    );
    let evs = media_events(&mut rx);
    // 4 éléments lisibles — le <video> sans source n'émet
    // rien (honnêteté : rien à jouer).
    assert_eq!(evs.len(), 4, "4 MediaOffered attendus : {evs:?}");

    let offer = |i: usize| match &evs[i] {
        CoreEvent::MediaOffered {
            media,
            kind,
            rect,
            url,
            mime,
            controls,
            autoplay,
            ..
        } => (
            *media,
            *kind,
            *rect,
            url.clone(),
            mime.clone(),
            *controls,
            *autoplay,
        ),
        e => panic!("MediaOffered attendu, reçu {e:?}"),
    };

    let (id, kind, rect, url, mime, controls, autoplay) = offer(0);
    assert_eq!((id, kind), (1, MediaKind::Video));
    assert_eq!(url, "http://h.test/v.mp4");
    assert_eq!(mime.as_deref(), Some("video/mp4"));
    assert!(controls && autoplay);
    assert!(rect.w > 0.0 && rect.h > 0.0, "vidéo = vraie boîte");

    let (_, kind, _, url, mime, controls, _) = offer(1);
    assert_eq!(kind, MediaKind::Video);
    assert_eq!(url, "http://h.test/w.webm");
    assert_eq!(mime.as_deref(), Some("video/webm"), "type du <source>");
    assert!(!controls);

    // `audio` sans controls : display:none au style UA →
    // pas de boîte, offre à 0×0 (pipeline seul).
    let (_, kind, rect, url, mime, controls, _) = offer(2);
    assert_eq!(
        (kind, url.as_str()),
        (MediaKind::Audio, "http://h.test/a.mp3")
    );
    assert_eq!(mime.as_deref(), Some("audio/mpeg"));
    assert!(!controls);
    assert_eq!(
        rect,
        MediaRect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0
        }
    );

    let (_, kind, rect, _, _, controls, _) = offer(3);
    assert_eq!(kind, MediaKind::Audio);
    assert!(controls);
    assert!(rect.w > 0.0, "audio controls = boîte UI réelle");

    // `media_list` reflète les offres.
    let list = s.media_list(tab);
    assert_eq!(list.len(), 4);
    assert!(list.iter().all(|m| m.claim.is_none()));
}

#[test]
fn media_gone_on_close_and_navigation() {
    let (mut s, tab, mut rx) = media_page("<video src=\"v.mp4\"></video>");
    let _ = media_events(&mut rx); // purge des offres

    // Retrait DOM : re-layout sans l'élément → MediaGone.
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.dom = Some(test_dom("<p>plus rien</p>"));
    }
    s.relayout(tab).unwrap();
    s.refresh_media(tab);
    let evs = media_events(&mut rx);
    assert_eq!(evs.len(), 1);
    assert!(matches!(evs[0], CoreEvent::MediaGone { media: 1, .. }));

    // Ré-offre puis fermeture d'onglet → Gone avant
    // `TabClosed` (l'hôte relâche son pipeline).
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.dom = Some(test_dom("<audio src=\"x.mp3\"></audio>"));
    }
    s.relayout(tab).unwrap();
    s.refresh_media(tab);
    let _ = media_events(&mut rx);
    s.close_tab(tab).unwrap();
    let evs = media_events(&mut rx);
    assert!(
        matches!(evs[0], CoreEvent::MediaGone { media: 2, .. }),
        "Gone avant TabClosed : {evs:?}"
    );
}

#[test]
fn media_claim_first_wins_and_state() {
    let (mut s, tab, _rx) = media_page("<video src=\"v.mp4\"></video>");
    // Premier arrivé gagne ; second → Taken (409 REST).
    s.media_claim(tab, 1, vec!["h264".into()]).unwrap();
    assert_eq!(
        s.media_claim(tab, 1, vec!["vp9".into()]),
        Err(MediaError::Taken)
    );
    // state exige un claim actif.
    let report = MediaReport {
        position_ms: 1_500,
        duration_ms: Some(60_000),
        buffered_ms: 5_000,
        state: MediaHostState::Playing,
        error: None,
    };
    s.media_report(tab, 1, report.clone()).unwrap();
    let m = &s.media_list(tab)[0];
    assert_eq!(m.claim.as_deref(), Some(&["h264".to_string()][..]));
    assert_eq!(m.state, Some(report));

    // Release → stubs ADR-0023 ; un claim neuf peut venir.
    s.media_release(tab, 1).unwrap();
    assert_eq!(
        s.media_report(
            tab,
            1,
            MediaReport {
                position_ms: 0,
                duration_ms: None,
                buffered_ms: 0,
                state: MediaHostState::Paused,
                error: None,
            },
        ),
        Err(MediaError::NotClaimed)
    );
    s.media_claim(tab, 1, vec!["vp9".into()]).unwrap();

    // Id mort → Dead (410 REST).
    assert_eq!(s.media_claim(tab, 99, vec![]), Err(MediaError::Dead));
    assert_eq!(s.media_release(tab, 99), Err(MediaError::Dead));
}

#[test]
fn media_ids_fresh_after_freeze_restore() {
    let (mut s, tab, mut rx) = media_page("<video src=\"v.mp4\"></video>");
    let _ = media_events(&mut rx);
    // Mort de page = close_media ; la restauration
    // (bfcache) ré-offre avec un id neuf (contrat).
    s.close_media(tab);
    let evs = media_events(&mut rx);
    assert!(matches!(evs[0], CoreEvent::MediaGone { media: 1, .. }));

    s.refresh_media(tab);
    let evs = media_events(&mut rx);
    match &evs[0] {
        CoreEvent::MediaOffered { media, .. } => {
            assert_eq!(*media, 2, "ré-offre = nouvel id, pas de réutilisation");
        }
        e => panic!("MediaOffered attendu : {e:?}"),
    }
}

#[test]
fn media_moved_on_relayout_only_when_rect_changes() {
    let (mut s, tab, mut rx) = media_page("<video src=\"v.mp4\"></video>");
    let _ = media_events(&mut rx);

    // Un relayout sans changement géométrique : rien.
    s.relayout(tab).unwrap();
    s.refresh_media(tab);
    assert!(media_events(&mut rx).is_empty(), "rect inchangé → muet");

    // Un bloc inséré au-dessus décale la vidéo → MediaMoved
    // (un seul, coalescence par refresh).
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.dom = Some(test_dom(
            "<div style=\"height:200px\"></div><video src=\"v.mp4\"></video>",
        ));
    }
    s.relayout(tab).unwrap();
    s.refresh_media(tab);
    let evs = media_events(&mut rx);
    assert_eq!(evs.len(), 1);
    match &evs[0] {
        CoreEvent::MediaMoved { media, rect, .. } => {
            assert_eq!(*media, 1, "id conservé — même élément");
            assert_ne!(
                *rect,
                MediaRect {
                    x: 0.0,
                    y: 0.0,
                    w: 0.0,
                    h: 0.0
                }
            );
            assert!(rect.y > 100.0, "vidéo décalée sous le bloc : {rect:?}");
        }
        e => panic!("MediaMoved attendu : {e:?}"),
    }
}

/// Page `body` commise + moteur jscore installé +
/// `refresh_media` — harnais de délégation JS média (71c).
fn media_js_page(body: &str) -> (Session, TabId, broadcast::Receiver<CoreEvent>) {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let tab = s.open_tab();
    let rx = s.subscribe();
    let html = format!("<!doctype html><html><body>{body}</body></html>");
    let eng = s.spawn_engine(tab, &html, true).unwrap();
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.dom = Some(test_dom(body));
        e.url = Some(Url::parse("http://h.test/m").unwrap());
        e.js = Some(eng);
    }
    s.relayout(tab).unwrap();
    s.refresh_media(tab);
    (s, tab, rx)
}

/// Évalue `src` sur le moteur de `tab` puis pompe les
/// microtasks (résolutions `play()`, callbacks `then`).
fn media_js_eval(s: &mut Session, tab: TabId, src: &str) -> String {
    s.tabs
        .get_mut(&tab)
        .and_then(|e| e.js.as_mut())
        .map_or_else(
            || "no-js".to_string(),
            |eng| {
                let out = eng.eval(src).unwrap_or_else(|e| format!("ERR:{e:?}"));
                eng.run_pending();
                out
            },
        )
}

/// Étape 71c — sans claim les stubs ADR-0023 tiennent ;
/// claimé, `canPlayType` reflète les codecs et les ordres
/// page (`play`/seek/volume) partent en `MediaCommand`.
#[test]
fn media_js_commands_only_when_claimed() {
    let (mut s, tab, mut rx) = media_js_page("<video id=\"v\" src=\"v.mp4\"></video>");
    let _ = media_events(&mut rx); // purge de l'offre

    // Non claimé : `play()` rejette, `canPlayType` avoue "".
    media_js_eval(
        &mut s,
        tab,
        "document.querySelector('video').play().then(             ()=>window.__r='ok',()=>window.__r='ko');'p'",
    );
    assert_eq!(media_js_eval(&mut s, tab, "window.__r"), "ko");
    assert_eq!(
        media_js_eval(
            &mut s,
            tab,
            "document.querySelector('video').canPlayType('video/mp4')"
        ),
        ""
    );

    // Claim → `canPlayType` reflète les codecs déclarés.
    s.media_claim(tab, 1, vec!["video/mp4".into(), "avc1.42".into()])
        .unwrap();
    assert_eq!(
        media_js_eval(
            &mut s,
            tab,
            "document.querySelector('video').canPlayType('video/mp4; codecs=\"avc1.42E01E\"')"
        ),
        "probably"
    );
    assert_eq!(
        media_js_eval(
            &mut s,
            tab,
            "document.querySelector('video').canPlayType('video/webm')"
        ),
        ""
    );

    // Ordres page → hôte : play/seek/volume drainés en
    // `MediaCommand` (id de surface rappelé).
    media_js_eval(
        &mut s,
        tab,
        "var v=document.querySelector('video');         v.play().then(()=>window.__p='ok',()=>window.__p='ko');         v.currentTime=3;v.volume=0.5;'ok'",
    );
    s.after_script(tab);
    let evs = media_events(&mut rx);
    assert!(
        evs.iter().any(|e| matches!(
            e,
            CoreEvent::MediaCommand {
                media: 1,
                cmd: MediaCmd::Play,
                ..
            }
        )),
        "ordre play attendu : {evs:?}"
    );
    assert!(
        evs.iter().any(|e| matches!(
            e,
            CoreEvent::MediaCommand {
                media: 1,
                cmd: MediaCmd::Seek { ms: 3000 },
                ..
            }
        )),
        "ordre seek 3000 attendu : {evs:?}"
    );
    assert!(
        evs.iter().any(|e| matches!(
            e,
            CoreEvent::MediaCommand {
                media: 1,
                cmd: MediaCmd::Volume { v },
                ..
            } if (*v - 0.5).abs() < 0.01
        )),
        "ordre volume 0.5 attendu : {evs:?}"
    );
}

/// Étape 71c — le rapport `state` hôte résout `play()`,
/// reflète les props et tire les events DOM ; release →
/// retour aux stubs honnêtes.
#[test]
fn media_js_report_resolves_play_and_reflects_props() {
    let (mut s, tab, _rx) = media_js_page("<video id=\"v\" src=\"v.mp4\"></video>");
    s.media_claim(tab, 1, vec!["video/mp4".into()]).unwrap();
    media_js_eval(
        &mut s,
        tab,
        "var v=document.querySelector('video');window.__log=[];         v.play().then(()=>window.__p='ok',()=>window.__p='ko');         ['loadedmetadata','play','playing','seeked','timeupdate','durationchange'].forEach(         t=>v.addEventListener(t,()=>window.__log.push(t)));v.currentTime=3;'ok'",
    );
    s.media_report(
        tab,
        1,
        MediaReport {
            position_ms: 3_000,
            duration_ms: Some(60_000),
            buffered_ms: 10_000,
            state: MediaHostState::Playing,
            error: None,
        },
    )
    .unwrap();
    assert_eq!(media_js_eval(&mut s, tab, "window.__p"), "ok");
    assert_eq!(media_js_eval(&mut s, tab, "v.paused"), "false");
    assert_eq!(media_js_eval(&mut s, tab, "v.currentTime===3"), "true");
    assert_eq!(media_js_eval(&mut s, tab, "v.readyState===4"), "true");
    assert_eq!(media_js_eval(&mut s, tab, "v.duration===60"), "true");
    assert_eq!(
        media_js_eval(
            &mut s,
            tab,
            "v.buffered.length===1 && v.buffered.end(0)===10"
        ),
        "true"
    );
    let log = media_js_eval(&mut s, tab, "window.__log.join(',')");
    for want in [
        "loadedmetadata",
        "play",
        "playing",
        "seeked",
        "timeupdate",
        "durationchange",
    ] {
        assert!(log.contains(want), "event {want} attendu dans {log}");
    }

    // Release → retour aux stubs : `play()` rejette à nouveau.
    s.media_release(tab, 1).unwrap();
    media_js_eval(
        &mut s,
        tab,
        "v.play().then(()=>window.__r2='ok',()=>window.__r2='ko');'p'",
    );
    assert_eq!(media_js_eval(&mut s, tab, "window.__r2"), "ko");
}

/// Étape 92 — le focus survit au déplacement : une mutation
/// (suggest google…) insère des siblings de même tag avant
/// le champ — le chemin `(tag, index)` meurt ou résoudrait
/// sur un voisin. `focused_node` vérifie l'ancre
/// `id`/`name`, retrouve l'élément déplacé et ré-écrit le
/// chemin au lieu de rendre `unfocused`.
#[test]
fn focus_reattaches_by_identity() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    let tab = s.open_tab();
    s.tabs.get_mut(&tab).unwrap().dom = Some(test_dom("<input name=\"q\"><p>x</p>"));
    // Focus par le vrai `set_focus` (chemin + ancre).
    let input = {
        let e = s.tabs.get(&tab).unwrap();
        find_by_attr(e.dom.as_ref().unwrap(), "name", "q").unwrap()
    };
    s.set_focus(tab, Some(input));
    let e = s.tabs.get(&tab).unwrap();
    assert_eq!(
        e.focused_key.as_ref().map(|(a, _)| a.as_str()),
        Some("name")
    );

    // (a) Mutation : un `<input>` inséré AVANT → le chemin
    // résoudrait sur le voisin — la vérification d'ancre
    // le détecte et ré-ancre sur le vrai champ.
    s.tabs.get_mut(&tab).unwrap().dom =
        Some(test_dom("<input name=\"dummy\"><input name=\"q\"><p>x</p>"));
    let n = s.focused_node(tab).expect("focus ré-ancré");
    let e = s.tabs.get(&tab).unwrap();
    let dom = e.dom.as_ref().unwrap();
    assert_eq!(
        dom.attribute(n, "name").unwrap().as_deref(),
        Some("q"),
        "le chemin ne doit pas résoudre sur le voisin"
    );
    // Le chemin est ré-écrit : il résout à nouveau, seul.
    assert_eq!(dom_path_node(dom, e.focused.as_ref().unwrap()), Some(n));

    // (b) Chemin complètement mort (le parent a disparu) :
    // l'ancre seule suffit au ré-ancrage.
    s.tabs.get_mut(&tab).unwrap().dom =
        Some(test_dom("<div><span><input name=\"q\"></span></div>"));
    let n = s.focused_node(tab).expect("focus ré-ancré sur chemin mort");
    let e = s.tabs.get(&tab).unwrap();
    let dom = e.dom.as_ref().unwrap();
    assert_eq!(dom.attribute(n, "name").unwrap().as_deref(), Some("q"));
    assert_eq!(dom_path_node(dom, e.focused.as_ref().unwrap()), Some(n));

    // (c) Élément réellement supprimé → `None` honnête,
    // pas de fausse cible.
    s.tabs.get_mut(&tab).unwrap().dom = Some(test_dom("<p>rien</p>"));
    assert!(s.focused_node(tab).is_none());
}

/// Étape 91 — peinture détachée : `dom`/`tree` vivent chez la
/// tâche d'apply — les entrées s'enfilent (`Ignored`, jamais
/// `unfocused` mensonger) ; une file non vide impose
/// l'enfilement même fenêtre retombée (FIFO strict, sinon une
/// frappe dépasserait le replay) ; `Scroll` consécutifs
/// coalescent ; le drain rejoue l'ordre à la réinsertion.
#[test]
fn deferred_inputs_fifo_and_coalesce() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    let tab = s.open_tab();
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.dom = Some(test_dom("<input id=\"q\" name=\"q\">"));
        e.paint_detached = true;
    }
    let input = {
        let e = s.tabs.get(&tab).unwrap();
        find_by_id(e.dom.as_ref().unwrap(), "q").unwrap()
    };
    s.set_focus(tab, Some(input));

    // Fenêtre détachée : tout s'enfile, rien ne traite.
    for ev in [
        InputEvent::Key {
            key: "a".into(),
            mods: KeyMods::NONE,
        },
        InputEvent::Scroll { dy: 10.0 },
        InputEvent::Scroll { dy: 5.0 },
        InputEvent::Key {
            key: "b".into(),
            mods: KeyMods::NONE,
        },
    ] {
        let r = s.handle_input_now(tab, ev).unwrap();
        assert_eq!(r, InputOutcome::Ignored, "détaché ⇒ différé");
    }
    {
        let e = s.tabs.get(&tab).unwrap();
        assert_eq!(
            e.deferred_inputs.len(),
            3,
            "les deux scrolls consécutifs coalescent"
        );
        assert!(matches!(
            e.deferred_inputs[1],
            InputEvent::Scroll { dy } if (dy - 15.0).abs() < f32::EPSILON
        ));
    }
    // Fenêtre retombée mais file non vide : la touche s'enfile
    // quand même — jamais devant les entrées plus anciennes.
    s.tabs.get_mut(&tab).unwrap().paint_detached = false;
    let r = s
        .handle_input_now(
            tab,
            InputEvent::Key {
                key: "c".into(),
                mods: KeyMods::NONE,
            },
        )
        .unwrap();
    assert_eq!(r, InputOutcome::Ignored, "file non vide ⇒ FIFO strict");
    assert_eq!(s.tabs.get(&tab).unwrap().deferred_inputs.len(), 4);

    // Drain à la réinsertion : rejoue FIFO dans l'ordre.
    s.drain_deferred_inputs(tab);
    let e = s.tabs.get(&tab).unwrap();
    assert!(e.deferred_inputs.is_empty());
    let v = e.dom.as_ref().unwrap().attribute(input, "value").unwrap();
    assert_eq!(v.as_deref(), Some("abc"));
}

// ------------------------------------------------------------------
// Inventaire mémoire du Tab — sonde M-H1. Chaque champ volumineux
// est compté en octets (capacités × size_of) ; l'écart entre la
// somme et le `live` de l'allocateur désigne le coupable opaque.
//
//   cargo test -p explorer-core --lib tab_mem_inventory -- --ignored --nocapture
// ------------------------------------------------------------------

fn dom_deep_bytes(dom: &Dom, n: explorer_dom::NodeId) -> usize {
    let mut b = dom.text(n).map_or(0, str::len);
    if let Ok(attrs) = dom.attributes(n) {
        b += attrs
            .iter()
            .map(|a| a.name.len() + a.value.len())
            .sum::<usize>();
    }
    b + dom
        .children(n)
        .unwrap_or_default()
        .iter()
        .map(|&c| dom_deep_bytes(dom, c))
        .sum::<usize>()
}

fn box_deep_bytes(bx: &explorer_layout::LayoutBox) -> usize {
    let mut b = std::mem::size_of::<explorer_layout::LayoutBox>()
        + bx.text.as_deref().map_or(0, str::len)
        + bx.children.capacity() * std::mem::size_of::<explorer_layout::LayoutBox>()
        + bx.lines.capacity() * std::mem::size_of::<explorer_layout::inline::LineBox>();
    for l in &bx.lines {
        b += l.fragments.capacity() * std::mem::size_of::<explorer_layout::inline::Fragment>();
        for f in &l.fragments {
            b += f.text.len();
        }
    }
    for c in &bx.children {
        b += box_deep_bytes(c);
    }
    b
}

/// Entrées+octets des custom properties `--*` d'un style de
/// boîte (sonde M-H1 — inflation par héritage plat).
fn style_vars(bx: &explorer_layout::LayoutBox, acc: &mut (usize, usize)) {
    for (k, v) in bx.style.iter() {
        if k.starts_with("--") {
            acc.0 += 1;
            acc.1 += k.len() + v.len();
        }
    }
    for c in &bx.children {
        style_vars(c, acc);
    }
}

/// RSS du processus en Mio via pwsh (sonde M-H1 — bissection
/// par drop : Windows décommit les gros blocs à la libération).
fn rss_mib() -> u64 {
    let out = std::process::Command::new("pwsh")
        .args([
            "-NoProfile",
            "-Command",
            &format!("(Get-Process -Id {}).WorkingSet64", std::process::id()),
        ])
        .output();
    out.ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|t| t.trim().parse::<u64>().ok())
        .map_or(0, |b| b / 1_048_576)
}
#[tokio::test]
#[ignore = "sonde réseau réel — diagnostic M-H1 manuel"]
#[allow(clippy::too_many_lines)] // inventaire exhaustif, pas de logique à extraire
async fn tab_mem_inventory() {
    use explorer_net::{
        decode::DecodingTransport, h1::HttpConfig, h2::AutoClient, policy::RedirectTransport,
        tls::TlsConnector,
    };
    let tls = Arc::new(TlsConnector::web_roots());
    let h1: Arc<dyn HttpTransport> = Arc::new(AutoClient::new(tls, HttpConfig::default()));
    let decoded: Arc<dyn HttpTransport> = Arc::new(DecodingTransport::wrap(h1, 16 * 1024 * 1024));
    let transport: Arc<dyn HttpTransport> = Arc::new(RedirectTransport::wrap(decoded, 8));
    let mut s = Session::new(transport, Arc::new(TestFont));
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let tab = s.open_tab();
    // Cible configurable (défaut : la page mesurée en M-H1) —
    // MEM_PROBE_URL=https://github.com/ cargo test … tab_mem_inventory
    let url = std::env::var("MEM_PROBE_URL")
        .unwrap_or_else(|_| "https://en.wikipedia.org/wiki/Rust_(programming_language)".into());
    s.navigate(tab, &url).await.unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        s.pump_net().await;
        s.pump_timers(tab);
        if s.drain_script_nav(tab).await > 0 {
            continue;
        }
        let idle = s.net_pending() == 0 && s.next_timer_deadline().is_none();
        if idle || std::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let e = s.tabs.get_mut(&tab).unwrap();
    let mib = |n: usize| n / 1_048_576;
    // DOM — arène opaque : live_count × Slot estimé (~120 o) + textes.
    let (nodes, dom_text) = e.dom.as_ref().map_or((0, 0), |d| {
        (d.live_count(), dom_deep_bytes(d, d.document()))
    });
    eprintln!("dom: {nodes} nœuds, texte+attrs {} Mio", mib(dom_text));
    let tree_b = e.tree.as_ref().map_or(0, box_deep_bytes);
    eprintln!(
        "boxtree: {} Mio (size_of LayoutBox={})",
        mib(tree_b),
        std::mem::size_of::<explorer_layout::LayoutBox>()
    );
    let (items_cap, spans_cap, list_txt) = e.list.as_ref().map_or((0, 0, 0), |l| {
        (
            l.items.capacity(),
            l.spans.len(),
            l.items
                .iter()
                .map(|i| match i {
                    explorer_render::DisplayItem::Placeholder { label, .. } => label.len(),
                    _ => 0,
                })
                .sum::<usize>(),
        )
    });
    eprintln!(
        "list: cap {} × {} o = {} Mio, spans {}, texte {} o",
        items_cap,
        std::mem::size_of::<explorer_render::DisplayItem>(),
        mib(items_cap * std::mem::size_of::<explorer_render::DisplayItem>()),
        spans_cap,
        list_txt
    );
    let frame_b = e.frame.as_ref().map_or(0, |f| f.pixels.capacity());
    eprintln!(
        "frame: {:?} {} Mio",
        e.frame.as_ref().map(|f| (f.width, f.height)),
        mib(frame_b)
    );
    let png_b = e.png.as_ref().map_or(0, Vec::len);
    let img_b: usize = e
        .images
        .replaced
        .values()
        .chain(e.images.backgrounds.values())
        .chain(e.images.svgs.values())
        .chain(e.images.masks.values())
        .map(|i| i.rgba.len())
        .sum();
    eprintln!("png {} Mio, images {} Mio", mib(png_b), mib(img_b));
    eprintln!(
        "linked_sheets {} o / {} feuilles",
        e.linked_sheets.iter().map(String::len).sum::<usize>(),
        e.linked_sheets.len()
    );
    let cs = std::mem::size_of::<explorer_css::ComputedStyle>();
    eprintln!(
        "prev_styles: {} entrées × ComputedStyle({} o) ≈ {} Mio (cap {})",
        e.prev_styles.len(),
        cs,
        mib(e.prev_styles.capacity() * (8 + cs)),
        e.prev_styles.capacity()
    );
    eprintln!(
        "hist {} entrées, pending_nav {}, js_pending_net {} ({} Mio corps), bfcache {}, dirty {}, media {}",
        e.hist.len(),
        e.pending_nav.len(),
        e.js_pending_net.len(),
        mib(e.js_pending_net.iter().map(|r| r.body.len()).sum()),
        e.bfcache.len(),
        e.dirty.len(),
        e.media.len()
    );
    eprintln!(
        "deferred {}, attrs {}, downloads session {}",
        e.deferred_inputs.len(),
        e.js_pending_attrs.len(),
        s.downloads.len()
    );
    // Quantification `--*` : le part soupçonné de l'inflation
    // ComputedStyle (héritage par copie plate des custom props).
    // R-57 : la table est `Arc`-partagée — les chiffres
    // ci-dessous sont **logiques** (entrées itérées × taille),
    // le physique est lu sur les drops RSS plus bas.
    let (mut var_entries, mut var_bytes, mut all_entries, mut all_bytes) =
        (0usize, 0usize, 0usize, 0usize);
    for st in e.prev_styles.values() {
        for (k, v) in st.iter() {
            all_entries += 1;
            all_bytes += k.len() + v.len();
            if k.starts_with("--") {
                var_entries += 1;
                var_bytes += k.len() + v.len();
            }
        }
    }
    let (te, tb, ve, vb) = (all_entries, all_bytes, var_entries, var_bytes);
    eprintln!(
        "prev_styles props: {te} entrées / {} Mio dont --* : {ve} / {} Mio (logique — Arc-partagé)",
        tb / 1_048_576,
        vb / 1_048_576
    );
    let mut acc = (0usize, 0usize);
    if let Some(t) = e.tree.as_ref() {
        style_vars(t, &mut acc);
    }
    eprintln!(
        "tree --* : {} entrées / {} Mio (logique — Arc-partagé)",
        acc.0,
        acc.1 / 1_048_576
    );
    // Bissection par champs — RSS auto-mesuré à chaque drop
    // (Windows rend les gros blocs VirtualAlloc → la chute
    // désigne le coupable ; les petits blocs restent dans le tas).
    eprintln!("rss {} Mio -- avant drops", rss_mib());
    macro_rules! drop_probe {
        ($label:literal, $act:expr) => {
            $act;
            eprintln!("rss {} Mio -- {}", rss_mib(), $label);
        };
    }
    drop_probe!("drop dom", e.dom = None);
    drop_probe!("drop tree", e.tree = None);
    drop_probe!("drop list", e.list = None);
    drop_probe!("drop frame", e.frame = None);
    drop_probe!(
        "drop font_cache",
        e.font_cache = explorer_render::GlyphCache::new()
    );
    drop_probe!(
        "drop images",
        e.images = explorer_render::PaintImages::from_replaced(explorer_render::ImageTable::new())
    );
    drop_probe!("drop prev_styles", e.prev_styles.clear());
    drop_probe!("drop linked_sheets", e.linked_sheets.clear());
    drop_probe!("drop js", e.js = None);
    drop_probe!("drop pending_nav/hist/net", {
        e.pending_nav.clear();
        e.hist.clear();
        e.js_pending_net.clear();
        e.dirty.clear();
        e.media.clear();
        e.deferred_inputs.clear();
        e.js_pending_attrs.clear();
    });
    eprintln!("-- fin");
}

/// Sonde JS terrain (plan NAV) — reproduit les `TypeError`
/// observés sur lobste.rs réel et attache chaque
/// `PipelineError` à l'API manquante ; mesure au passage
/// `dom_sync_stats`/`heap_stats` (JS-C37/39) en conditions
/// réelles — les instruments n'ont pas encore servi en vol.
///
/// `LOBSTE_PROBE_URL=https://lobste.rs/ cargo test -p
/// explorer-core --lib lobste_pipeline_errors -- --ignored
/// --nocapture`
#[tokio::test]
#[ignore = "sonde réseau réel — diagnostic JS terrain manuel"]
#[allow(clippy::too_many_lines)]
async fn lobste_pipeline_errors() {
    use explorer_net::{
        decode::DecodingTransport, h1::HttpConfig, h2::AutoClient, policy::RedirectTransport,
        tls::TlsConnector,
    };
    // Espion des URLs réellement demandées — la preuve
    // terrain du filtre `media=` (étape 96/97) : avec
    // `LOBSTE_PROBE_SCHEME=dark`, `dark-normal.css` doit
    // apparaître au lieu de `light-normal.css`.
    struct UrlSpy {
        inner: Arc<dyn HttpTransport>,
        seen: std::sync::Mutex<Vec<String>>,
        t0: std::time::Instant,
    }
    #[async_trait::async_trait]
    impl HttpTransport for UrlSpy {
        async fn fetch(&self, req: &Request) -> NetResult<Response> {
            if let Ok(mut v) = self.seen.lock() {
                v.push(req.url.to_string());
            }
            eprintln!("[fetch +{}s] {}", self.t0.elapsed().as_secs(), req.url);
            let r = self.inner.fetch(req).await;
            eprintln!(
                "[done  +{}s] {}",
                self.t0.elapsed().as_secs(),
                r.as_ref().map_or(0, |resp| resp.status)
            );
            r
        }
    }
    let tls = Arc::new(TlsConnector::web_roots());
    let h1: Arc<dyn HttpTransport> = Arc::new(AutoClient::new(tls, HttpConfig::default()));
    let decoded: Arc<dyn HttpTransport> = Arc::new(DecodingTransport::wrap(h1, 16 * 1024 * 1024));
    let redirect: Arc<dyn HttpTransport> = Arc::new(RedirectTransport::wrap(decoded, 8));
    let t_start = std::time::Instant::now();
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "sync_profile=info".into()),
        )
        .with_writer(std::io::stderr)
        .try_init();
    let spy = Arc::new(UrlSpy {
        inner: redirect,
        seen: std::sync::Mutex::new(Vec::new()),
        t0: t_start,
    });
    let transport: Arc<dyn HttpTransport> = spy.clone();
    let mut s = Session::new(transport, Arc::new(TestFont));
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    // `LOBSTE_PROBE_SCHEME=dark` → env sombre (étape 97) :
    // lobste.rs doit fetcher `dark-normal.css` au lieu de
    // `light-normal.css` via `<link media>` filtré.
    if std::env::var("LOBSTE_PROBE_SCHEME").as_deref() == Ok("dark") {
        s.set_color_scheme(explorer_css::ColorScheme::Dark);
    }
    let mut rx = s.subscribe();
    let tab = s.open_tab();
    let url = std::env::var("LOBSTE_PROBE_URL").unwrap_or_else(|_| "https://lobste.rs/".into());
    eprintln!("[+{}s] navigate…", t_start.elapsed().as_secs());
    s.navigate(tab, &url).await.unwrap();
    eprintln!("[+{}s] navigate rendu", t_start.elapsed().as_secs());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let mut errors = Vec::new();
    let mut navs = 0usize;
    loop {
        // Deadline en TÊTE de boucle : un `continue`
        // (navs script en file — hang dev.to mesuré >10 min)
        // sautait le check en fin de boucle et tournait sans
        // fin.
        if std::time::Instant::now() >= deadline {
            break;
        }
        while let Ok(ev) = rx.try_recv() {
            if let CoreEvent::PipelineError { message, .. } = ev {
                errors.push(message);
            }
        }
        s.pump_net().await;
        s.pump_timers(tab);
        let n = s.drain_script_nav(tab).await;
        if n > 0 {
            navs += n;
            continue;
        }
        let idle = s.net_pending() == 0 && s.next_timer_deadline().is_none();
        if idle {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    eprintln!(
        "diagnostic pompe : {navs} navs script drainées, \
         net_pending={}, timer_armé={}",
        s.net_pending(),
        s.next_timer_deadline().is_some()
    );
    while let Ok(ev) = rx.try_recv() {
        if let CoreEvent::PipelineError { message, .. } = ev {
            errors.push(message);
        }
    }
    let nav_errors = errors.len();
    // Grille de survol du harnais `site_smoke` : les listeners
    // `mouseover`/`mousemove` sont le déclencheur des TypeErrors
    // observés en vol — l'évaluation top-level seule est propre.
    let mut y = 40.0f32;
    while y <= 704.0 {
        let mut x = 40.0f32;
        while x <= 968.0 {
            let _ = s.handle_input_now(tab, InputEvent::MouseMove { x, y });
            while let Ok(ev) = rx.try_recv() {
                if let CoreEvent::PipelineError { message, .. } = ev {
                    errors.push(message);
                }
            }
            x += 64.0;
        }
        y += 48.0;
    }
    // Rafale clavier (étape 103) : les listeners `keydown`/
    // `keyup` lisent désormais `key`/`code`/`keyCode`/`which` —
    // un handler réel qui se trompe de prop lève un PipelineError
    // où `undefined` passait muet avant.
    for key in ["a", "Enter", "Escape", "ArrowDown", "j", "k"] {
        let _ = s.handle_input_now(
            tab,
            InputEvent::Key {
                key: key.into(),
                mods: KeyMods::NONE,
            },
        );
        let _ = s.handle_input_now(
            tab,
            InputEvent::KeyUp {
                key: key.into(),
                mods: KeyMods::NONE,
            },
        );
        while let Ok(ev) = rx.try_recv() {
            if let CoreEvent::PipelineError { message, .. } = ev {
                errors.push(message);
            }
        }
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        s.pump_net().await;
        s.pump_timers(tab);
        while let Ok(ev) = rx.try_recv() {
            if let CoreEvent::PipelineError { message, .. } = ev {
                errors.push(message);
            }
        }
        let idle = s.net_pending() == 0 && s.next_timer_deadline().is_none();
        if idle || std::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    while let Ok(ev) = rx.try_recv() {
        if let CoreEvent::PipelineError { message, .. } = ev {
            errors.push(message);
        }
    }
    eprintln!(
        "-- {url} : {nav_errors} PipelineError nav + {} survol",
        errors.len() - nav_errors
    );
    for (i, m) in errors.iter().enumerate() {
        eprintln!("  [{i}] {m}");
    }
    // Feuilles réellement demandées (preuve du filtre
    // `media=`/`prefers-color-scheme` en vol).
    if let Ok(v) = spy.seen.lock() {
        let css: Vec<&str> = v
            .iter()
            .filter(|u| u.contains(".css"))
            .map(String::as_str)
            .collect();
        eprintln!("feuilles fetchées ({}):", css.len());
        for u in css {
            eprintln!("  {u}");
        }
    }
    let e = s.tabs.get_mut(&tab).unwrap();
    if let Some(eng) = e.js.as_mut() {
        if let Some(st) = eng.dom_sync_stats() {
            eprintln!("dom_sync_stats: {st:?}");
        }
        if let Some(st) = eng.heap_stats() {
            eprintln!("heap_stats: {st:?}");
        }
    }
}

/// Étape 105 — `KeyMods::from_bits` décode le bitmask
/// FFI/REST (`1`=ctrl, `2`=shift, `4`=alt, `8`=meta) ;
/// les bits inconnus sont ignorés, jamais de panic.
#[test]
fn keymods_from_bits_decode() {
    assert_eq!(KeyMods::from_bits(0), KeyMods::NONE);
    assert!(KeyMods::from_bits(1).ctrl);
    assert!(KeyMods::from_bits(2).shift);
    assert!(KeyMods::from_bits(4).alt);
    assert!(KeyMods::from_bits(8).meta);
    let all = KeyMods::from_bits(0xFF);
    assert!(all.ctrl && all.shift && all.alt && all.meta);
    assert!(KeyMods {
        ctrl: true,
        ..KeyMods::NONE
    }
    .chord());
    assert!(!KeyMods {
        shift: true,
        ..KeyMods::NONE
    }
    .chord());
}

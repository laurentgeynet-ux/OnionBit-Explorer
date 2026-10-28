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
                same_doc: false,
                state: None,
            },
            HistEntry {
                req: Request::get(url_b.clone()),
                failed: false,
                same_doc: false,
                state: None,
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
            adopted_sheets: Vec::new(),
            defined_names: Vec::new(),
            prev_styles: HashMap::new(),
            dirty: Vec::new(),
            emitted: 0,
            last_req: None,
            frames: Vec::new(),
            frame_nodes: Vec::new(),
            referrer_policy: None,
            base_url: None,
            base_target: None,
            refresh: None,
            csp: Vec::new(),
            img_bodies: HashMap::new(),
            bg_aliases: HashMap::new(),
            mask_aliases: HashMap::new(),
            frame_ctxs: Vec::new(),
            style_dirty: Vec::new(),
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
            same_doc: false,
            state: None,
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
            adopted_sheets: Vec::new(),
            defined_names: Vec::new(),
            prev_styles: HashMap::new(),
            dirty: Vec::new(),
            emitted: 0,
            last_req: None,
            frames: Vec::new(),
            frame_nodes: Vec::new(),
            referrer_policy: None,
            base_url: None,
            base_target: None,
            refresh: None,
            csp: Vec::new(),
            img_bodies: HashMap::new(),
            bg_aliases: HashMap::new(),
            mask_aliases: HashMap::new(),
            frame_ctxs: Vec::new(),
            style_dirty: Vec::new(),
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
            same_doc: false,
            state: None,
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
        dropped_emits: 0,
        first_cycle_path: None,
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
    let eng = s.spawn_engine(tab, &html, true, "");
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
    let eng = s.spawn_engine(tab, html, true, "");
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

/// 53f — tail de commit pendant le détachement : les
/// transitions `readyState` + événements document sont
/// tamponnées puis rejouées à la réinsertion, dans l'ordre.
/// Sans le tampon elles mouraient muet (`entry.js = None`)
/// — `readyState` figé à `"loading"` et les portes
/// `readystatechange` des pages pendantes à jamais
/// (mesuré sur github : `element-registry` lazy inerte).
#[test]
fn doc_lifecycle_replays_after_detached_pump() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let tab = s.open_tab();
    let html = "<!doctype html><html><body><div id=\"x\"></div></body></html>";
    let eng = s.spawn_engine(tab, html, true, "");
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        let mut tb = TreeBuilder::new();
        tb.push(html);
        tb.finish();
        e.dom = Some(tb.into_dom());
        e.url = Some(Url::parse("http://h.test/lc").unwrap());
        e.js = eng;
    }
    // Listener page : trace chaque transition vue.
    let _ = s.tabs.get_mut(&tab).and_then(|e| e.js.as_mut()).map(|eng| {
        eng.eval(
            "document.addEventListener('readystatechange',function(){\
                 document.getElementById('x').textContent+='>'+document.readyState;});",
        )
    });

    // La pompe emporte le moteur (fenêtre détachée ouverte).
    let (eng, _budget, epoch, sync_ok) = s.take_js_engine(tab).unwrap();
    assert!(s.tabs.get(&tab).unwrap().js.is_none());

    // Le tail de commit court SANS moteur → tout tamponne.
    s.set_doc_ready_state(tab, explorer_js::JS_CTX_TOP, "interactive");
    s.dispatch_doc_event(tab, "readystatechange");
    s.dispatch_doc_event(tab, "DOMContentLoaded");
    s.set_doc_ready_state(tab, explorer_js::JS_CTX_TOP, "complete");
    s.dispatch_doc_event(tab, "readystatechange");
    s.dispatch_doc_event(tab, "load");
    assert_eq!(
        s.tabs.get(&tab).unwrap().pending_doc_lifecycle.len(),
        6,
        "les six étapes du cycle sont tamponnées"
    );

    // Réinsertion → rejeu dans l'ordre.
    s.commit_js_pump(tab, epoch, Some((0, None, eng)), sync_ok);
    assert!(s.tabs.get(&tab).unwrap().pending_doc_lifecycle.is_empty());

    let out = s
        .tabs
        .get_mut(&tab)
        .and_then(|e| e.js.as_mut())
        .and_then(|eng| {
            eng.eval("document.readyState+'::'+document.getElementById('x').textContent")
                .ok()
        })
        .unwrap_or_default();
    assert_eq!(
        out.trim(),
        "complete::>interactive>complete",
        "transitions rejouées à la réinsertion : {out}"
    );
}

/// Trou de liveness dyn-subresources : `<link rel=stylesheet>`
/// inséré par JS ne devait son fetch qu'à `after_script`
/// (gate `jobs>0` de la pompe) — une page qui marque le lien
/// sans ré-armer de job laissait `pending_dyn_links` mourir
/// de faim : `load`/`error` jamais livrés, la promesse
/// `c.e` de webpack suspendue (mesuré github : chunks CSS
/// `n.f.css` → `link.onload`, lazy-CE jamais définis). La
/// pompe draine désormais `drain_script_net` à chaque
/// battement — avec **zéro** job en file.
#[tokio::test]
async fn dyn_link_drains_without_pending_jobs() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let tab = s.open_tab();
    let html = "<!doctype html><html><head></head><body></body></html>";
    let eng = s.spawn_engine(tab, html, true, "");
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        let mut tb = TreeBuilder::new();
        tb.push(html);
        tb.finish();
        e.dom = Some(tb.into_dom());
        e.url = Some(Url::parse("http://h.test/dl").unwrap());
        e.js = eng;
    }
    // `n.f.css` webpack : lien posé par propriétés, attaché
    // à <head> — aucun timer/promesse armé derrière.
    let _ = s.tabs.get_mut(&tab).and_then(|e| e.js.as_mut()).map(|eng| {
        eng.eval(
            "var l=document.createElement('link');\
             l.rel='stylesheet';\
             l.href='http://h.test/x.css';\
             document.head.appendChild(l);",
        )
    });
    // Pompe silencieuse (jobs=0) — le drain doit émettre le
    // lien malgré l'absence de jobs.
    s.pump_timers(tab);
    let pending = s
        .tabs
        .get_mut(&tab)
        .and_then(|e| e.js.as_mut())
        .map_or(999, |eng| eng.drain_dyn_links().len());
    assert_eq!(
        pending, 0,
        "le lien dyn doit être drainé : {pending} restant"
    );
}

/// 141b — `animationstart` synthétique : la cascade qui
/// enregistre une animation `@keyframes` nouvelle sur un
/// élément émet l'événement bullant sur le miroir, props
/// `animationName`/`elapsedTime`/`pseudoElement` (spec
/// `AnimationEvent`). Sans lui, la découverte lazy des
/// registries CE (`x(e)` → `m.get(e.animationName)`,
/// github catalyst) ne voyait aucune insertion post-load
/// — `define` jamais atteint.
#[tokio::test]
async fn animationstart_fires_for_new_keyframe_animation() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let tab = s.open_tab();
    let html = "<!doctype html><html><head><style>\
                @keyframes ghz { from { opacity:.9 } to { opacity:1 } }\
                #x { animation-name: ghz; animation-duration: 5s }\
                </style></head><body><div id=\"x\"></div></body></html>";
    let eng = s.spawn_engine(tab, html, true, "");
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        let mut tb = TreeBuilder::new();
        tb.push(html);
        tb.finish();
        e.dom = Some(tb.into_dom());
        e.url = Some(Url::parse("http://h.test/as").unwrap());
        e.js = eng;
    }
    let _ = s.tabs.get_mut(&tab).and_then(|e| e.js.as_mut()).map(|eng| {
        eng.eval(
            "window.__as='none';\
             document.getElementById('x').addEventListener('animationstart',\
             function(e){window.__as=e.animationName+'@'+e.target.tagName;});",
        )
    });
    // Cascade → naissance enregistrée → drain pompe →
    // événement livré au listener miroir.
    s.relayout(tab).unwrap();
    s.pump_timers(tab);
    let out = s
        .tabs
        .get_mut(&tab)
        .and_then(|e| e.js.as_mut())
        .and_then(|eng| eng.eval("window.__as").ok())
        .unwrap_or_default();
    assert_eq!(out.trim(), "ghz@DIV", "animationstart livré : {out}");
}

/// Étape 143 — pont lazy-CE : feuille **construite**
/// (`new CSSStyleSheet` + `insertRule` + `adoptedStyleSheets`)
/// cascadée réellement, et `:defined` lu sur le registre
/// miroir. Mécanisme github catalyst à l'identique :
/// `sel:not(:defined){animation:nom}` → naissance →
/// `animationstart` → découverte ; puis `define` bascule
/// l'élément en `:defined`.
#[tokio::test]
async fn adopted_sheet_defined_selector_drives_lazy_discovery() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let tab = s.open_tab();
    let html = "<!doctype html><html><body><x-lazy></x-lazy></body></html>";
    let eng = s.spawn_engine(tab, html, true, "");
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        let mut tb = TreeBuilder::new();
        tb.push(html);
        tb.finish();
        e.dom = Some(tb.into_dom());
        e.url = Some(Url::parse("http://h.test/lazy").unwrap());
        e.js = eng;
    }
    // Feuille construite à la catalyst + listener capture.
    let _ = s.tabs.get_mut(&tab).and_then(|e| e.js.as_mut()).map(|eng| {
        eng.eval(
            "window.__as='none';\
             document.addEventListener('animationstart',function(e){\
             window.__as=e.animationName+'@'+e.target.tagName;},true);\
             var sh=new CSSStyleSheet();\
             sh.insertRule('@keyframes lazydetect { from {opacity:.9} to {opacity:1} }');\
             sh.insertRule('x-lazy:not(:defined){animation:lazydetect 5s !important}');\
             document.adoptedStyleSheets=[sh];",
        )
    });
    // Drain (moteur présent) → feuille adoptée + registre
    // poussés au Dom → cascade → naissance → événement.
    s.drain_script_net(tab);
    s.relayout(tab).unwrap();
    s.pump_timers(tab);
    let out = s
        .tabs
        .get_mut(&tab)
        .and_then(|e| e.js.as_mut())
        .and_then(|eng| eng.eval("window.__as").ok())
        .unwrap_or_default();
    assert_eq!(
        out.trim(),
        "lazydetect@X-LAZY",
        "animationstart lazy : {out}"
    );
    // `define` → drain pousse le nom → `x-lazy:defined`
    // matche désormais (le registre vivant du Dom session).
    let _ = s.tabs.get_mut(&tab).and_then(|e| e.js.as_mut()).map(|eng| {
        eng.eval(
            "customElements.define('x-lazy',class extends HTMLElement{});\
             window.__def=document.querySelector('x-lazy')!==null",
        )
    });
    s.drain_script_net(tab);
    {
        let entry = s.tabs.get_mut(&tab).unwrap();
        let dom = entry.dom.as_ref().unwrap();
        // BFS — `x-lazy` est le seul élément à tiret.
        let mut found = None;
        let mut queue = vec![dom.document()];
        while let Some(n) = queue.pop() {
            if dom.tag_name(n).is_ok_and(|t| t == "x-lazy") {
                found = Some(n);
                break;
            }
            if let Ok(ch) = dom.children(n) {
                queue.extend(ch.iter().rev());
            }
        }
        let node = found.expect("x-lazy dans le DOM");
        let sel = explorer_css::parse_selector_list("x-lazy:defined")
            .expect("sélecteur parse")
            .remove(0);
        assert!(
            explorer_css::matches(dom, node, &sel),
            "x-lazy défini doit matcher :defined"
        );
    }
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
    let eng = s.spawn_engine(tab, &html, true, "").unwrap();
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
        let is_scroll = matches!(ev, InputEvent::Scroll { .. });
        let r = s.handle_input_now(tab, ev).unwrap();
        // Détaché ⇒ différé ; le scroll rapporte sa position
        // **projetée** plutôt qu'un `Ignored` (vol 13).
        if is_scroll {
            assert!(matches!(r, InputOutcome::Scrolled { .. }));
        } else {
            assert_eq!(r, InputOutcome::Ignored, "détaché ⇒ différé");
        }
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

/// Issue #23 — mort permanente des sondes : un futur de
/// navigation droppé pendant le rendu détaché (timeout du
/// client HTTP de `POST /navigate`, drain interrompu)
/// perdait la payload avec `paint_detached` collé —
/// `/dom`/`/layout`/`/mem` 404 à vie, scroll inerte. La
/// garde `DetachedPaint` parque l'orpheline ; la prochaine
/// prise de verrou la réinsère.
#[test]
fn orphaned_nav_paint_is_reclaimed() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    let tab = s.open_tab();
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.dom = Some(test_dom("<p>page vivante</p>"));
        e.nav_epoch = 3;
    }
    // Détachement identique à `nav_commit_shared`.
    let apply::NavPaint::Ready(g, _ctx, _ops) = s.take_nav_paint(tab, 3) else {
        panic!("payload attendue")
    };
    // Fenêtre détachée : les sondes meurent (constat).
    assert!(s.dom_dump(tab).is_none());
    // Le détenteur meurt en vol — le futur droppé détruit
    // la garde qui parque la payload.
    drop(g);
    // Première opportunité sous verrou (ici : la sonde du
    // handler API qui appelle reclaim avant son dump) —
    // l'onglet récupère son état, le drapeau retombe.
    s.reclaim_orphaned_paint(tab);
    assert!(s.dom_dump(tab).unwrap().contains("page vivante"));
    assert!(!s.tabs.get(&tab).unwrap().paint_detached);
    // Le cycle de peinture n'est plus bloqué (la boucle
    // `Busy` de `nav_commit_shared` repart).
    assert!(matches!(
        s.take_nav_paint(tab, 3),
        apply::NavPaint::Ready(..)
    ));
}

/// Issue #23 — la garde ne ressuscite jamais une page
/// supplantée : un orphelin d'epoch périmée est jeté à la
/// réclamation (la nav plus récente porte son propre
/// rendu) ; le drapeau est libéré quand même pour ne pas
/// bloquer la peinture suivante.
#[test]
fn orphaned_nav_paint_stale_epoch_is_dropped() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    let tab = s.open_tab();
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.dom = Some(test_dom("<p>page A</p>"));
        e.nav_epoch = 3;
    }
    let apply::NavPaint::Ready(g, _ctx, _ops) = s.take_nav_paint(tab, 3) else {
        panic!("payload attendue")
    };
    // Une nav plus récente démarre pendant que la payload
    // est dehors — puis le détenteur meurt.
    s.tabs.get_mut(&tab).unwrap().nav_epoch = 4;
    drop(g);
    s.reclaim_orphaned_paint(tab);
    let e = s.tabs.get(&tab).unwrap();
    assert!(!e.paint_detached);
    // Epoch périmée → jetée (la nav epoch 4 peindra la
    // sienne) — le DOM n'est pas ressuscité.
    assert!(s.dom_dump(tab).is_none());
}

/// Issue #23 — même garde sur la voie apply détaché
/// (`drain_applies_shared`) : un drain droppé en vol
/// réinsère aussi sa payload.
#[test]
fn orphaned_apply_paint_is_reclaimed() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    let tab = s.open_tab();
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.dom = Some(test_dom("<p>page vivante</p>"));
        e.nav_epoch = 1;
    }
    s.pending_applies.push_back(apply::PendingApply {
        tab,
        epoch: 1,
        html: "<p>resync</p>".into(),
        dirty: None,
        canvas_ops: explorer_js::CanvasOpsTable::new(),
        relayout: true,
        journal: None,
        ids: Vec::new(),
        tags: Vec::new(),
    });
    let Some((_job, g, _ctx, _t)) = s.take_next_apply() else {
        panic!("apply attendu");
    };
    assert!(s.dom_dump(tab).is_none());
    drop(g);
    s.reclaim_orphaned_paint(tab);
    assert!(s.dom_dump(tab).unwrap().contains("page vivante"));
    assert!(!s.tabs.get(&tab).unwrap().paint_detached);
}

/// Issue #23 — un commit qui arrive **après** la
/// libération du drapeau (rearm/réclamation) parque la
/// payload au lieu de la jeter : l'orpheline est reprise
/// à la réclamation suivante.
#[test]
fn nav_paint_commit_on_cleared_flag_is_parked() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    let tab = s.open_tab();
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.dom = Some(test_dom("<p>page vivante</p>"));
        e.nav_epoch = 3;
    }
    let apply::NavPaint::Ready(g, _ctx, _ops) = s.take_nav_paint(tab, 3) else {
        panic!("payload attendue")
    };
    let p = g.into_payload();
    // Le drapeau retombe avant le commit (rearm).
    s.rearm_nav_paint(tab);
    assert!(!s.commit_nav_paint(tab, 3, p));
    // La payload n'est pas perdue : la réclamation la
    // réinsère (epoch encore vivante).
    s.reclaim_orphaned_paint(tab);
    assert!(s.dom_dump(tab).unwrap().contains("page vivante"));
}

/// §B-17 — trou de cohérence DOM↔styles de boîte : un
/// `setAttribute` livré par une synchro **`relayout=false`**
/// (sous-ressources en vol) atteint le DOM commis sans
/// déclencher de relayout ; si l'apply relayout suivant a un
/// `dirty` ailleurs dans le document, le sous-arbre muté
/// garde son `bx.style` d'avant mutation **indéfiniment**
/// (vol santemag : drawer `inert` resté ciblable — ancres du
/// sous-menu navigables). Le diff d'attributs sur la baseline
/// cléée doit transformer la mutation en dette de restyle,
/// fusionnée à la cible du prochain relayout.
#[test]
fn attr_diff_restyle_subtree_hors_dirty() {
    // Le hit peut être un texte — on remonte au premier
    // ancêtre élément (déclarée en tête :
    // `items_after_statements`).
    fn first_el(dom: &Dom, mut cur: explorer_dom::NodeId) -> Option<explorer_dom::NodeId> {
        loop {
            if dom.tag_name(cur).is_ok() {
                return Some(cur);
            }
            cur = dom.parent(cur).ok().flatten()?;
        }
    }
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    let tab = s.open_tab();
    s.tabs.get_mut(&tab).unwrap().nav_epoch = 1;
    // `tags` vide → `build_keymap` tombe sur le zip
    // positionnel : tout nœud moteur prend la clé = rang
    // pré-ordre + 1 — baseline cléée complète sans compter.
    let ids: Vec<u64> = (1..=64).collect();
    // `div#o` APRÈS le `a` dans le document → peint
    // par-dessus : tant que `inert` n'a pas restylé le
    // sous-arbre (`pe:none`), le hit doit tomber sur `div#o`.
    let v1 = "<html><body>\
        <a href=/cible style=\"position:absolute;left:0;top:0;width:200px;height:40px;display:block\">lien</a>\
        <div id=o style=\"position:absolute;left:0;top:0;width:300px;height:300px\">ov</div>\
        <div id=z>z</div>\
        </body></html>";
    let v2 = v1.replace("<div id=o ", "<div id=o inert ");
    // sync1 : baseline cléée + relayout complet → keymap
    // peuplée, boîtes initiales (overlay ciblable).
    s.apply_dom_sync(tab, v1, None, true, Some(ids.clone()), Some(Vec::new()));
    // sync2 : `inert` livré SANS relayout — DOM commis
    // patché, boîtes figées ; le diff doit partir en dette.
    s.apply_dom_sync(tab, &v2, None, false, Some(ids.clone()), Some(Vec::new()));
    assert!(
        !s.tabs.get(&tab).unwrap().style_dirty.is_empty(),
        "le changement `inert` sans relayout doit devenir dette de restyle"
    );
    // État stale (le bug §B-17) : `inert` est dans le DOM
    // commis mais la boîte garde `pe:auto` → l'overlay
    // continue de voler le hit.
    let stale = s
        .node_at(tab, 10.0, 20.0)
        .and_then(|n| first_el(s.tabs.get(&tab).unwrap().dom.as_ref().unwrap(), n))
        .and_then(|n| {
            s.tabs
                .get(&tab)
                .unwrap()
                .dom
                .as_ref()?
                .attributes(n)
                .ok()?
                .iter()
                .find(|a| a.name == "id")
                .map(|a| a.value.clone())
        });
    assert_eq!(
        stale.as_deref(),
        Some("o"),
        "boîte figée attendue avant couverture"
    );
    // sync3 : relayout dont le `dirty` ne couvre que `#z` —
    // sans la dette, l'overlay garderait `pointer-events:auto`.
    let dirty_z = vec![
        ("html".to_string(), 0u32),
        ("body".to_string(), 0u32),
        ("div".to_string(), 1u32),
    ];
    s.apply_dom_sync(
        tab,
        &v2,
        Some(&dirty_z),
        true,
        Some(ids.clone()),
        Some(Vec::new()),
    );
    // Clic dans l'overlay posé sur le lien : la dette a dû
    // restylé le sous-arbre — `pe:none` hérité → le hit
    // traverse l'overlay et frappe l'ancre dessous.
    let hit = s.node_at(tab, 10.0, 20.0);
    let tag = hit
        .and_then(|n| first_el(s.tabs.get(&tab).unwrap().dom.as_ref().unwrap(), n))
        .and_then(|n| {
            s.tabs
                .get(&tab)
                .unwrap()
                .dom
                .as_ref()?
                .tag_name(n)
                .ok()
                .map(str::to_string)
        });
    assert_eq!(
        tag.as_deref(),
        Some("a"),
        "l'overlay `inert` doit être transparent au hit-test (hit={hit:?} tag={tag:?})"
    );
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

/// Issue #19 — `put_paint` (ici via `relayout`) publie la
/// table de géométrie clée `element_path` (union des boîtes
/// du nœud) ; `set_scroll` propage le scroll sans rebuild ;
/// `clear_geometry` vide les rects du document mort (jamais
/// de chemins recyclés → mesures d'un autre document).
#[test]
fn geometry_published_at_relayout() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    let tab = s.open_tab();
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.dom = Some(test_dom("<div><p>bonjour</p></div>"));
        e.url = Some(Url::parse("http://h.test/geom").unwrap());
    }
    s.relayout(tab).unwrap();
    let geom = s.tabs.get(&tab).unwrap().geom.clone();
    let div_path = vec![
        ("html".to_string(), 0u32),
        ("body".to_string(), 0),
        ("div".to_string(), 0),
    ];
    let mut p_path = div_path.clone();
    p_path.push(("p".to_string(), 0));
    {
        let g = geom.lock().unwrap();
        let div = g.rects.get(&div_path).expect("div mesuré");
        assert!(div.rect.w > 0.0 && div.rect.h > 0.0);
        assert!(!div.fixed);
        let p = g.rects.get(&p_path).expect("p mesuré");
        assert!(p.rect.h > 0.0);
    }
    // Scroll propagé sans rebuild (les rects survivent).
    s.tabs.get_mut(&tab).unwrap().set_scroll(42.0);
    {
        let g = geom.lock().unwrap();
        assert_eq!(g.scroll_y, 42.0);
        assert!(g.rects.contains_key(&div_path));
    }
    // Document mort → table vide.
    s.tabs.get_mut(&tab).unwrap().clear_geometry();
    assert!(geom.lock().unwrap().rects.is_empty());
}

/// Sonde `/eval` — `eval_shared` évalue dans le
/// contexte top-level de l'onglet avec le même contrat de
/// détachement que la pompe : valeur sérialisée rendue,
/// jobs armés par l'éval drainés dans le même tour, erreurs
/// JS rendues en `Err` (jamais de panic), `Unavailable`
/// sans moteur.
#[tokio::test]
async fn eval_shared_evalue_drain_et_tolere() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    s.set_js_factory(explorer_js::JscoreEngine::factory());
    let tab = s.open_tab();
    let tab_nu = s.open_tab();
    let html = "<!doctype html><html><body><p>x</p><script></script></body></html>";
    let eng = s.spawn_engine(tab, html, true, "").unwrap();
    s.tabs.get_mut(&tab).unwrap().js = Some(eng);
    let sm = tokio::sync::Mutex::new(s);

    // Onglet sans moteur → Unavailable (rien n'a tourné).
    assert!(matches!(
        eval_shared(&sm, tab_nu, "1".to_string()).await,
        EvalReply::Unavailable
    ));

    // Valeur immédiate.
    match eval_shared(&sm, tab, "40 + 2".to_string()).await {
        EvalReply::Done { value, .. } => assert_eq!(value.as_deref(), Ok("42")),
        EvalReply::Unavailable => panic!("moteur attendu"),
    }

    // L'éval arme une microtask — le drain interne la joue
    // avant réinsertion : la seconde éval la lit sans pompe.
    let _ = eval_shared(
        &sm,
        tab,
        "Promise.resolve().then(function(){window.__m = 9;}); 0".to_string(),
    )
    .await;
    match eval_shared(&sm, tab, "window.__m".to_string()).await {
        EvalReply::Done { value, jobs: _ } => assert_eq!(value.as_deref(), Ok("9")),
        EvalReply::Unavailable => panic!("moteur attendu"),
    }

    // Erreur JS → `Err` sérialisée, pas de panic.
    match eval_shared(&sm, tab, "null.champ".to_string()).await {
        EvalReply::Done { value, .. } => assert!(value.is_err()),
        EvalReply::Unavailable => panic!("moteur attendu"),
    }

    // Le moteur a été réinséré : une éval suivante reste
    // serviable (preuve du round-trip take/commit).
    match eval_shared(&sm, tab, "window.__m + 1".to_string()).await {
        EvalReply::Done { value, .. } => assert_eq!(value.as_deref(), Ok("10")),
        EvalReply::Unavailable => panic!("moteur réinséré attendu"),
    }
}

/// Boucle nav-auto bornée (cdiscount : 3 812 navigations
/// en 16 min, process tué à la main) — `max_auto_navs`
/// débitée par les intents script et le meta refresh,
/// jamais par les clics utilisateur.
#[test]
fn auto_nav_budget_caps_script_and_refresh() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    let tab = s.open_tab();
    s.max_auto_navs = 4;
    let act = || {
        DrainAct::Req(
            Box::new(Request::get(Url::parse("http://x.test/").unwrap())),
            true,
        )
    };
    // Intents script : le budget en laisse passer 4, le
    // reste est jeté (retour `true` — le drain s'arrête,
    // pas de re-file dans le moteur).
    for _ in 0..10 {
        assert!(s.enqueue_nav_act(tab, act(), true));
    }
    assert_eq!(s.tabs.get(&tab).unwrap().pending_nav.len(), 4);
    // Meta refresh : pareil — échéance échue, budget déjà
    // épuisé → rien n'est enfilé.
    s.tabs.get_mut(&tab).unwrap().refresh_at =
        Some((Instant::now(), Url::parse("http://x.test/again").unwrap()));
    s.pump_timers(tab);
    assert_eq!(s.tabs.get(&tab).unwrap().pending_nav.len(), 4);
    // Le clic utilisateur ne débite pas le budget — un
    // `auto=false` enfile toujours.
    assert!(s.enqueue_nav_act(tab, act(), false));
    assert_eq!(s.tabs.get(&tab).unwrap().pending_nav.len(), 5);
}

/// Vol rakuten/soundcloud — un `window.open`/`<a
/// target=_blank>` dont la réponse est un téléchargement
/// (ex. `.js` servi `text/javascript`) laissait un onglet
/// parasite vierge : `close_stillborn_popup` le referme et
/// ré-attribue les pendings à l'opener. Un contexte neuf
/// réellement navigué (URL + historique) n'est pas touché.
#[test]
fn stillborn_popup_closes_and_downloads_reattributed() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    let opener = s.open_tab();
    let url = Url::parse("http://h.test/page").unwrap();
    s.tabs.get_mut(&opener).unwrap().url = Some(url.clone());
    let popup = s.named_or_new_tab(opener, "");
    s.downloads.insert(
        7,
        PendingDownload {
            tail: NavTail {
                epoch: 0,
                push: true,
                prev_title: None,
                req: Request::get(url.clone()),
            },
            id: 7,
            tab: popup,
            url,
            filename_hint: None,
            mime: "text/javascript".to_string(),
            size: None,
            body: b"js".to_vec(),
        },
    );
    s.close_stillborn_popup(opener, popup);
    assert!(!s.tabs.contains_key(&popup), "popup vierge refermée");
    assert_eq!(
        s.pending_downloads(opener).len(),
        1,
        "le pending suit l'opener, pas l'onglet mort"
    );

    // Un contexte neuf qui a vraiment atterri (page ou
    // page d'erreur — `url`/`hist` peuplés) survit.
    let landed = s.named_or_new_tab(opener, "");
    {
        let e = s.tabs.get_mut(&landed).unwrap();
        e.url = Some(Url::parse("http://h.test/ok").unwrap());
        e.hist.push(HistEntry {
            req: Request::get(Url::parse("http://h.test/ok").unwrap()),
            failed: true,
            same_doc: false,
            state: None,
        });
    }
    s.close_stillborn_popup(opener, landed);
    assert!(s.tabs.contains_key(&landed), "onglet atterri conservé");
    // `close_stillborn_popup` sur l'opener lui-même : no-op.
    s.close_stillborn_popup(opener, opener);
    assert!(s.tabs.contains_key(&opener));
}

/// `download_meta` : un type **script** servi en top-level
/// n'est pas un document — `text/javascript` serait sinon
/// ratatché à `text/*` et peint en brut (vol rakuten).
#[test]
fn download_meta_rejects_script_mimes() {
    let h = vec![(
        "content-type".to_string(),
        "text/javascript; charset=utf-8".to_string(),
    )];
    let meta = download_meta(&h).expect("script top-level = téléchargement");
    assert_eq!(meta.mime, "text/javascript");
    let h = vec![(
        "content-type".to_string(),
        "application/javascript".to_string(),
    )];
    assert!(download_meta(&h).is_some());
    let h = vec![("content-type".to_string(), "text/html".to_string())];
    assert!(download_meta(&h).is_none(), "html reste un document");
    let h = vec![("content-type".to_string(), "text/plain".to_string())];
    assert!(download_meta(&h).is_none(), "text/plain reste un document");
}

/// Vol google SERP — la méta refresh enfermée dans un
/// `<noscript>` est rawtext quand le scripting est levé
/// (CAP-155) : invisible pour `meta_refresh`. Sur une page
/// de **fallback pur** (body = `noscript`/`script`
/// seulement) elle est honorée ; une page riche avec un
/// `noscript` annexe garde la spec (`None`).
#[test]
fn noscript_meta_refresh_fallback_pure_only() {
    let mut tb = TreeBuilder::new();
    tb.set_scripting(true);
    tb.push("<html><body><script>x()</script><noscript><meta http-equiv=refresh content=\"0;url=/retry\"></noscript></body></html>");
    tb.finish();
    let dom = tb.into_dom();
    assert!(meta_refresh(&dom).is_none(), "rawtext : pas d'élément meta");
    let (secs, raw) = noscript_meta_refresh(&dom).expect("fallback pur : méta honorée");
    assert_eq!(secs, 0.0);
    assert_eq!(raw.as_deref(), Some("/retry"));

    // Page riche + noscript annexe → spec : rien.
    let mut tb = TreeBuilder::new();
    tb.set_scripting(true);
    tb.push("<html><body><p>contenu</p><noscript><meta http-equiv=refresh content=\"0;url=/r\"></noscript></body></html>");
    tb.finish();
    assert!(
        noscript_meta_refresh(&tb.into_dom()).is_none(),
        "page riche : la méta noscript reste ignorée"
    );

    // Texte non blanc hors repli → pas un fallback pur.
    let mut tb = TreeBuilder::new();
    tb.set_scripting(true);
    tb.push("<html><body>bonjour<noscript><meta http-equiv=refresh content=\"0;url=/r\"></noscript></body></html>");
    tb.finish();
    assert!(noscript_meta_refresh(&tb.into_dom()).is_none());
}

/// Vol 32 dropbox — `<link rel="preload" as="style">` : le
/// swap JS `onload→rel=stylesheet` meurt quand le JS l'est ;
/// la feuille préchargée est collectée comme un stylesheet
/// (repli produit — un `as=style` existe pour être appliqué).
/// `as=script` et `preload` nu restent hors collecte.
#[test]
fn collect_stylesheet_links_preload_as_style() {
    let dom = test_dom(
        r#"<link rel=preload as=style href="/a.css"><link rel=preload as=script href="/b.js"><link rel=stylesheet href="/c.css"><link rel="stylesheet preload" as=style href="/d.css">"#,
    );
    let hrefs: Vec<String> = collect_stylesheet_links(&dom)
        .into_iter()
        .map(|(h, ..)| h)
        .collect();
    assert_eq!(hrefs, vec!["/a.css", "/c.css", "/d.css"]);
}

/// B-4 data.gouv — `<link rel=modulepreload>` et
/// `rel=preload as=script|module` sèment le prefetch des
/// graphes de modules (chunks Nuxt hors `import`
/// statique). `preload` nu et `as=style|font|image`
/// restent hors collecte — pas des modules.
#[test]
fn collect_module_preloads_filters_as() {
    let dom = test_dom(
        r#"<link rel=modulepreload href="/a.js"><link rel=preload as=script href="/b.js"><link rel=preload as=module href="/c.mjs"><link rel=preload as=style href="/d.css"><link rel=preload as=font href="/e.woff2"><link rel=preload href="/f.js"><link rel=stylesheet href="/g.css">"#,
    );
    assert_eq!(
        collect_module_preloads(&dom),
        vec!["/a.js", "/b.js", "/c.mjs"]
    );
}

/// Vol 13 lemonde/bbc — les `<img>` à `data-src` (lazy JS
/// mort) n'étaient jamais fetchées : repli produit sur les
/// attributs lazy usuels quand ni `srcset` ni `src` n'est
/// posé. `src`/`srcset` gagnent s'ils existent.
#[test]
fn collect_images_falls_back_to_lazy_data_src() {
    let dom = test_dom("<img data-src=\"/a.png\"><img src=\"/b.png\" data-src=\"/c.png\"><img srcset=\"/d.png 1x\" data-src=\"/e.png\"><img>");
    let srcs: Vec<String> = collect_images(&dom, 800.0, 1.0)
        .into_iter()
        .map(|(_, s)| s)
        .collect();
    assert_eq!(srcs, vec!["/a.png", "/b.png", "/d.png"]);
}

/// §B-20 marmiton — `data-srcset` porte une **syntaxe
/// srcset** (`url 1x, url2 2x`), pas une URL : rangée dans
/// `LAZY_SRCS` brute, `Url::resolve` la rejetait et l'image
/// ne partait jamais. Elle passe par `pick_srcset` comme
/// `srcset` (sélection `w`/`x` + `sizes`), et prime
/// `data-src` de même préséance que `srcset` sur `src`.
#[test]
fn collect_images_parses_lazy_data_srcset() {
    let dom = test_dom(
        r#"<img data-srcset="/w46.webp 1x, /w92.webp 2x" data-src="/fallback.png">
           <img data-srcset="/w200.webp 200w, /w800.webp 800w" sizes="200px">
           <img data-srcset="/a.webp 1x" data-src="/b.webp">
           <img srcset="/s1.png 1x, /s2.png 2x" data-srcset="/never.webp 1x">"#,
    );
    let srcs: Vec<String> = collect_images(&dom, 800.0, 1.0)
        .into_iter()
        .map(|(_, s)| s)
        .collect();
    // dpr=1 → candidat `1x` ; `sizes=200px` → `200w` à
    // densité 1. `srcset` réel toujours premier.
    assert_eq!(srcs, vec!["/w46.webp", "/w200.webp", "/a.webp", "/s1.png"]);
    // dpr=2 → le candidat `2x` l'emporte sur `data-src`.
    let dom =
        test_dom(r#"<img data-srcset="/w46.webp 1x, /w92.webp 2x" data-src="/fallback.png">"#);
    let srcs: Vec<String> = collect_images(&dom, 800.0, 2.0)
        .into_iter()
        .map(|(_, s)| s)
        .collect();
    assert_eq!(srcs, vec!["/w92.webp"]);
}

/// Vol 13 franceinfo — un `Scroll` pendant la peinture
/// détachée (apply hors verrou) est **accepté** : le
/// `scroll_y` rapporté projette les deltas en file, le
/// drain rejoue ensuite le scroll réel au lieu d'un
/// `Ignored` abandonné par le client.
#[tokio::test]
async fn deferred_scroll_reports_projected_position() {
    let mut s = Session::new(Arc::new(FailTransport), Arc::new(TestFont));
    let tab = s.open_tab();
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.doc_h = 10_000.0;
        e.scroll_y = 100.0;
        e.paint_detached = true;
    }
    let out = s
        .handle_input(tab, InputEvent::Scroll { dy: 900.0 })
        .await
        .unwrap();
    let InputOutcome::Scrolled { scroll_y } = out else {
        panic!("scroll différé refusé : {out:?}");
    };
    assert_eq!(scroll_y, 1000.0, "projection = position + file");
    // Le replay effectif applique le delta enfilé — DOM +
    // frame factices posés pour court-circuiter le
    // lazy-heal (un relayout écraserait `doc_h`).
    {
        let e = s.tabs.get_mut(&tab).unwrap();
        e.paint_detached = false;
        e.dom = Some(test_dom("<p>x</p>"));
        e.frame = Some(Frame {
            width: 8,
            height: 8,
            pixels: vec![0; 8 * 8 * 4],
            scale: 1.0,
        });
    }
    s.drain_deferred_inputs(tab);
    assert_eq!(s.tabs.get(&tab).unwrap().scroll_y, 1000.0);
}

/// Vol 23 DDG `/html` — `noscript` **dans le head** avec
/// body riche : le redirect no-JS est honoré quand même
/// (pattern d'entrée de gamme, indépendant du contenu).
#[test]
fn noscript_head_meta_refresh_honored_on_rich_page() {
    let mut tb = TreeBuilder::new();
    tb.set_scripting(true);
    tb.push("<html><head><noscript><meta http-equiv=refresh content=\"0;url=/?q=x\"></noscript></head><body><form>q</form><p>résultats</p></body></html>");
    tb.finish();
    let r = noscript_meta_refresh(&tb.into_dom());
    assert_eq!(r.and_then(|(_, u)| u).as_deref(), Some("/?q=x"));
}

/// Vol 25 (kayak) — signature du disjoncteur « moteur
/// figé » : `Dead` et `Timeout` (kind typé — JS-C156)
/// arment ; la chaîne héritée du proxy reste acceptée en
/// repli ; une vraie exception JS n'arme pas.
#[test]
fn engine_stall_signature() {
    assert!(Session::is_engine_stall(&explorer_js::JsError::Dead));
    assert!(Session::is_engine_stall(&explorer_js::JsError::Timeout));
    assert!(Session::is_engine_stall(&explorer_js::JsError::Uncaught(
        "moteur figé — éval dépassée".into()
    )));
    assert!(!Session::is_engine_stall(&explorer_js::JsError::Uncaught(
        "TypeError: x is not a function".into()
    )));
}

/// Étape 151 / §B-54 (flixbus vol 93) — une passe `poll_net`
/// qui dépasse `poll_net_budget` reporte les `NetDone`
/// restants en `pending_net_done` (ordre conservé, relivrés
/// en tête à la passe suivante) au lieu de tenir le verrou
/// session sur une rafale de livraisons multi-secondes.
#[test]
fn poll_net_budget_defere_les_livraisons_excedentaires() {
    let mut s = Session::with_config(
        Arc::new(FailTransport),
        Arc::new(TestFont),
        SessionConfig::default(),
    );
    let tab = s.open_tab();
    let mk = |handle: u64| NetDone {
        tab,
        handle,
        cross: false,
        mode: explorer_net::http::FetchMode::Cors,
        credentials: explorer_net::http::FetchCredentials::Omit,
        page_origin: "https://x.test".to_string(),
        req_url: Url::parse("https://x.test/r").expect("url"),
        partition: None,
        out: Err(explorer_net::error::NetError::InvalidResponse(
            "t".to_string(),
        )),
        script: None,
        frame: None,
        img: None,
        link: None,
        dyn_module: None,
        import: None,
    };
    // Passe déjà hors budget → tout est reporté, ordre intact.
    s.drain_net_done(
        vec![mk(0), mk(1), mk(2)],
        std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(60))
            .expect("instant passé"),
    );
    assert_eq!(s.pending_net_done.len(), 3);
    assert_eq!(s.pending_net_done.front().map(|d| d.handle), Some(0));
    assert_eq!(s.pending_net_done.back().map(|d| d.handle), Some(2));
    // La passe suivante (budget frais) relivre la file en
    // tête — avant toute nouvelle arrivée.
    s.net_tx.send(mk(3)).expect("send");
    let n = s.poll_net();
    assert_eq!(n, 4);
    assert!(s.pending_net_done.is_empty());
}

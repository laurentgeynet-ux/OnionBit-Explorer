// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests des valeurs typées, expansion des shorthands et
//! pseudo-classes dynamiques (étape 16).

use explorer_css::value::{expand_shorthand, parse_color, parse_length, Color, Unit};
use explorer_css::{matches_with_state, parse_selector_list, Origin, StyleEngine, StyleEnv};
use explorer_dom::{Dom, NodeId};

// ── longueurs ────────────────────────────────────────────────

#[test]
fn longueurs_absolues_et_relatives() {
    let l = parse_length("12px").unwrap();
    assert_eq!((l.value, l.unit), (12.0, Unit::Px));
    assert_eq!(parse_length("1.5em").unwrap().unit, Unit::Em);
    assert_eq!(parse_length("2rem").unwrap().unit, Unit::Rem);
    assert_eq!(parse_length("-3pt").unwrap().value, -3.0);
    assert_eq!(parse_length("0").unwrap().value, 0.0);
    assert_eq!(parse_length("2in").unwrap().unit, Unit::In);
    // Facteurs absolus → px.
    assert_eq!(Unit::In.px_factor(), Some(96.0));
    assert_eq!(Unit::Pt.px_factor(), Some(96.0 / 72.0));
    assert_eq!(Unit::Pc.px_factor(), Some(16.0));
    assert_eq!(Unit::Em.px_factor(), None); // relative : used value
}

#[test]
fn longueurs_invalides() {
    for bad in ["abc", "10xx", "auto", "medium", "", "px", "--"] {
        assert!(parse_length(bad).is_none(), "attendu None : {bad}");
    }
}

// ── couleurs ─────────────────────────────────────────────────

#[test]
fn couleurs_hex_rgb_nommees() {
    assert_eq!(
        parse_color("#f00"),
        Some(Color {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        })
    );
    assert_eq!(
        parse_color("#00ff00"),
        Some(Color {
            r: 0,
            g: 255,
            b: 0,
            a: 255
        })
    );
    assert_eq!(parse_color("#0000ff80").unwrap().a, 128);
    assert_eq!(parse_color("rgb(255, 0, 0)").unwrap().r, 255);
    // 0.5 * 255 = 127.5 → arrondi à 128.
    assert_eq!(parse_color("rgba(0, 0, 0, 0.5)").unwrap().a, 128);
    assert_eq!(parse_color("rgb(100%, 0%, 0%)").unwrap().r, 255);
    assert_eq!(parse_color("red").unwrap().r, 255);
    assert_eq!(parse_color("transparent"), Some(Color::TRANSPARENT));
    for bad in ["#", "#12", "#xyz", "rgb(1,2)", "mauve?", ""] {
        assert!(parse_color(bad).is_none(), "attendu None : {bad}");
    }
}

// ── shorthands ────────────────────────────────────────────────

#[test]
fn shorthand_margin_quatre_cotes() {
    let e = expand_shorthand("margin", "1px").unwrap();
    assert_eq!(
        e,
        vec![
            ("margin-top".to_string(), "1px".to_string()),
            ("margin-right".to_string(), "1px".to_string()),
            ("margin-bottom".to_string(), "1px".to_string()),
            ("margin-left".to_string(), "1px".to_string()),
        ]
    );
    let e = expand_shorthand("margin", "1px 2px").unwrap();
    assert_eq!(e[0].1, "1px"); // top
    assert_eq!(e[1].1, "2px"); // right
    assert_eq!(e[2].1, "1px"); // bottom
    assert_eq!(e[3].1, "2px"); // left
    let e = expand_shorthand("padding", "1px 2px 3px").unwrap();
    assert_eq!(e[2].1, "3px"); // bottom
    assert_eq!(e[3].1, "2px"); // left = right
    let e = expand_shorthand("margin", "1px 2px 3px 4px").unwrap();
    assert_eq!(e[3].1, "4px");
    assert!(expand_shorthand("margin", "1px 2px 3px 4px 5px").is_none());
}

#[test]
fn shorthand_border() {
    let e = expand_shorthand("border", "2px solid red").unwrap();
    assert!(e.contains(&("border-top-width".into(), "2px".into())));
    assert!(e.contains(&("border-top-style".into(), "solid".into())));
    assert!(e.contains(&("border-top-color".into(), "red".into())));
    assert!(e.contains(&("border-left-color".into(), "red".into())));
    // Ordre libre.
    let e = expand_shorthand("border", "red solid 2px").unwrap();
    assert_eq!(e.len(), 12);
    // Une seule composante.
    let e = expand_shorthand("border", "solid").unwrap();
    assert_eq!(e.len(), 4);
    assert!(e.iter().all(|(n, _)| n.ends_with("-style")));
    // border-width 4 côtés.
    let e = expand_shorthand("border-width", "1px 2px").unwrap();
    assert_eq!(e.len(), 4);
    assert_eq!(e[1].1, "2px");
}

#[test]
fn shorthand_font_et_overflow() {
    let e = expand_shorthand("font", "italic bold 12px/1.5 serif").unwrap();
    assert!(e.contains(&("font-style".into(), "italic".into())));
    assert!(e.contains(&("font-weight".into(), "bold".into())));
    assert!(e.contains(&("font-size".into(), "12px".into())));
    assert!(e.contains(&("line-height".into(), "1.5".into())));
    assert!(e.contains(&("font-family".into(), "serif".into())));
    let e = expand_shorthand("font", "1em monospace").unwrap();
    assert!(e.contains(&("font-size".into(), "1em".into())));
    assert!(e.contains(&("font-family".into(), "monospace".into())));
    let e = expand_shorthand("overflow", "hidden auto").unwrap();
    assert_eq!(
        e,
        vec![
            ("overflow-x".into(), "hidden".into()),
            ("overflow-y".into(), "auto".into())
        ]
    );
}

// ── cascade + shorthands + valeurs typées ────────────────────

fn fixture() -> (Dom, NodeId) {
    let mut dom = Dom::new();
    let doc = dom.document();
    let div = dom.create_element("div").unwrap();
    dom.append_child(doc, div).unwrap();
    (dom, div)
}

#[test]
fn shorthand_en_cascade() {
    let (dom, div) = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "div { margin: 5px 10px; border: 1px solid red }",
        Origin::Author,
    );
    let s = e.compute(&dom, div, &StyleEnv::default());
    assert_eq!(s.get("margin-top"), Some("5px"));
    assert_eq!(s.get("margin-right"), Some("10px"));
    assert_eq!(s.get("border-top-width"), Some("1px"));
    assert_eq!(s.get("border-top-style"), Some("solid"));
    assert_eq!(s.get("border-top-color"), Some("red"));
    // Le longhand gagne s'il arrive après (même spécificité,
    // ordre source).
    e.add_css("div { margin-top: 9px }", Origin::Author);
    assert_eq!(
        e.compute(&dom, div, &StyleEnv::default()).get("margin-top"),
        Some("9px")
    );
}

#[test]
fn accesseurs_types() {
    let (dom, div) = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "div { margin-top: 2.5em; color: #f00; border-top-color: currentcolor; z-index: 7 }",
        Origin::Author,
    );
    let s = e.compute(&dom, div, &StyleEnv::default());
    let l = s.length("margin-top").unwrap();
    assert_eq!((l.value, l.unit), (2.5, Unit::Em));
    assert_eq!(s.color("color").unwrap().r, 255);
    // currentcolor → la couleur de l'élément.
    assert_eq!(s.color("border-top-color").unwrap().r, 255);
    assert_eq!(s.number("z-index"), Some(7.0));
}

// ── pseudo-classes dynamiques ────────────────────────────────

#[test]
fn dynamiques_selon_etat() {
    let (dom, div) = fixture();
    let mut e = StyleEngine::new();
    e.add_css(
        "div:hover { color: red } div { color: blue }",
        Origin::Author,
    );
    // Sans état → :hover faux → blue.
    let env = StyleEnv::default();
    assert_eq!(e.compute(&dom, div, &env).get("color"), Some("blue"));
    // État hover actif → red.
    let s = e.compute_with_state(&dom, div, &env, &|name| name == "hover");
    assert_eq!(s.get("color"), Some("red"));
    // Autre état → toujours blue.
    let s = e.compute_with_state(&dom, div, &env, &|name| name == "focus");
    assert_eq!(s.get("color"), Some("blue"));
}

#[test]
fn link_structurel() {
    let mut dom = Dom::new();
    let doc = dom.document();
    let a = dom.create_element("a").unwrap();
    dom.set_attribute(a, "href", "x").unwrap();
    dom.append_child(doc, a).unwrap();
    let b = dom.create_element("a").unwrap();
    dom.append_child(doc, b).unwrap();
    let sels = parse_selector_list("a:link").unwrap();
    // :link matche a[href] même sans état (structurel).
    assert!(matches_with_state(&dom, a, &sels[0], &|_| false));
    assert!(!matches_with_state(&dom, b, &sels[0], &|_| false));
    // :visited jamais (pas de tracking).
    let sels = parse_selector_list("a:visited").unwrap();
    assert!(!matches_with_state(&dom, a, &sels[0], &|_| true));
}

#[test]
fn shorthand_background_size_et_gradient() {
    // Issue #10 — `pos / size` ne doit plus dropper la
    // déclaration : couleur, position, repeat ET size émis.
    let e = expand_shorthand("background", "red 50% / contain no-repeat").unwrap();
    let get = |n: &str| e.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
    assert_eq!(get("background-color"), Some("red"));
    assert_eq!(get("background-position"), Some("50%"));
    assert_eq!(get("background-size"), Some("contain"));
    assert_eq!(get("background-repeat"), Some("no-repeat"));
    assert_eq!(get("background-image"), Some("none"));

    // `/` collé ou séparé, size à 2 valeurs.
    let e = expand_shorthand("background", "url(a.png) center/cover").unwrap();
    let get = |n: &str| e.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
    assert_eq!(get("background-image"), Some("url(a.png)"));
    assert_eq!(get("background-position"), Some("center"));
    assert_eq!(get("background-size"), Some("cover"));

    // Fonction image : le gradient, espaces internes compris,
    // est UN atome du shorthand.
    let e = expand_shorthand(
        "background",
        "linear-gradient(to right, red, blue) no-repeat",
    )
    .unwrap();
    let get = |n: &str| e.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
    assert_eq!(
        get("background-image"),
        Some("linear-gradient(to right, red, blue)")
    );
    assert_eq!(get("background-repeat"), Some("no-repeat"));
    assert_eq!(get("background-size"), Some("auto"));

    // Composantes après la taille : `repeat` ressort du mode
    // size, la déclaration survit.
    let e = expand_shorthand("background", "url(a.png) center / cover no-repeat fixed").unwrap();
    let get = |n: &str| e.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
    assert_eq!(get("background-size"), Some("cover"));
    assert_eq!(get("background-repeat"), Some("no-repeat"));
    assert_eq!(get("background-attachment"), Some("fixed"));

    // Rejets préservés : doublon de couleur, token inconnu,
    // taille non déclarée derrière `/`, trop de segments.
    assert!(expand_shorthand("background", "red pink").is_none());
    assert!(expand_shorthand("background", "frobnicate").is_none());
    assert!(expand_shorthand("background", "url(a.png) cover").is_none());
    assert!(expand_shorthand("background", "url(a.png) /cover/x/y").is_none());
}

#[test]
fn shorthand_columns() {
    // R-89 : `columns` = `[width] [count]` ordre libre —
    // l'accueil FR n'utilise que les longhands mais le
    // raccourci couvre la forme compacte.
    let e = expand_shorthand("columns", "10em 3").unwrap();
    assert_eq!(
        e,
        vec![
            ("column-width".to_string(), "10em".to_string()),
            ("column-count".to_string(), "3".to_string()),
        ]
    );
    let e = expand_shorthand("columns", "auto 2").unwrap();
    assert_eq!(e[1].1, "2");
    assert_eq!(e[0].1, "auto");
}

// ── R-107 : expansion `background` multi-couches ────────────

#[test]
fn background_shorthand_fusionne_les_couches() {
    // `background: url(a) no-repeat, linear-gradient(r,b)` :
    // les longhands sortent jointes par couche ; la couleur ne
    // vient que de la dernière couche (spec §3.10).
    let v = expand_shorthand(
        "background",
        "url(a.png) no-repeat, linear-gradient(red, blue)",
    )
    .expect("couches valides");
    let get = |n: &str| v.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
    assert_eq!(
        get("background-image"),
        Some("url(a.png), linear-gradient(red, blue)")
    );
    assert_eq!(get("background-repeat"), Some("no-repeat, repeat"));
    // Couleur sur la couche non finale → déclaration invalide.
    assert!(expand_shorthand("background", "red, url(a.png)").is_none());
    // Couleur sur la dernière couche → OK.
    let v = expand_shorthand("background", "url(a.png), red").expect("couleur finale");
    assert!(v.iter().any(|(n, v)| n == "background-color" && v == "red"));
}

#[test]
fn oklch_lab_lch_hsl_parse() {
    // R-115 — css-color-4 : oklch/oklab/lab/lch/hsl parsés.
    assert_eq!(parse_color("hsl(240 100% 50%)").unwrap().b, 255);
    assert_eq!(
        parse_color("hsl(0, 100%, 50%)"),
        Some(Color {
            r: 255,
            g: 0,
            b: 0,
            a: 255
        })
    );
    assert!(parse_color("oklch(0.5 0.1 180)").is_some());
    assert!(parse_color("oklab(0.5 0.1 -0.1)").is_some());
    assert!(parse_color("lab(50% 40 -30)").is_some());
    assert!(parse_color("lch(50% 40 240)").is_some());
}

/// Vol 2026-10-26 — `inset` s'éclate en `top/right/bottom/
/// left` (liens overlay `inset:0` du Guardian à boîtes 0x0).
#[test]
fn inset_expansion() {
    let got = expand_shorthand("inset", "0").expect("inset:0");
    assert_eq!(
        got,
        vec![
            ("top".to_string(), "0".to_string()),
            ("right".to_string(), "0".to_string()),
            ("bottom".to_string(), "0".to_string()),
            ("left".to_string(), "0".to_string()),
        ]
    );
    let got = expand_shorthand("inset", "10px 20px").expect("inset 2v");
    assert_eq!(got[0].1, "10px");
    assert_eq!(got[1].1, "20px");
    assert_eq!(got[2].1, "10px");
    assert_eq!(got[3].1, "20px");
}

#[test]
fn expand_proprietes_logiques_ltr() {
    // R-124/R-183 — les longhands passent verbatim (`None`),
    // les shorthands éclatent en noms LOGIQUES : le mapping
    // vers le repère de flux attend la cascade
    // (`resolve_logical_props`, verrou `writing_mode_resout_
    // les_props_logiques` dans cascade.rs).
    let g = |n, v| expand_shorthand(n, v).unwrap();
    assert_eq!(expand_shorthand("padding-inline-start", "8px"), None);
    assert_eq!(expand_shorthand("margin-inline-end", "4px"), None);
    assert_eq!(expand_shorthand("inset-inline-start", "0"), None);
    assert_eq!(expand_shorthand("block-size", "100px"), None);
    // Shorthands `-inline`/`-block` : 1 valeur répétée ou 2.
    assert_eq!(
        g("margin-inline", "10px 20px"),
        vec![
            ("margin-inline-start".into(), "10px".into()),
            ("margin-inline-end".into(), "20px".into())
        ]
    );
    assert_eq!(
        g("padding-block", "6px"),
        vec![
            ("padding-block-start".into(), "6px".into()),
            ("padding-block-end".into(), "6px".into())
        ]
    );
    assert_eq!(
        g("border-inline", "1px solid red"),
        vec![
            ("border-inline-start-width".into(), "1px".into()),
            ("border-inline-start-style".into(), "solid".into()),
            ("border-inline-start-color".into(), "red".into()),
            ("border-inline-end-width".into(), "1px".into()),
            ("border-inline-end-style".into(), "solid".into()),
            ("border-inline-end-color".into(), "red".into())
        ]
    );
}

#[test]
fn expand_place_shorthands() {
    // R-125 — place-items/content/self (css-align-3).
    let g = |n, v| expand_shorthand(n, v).unwrap();
    assert_eq!(
        g("place-items", "center"),
        vec![
            ("align-items".into(), "center".into()),
            ("justify-items".into(), "center".into())
        ]
    );
    assert_eq!(
        g("place-self", "end start"),
        vec![
            ("align-self".into(), "end".into()),
            ("justify-self".into(), "start".into())
        ]
    );
    assert_eq!(
        g("place-content", "space-between"),
        vec![
            ("align-content".into(), "space-between".into()),
            ("justify-content".into(), "space-between".into())
        ]
    );
}

#[test]
fn unites_viewport_dynamiques_dvh() {
    // B-28 (uMap) : `100dvh`/`100svh`/`100lvh` doivent
    // résoudre comme `100vh` — sans chrome mobile les bases
    // coïncident (css-values-4 §6.1.3).
    for u in ["dvh", "svh", "lvh", "dvw", "svw", "lvw"] {
        let l = parse_length(&format!("100{u}")).expect(u);
        let expected = if u.ends_with('w') { Unit::Vw } else { Unit::Vh };
        assert_eq!(l.unit, expected, "{u}");
    }
    for u in ["dvmin", "svmin", "lvmin", "dvmax", "svmax", "lvmax"] {
        let l = parse_length(&format!("100{u}")).expect(u);
        let expected = if u.ends_with("min") {
            Unit::Vmin
        } else {
            Unit::Vmax
        };
        assert_eq!(l.unit, expected, "{u}");
    }
}

/// R-208 — les shorthands multi-valeurs acceptent les fonctions
/// avec espaces internes (`calc()`/`var()`/`min()`/`max()`) : le
/// split whitespace se fait à profondeur de parenthèses 0.
/// Avant : `padding:calc(0.25rem * 4)` était éclaté en morceaux
/// invalides et la déclaration entière sautait (Tailwind v4).
#[test]
fn r208_shorthand_fonctions_avec_espaces_internes() {
    let e = expand_shorthand("padding", "calc(0.25rem * 4)").unwrap();
    assert_eq!(e.len(), 4);
    assert_eq!(
        e[0],
        ("padding-top".to_string(), "calc(0.25rem * 4)".to_string())
    );
    assert_eq!(
        e[3],
        ("padding-left".to_string(), "calc(0.25rem * 4)".to_string())
    );
    // Deux valeurs dont une fonction.
    let e = expand_shorthand("margin", "calc(1px + 2px) auto").unwrap();
    assert_eq!(e.len(), 4);
    assert_eq!(e[0].1, "calc(1px + 2px)");
    assert_eq!(e[1].1, "auto");
    assert_eq!(e[2].1, "calc(1px + 2px)");
    // inset + var non résolu au verbatim.
    let e = expand_shorthand("inset", "calc(var(--gap) * 2) 0").unwrap();
    assert_eq!(
        e[0],
        ("top".to_string(), "calc(var(--gap) * 2)".to_string())
    );
    assert_eq!(e[1], ("right".to_string(), "0".to_string()));
}

/// R-214 — `color-mix(in srgb, …)` et syntaxe relative
/// `oklch(from <couleur> …)` (css-color-5) : le fond
/// `bg-brand-50/50` de plyr.io est `color-mix(in srgb,
/// oklch(from var(--brand) l c h) 50%, transparent)` — les
/// deux formes doivent se résoudre (vol 771).
#[test]
fn color_mix_et_oklch_relatif() {
    // color-mix : blanc 50% + transparent(noir,a=0) → rgba≈(128,a≈0.5).
    let c = parse_color("color-mix(in srgb, #ffffff 50%, transparent)").unwrap();
    assert!(
        (120..=135).contains(&c.r) && c.a > 100 && c.a < 160,
        "mix 50/50 : {c:?}"
    );
    // Sans % → 50/50.
    let c = parse_color("color-mix(in srgb, #000, #fff)").unwrap();
    assert!((110..=145).contains(&c.r), "mix défaut : {c:?}");
    // Relatif : `oklch(from <src> l c h)` = source elle-même.
    let src = parse_color("oklch(0.7 0.15 250)").unwrap();
    let rel = parse_color("oklch(from oklch(0.7 0.15 250) l c h)").unwrap();
    assert!(
        src.r.abs_diff(rel.r) <= 4 && src.g.abs_diff(rel.g) <= 4 && src.b.abs_diff(rel.b) <= 4,
        "relatif ≈ source : {src:?} vs {rel:?}"
    );
    // Canal modifié par calc : `calc(l * 0.5)` assombrit.
    let dark = parse_color("oklch(from #ffffff calc(l * 0.5) c h)").unwrap();
    assert!(dark.r < 150, "l*0.5 assombrit : {dark:?}");
    // Chaîne plyr complète : mix(oklch(from …) 50%, transparent).
    let plyr =
        parse_color("color-mix(in srgb, oklch(from oklch(0.9 0.05 250) l c h) 50%, transparent)")
            .unwrap();
    assert!(plyr.a > 100 && plyr.a < 160, "plyr bg : {plyr:?}");
}

/// R-252 — shorthand `mask` (css-masking-1) : couche complète
/// `image position/size repeat composite mode box×2` éclatée
/// en longhands standards ; `-webkit-mask` alias.
#[test]
fn mask_shorthand_expande() {
    let e = expand_shorthand(
        "mask",
        "url(icon.svg) center / cover no-repeat padding-box content-box subtract luminance",
    )
    .unwrap();
    let get = |n: &str| e.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
    assert_eq!(get("mask-image"), Some("url(icon.svg)"));
    assert_eq!(get("mask-position"), Some("center"));
    assert_eq!(get("mask-size"), Some("cover"));
    assert_eq!(get("mask-repeat"), Some("no-repeat"));
    // Deux boxes : origin puis clip.
    assert_eq!(get("mask-origin"), Some("padding-box"));
    assert_eq!(get("mask-clip"), Some("content-box"));
    assert_eq!(get("mask-composite"), Some("subtract"));
    assert_eq!(get("mask-mode"), Some("luminance"));
}

/// `-webkit-mask` minimal : image seule → longhands
/// standards aux initiales ailleurs (le shorthand
/// réinitialise tout, comme `background`).
#[test]
fn webkit_mask_image_seule() {
    let e = expand_shorthand("-webkit-mask", "url(i.svg)").unwrap();
    let get = |n: &str| e.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
    assert_eq!(get("mask-image"), Some("url(i.svg)"));
    assert_eq!(get("mask-repeat"), Some("repeat"));
    assert_eq!(get("mask-mode"), Some("match-source"));
}

/// Multi-couches : longhands joints par `, ` dans l'ordre.
#[test]
fn mask_multi_couches() {
    let e = expand_shorthand("mask", "url(a), url(b) 50%").unwrap();
    let get = |n: &str| e.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
    assert_eq!(get("mask-image"), Some("url(a), url(b)"));
    assert_eq!(get("mask-position"), Some("0% 0%, 50%"));
}

/// Mot-clé CSS-wide sur un shorthand (css-cascade-5
/// §defaulting) : chaque longhand de l'expansion reçoit le
/// mot-clé — les grammaires strictes le droppaient
/// (jauge `affectsv` : `outline:inherit` & co).
#[test]
fn css_wide_sur_shorthands() {
    for (sh, n) in [
        ("outline", 3),
        ("column-rule", 3),
        ("flex-flow", 2),
        ("margin", 4),
        ("place-self", 2),
        ("gap", 2),
        ("border-top", 3),
    ] {
        let e = expand_shorthand(sh, "inherit").unwrap_or_else(|| panic!("{sh} expansion"));
        assert_eq!(e.len(), n, "{sh} : {n} longhands attendus");
        assert!(
            e.iter().all(|(_, v)| v == "inherit"),
            "{sh} : chaque longhand = inherit ({e:?})"
        );
    }
}

/// R-258 — `font-size` : mots-clés css-fonts-4 `xxx-large` +
/// `math` acceptés à la validation (table stoppée à `xx-large`
/// → `value.dropped.font-size` terrain).
#[test]
fn font_size_mots_cles_css4() {
    use explorer_css::value::value_is_valid_for;
    for v in ["xxx-large", "math", "smaller", "larger", "12px", "150%"] {
        assert!(value_is_valid_for("font-size", v), "{v} refusé");
    }
}

// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! R-303 — `CounterStyle::label`/`resolve` : descripteurs
//! css-counter-styles-3 portés (`additive-symbols`,
//! `negative`, `pad`, `range`, `fallback` chaîné,
//! `system: fixed <N>`), bornes spec (60 codepoints),
//! systèmes corrigés (`numeric` base-N, `symbolic`
//! répétitif, `cyclic` modulaire négatifs inclus).

use std::collections::HashMap;

use explorer_css::{CounterStyle, Origin, StyleEngine};

/// Enregistre les `@counter-style` d'une feuille et rend
/// le registre — la feuille est la forme réelle de
/// déclaration (parsing des descripteurs inclus).
fn registry(css: &str) -> HashMap<String, CounterStyle> {
    let mut eng = StyleEngine::new();
    eng.add_css(css, Origin::Author);
    eng.counter_styles.clone()
}

fn style(css: &str, name: &str) -> CounterStyle {
    registry(css)
        .get(name)
        .unwrap_or_else(|| panic!("style {name} enregistré"))
        .clone()
}

#[test]
fn additive_glouton_spec() {
    // Exemple spec §3.1.6 — dé en poids décroissants.
    let cs = style(
        "@counter-style d{system:additive;additive-symbols:6 \"a6\",5 \"a5\",4 \"a4\",3 \"a3\",2 \"a2\",1 \"a1\"}",
        "d",
    );
    assert_eq!(cs.label(1), "a1.");
    assert_eq!(cs.label(6), "a6.");
    assert_eq!(cs.label(11), "a6a5.");
    assert_eq!(cs.label(13), "a6a6a1.");
}

#[test]
fn additive_poids_zero_et_echec_repli() {
    // Poids 0 requis pour représenter 0 ; sans lui → repli.
    let cs = style(
        "@counter-style d{system:additive;additive-symbols:1 \"x\",0 \"o\"}",
        "d",
    );
    assert_eq!(cs.label(0), "o.");
    assert_eq!(cs.label(3), "xxx.");
    // Décomposition impossible (reste 1 sans poids) →
    // repli decimal (spec : l'algo glouton peut échouer
    // même quand une représentation existe).
    let cs = style(
        "@counter-style d{system:additive;additive-symbols:3 \"a\",2 \"b\"}",
        "d",
    );
    assert_eq!(cs.label(4), "4.");
}

#[test]
fn additive_ordre_invalide_ignore() {
    // Poids non strictement décroissants → descripteur
    // invalide entier (spec §3.8) → irreprésentable → repli.
    let cs = style(
        "@counter-style d{system:additive;additive-symbols:1 \"a\",2 \"b\"}",
        "d",
    );
    assert_eq!(cs.label(1), "1.");
}

#[test]
fn numeric_base_n() {
    // Spec : ≥2 symboles = chiffres positionnels base-N.
    let cs = style(
        "@counter-style bin{system:numeric;symbols:\"0\" \"1\"}",
        "bin",
    );
    assert_eq!(cs.label(0), "0.");
    assert_eq!(cs.label(5), "101.");
    let cs = style(
        "@counter-style d{system:numeric;symbols:\"a\" \"b\" \"c\" \"d\"}",
        "d",
    );
    // Base-4 : 1→b, 4→ba (10 en base 4).
    assert_eq!(cs.label(4), "ba.");
    // <2 symboles → tolérance chiffres décimaux (règle
    // spec-invalide, dégradé honnête).
    let cs = style("@counter-style d{system:numeric}", "d");
    assert_eq!(cs.label(42), "42.");
}

#[test]
fn alphabetic_portee_et_bijectif() {
    let cs = style(
        "@counter-style a{system:alphabetic;symbols:\"x\" \"y\"}",
        "a",
    );
    assert_eq!(cs.label(1), "x.");
    assert_eq!(cs.label(2), "y.");
    // Numération bijective : 3 = "xx" (colonne Excel —
    // x=1, y=2, xx=3, xy=4, yx=5…).
    assert_eq!(cs.label(3), "xx.");
    assert_eq!(cs.label(5), "yx.");
    // Portée auto alphabetic = 1..∞ → 0 et négatifs au repli.
    assert_eq!(cs.label(0), "0.");
    assert_eq!(cs.label(-3), "-3.");
}

#[test]
fn symbolic_repete_le_symbole() {
    // Spec §3.1.3 : à chaque tour le symbole se répète —
    // "*" → *, **, *** (différent de cyclic).
    let cs = style("@counter-style s{system:symbolic;symbols:\"*\" \"†\"}", "s");
    assert_eq!(cs.label(1), "*.");
    assert_eq!(cs.label(2), "†.");
    assert_eq!(cs.label(3), "**.");
    assert_eq!(cs.label(4), "††.");
    assert_eq!(cs.label(5), "***.");
}

#[test]
fn cyclic_negatifs_modulaires() {
    // Cyclic couvre ±∞ — négatifs bouclent sans signe.
    let cs = style("@counter-style c{system:cyclic;symbols:\"◆\" \"■\"}", "c");
    assert_eq!(cs.label(-1), "◆.");
    assert_eq!(cs.label(0), "■.");
    assert_eq!(cs.label(1), "◆.");
}

#[test]
fn fixed_premier_symbole_et_repli() {
    let cs = style("@counter-style f{system:fixed 0;symbols:\"A\" \"B\"}", "f");
    assert_eq!(cs.label(0), "A.");
    assert_eq!(cs.label(1), "B.");
    // Épuisé → repli decimal.
    assert_eq!(cs.label(2), "2.");
    // Défaut : premier symbole = 1.
    let cs = style("@counter-style f{system:fixed;symbols:\"A\"}", "f");
    assert_eq!(cs.label(1), "A.");
    assert_eq!(cs.label(0), "0.");
}

#[test]
fn negative_enrobage_et_defaut() {
    let cs = style("@counter-style n{system:numeric;negative:\"(\" \")\"}", "n");
    assert_eq!(cs.label(-5), "(5).");
    assert_eq!(cs.label(5), "5.");
    // Un seul symbole = préfixe seul.
    let cs = style("@counter-style n{system:numeric;negative:\"!\"}", "n");
    assert_eq!(cs.label(-1), "!1.");
    // Défaut spec : `"-"` préfixe.
    let cs = style("@counter-style n{system:numeric}", "n");
    assert_eq!(cs.label(-7), "-7.");
    // `cyclic` n'utilise pas de signe → ignoré.
    let cs = style(
        "@counter-style n{system:cyclic;symbols:\"a\";negative:\"!\"}",
        "n",
    );
    assert_eq!(cs.label(-2), "a.");
}

#[test]
fn pad_longueur_cible() {
    let cs = style("@counter-style p{system:numeric;pad:3 \"0\"}", "p");
    assert_eq!(cs.label(5), "005.");
    assert_eq!(cs.label(42), "042.");
    // Déjà assez long → inchangé.
    assert_eq!(cs.label(1234), "1234.");
    // Le signe négatif compte dans la cible (spec §3.6) :
    // "-5" → différence 1 → "-05".
    assert_eq!(cs.label(-5), "-05.");
    let cs = style(
        "@counter-style p{system:numeric;pad:3 \"0\";negative:\"(\" \")\"}",
        "p",
    );
    // "(5" + ")" = 3 clusters → pas de padding → "(5)".
    assert_eq!(cs.label(-5), "(5).");
}

#[test]
fn range_union_et_invalide() {
    let cs = style(
        "@counter-style r{system:cyclic;symbols:\"a\" \"b\";range:2 3}",
        "r",
    );
    assert_eq!(cs.label(2), "b.");
    assert_eq!(cs.label(3), "a.");
    // Hors portée → repli decimal.
    assert_eq!(cs.label(1), "1.");
    assert_eq!(cs.label(4), "4.");
    // Union + `infinite`.
    let cs = style(
        "@counter-style r{system:cyclic;symbols:\"a\" \"b\";range:2 3,infinite 0}",
        "r",
    );
    assert_eq!(cs.label(-9), "a.");
    assert_eq!(cs.label(1), "1.");
    // Paire inversée → descripteur entier ignoré → portée
    // auto cyclic (±∞) → tous cyclent.
    let cs = style(
        "@counter-style r{system:cyclic;symbols:\"a\" \"b\";range:4 2}",
        "r",
    );
    assert_eq!(cs.label(4), "b.");
}

#[test]
fn fallback_predefini_et_prefixe_du_specifie() {
    // Le corps vient du repli, préfixe/suffixe du style
    // spécifié (spec §2.2, note explicite).
    let cs = style(
        "@counter-style t{system:fixed;symbols:\"A\";fallback:lower-roman;prefix:\"[\";suffix:\"]\"}",
        "t",
    );
    assert_eq!(cs.label(1), "[A]");
    assert_eq!(cs.label(4), "[iv]");
}

#[test]
fn fallback_nomme_chainage_et_cycles() {
    let reg = registry(
        "@counter-style a{system:fixed;symbols:\"A\";fallback:b}         @counter-style b{system:cyclic;symbols:\"x\" \"y\";suffix:\"!\"}",
    );
    let a = CounterStyle::resolve(&reg, "a").expect("a résolu");
    // Hors portée fixed → corps du style `b` (cyclic n=2 →
    // symbole 2) avec le suffixe de `a` (spécifié), pas le
    // `!` de `b`.
    assert_eq!(a.label(2), "y.");
    // Cycle a→b→a : la chaîne est coupée à la
    // construction → repli terminal decimal, jamais boucle.
    let reg = registry(
        "@counter-style a{system:fixed;symbols:\"A\";fallback:b}         @counter-style b{system:fixed;symbols:\"B\";fallback:a}",
    );
    let a = CounterStyle::resolve(&reg, "a").expect("a résolu");
    // n=2 : a épuisé → b épuisé → `a` déjà vu à la
    // résolution → chaîne coupée → builtin `a` inconnu →
    // decimal.
    assert_eq!(a.label(2), "2.");
}

#[test]
fn borne_60_codepoints_repli() {
    // symbolic n=100 → 100 symboles > 60 → repli (spec
    // §2.2 autorise le repli au-delà de 60 codepoints) —
    // borne budget contre les feuilles hostiles.
    let cs = style("@counter-style s{system:symbolic;symbols:\"*\"}", "s");
    assert_eq!(cs.label(59), "*".repeat(59) + ".");
    assert_eq!(cs.label(100), "100.");
    // Même borne côté pad : cible > 60 → repli.
    let cs = style("@counter-style p{system:numeric;pad:100 \"0\"}", "p");
    assert_eq!(cs.label(7), "7.");
}

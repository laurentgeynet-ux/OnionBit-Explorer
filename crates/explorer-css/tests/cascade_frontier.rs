// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Restyle de frontière (R-292) — `compute_scoped_frontier` :
//! couverture des ancêtres, frères précédents et suivants des
//! touchés ; élagage des sous-arbres prouvés stables ;
//! équivalence avec la cascade complète après `apply_scoped`.

use explorer_css::{Origin, StyleEngine, StyleEnv, StyleMap};
use explorer_dom::{Dom, NodeId};

fn env() -> StyleEnv {
    StyleEnv::default()
}

/// `body > div#host > div#mid > p#t`, plus une fratrie
/// `body > (div#f1, span#f2, section#f3 > p#f4)` pour les
/// tests de combinators et un sous-arbre profond intact.
struct Fixture {
    dom: Dom,
    host: NodeId,
    mid: NodeId,
    t: NodeId,
    f1: NodeId,
    f2: NodeId,
    f3: NodeId,
    f4: NodeId,
}

fn fixture() -> Fixture {
    let mut dom = Dom::new();
    let doc = dom.document();
    let body = dom.create_element("body").unwrap();
    dom.append_child(doc, body).unwrap();

    let host = dom.create_element("div").unwrap();
    dom.set_attribute(host, "id", "host").unwrap();
    dom.append_child(body, host).unwrap();
    let mid = dom.create_element("div").unwrap();
    dom.set_attribute(mid, "id", "mid").unwrap();
    dom.append_child(host, mid).unwrap();
    let t = dom.create_element("p").unwrap();
    dom.set_attribute(t, "id", "t").unwrap();
    dom.append_child(mid, t).unwrap();

    let f1 = dom.create_element("div").unwrap();
    dom.set_attribute(f1, "id", "f1").unwrap();
    dom.append_child(body, f1).unwrap();
    let f2 = dom.create_element("span").unwrap();
    dom.set_attribute(f2, "id", "f2").unwrap();
    dom.append_child(body, f2).unwrap();
    let f3 = dom.create_element("section").unwrap();
    dom.set_attribute(f3, "id", "f3").unwrap();
    dom.append_child(body, f3).unwrap();
    let f4 = dom.create_element("p").unwrap();
    dom.set_attribute(f4, "id", "f4").unwrap();
    dom.append_child(f3, f4).unwrap();

    Fixture {
        dom,
        host,
        mid,
        t,
        f1,
        f2,
        f3,
        f4,
    }
}

fn eng(css: &str) -> StyleEngine {
    let mut e = StyleEngine::new();
    if !css.is_empty() {
        e.add_css(css, Origin::Author);
    }
    e
}

/// `merged = base ⊕ frontier` vs `compute_tree` — invariant
/// fort : tout nœud vivant a le style de la cascade complète.
/// Renvoie `merged` pour les assertions fines.
fn assert_equiv(
    e: &StyleEngine,
    dom: &Dom,
    env: &StyleEnv,
    base: &StyleMap,
    frontier: &[NodeId],
) -> StyleMap {
    let keyed: Vec<(NodeId, Option<&str>)> = frontier.iter().map(|&n| (n, None)).collect();
    assert_equiv_keyed(e, dom, env, base, &keyed)
}

/// Variante cléée (R-297) : `Some(attr)` = mutation
/// d'attribut, `None` = structurelle/contenu.
fn assert_equiv_keyed(
    e: &StyleEngine,
    dom: &Dom,
    env: &StyleEnv,
    base: &StyleMap,
    frontier: &[(NodeId, Option<&str>)],
) -> StyleMap {
    let sub = e.compute_scoped_frontier(dom, dom.document(), env, base, frontier);
    let mut merged = base.clone();
    merged.apply_scoped(sub);
    let full = e.compute_tree(dom, dom.document(), env);
    for (n, s) in full.iter() {
        assert_eq!(
            merged.get(n),
            Some(s),
            "nœud {n:?} diverge de la cascade complète"
        );
    }
    merged
}

// ── couverture : ancêtres ───────────────────────────────────

/// `A:has(.x)` au-dessus du touché bascule quand `.x` arrive —
/// la chaîne ancêtre est évaluée et le changement propagé.
#[test]
fn frontier_ancetres_has_evalues() {
    let mut f = fixture();
    let e = eng("div:has(.x) { color: rgb(1, 2, 3) }");
    let base = e.compute_tree(&f.dom, f.dom.document(), &env());
    // Avant : aucun ancêtre ne matche `:has(.x)`.
    assert_ne!(
        base.get(f.mid).and_then(|s| s.get("color")),
        Some("rgb(1, 2, 3)")
    );

    f.dom.set_attribute(f.t, "class", "x").unwrap();
    let merged = assert_equiv(&e, &f.dom, &env(), &base, &[f.t]);
    assert_eq!(
        merged.get(f.mid).and_then(|s| s.get("color")),
        Some("rgb(1, 2, 3)"),
        "l'ancêtre :has doit basculer"
    );
    assert_eq!(
        merged.get(f.host).and_then(|s| s.get("color")),
        Some("rgb(1, 2, 3)"),
        "la bascule couvre toute la chaîne"
    );
}

/// `E:has(+ .x)` sur le frère précédent direct du touché —
/// le sujet vit à gauche du match.
#[test]
fn frontier_frere_precedent_direct_has() {
    let mut f = fixture();
    // `f1` (div) précède immédiatement `f2` (span) sous body.
    let e = eng("div:has(+ .x) { color: rgb(7, 8, 9) }");
    let base = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_ne!(
        base.get(f.f1).and_then(|s| s.get("color")),
        Some("rgb(7, 8, 9)")
    );

    f.dom.set_attribute(f.f2, "class", "x").unwrap();
    let merged = assert_equiv(&e, &f.dom, &env(), &base, &[f.f2]);
    assert_eq!(
        merged.get(f.f1).and_then(|s| s.get("color")),
        Some("rgb(7, 8, 9)"),
        "le frère précédent `E:has(+ .x)` doit basculer"
    );
}

/// `E:has(~ .x)` — sujet à gauche de n'importe quelle
/// distance : `f1` (div) est suivi de `f3` (section) qui
/// prend `.x`.
#[test]
fn frontier_frere_precedent_lointain_has() {
    let mut f = fixture();
    let e = eng("div:has(~ .x) { color: rgb(4, 5, 6) }");
    let base = e.compute_tree(&f.dom, f.dom.document(), &env());

    f.dom.set_attribute(f.f3, "class", "x").unwrap();
    let merged = assert_equiv(&e, &f.dom, &env(), &base, &[f.f3]);
    assert_eq!(
        merged.get(f.f1).and_then(|s| s.get("color")),
        Some("rgb(4, 5, 6)"),
        "le sujet `E:has(~ .x)` à distance doit basculer"
    );
}

// ── couverture : frères suivants ────────────────────────────

/// `E + F` / `E ~ F` : les frères suivants du touché sont
/// évalués et basculent.
#[test]
fn frontier_freres_suivants_combinators() {
    let mut f = fixture();
    let e = eng(".on + span { color: rgb(1, 1, 1) } \
         .on ~ section { color: rgb(2, 2, 2) }");
    let base = e.compute_tree(&f.dom, f.dom.document(), &env());

    f.dom.set_attribute(f.f1, "class", "on").unwrap();
    let merged = assert_equiv(&e, &f.dom, &env(), &base, &[f.f1]);
    assert_eq!(
        merged.get(f.f2).and_then(|s| s.get("color")),
        Some("rgb(1, 1, 1)"),
        "le frère suivant direct `.on + span` doit basculer"
    );
    assert_eq!(
        merged.get(f.f3).and_then(|s| s.get("color")),
        Some("rgb(2, 2, 2)"),
        "le frère suivant général `.on ~ section` doit basculer"
    );
}

/// Canal silencieux `E ~ F G` : `section` ne change PAS de
/// computed (aucune règle ne le vise) alors que la vérité du
/// composé `E ~ F` bascule — `deep_relational` force la
/// descente de son sous-arbre pour rattraper `p`.
#[test]
fn frontier_canal_silencieux_sibling_ancetre() {
    let mut f = fixture();
    // Aucune règle `… ~ section` seul : le computed de f3 est
    // identique avant/après — seul le sélecteur ancêtre
    // `.on ~ section` de la règle profonde bascule.
    let e = eng(".on ~ section p { color: rgb(9, 9, 9) }");
    let base = e.compute_tree(&f.dom, f.dom.document(), &env());
    let before = base.get(f.f3).cloned();

    f.dom.set_attribute(f.f1, "class", "on").unwrap();
    let merged = assert_equiv(&e, &f.dom, &env(), &base, &[f.f1]);
    assert_eq!(
        merged.get(f.f4).and_then(|s| s.get("color")),
        Some("rgb(9, 9, 9)"),
        "le descendant sous composé-ancêtre frère doit basculer"
    );
    // Preuve du canal : le computed de `section` n'a pas
    // bougé (aucune règle ne le cible) — la descente n'est
    // pas motivée par son propre diff.
    assert_eq!(merged.get(f.f3), before.as_ref());
}

// ── bornes : héritage et élagage ────────────────────────────

/// Le style du touché change → les descendants héritent :
/// descente inconditionnelle du sous-arbre touché.
#[test]
fn frontier_heritage_descend_sous_touche() {
    let mut f = fixture();
    // `.on` change `font-style` — héritée par les descendants
    // (propriété héritée) même sans règle les visant.
    let e = eng(".on { font-style: italic }");
    let base = e.compute_tree(&f.dom, f.dom.document(), &env());

    f.dom.set_attribute(f.f3, "class", "on").unwrap();
    let merged = assert_equiv(&e, &f.dom, &env(), &base, &[f.f3]);
    assert_eq!(
        merged.get(f.f4).and_then(|s| s.get("font-style")),
        Some("italic"),
        "le descendant du touché hérite du nouveau style"
    );
}

/// Sous-arbre sans rapport avec la mutation : non visité —
/// la map émise ne le contient pas (preuve d'élagage) et le
/// style persisté fait foi.
#[test]
fn frontier_elague_le_sous_arbre_stable() {
    let mut f = fixture();
    let e = eng(".on { font-style: italic }");
    let base = e.compute_tree(&f.dom, f.dom.document(), &env());

    f.dom.set_attribute(f.f1, "class", "on").unwrap();
    let sub = e.compute_scoped_frontier(&f.dom, f.dom.document(), &env(), &base, &[(f.f1, None)]);
    // `mid`, `t` (sous-arbre profond disjoint) et `f4`
    // (sous un frère évalué-inchangé) restent hors émission.
    assert!(sub.get(f.t).is_none(), "sous-arbre disjoint élagué");
    assert!(
        sub.get(f.f4).is_none(),
        "descendant d'un évalué stable élagué"
    );
    // Mais la chaîne et les frères sont évalués.
    assert!(sub.get(f.f1).is_some());
    assert!(sub.get(f.f2).is_some(), "frère suivant du touché évalué");
    // La map fusionnée reste équivalente.
    let mut merged = base.clone();
    merged.apply_scoped(sub);
    let full = e.compute_tree(&f.dom, f.dom.document(), &env());
    for (n, s) in full.iter() {
        assert_eq!(merged.get(n), Some(s), "nœud {n:?} diverge");
    }
}

/// `frontier` vide → map vide (`apply_scoped` = no-op).
#[test]
fn frontier_vide_est_noop() {
    let f = fixture();
    let e = eng("");
    let base = e.compute_tree(&f.dom, f.dom.document(), &env());
    let sub = e.compute_scoped_frontier(&f.dom, f.dom.document(), &env(), &base, &[]);
    assert!(sub.iter().next().is_none());
}

// ── structurel : mutation de fratrie ────────────────────────

/// Insertion : NAV signale le **parent** comme touché →
/// restyle inconditionnel de sa fratrie — les `:nth-*` des
/// frères suivants basculent.
#[test]
fn frontier_insertion_parent_touche() {
    let mut f = fixture();
    let e = eng("span:nth-child(3) { color: rgb(5, 5, 5) }");
    let base = e.compute_tree(&f.dom, f.dom.document(), &env());
    // f2 = span, enfant n°3 de body (host=1, f1=2, f2=3,
    // f3=4) — `span:nth-child(3)` matche avant insertion.
    assert_eq!(
        base.get(f.f2).and_then(|s| s.get("color")),
        Some("rgb(5, 5, 5)")
    );
    // Insérer un élément AVANT f2 → f2 passe n°4.
    let body = f.dom.parent(f.f1).unwrap().unwrap();
    let ins = f.dom.create_element("div").unwrap();
    f.dom.set_attribute(ins, "id", "ins").unwrap();
    f.dom.insert_before(body, ins, Some(f.f2)).unwrap();
    let merged = assert_equiv(&e, &f.dom, &env(), &base, &[body]);
    assert_ne!(
        merged.get(f.f2).and_then(|s| s.get("color")),
        Some("rgb(5, 5, 5)"),
        "le décalage nth-child du frère suivant doit basculer"
    );
    // Et le nouvel inséré a son style calculé (nœud absent
    // de `base`, couvert par le sous-arbre du parent touché).
    assert!(merged.get(ins).is_some(), "le nœud inséré est stylé");
}

// ── R-297 — clé de mutation : descente sous un touché d'attr ─

/// `SetAttr` dont l'attribut n'apparaît dans AUCUN composé
/// non-sujet + computed inchangé ⇒ le sous-arbre du touché
/// est élagué (cas `data-is-billboard-visible` : restyle
/// d'un conteneur géant sans sélecteur dépendant).
#[test]
fn frontier_attr_hors_selecteurs_elaque_le_sous_arbre() {
    let mut f = fixture();
    // `.c` existe en position SUJET seule — `mid` prend la
    // classe mais aucun composé ancêtre ne lit `class`.
    let e = eng(".c { color: rgb(8, 8, 8) }");
    let base = e.compute_tree(&f.dom, f.dom.document(), &env());

    f.dom.set_attribute(f.mid, "data-zzz", "1").unwrap();
    let sub = f
        .dom
        .element(f.mid)
        .map(|_| {
            let e2 = &e;
            e2.compute_scoped_frontier(
                &f.dom,
                f.dom.document(),
                &env(),
                &base,
                &[(f.mid, Some("data-zzz"))],
            )
        })
        .unwrap();
    // `t` (descendant de `mid`) n'est PAS ré-émis — preuve
    // de l'élagage ; l'invariant global reste vérifié par
    // `assert_equiv_keyed` ci-dessous.
    assert!(
        sub.get(f.t).is_none(),
        "attr hors sélecteurs + computed stable ⇒ sous-arbre élagué"
    );
    let merged = assert_equiv_keyed(&e, &f.dom, &env(), &base, &[(f.mid, Some("data-zzz"))]);
    assert!(merged.get(f.t).is_some(), "le style persisté fait foi");
}

/// Même attr mais présent dans un composé ancêtre
/// (`[data-flag] p`) ⇒ la descente a lieu et le descendant
/// bascule — l'élagage ne doit jamais perdre un restyle.
#[test]
fn frontier_attr_en_compose_ancetre_descend() {
    let mut f = fixture();
    let e = eng("[data-flag] p { color: rgb(9, 9, 9) }");
    let base = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_ne!(
        base.get(f.t).and_then(|s| s.get("color")),
        Some("rgb(9, 9, 9)")
    );

    f.dom.set_attribute(f.mid, "data-flag", "1").unwrap();
    let merged = assert_equiv_keyed(&e, &f.dom, &env(), &base, &[(f.mid, Some("data-flag"))]);
    assert_eq!(
        merged.get(f.t).and_then(|s| s.get("color")),
        Some("rgb(9, 9, 9)"),
        "le descendant sous le composé ancêtre doit basculer"
    );
}

/// Fan-out de fratrie borné (R-297) : sous une frontière
/// 100 % `Some(attr)` la garde `:has` amont rend tout flip
/// de frère précédent impossible et `:nth-last-*` est un
/// comptage ; un frère suivant ne peut basculer que si
/// l'attr vit en composé non-sujet. `data-zzz` n'est dans
/// aucun sélecteur ⇒ ni frères précédents de chaîne ni
/// frères suivants du touché ne sont ré-émis.
#[test]
fn frontier_attr_ne_visite_pas_les_freres() {
    let mut f = fixture();
    // `.x` en position sujet seule — aucun composé
    // non-sujet ne lit `data-zzz` (ni `class`/`id`).
    let e = eng(".x { color: rgb(4, 4, 4) }");
    let base = e.compute_tree(&f.dom, f.dom.document(), &env());

    f.dom.set_attribute(f.f2, "data-zzz", "1").unwrap();
    let sub = e.compute_scoped_frontier(
        &f.dom,
        f.dom.document(),
        &env(),
        &base,
        &[(f.f2, Some("data-zzz"))],
    );
    for n in [f.host, f.mid, f.t, f.f1, f.f3, f.f4] {
        assert!(
            sub.get(n).is_none(),
            "{n:?} hors touché+spine ne doit pas être ré-émis"
        );
    }
    let probe = explorer_css::take_frontier_probe();
    assert_eq!(
        (probe.visited, probe.touched, probe.descend_attr),
        (3, 1, 0),
        "seuls doc+body+f2 sont évalués"
    );
    let merged = assert_equiv_keyed(&e, &f.dom, &env(), &base, &[(f.f2, Some("data-zzz"))]);
    assert!(merged.get(f.f3).is_some(), "le style persisté fait foi");
}

/// L'attr vit dans un composé non-sujet frère
/// (`#f1[data-flag] + #f2`) ⇒ le frère suivant bascule : la
/// visite des suivants reste obligatoire quand
/// `attr_descends` le dicte.
#[test]
fn frontier_attr_compose_suivant_visite_les_suivants() {
    let mut f = fixture();
    let e = eng("#f1[data-flag] + #f2 { color: rgb(7, 7, 7) }");
    let base = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_ne!(
        base.get(f.f2).and_then(|s| s.get("color")),
        Some("rgb(7, 7, 7)")
    );

    f.dom.set_attribute(f.f1, "data-flag", "1").unwrap();
    let merged = assert_equiv_keyed(&e, &f.dom, &env(), &base, &[(f.f1, Some("data-flag"))]);
    assert_eq!(
        merged.get(f.f2).and_then(|s| s.get("color")),
        Some("rgb(7, 7, 7)"),
        "le frère suivant doit basculer via `+` sur l'attr muté"
    );
}

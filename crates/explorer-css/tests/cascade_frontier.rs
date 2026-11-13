// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Restyle de frontière (R-292) — `compute_scoped_frontier` :
//! couverture des ancêtres, frères précédents et suivants des
//! touchés ; élagage des sous-arbres prouvés stables ;
//! équivalence avec la cascade complète après `apply_scoped`.

use explorer_css::{Origin, StyleEngine, StyleEnv, StyleMap, TouchedDeps};
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

// ── R-298 — frontière à dépendances `:has` ──────────────────

/// Canal silencieux `E:has(x) .d` sur sujet ANCÊTRE : la
/// vérité `:has` de `host` bascule quand `t` prend `.x`, mais
/// le computed de `host` ne change pas (la règle ne style que
/// `span`) — `changed` seul raterait `sp` dans la branche
/// disjointe. `deps` force la descente des sujets plausibles.
#[test]
fn frontier_deps_has_canal_silencieux_ancetre() {
    let mut f = fixture();
    // `host > branch > sp` — branche disjointe de la spine
    // (ni ancêtre, ni frère précédent/suivant du touché).
    let branch = f.dom.create_element("div").unwrap();
    f.dom.append_child(f.host, branch).unwrap();
    let sp = f.dom.create_element("span").unwrap();
    f.dom.append_child(branch, sp).unwrap();
    let e = eng("div:has(.x) span { color: rgb(6, 6, 6) }");
    let base = e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_ne!(
        base.get(sp).and_then(|s| s.get("color")),
        Some("rgb(6, 6, 6)")
    );

    f.dom.set_attribute(f.t, "class", "x").unwrap();
    // Sans `deps` : la branche n'est même pas visitée.
    let sub_dry = e.compute_scoped_frontier(
        &f.dom,
        f.dom.document(),
        &env(),
        &base,
        &[(f.t, Some("class"))],
    );
    assert!(
        sub_dry.get(sp).is_none(),
        "sans deps, la branche disjointe reste hors émission"
    );
    // Avec `deps` : `host` (sujet plausible `div`) obtient
    // `descend_all` → `sp` est rattrapé.
    let deps = TouchedDeps {
        classes: ["x".to_string()].into_iter().collect(),
        ..Default::default()
    };
    let sub = e.compute_scoped_frontier_deps(
        &f.dom,
        f.dom.document(),
        &env(),
        &base,
        &[(f.t, Some("class"))],
        Some(&deps),
    );
    assert_eq!(
        sub.get(sp).and_then(|s| s.get("color")),
        Some("rgb(6, 6, 6)"),
        "le descendant `.d` sous sujet `:has` plausible doit basculer"
    );
    let probe = explorer_css::take_frontier_probe();
    assert!(probe.descend_has > 0, "la sonde compte la descente sujet");
    let mut merged = base.clone();
    merged.apply_scoped(sub);
    let full = e.compute_tree(&f.dom, f.dom.document(), &env());
    for (n, s) in full.iter() {
        assert_eq!(merged.get(n), Some(s), "nœud {n:?} diverge");
    }
    assert_eq!(
        merged.get(f.host),
        base.get(f.host),
        "le computed du sujet n'a pas bougé — canal silencieux prouvé"
    );
}

/// Canal silencieux `E:has(+ x) .d` sur sujet FRÈRE
/// PRÉCÉDENT : `f1` vit à gauche du match — hors `deps` il
/// n'est pas visité (frontière attr-only) ; `deps` ouvre la
/// couverture des frères précédents ET la descente sujet.
#[test]
fn frontier_deps_has_canal_silencieux_frere() {
    let mut f = fixture();
    let sp1 = f.dom.create_element("span").unwrap();
    f.dom.append_child(f.f1, sp1).unwrap();
    // f1 (div) précède immédiatement f2 sous body.
    let e = eng("div:has(+ .x) span { color: rgb(1, 2, 3) }");
    let base = e.compute_tree(&f.dom, f.dom.document(), &env());

    f.dom.set_attribute(f.f2, "class", "x").unwrap();
    let deps = TouchedDeps {
        classes: ["x".to_string()].into_iter().collect(),
        ..Default::default()
    };
    let sub = e.compute_scoped_frontier_deps(
        &f.dom,
        f.dom.document(),
        &env(),
        &base,
        &[(f.f2, Some("class"))],
        Some(&deps),
    );
    assert_eq!(
        sub.get(sp1).and_then(|s| s.get("color")),
        Some("rgb(1, 2, 3)"),
        "le frère-sujet plausible doit descendre pour `.d`"
    );
    let mut merged = base.clone();
    merged.apply_scoped(sub);
    let full = e.compute_tree(&f.dom, f.dom.document(), &env());
    for (n, s) in full.iter() {
        assert_eq!(merged.get(n), Some(s), "nœud {n:?} diverge");
    }
    assert_eq!(merged.get(f.f1), base.get(f.f1));
}

/// `deps` qui n'intersectent pas l'arg `:has` ⇒ aucune
/// descente sujet, émission identique au chemin sec.
#[test]
fn frontier_deps_arg_non_touchee_elague() {
    let mut f = fixture();
    let branch = f.dom.create_element("div").unwrap();
    f.dom.append_child(f.host, branch).unwrap();
    let sp = f.dom.create_element("span").unwrap();
    f.dom.append_child(branch, sp).unwrap();
    let e = eng("div:has(.x) span { color: rgb(6, 6, 6) }");
    let base = e.compute_tree(&f.dom, f.dom.document(), &env());

    f.dom.set_attribute(f.t, "class", "y").unwrap();
    let deps = TouchedDeps {
        classes: ["y".to_string()].into_iter().collect(),
        ..Default::default()
    };
    let sub = e.compute_scoped_frontier_deps(
        &f.dom,
        f.dom.document(),
        &env(),
        &base,
        &[(f.t, Some("class"))],
        Some(&deps),
    );
    assert!(
        sub.get(sp).is_none(),
        "arg `.x` non touchée ⇒ la branche reste élaguée"
    );
    let probe = explorer_css::take_frontier_probe();
    assert_eq!(probe.descend_has, 0);
}

/// `all` (deps inconnues — index caduc ou appelant hors
/// journal) ⇒ tout composé non-sujet porteur de `:has`
/// qualifie ses sujets plausibles : couverture conservatrice
/// sans faux négatif.
#[test]
fn frontier_deps_all_couvre_les_sujets() {
    let mut f = fixture();
    let branch = f.dom.create_element("div").unwrap();
    f.dom.append_child(f.host, branch).unwrap();
    let sp = f.dom.create_element("span").unwrap();
    f.dom.append_child(branch, sp).unwrap();
    let e = eng("div:has(.x) span { color: rgb(6, 6, 6) }");
    let base = e.compute_tree(&f.dom, f.dom.document(), &env());

    f.dom.set_attribute(f.t, "data-z", "1").unwrap();
    let deps = TouchedDeps {
        all: true,
        ..Default::default()
    };
    let sub = e.compute_scoped_frontier_deps(
        &f.dom,
        f.dom.document(),
        &env(),
        &base,
        &[(f.t, Some("data-z"))],
        Some(&deps),
    );
    // `data-z` ne fait pas basculer `.x` — le span garde
    // son état, mais la couverture l'a visité (émis ou non,
    // l'invariant cascade complète tient).
    let probe = explorer_css::take_frontier_probe();
    assert!(
        probe.descend_has > 0,
        "`all` doit armer la couverture sujet"
    );
    let mut merged = base.clone();
    merged.apply_scoped(sub);
    let full = e.compute_tree(&f.dom, f.dom.document(), &env());
    for (n, s) in full.iter() {
        assert_eq!(merged.get(n), Some(s), "nœud {n:?} diverge");
    }
}

// ── R-299 — delta de feuilles ───────────────────────────────

/// `merged = base ⊕ delta(new, old)` ≡ `compute_tree(new)` —
/// l'invariant fort du restyle par delta, mêmes conventions
/// que [`assert_equiv`]. Renvoie `merged` pour assertions fines.
fn assert_equiv_delta(
    new_e: &StyleEngine,
    old_e: &StyleEngine,
    dom: &Dom,
    base: &StyleMap,
) -> StyleMap {
    let sub = new_e
        .compute_sheet_delta(
            dom,
            dom.document(),
            &env(),
            base,
            &[],
            &|_| false,
            None,
            None,
            old_e,
        )
        .expect("delta prouvable attendu");
    let mut merged = base.clone();
    merged.apply_scoped(sub);
    let full = new_e.compute_tree(dom, dom.document(), &env());
    for (n, s) in full.iter() {
        assert_eq!(
            merged.get(n),
            Some(s),
            "nœud {n:?} diverge de la cascade complète"
        );
    }
    merged
}

/// Ajout d'une règle en fin : seuls les matchs du nouveau
/// sélecteur sont seedés — restyle borné, résultat complet.
#[test]
fn delta_ajout_restyle_borne() {
    let f = fixture();
    let old_e = eng("div { color: rgb(9, 9, 9) }");
    let new_e = eng("div { color: rgb(9, 9, 9) } p#t { color: rgb(1, 2, 3) }");
    let base = old_e.compute_tree(&f.dom, f.dom.document(), &env());
    let merged = assert_equiv_delta(&new_e, &old_e, &f.dom, &base);
    assert_eq!(
        merged.get(f.t).and_then(|s| s.get("color")),
        Some("rgb(1, 2, 3)")
    );
    // Hors match : entrée identique à la base.
    assert_eq!(merged.get(f.f1), base.get(f.f1));
    let probe = explorer_css::take_frontier_probe();
    assert_eq!(probe.delta_seeds, 1, "un seul seed attendu (t)");
    assert!(
        probe.visited <= 8,
        "marche bornée, visited={}",
        probe.visited
    );
}

/// Retrait d'une règle : les éléments qu'elle matchait sont
/// seedés (son évaluation sur le DOM courant les trouve) —
/// retour aux déclarations survivantes.
#[test]
fn delta_retrait_restyle_borne() {
    let f = fixture();
    let old_e = eng("div { color: rgb(9, 9, 9) } p#t { color: rgb(1, 2, 3) }");
    let new_e = eng("div { color: rgb(9, 9, 9) }");
    let base = old_e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_eq!(
        base.get(f.t).and_then(|s| s.get("color")),
        Some("rgb(1, 2, 3)")
    );
    let merged = assert_equiv_delta(&new_e, &old_e, &f.dom, &base);
    assert_ne!(
        merged.get(f.t).and_then(|s| s.get("color")),
        Some("rgb(1, 2, 3)"),
        "la règle retirée ne doit plus produire sa déclaration"
    );
}

/// Insertion en milieu de table : la zone médiane marque les
/// règles voisines — le delta reste complet.
#[test]
fn delta_insertion_milieu_equiv() {
    let f = fixture();
    let old_e = eng("div { color: rgb(9, 9, 9) } section { color: rgb(8, 8, 8) }");
    let new_e = eng(
        "div { color: rgb(9, 9, 9) } p { color: rgb(1, 2, 3) } section { color: rgb(8, 8, 8) }",
    );
    let base = old_e.compute_tree(&f.dom, f.dom.document(), &env());
    let merged = assert_equiv_delta(&new_e, &old_e, &f.dom, &base);
    assert_eq!(
        merged.get(f.f4).and_then(|s| s.get("color")),
        Some("rgb(1, 2, 3)")
    );
}

/// Réordonnancement : la zone médiane couvre les deux règles —
/// les matchs des deux sont seedés, équivalence complète.
#[test]
fn delta_reordonnancement_equiv() {
    let f = fixture();
    let old_e = eng("div { color: rgb(9, 9, 9) } p { color: rgb(1, 2, 3) }");
    let new_e = eng("p { color: rgb(1, 2, 3) } div { color: rgb(9, 9, 9) }");
    let base = old_e.compute_tree(&f.dom, f.dom.document(), &env());
    assert_equiv_delta(&new_e, &old_e, &f.dom, &base);
}

/// Tables identiques mais DOM grossi : les nœuds jamais stylés
/// sont seedés structurels (clé `None`), l'équivalence tient.
#[test]
fn delta_neud_jamais_style_seed_structurel() {
    let mut f = fixture();
    let old_e = eng("p { color: rgb(1, 2, 3) }");
    let base = old_e.compute_tree(&f.dom, f.dom.document(), &env());
    let n = f.dom.create_element("p").unwrap();
    f.dom.append_child(f.host, n).unwrap();
    let new_e = eng("p { color: rgb(1, 2, 3) }");
    let merged = assert_equiv_delta(&new_e, &old_e, &f.dom, &base);
    assert_eq!(
        merged.get(n).and_then(|s| s.get("color")),
        Some("rgb(1, 2, 3)"),
        "le nœud jamais stylé doit recevoir la cascade complète"
    );
    let probe = explorer_css::take_frontier_probe();
    assert_eq!(probe.delta_seeds, 1);
}

/// La frontière journal fusionne avec les seeds delta — un
/// `SetAttr` simultané au rebuild est couvert par les deux.
#[test]
fn delta_fusionne_frontiere_journal() {
    let mut f = fixture();
    f.dom.set_attribute(f.f1, "class", "x").unwrap();
    let old_e = eng("");
    let base = old_e.compute_tree(&f.dom, f.dom.document(), &env());
    let new_e = eng(".x { color: rgb(1, 2, 3) }");
    let sub = new_e
        .compute_sheet_delta(
            &f.dom,
            f.dom.document(),
            &env(),
            &base,
            &[(f.f1, Some("class"))],
            &|_| false,
            None,
            None,
            &old_e,
        )
        .expect("delta prouvable");
    let mut merged = base.clone();
    merged.apply_scoped(sub);
    let full = new_e.compute_tree(&f.dom, f.dom.document(), &env());
    for (n, s) in full.iter() {
        assert_eq!(merged.get(n), Some(s), "nœud {n:?} diverge");
    }
    assert_eq!(
        merged.get(f.f1).and_then(|s| s.get("color")),
        Some("rgb(1, 2, 3)")
    );
}

/// Gate `@property` : le registre affecte les valeurs calculées
/// sans sélecteur changé → repli (`None`).
#[test]
fn delta_property_registry_differe_repli() {
    let f = fixture();
    let old_e = eng("");
    let base = old_e.compute_tree(&f.dom, f.dom.document(), &env());
    let new_e = eng(
        "@property --brand { syntax: '<color>'; inherits: false; initial-value: rgb(255,0,0) }",
    );
    assert!(new_e
        .compute_sheet_delta(
            &f.dom,
            f.dom.document(),
            &env(),
            &base,
            &[],
            &|_| false,
            None,
            None,
            &old_e,
        )
        .is_none());
}

/// Gate `@namespace` : la résolution des sélecteurs `ns|tag`
/// change sans sélecteur changé → repli (`None`).
#[test]
fn delta_namespaces_different_repli() {
    let f = fixture();
    let old_e = eng("");
    let base = old_e.compute_tree(&f.dom, f.dom.document(), &env());
    let new_e = eng("@namespace svg \"urn:x\"");
    assert!(new_e
        .compute_sheet_delta(
            &f.dom,
            f.dom.document(),
            &env(),
            &base,
            &[],
            &|_| false,
            None,
            None,
            &old_e,
        )
        .is_none());
}

# ADR-0013 — Bindings DOM : miroir synchronisé

Statut : Acceptée (2026-10-19).

## Contexte

L'étape 35 (bindings DOM) doit exposer `document`, les éléments
et leurs mutations au JavaScript tout en respectant trois
contraintes qui se tirent en sens opposé :

1. **Single-writer DOM** (AGENTS.md §4.8) : `explorer-dom` vit
   sur le flux session, jamais de partage concurrent.
2. **`dillo_js::Engine` est `!Send`** (tas `Rc`/`RefCell`) : il
   vit sur un thread dédié (ADR-0011/étape 34) — impossible
   d'y donner un accès direct à l'arène `explorer-dom` sans
   `unsafe` ou verrous croisés.
3. **Autonomie de `dillo-js`** : réécrire la surface DOM
   (~1200 lignes de `dom_host`) comme `Host` objects sur
   l'arène réelle dupliquerait un code que l'équipe JS fait
   évoluer en parallèle — divergence garantie.

ADR-0007 qualifiait le mini-DOM de `dillo-js` de « harnais de
test » ; cette ADR révise ce point à la lumière de ce que son
API offre réellement : `Engine::install_dom(&DomRef)` installe
toute la surface navigateur (`document`, `navigator`,
`window`, collections live) sur **un DOM fourni par
l'embedder**, et `Dom` expose un *dirty bit* +
`serialize_document()` conçus explicitement pour la
resynchronisation de rendu.

## Décision

**Miroir synchronisé plutôt que partage d'arène.**

- Le worker JS construit `dillo_js::Dom::from_html(source)` et
  l'installe — les scripts lisent/mutent *le miroir* (leur
  vérité DOM), jamais `explorer-dom` directement. Les `NodeId`
  générationnels ne traversent aucune frontière : chaque arène
  garde ses identifiants internes.
- `JsEngine` expose `install_dom(html)` et `sync_dom() ->
  Option<String>` ; après l'évaluation des scripts, le core
  sonde le dirty bit. S'il est levé, `serialize_document()`
  est **re-parsé par `explorer-html`** en un nouvel
  `explorer-dom` — l'invalidation = re-parse borné +
  relayout/ repaint existants (ADR-0012), sans mécanisme
  nouveau.
- Consommer le dirty bit du parse initial à l'install pour ne
  resynchroniser que les vraies mutations de script.

## Justification

- **Zéro surface réécrite** : toute la couverture DOM de
  `dillo-js` (querySelector, createElement, textContent,
  innerHTML, listeners…) bénéficie au navigateur telle
  quelle, et chaque progrès amont est récupéré
  automatiquement.
- **Un seul sens de copie** : mirror → moteur via
  sérialisation HTML — déjà tolérante aux divergences (le
  re-parse est le pipeline normal, borné, sans `unsafe`).
- **Aucun `NodeId` exposé** : pas de handle pouvant
  survivre à une re-parse — la règle « handle obsolète =
  erreur typée » d'ADR-0008 est évacuée par construction.
- L'alternative (host objects custom sur
  `Arc<Mutex<explorer-dom>>`) impose soit du blocking depuis
  le thread JS vers la session (deadlock possible avec le
  mutex API), soit un protocole de messages — pour une
  fidélité que la resynchro par re-parse obtient déjà en
  pratique au prix d'une page re-parsée (bornée).

## Conséquences

- **Limites assumées** : le miroir re-parse le HTML *source*,
  il peut diverger légèrement des corrections du
  tree-builder HTML5 ; la synchro n'est déclenchée qu'en fin
  de navigation (les mutations par événements — étape 36 —
  ajouteront des points `sync_dom` dans `handle_input` /
  timers) ; `document.title=` n'est pas un setter amont.
- La sérialisation-reparse est O(document) par synchro —
  acceptable à cette étape ; si le coût devient mesurable,
  le point d'extension est un diff d'arbre miroir→réel (même
  interface `sync_dom`, autre transport).
- ADR-0007 est amendé : le « mini-DOM harnais » devient le
  **DOM miroir script-facing** — sa raison d'être
  architecturale.
</content>

# ADR-0009 — Stack texte et fontes : primitives pur Rust, architecture anticipée

Statut : Acceptée (2026-10-19) — architecture à honorer dès l'étape 17
(arbre de boîtes), dépendances épinglées à l'étape 18.

## Contexte

L'étape 18 conditionne tout le layout inline : sans métriques
réelles, pas de line boxes ni de word wrap corrects. Mais « mesurer
des glyphes » cache la pile complète du texte web :

- chargement et matching de fontes (familles, graisses, fallback) ;
- shaping (ligatures, arabes, indic, positionnement de diacritiques) ;
- bidirectionnalité (UAX #9) ;
- opportunités de coupure de ligne (UAX #14) ;
- rasterisation de glyphes (+ webfonts WOFF/WOFF2 en phase 10).

Options : assembler des primitives pur Rust, ou adopter un stack
texte complet (`cosmic-text`, `parley`) — qui embarquerait un
mini-moteur de layout texte, frontière contestable avec la mission
« moteur écrit ici » (ADR-0001).

## Décision

**Assemblage de primitives pur Rust** (versions épinglées à l'étape
18, ≥ 7 jours de publication — AGENTS.md §4.2) :

- chargement/indexation de fontes : `fontdb` + `ttf-parser`
  (métriques) ;
- shaping : `rustybuzz` (port safe de HarfBuzz) ;
- bidi : `unicode-bidi` ;
- coupures de ligne : `unicode-linebreak` ;
- rasterisation de glyphes : `swash` ou le chemin texte de
  `tiny-skia` — arbitré avec la phase 5.

Conséquences d'architecture (dès l'étape 17) :

- le layout inline travaille sur des **runs de texte itemisés**
  (script, direction, fonte résolue), jamais sur des chaînes plates ;
- toute mesure passe par le stack de fontes — aucune métrique fixe
  de test ne se propage au-delà des fixtures ;
- une **fonte fixture embarquée** dans `explorer-test-support` rend
  les golden tests headless reproductibles sans fontes système ;
- HiDPI : la mesure se fait en unités indépendantes du device ; le
  facteur d'échelle s'applique à la rasterisation (étape 23) ;
- la résolution de fontes accepte d'ores et déjà des fontes chargées
  par le document (abstraction `FontSource`) — WOFF/WOFF2 et
  `font-display` restent phase 10, mais l'interface n'aura pas à
  changer ;
- **frontière I/O explicite** : `fontdb` interroge le filesystem et
  le registre — c'est de l'infrastructure, pas du domaine.
  `explorer-layout` définit le port `FontProvider`/`FontSource` et
  ne connaît que lui ; l'implémentation `fontdb` vit en couche
  infrastructure, instanciée et injectée par `explorer-core`
  (composition root). Le domaine `layout` reste pur — aucun accès
  disque, tests déterministes sur fonte fixture.

## Justification

- `rustybuzz`, `unicode-bidi`, `unicode-linebreak`, `ttf-parser`
  sont des implémentations d'algorithmes Unicode normalisés — de
  l'infrastructure, pas du moteur. Réécrire le shaping serait du
  risque sans valeur différentiante (même logique que
  `httparse`/`rustls`, ADR-0001).
- Un stack complet (`cosmic-text`) importerait des décisions de
  layout (wrap, alignement, sélection) que ce dépôt veut posséder —
  exclu par défaut, comme `html5ever`.
- Architecturer l'inline sur des runs itemisés dès l'étape 17 évite
  de couler le layout dans une API « ASCII monospace » impossible à
  étendre — le palier 1 seul exige déjà le multilingue (Wikipedia).

## Conséquences

- Couverture initiale assumée : shaping complet via `rustybuzz` ;
  les scripts non latins sont testés dès le palier 1.
- Ces crates sont pur Rust, sans `*-sys` — conformes à ADR-0001 et à
  la règle `#![forbid(unsafe_code)]` côté code maison.
- Le découpage en runs impose une passe d'itemisation dans
  `explorer-layout` (texte → runs → segments shapés → line boxes) —
  documentée dans l'ADR de phase 4.

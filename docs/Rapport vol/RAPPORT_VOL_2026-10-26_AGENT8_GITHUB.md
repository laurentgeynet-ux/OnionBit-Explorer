<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 8 — github.com (approfondi)

Date : 2026-10-26. Daemon : `C:/obe-vol-head/target/release/explorer-daemon.exe`
(--release), port 9392, `EXPLORER_VIEWPORT=1366x900`, PID 4028 (tué en fin de
vol). Onglet unique (id=1), parcours en navigations successives + un clic-lien
réel. Aucun code modifié.

Artefacts : `captures/vol8_*.png`, `logs/vol8_*_{dom,layout,console}.*`.

## Synthèse du parcours

| Étape | URL | doc_h | Console | Verdict |
|---|---|---|---|---|
| home | github.com | 23 697 (**clamped**) | vide (404 « pas de journal ») | méga-menu déplié, textes superposés |
| trending | github.com/trending | 6 231 | vide | liste des repos **correctement rendue** |
| clic repo | github.com/Panniantong/Agent-Reach | — | `getAttribute` ×1 | navigation par clic OK |
| repo | github.com/rust-lang/rust | 7 504 | getAttribute + yield ×2 | fichiers + README rendus |
| fichier | …/blob/main/LICENSE-APACHE (via clic) | — | + yield ×1 | vue code OK |
| recherche | github.com/search?q=explorer&type=repositories | 9 800 | idem | **résultats vides** (« couldn't find anything ») |
| back | → LICENSE-APACHE | 10 307 | — | bfcache OK, frame restaurée |
| forward | → search | 9 800 | — | bfcache OK |

`/mem` final : `dom_nodes=2271`, `js_heap=43 MB`, `bfcache_pages=4`,
`total_bytes≈526 MB`, `journal_applies=1`, `journal_fallbacks=8`.

## Erreurs console verbatim (ring 512, `GET /console`)

```
exception js : TypeError: `getAttribute` is not a function (on HTMLDocument)
    at <anonymous> (https://github.githubassets.com/assets/behaviors-13ddd39284168ce5.js:2:69915)
    at m (./pb0-3a1794e99794195c.js:2:2682)
```

```
exception js : SyntaxError: yield outside a generator
    at <anonymous> (./4u-bf6e08201764cb0a.js:2:551)
    at e (./4u-bf6e08201764cb0a.js:2:1710)
    at Q (./4u-bf6e08201764cb0a.js:2:390)
    at eg (./4u-bf6e08201764cb0a.js:2:37580)
    at <anonymous> (https://github.githubassets.com/assets/code-view-cd96b63c56522167.js:5:85297)
    at n (./wp-runtime-602588a9ee53015d.js:2:29)
    at <anonymous> (https://github.githubassets.com/assets/code-view-cd96b63c56522167.js:2:124570)
    at n (./wp-runtime-602588a9ee53015d.js:2:29) — module «…/code-view-cd96b63c56522167.js»
```

Le `SyntaxError yield` revient **3×** (une par page repo/fichier/recherche) —
module `code-view` rechargé à chaque navigation et systématiquement en échec.
Le `TypeError getAttribute` apparaît dès le premier chargement d'une page
« behaviors » (trending→clic) puis persiste dans le ring.

## Bissection

### E1 — `getAttribute is not a function (on HTMLDocument)`
- Fichier : `behaviors-*.js` (bundle bootstrap des comportements GitHub),
  position minifiée 2:69915, appelé depuis `m` dans `pb0-*.js` (runtime
  module bootstrap).
- Hypothèses ordonnées :
  1. Le code fait `document.getAttribute(name)` — illegal en DOM réel
     (getAttribute est sur `Element`), mais GitHub dépend en pratique d'un
     proxy/`document.documentElement` déréférencé avant ; plus probable :
     le moteur `explorer-js` n'expose pas `document.documentElement` ou
     `Element.getAttribute` sur l'objet retourné, et l'erreur est remontée
     avec le mauvais receiver (HTMLDocument). À vérifier dans le DOM dump :
     `vol8_repo_dom.txt` montre les attributs `data-*` présents sur `<html>`.
  2. `querySelector` retourne le document quand le sélecteur matche `<html>`
     (racine) — appel `getAttribute` direct dessus → receiver HTMLDocument.
  3. Hypothèse minime : `HTMLDocument` doit être traité comme un `Element`
     fantôme pour `getAttribute` sur certains chemins — non standard, à
     confirmer en lisant la source déminifiée (`behaviors-…js` offset 69915).

### E2 — `SyntaxError: yield outside a generator` dans `4u-*.js`
- Chunk webpack `4u-bf6e08201764cb0a.js` chargé par `code-view-*.js`.
- Le message est une erreur **de parse/compile**, pas d'exécution : le code
  transpilé (probablement regenerator/TS async → function*) place `yield`
  dans une fonction que `explorer-jscore` n'analyse pas comme générateur.
- Hypothèses ordonnées :
  1. `yield` à l'intérieur d'une `async function*` / arrow transformée —
     support des générateurs incomplet (fonction imbriquée, `yield*`,
     paramètres par défaut…).
  2. Le parseur traite `function* ()` en mode sloppy→strict différemment
     et redescend `yield` hors contexte.
  3. À reproduire en isolant `4u-*.js` offset 2:551 dans un test262-like.
- Conséquence produit : tout le sous-arbre `code-view` (file browser React,
  search results `react-partial`) est mort → explique la recherche vide et
  les zones réactives non hydratées.

## Constats mesurés par étape

### Home (vol8_home.png / _scroll.png)
- Le méga-menu de navigation (Product/Solutions/Open Source…, normalement
  `display:none` jusqu'à hover) est **rendu entièrement déplié** et occupe
  ~1500 px verticaux ; textes superposés (« Features » / « BY USE CASE »).
  Classification : **CSS/DOM** — les menus GitHub sont des `<details>` /
  `[hidden]`/`.dropdown-menu` masqués par cascade ; probable non-prise en
  compte d'un sélecteur d'état (`details:not([open]) > .dropdown-menu`) ou
  de l'attribut `hidden` sur custom elements.
- `doc_h=23697` **clamped** (layout 22 546) — plafond de hauteur atteint.
- Scroll : `scroll_y` grimpe (800→2400) mais la frame après scroll montre
  le même bandeau + menu déplié avec collisions de texte — scroll partiellement
  inefficace sur cette page clampée. Classification **layout/render**.
- Grille mousemove 3×6 sur le header : `editable:false` partout, un seul
  lien trouvé (`/enterprise` à (50,50)) — le champ recherche du header
  n'est **pas hit-testable** (absent ou recouvert par le méga-menu déplié).
- Console : **404 « pas de journal »** sur home ET trending — zéro erreur
  enregistrée au boot (contraste avec les vols précédents qui logguaient
  `is not a function` ; ici le boot passe sans exception jusqu'au clic).

### Trending (vol8_trending.png / _scroll.png)
- Liste des dépôts du jour **rendue lisiblement** : titres, descriptions,
  compteurs d'étoiles, rangée par rangée — meilleure page du parcours.
- Avatars remplacés par des glyphes cassés (images non décodées affichées
  en placeholder). Classification **render/images**.
- `GET /frame` a renvoyé un **PNG de 0 octet** juste après la nav (HTTP 200,
  corps vide) ; retry 5 s plus tard → 17 063 625 o. Transitoire de
  rasterisation — à noter pour les agents (retry `/frame` comme `/dom`).
- Scroll 3×2000 : `scroll_y` 2000→4000→5331 (max doc 6 231-900) — scroll
  **fonctionne** ici, frame finale montre le pied de page.
- Hover grille : seuls liens trouvés en dehors du menu ; `editable:false`.
- Clic (250,120) sur le titre « Panniantong / Agent-Reach » →
  `{"result":"navigated"}` : **le clic-lien navigue**.

### Repo rust-lang/rust (vol8_repo.png / _scroll.png)
- Page repo correctement structurée : fil d'ariane, liste des fichiers
  (chaque ligne dans un cadre — bordures de tableau visibles), README rendu
  avec logo Rust, sections Why Rust/Quick Start/License lisibles.
- Sidebar About présente. Onglets Code/Issues/PR présents dans le DOM.
- Méga-menu toujours déplié au-dessus (moitié haute de la page noire).
- Console : apparition de l'erreur E1 + E2 ×2 (voir verbatim).
- Scroll 3×2500 → 6 604 (max), hit-test trouve `…/blob/main/LICENSE-APACHE`
  à (200,480) ; clic → navigation OK.

### Vue fichier LICENSE-APACHE (vol8_file.png)
- Lignes de code rendues avec numérotation implicite et coloration
  minimale ; header « code » (breadcrumb, boutons) présent.
- Même erreur E2 (+1 occurrence) — `code-view` replanté à chaque nav.

### Recherche (vol8_search.png)
- Page de résultats **quasi vide** : en-tête « Repository search results »,
  icône 🎃 « couldn't find anything », skeletons de lignes vides —
  les `react-partial`/`embedded-data` des résultats ne sont pas hydratés.
  Conséquence directe probable de E2 (webpack chunk mort → React jamais
  monté). Classification **JS** (avec retombée DOM).
- DOM récupéré (200, 1er essai — pas de 404 transitoire ici).

### Back / Forward
- `POST back` → LICENSE-APACHE, `can_forward:true`, `doc_h=10307` —
  **restauration bfcache correcte** (frame vol8_back.png identique à la
  vue fichier).
- `POST forward` → search, doc_h=9800 identique — cohérent.
- 4 pages en bfcache, ~136 MB de frames + 208 MB de heap JS retenus —
  coût mémoire notable (constat perf, pas bug).

## Constats actionnables

1. **[JS, bloquant] `SyntaxError: yield outside a generator`** —
   `4u-bf6e08201764cb0a.js:2:551` via `code-view-cd96b63c56522167.js`.
   Tue l'hydratation React (recherche vide, code-view mort). Extraire le
   chunk et le réduire à un cas minimal générateur dans explorer-jscore.
2. **[JS] `TypeError: getAttribute is not a function (on HTMLDocument)`** —
   `behaviors-13ddd39284168ce5.js:2:69915`. Vérifier si `document` reçoit
   un appel `getAttribute` (receiver d'après le message) ou si
   `documentElement`/`querySelector(':root')` retourne le document.
3. **[CSS/layout, majeur] Méga-menu GitHub toujours déplié** — tous les
   sous-menus du header visibles et empilés (~1500 px), textes superposés.
   Suspects : `details:not([open])`, attribut `hidden`, `.dropdown-menu`,
   sélecteur d'état ou `position:absolute` non résolu sur custom elements.
4. **[layout] `doc_h` clampé à ~23 697 sur la home** — `doc_h_clamped:true`,
   scroll ensuite incohérent visuellement (collisions de texte en frame).
5. **[render] Avatars/images en glyphes cassés** sur trending (images
   `avatars.githubusercontent.com` non rasterisées — `images_bytes` non nul
   mais affichage placeholder).
6. **[API] `/frame` peut renvoyer 200 + corps vide** juste après navigation
   lourde (observé sur trending, search : 1er fetch vide, retry OK) —
   documenter le retry aux côtés du 404 transitoire `/dom` dans vol-agent.md.
7. **[hit-test] Champ de recherche header jamais `editable`** — grille
   3×6 sur y∈[10,80] : aucun hit editable, un seul lien (/enterprise).
   Probablement recouvert par le menu déplié (lié au constat 3) ou non
   rendu (composant `qbsearch-input` custom element).
8. **[perf] ~526 MB retenus après 5 navigations** (bfcache 4 pages ≈
   136 MB frames + 208 MB JS heap) — surveiller la politique d'éviction.
9. **[positif] bfcache back/forward fiable** (titres, doc_h, frames
   restaurés) ; clic-lien navigue ; trending et vue fichier lisibles.
10. **[positif] Boot sans erreur JS** sur home+trending (404 console
    significative) — les erreurs `is not a function` des vols précédents
    n'apparaissent qu'à partir de behaviors/code-view.

## Fin de vol
Onglet 1 fermé (`DELETE`), daemon PID 4028 tué (`kill 4028`). Aucune
modification de code. Log daemon : `C:/OnionBit-Explorer/_vol8_daemon.log`
(vide — stdout bufferisé).

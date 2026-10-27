<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 43 — Caractérisation morts silencieuses §B-4

- **Date** : 2026-10-27
- **Binaire** : `C:/obe-vol-target/release/explorer-daemon.exe` — tag
  **`[V4-main]`** (inclut le WIP `depth_limit` de l'équipe fix —
  JS-C/J2 en cours)
- **Viewport** : 1366×900 (`EXPLORER_VIEWPORT`) — port 9425 — daemon
  **PID 2112** — journal `_vol43_daemon.log` → `logs/daemons/`
- **Protocole** : `docs/vol-agent.md` §8 — chaque constat porte `crate`
  + ID §B ; statuts canoniques §G.
- **Mission** : caractériser le bras §B-4 « morts silencieuses » sur
  trois cibles déjà cataloguées muettes — twitch.tv, archive.org,
  windy.com (importmap). Question posée : *console vide ou non ?
  scripts jamais drainés vs exceptions post-instanciation ?*
- **RAM** : surveillée par `tasklist` à chaque étape (poste crashé à
  la vague précédente, seuil d'alerte 2,5 Go). Relevés PID 2112 :
  **373 Mio** au démarrage → **500 Mio** après twitch → **546 Mio**
  après windy. Stable, aucune alerte — 3 cibles volées comme prévu.

## Verdicts en une table

| Cible | ID §B | Verdict `[V4-main]` |
|---|---|---|
| twitch.tv | §B-4 mort silencieuse | **TOUJOURS PRÉSENT — bras nouveau, le plus silencieux** : scripts drainés **et** évalués avec succès (44/44 `ok=true`, bundles externes jusqu'à 355 Ko), zéro `ok=false`, zéro WARN, console vide, app jamais montée |
| archive.org | §B-4 mort silencieuse | **DÉGRADÉ EN MIEUX (tracé)** : modules instanciés (heap 34,8 Mio), boot mort sur `unhandledrejection` **verbatim en console** — le bras « exception post-instanciation » est désormais observable |
| windy.com | §B-4 (bras importmap) | **TOUJOURS PRÉSENT** : 1 seul eval (inline 944 o), les 2 `type="module"` + importmap jamais drainés ni tracés, heap 8,7 Mio — modules inertes, recoupe §B-5/J3 |

Réponse à la question de la mission : **les deux bras existent, et il
y en a un troisième**. Sur trois cibles « muettes » du catalogue :
une seule est encore intégralement silencieuse (windy = modules jamais
drainés), une est sortie du silence par `unhandledrejection`
(archive.org = exception post-instanciation tracée), et twitch
démontre un bras non décrit jusqu'ici : **drain + éval réussis, mort
sans effet ni rejet** — le code a tourné sans erreur mais n'a produit
aucun montage.

---

## 1. Twitch — https://www.twitch.tv — §B-4 **TOUJOURS PRÉSENT (bras « drain OK, mort sans trace »)**

- **Titre** : « Twitch » — doc_h 900 px (= viewport), DOM 226 Ko /
  244 nœuds, layout 22 boîtes, heap JS 7,2 Mio.
- **Frame** `captures/vol43_twitch.png` (scroll identique —
  `vol43_twitch_scroll.png`) : **squelette SSR peint** — logo
  Twitch, barres de nav placeholder, spinner `shell-loader` centré.
  `#root` contient uniquement `shell-nav` + `shell-loader`
  (`logs/vol43_twitch_dom.txt` l. 285-345) : React n'a jamais
  remplacé le shell.
- **Console** : « pas de journal » (`logs/vol43_twitch_console.txt`).
- **Traçage fin** — la signature est *différente* de pinterest
  (vol 38 : 10 évals inline, 0 externe) : le journal daemon montre
  **44 `script: eval…` → 44 `eval fin`, tous `ok=true`, zéro
  `ok=false`, zéro WARN, zéro ERROR** sur tout le vol de tab=1.
  Les tailles prouvent que des **bundles externes ont été fetchés et
  évalués** : 354 766 o, 189 871 o, 186 579 o, 135 064 o, 97 174 o,
  83 033 o… (~30 scripts externes + ~10 inline). Le DOM porte 105
  `<script src="https://assets.twitch.tv/…">` : une partie des
  chunks (probablement les `<script>` différés/lazy du runtime
  webpack) n'apparaît pas dans les évals — **sans WARN fetch ni
  refus tracé**.
- **Caractérisation** : le bras §B-4 ici n'est **ni** « scripts
  jamais drainés » (44 évals dont de gros bundles) **ni**
  « exception post-instanciation » (rien n'est tracé — même pas un
  `unhandledrejection`). C'est un troisième bras : **exécution
  nominale sans effet** — le code tourne `ok`, l'app ne monte pas,
  aucune promesse ne rejette (ou le rejet est intercepté avant
  l'instrumentation JS-C154). Suspects : une API manquante consommée
  silencieusement (retour `undefined` accepté), un timer/microtask
  de montage jamais honoré, ou un `catch{}` applicatif avalant
  l'erreur.
- **Constat adjacent perf** : l'animation CSS infinie du spinner
  (`animation: shell-loading-spinner 1s infinite linear`) entretient
  une boucle `apply: cascade→boxtree→layout→repaint` complète
  **toutes les ~75 ms** (~13 Hz) pendant toute la vie de l'onglet —
  ~15 ms par cycle. Sur une page morte au spinner, le moteur refait
  le pipeline entier à chaque frame d'animation → parenté §B-perf
  (repaint incrémental) : une `transform` animée ne devrait pas
  recoûter cascade+boxtree+layout.
- Scroll ×2 : `scroll_y` reste 0 (doc_h = viewport, rien à scroller).

## 2. Archive.org — https://archive.org — §B-4 **DÉGRADÉ EN MIEUX : la mort est tracée**

- **Titre** : « Internet Archive: Digital Library… » — doc_h 0 px,
  DOM 9,7 Ko / 108 nœuds, layout 10 boîtes toutes 0×0
  (`<app-root>` custom element jamais peuplé), heap JS **34,8 Mio**,
  `journal_fallbacks=1` (WARN `journal DOM rejeté → repli re-parse
  ADR-0025 why=Fingerprint`).
- **Frame** `captures/vol43_archive.png` : **blanc total** — app
  Lit/webcomponents (`webcomponents-loader.js`, `polyfill-support.js`,
  1 `type="module"` `index-CKRP8DXx.js`, bundle `webcomponents-sd-ce-
  pf.js` injecté) jamais montée.
- **Console — NON vide** (`logs/vol43_archive_console.txt`,
  verbatim) :
  - `unhandledrejection: TypeError: cannot read 'append' of
    Undefined at <anonymous>
    (https://archive.org/offshoot_assets/index-CKRP8DXx.js:2:3910)`
    (×2 puis ×5 — pile 5 frames interne au bundle) ;
  - `unhandledrejection: TypeError: cannot read 'get' of
    Undefined at <anonymous> (./dist-DVfk5P_2.js:1:963)`.
- **Traçage fin** : 2 évals inline `ok=true` (3 805 + 6 417 o) ; le
  chemin module ne logue pas `eval fin` (constat vol 38) mais heap
  34,8 Mio ⇒ graphe de modules chargé **et instancié**. La mort est
  donc post-instanciation — et **elle remonte en console** : le bras
  « exception aval » de §B-4 est désormais observé (crédit JS-C154
  `unhandledrejection`→PipelineError, dédup « ×N » opérante).
- **Diagnostic** : `cannot read 'append' of Undefined` —
  probablement `el.append(...)` sur un nœud `undefined` (custom
  element non upgradé / `document.createElement` ou sélecteur
  retournant `undefined`) → parenté §B-14/§B-15 (custom elements /
  APIs DOM résiduelles). Signature `get`/`append` `of Undefined` =
  matrice §B-14 à alimenter.
- Scroll ×2 : `scroll_y` reste 0 (doc_h 0).

## 3. Windy — https://www.windy.com — §B-4 **TOUJOURS PRÉSENT (bras « modules+importmap inertes »)**

- **Titre** : « Windy: Wind map & weather forecast » — doc_h 0 →
  shell SSR, DOM 10,3 Ko / 108 nœuds, layout 40 boîtes, heap JS
  8,7 Mio, `journal_fallbacks=1`.
- **Frame** `captures/vol43_windy.png` (scroll identique) : fond
  gris carte + logo « Windy.com » centré-haut + attribution
  « © OpenStreetMap contributors » en bas à droite — le **shell
  statique peint**, l'app carte (leaflet-gl + index.js) jamais
  montée : squelette `data-plugin="…"` présent dans le DOM
  (`logs/vol43_windy_dom.txt` l. 230+) mais aucun plugin
  initialisé.
- **Console** : « pas de journal ».
- **Traçage fin** : **1 seule éval** (`ok=true`, 944 o — un inline ;
  l'inline `window.W={…}` avec `preparedFetchRequests` est présent
  dans le DOM). Les **2 scripts `type="module"`** (`leaflet-gl.js`
  `async`, `index.js`) et l'**importmap** (`@windyCore`→`index.js`,
  `@leafletGl`→`leaflet-gl.js`, `logs/vol43_windy_dom.txt` l. 56-70)
  ne produisent **ni eval, ni WARN fetch, ni erreur** — et le heap
  8,7 Mio (vs 34,8 Mio archive) suggère que le graphe de modules
  n'a **pas** été instancié.
- **Caractérisation** : bras « modules jamais drainés » — recoupe
  §B-5/J3 (spec non extrait d'import map : les specifiers
  `@windyCore`/`@leafletGl` passent par l'import map ; les deux
  `src` directs `type="module"` eux-mêmes restent sans trace → le
  drain des modules top-level est le suspect premier, pas seulement
  la résolution de specifiers). Cible verbatim de la ligne §B-4
  (« windy/flightradar — modules+importmap inertes ») : confirmé
  inchangé en `[V4-main]`.
- Scroll ×2 : `scroll_y` reste 0.

---

## Synthèse §8

- **`[V4-main]`** : **TOUJOURS PRÉSENT** §B-4 sur twitch et windy ;
  **DÉGRADÉ EN MIEUX** sur archive.org (mort fonctionnellement
  identique — page blanche — mais **tracée** en console, fin du
  silence pour le bras « exception post-instanciation »).
- **Lecture transverse de §B-4 — trois bras désormais
  discriminés** :
  1. **Modules jamais drainés, zéro trace** (windy : 1 eval inline,
     2 `type="module"` + importmap inertes, heap 8,7 Mio) — recoupe
     §B-19/J7 ordonnancement + §B-5/J3 MapLoader.
  2. **Exception post-instanciation — désormais tracée**
     (archive.org : `unhandledrejection` verbatim en console, heap
     34,8 Mio). Le bras (b) du vol 38 n'est **plus silencieux** —
     il reste fonctionnellement mort mais observable → §B-4 peut
     perdre cette sous-catégorie au profit des entrées §B-14
     (`append`/`get` of Undefined).
  3. **Drain + éval réussis, mort sans rejet ni trace — bras
     nouveau** (twitch : 44/44 `ok=true`, 0 WARN). Ni le disjoncteur
     ni `unhandledrejection` ne voient cette mort : le code a
     *réussi* sans produire l'effet attendu. C'est le bras le plus
     dur : l'instrumentation à poser serait un compteur
     « évals sans mutation DOM significative » (T2 `evals_sans_mutation`
     — ici justement ~44 évals pour 0 croissance du `#root`).
- **Points d'attention pour SYNTHESE** :
  - §B-4 — reformuler en trois bras : (a) drains modules/defer
    inertes (windy — importmap : le drain top-level lui-même semble
    absent, pas seulement la résolution de specifiers) ; (b)
    exceptions post-instanciation **tracées** (archive.org —
    retirer du « silencieux ») ; (c) **exécution réussie sans
    effet** (twitch — nouveau, le plus opaque).
  - §B-14 — ajouter `cannot read 'append'/'get' of Undefined`
    (archive.org index-CKRP8DXx.js — custom element Lit ?).
  - §B-perf / T-note — animation CSS `infinite` ⇒ boucle
    `apply` complète ~13 Hz en continu (spinner twitch) : le
    repaint d'une `transform` animée recoûte cascade+boxtree+layout.
  - RAM : rien à signaler (546 Mio max, PID 2112 — seuil 2,5 Go
    jamais approché ; les deux autres daemons du poste non touchés).
- **Artefacts** : `captures/vol43_{twitch,twitch_scroll,archive,
  windy,windy_scroll}.png` ; `logs/vol43_{twitch,archive,windy}_
  {console,dom,layout,mem}.txt` ; journal daemon
  `logs/daemons/vol43_daemon.log`.

<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 55 — Applications web lourdes (2026-10-27)

- **Binaire** : `[V4-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (WIP `depth_limit` embarqué), `--port 9437`,
  `EXPLORER_VIEWPORT=1366x900`, journal `_vol55_daemon.log`
  (racine, copié `logs/daemons/_vol55_daemon.log`, 118 516 lignes)
- **Cibles** : www.photopea.com · app.diagrams.net · regex101.com —
  famille « vraies applications web » (canvas §B-16, SPA JS lourde,
  éditeur code)
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de
  `SYNTHESE.md`
- **RAM daemon (PID 17436)** : 375 Mo au boot → 431 Mo (photopea) →
  681 Mo (diagrams) → **908 Mo pic** (regex101) → DELETE des 3
  onglets → **408 Mo, stable +20 s — libération complète**.
  **§B-30 non reproduit** : la fuite post-DELETE de manomano
  (vol 53) ne se manifeste pas ici — cohérent avec l'hypothèse
  « tâche navigate orpheline in-flight » (ici tous les POST
  navigate ont retourné).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| www.photopea.com | **Landing ~95 % fidèle** ; éditeur injoignable | `unhandledrejection: NotSupportedError: no supported media` | **§B-31 proposé** — clic `<button onclick>` `ignored` (point d'entrée de l'app) · image non chargée au scroll |
| app.diagrams.net | **Splash SVG seul — app jamais montée** | vide (« pas de journal ») | **§B-4(a) confirmé** — 3 scripts externes (dont `defer`) jamais évalués, 0 fetch warn ; **éval `ok=false` sans entrée console** (trou d'observabilité) |
| regex101.com | **SPA montée — contenu complet, rendu dégradé** | `unhandledrejection: TypeError: 'json' is not a function (on Object)` | §B-14 matrice (fetch `Response.json`) · **§B-32 proposé** — rasters/icônes pixelisés basse résolution |

Acquis : zéro panic, zéro stack overflow, zéro wedge API, RAM
intégralement libérée au DELETE, journal 3 WARN seulement
(`journal DOM rejeté` ×3 — tous recouverts, `journal_applies:1`
partout). Deux applications lourdes sur trois montent du contenu ;
regex101 prouve qu'une SPA React-like complète peut s'afficher —
le verrou reste côté JS (APIs manquantes, scripts non drainés),
pas côté pipeline.

---

## 1. www.photopea.com — landing fidèle, éditeur verrouillé par le clic `ignored`

- `POST navigate` → titre « Photopea | Online Photo Editor »,
  `doc_h` 2958, URL finale `https://www.photopea.com/`.
- `/console` verbatim (`logs/vol55_photopea_console.json`) :
  ```
  {"console":["unhandledrejection: NotSupportedError: no supported media"]}
  ```
- `/mem` : 394 nœuds DOM, 190 boîtes, 469 display items,
  `js_heap` 527 Ko, `images_bytes` 26,8 Mo, `journal_applies:1`,
  `journal_fallbacks:0`.
- `/dom` 200 (996 lignes), `/layout` 200 (190 lignes).

### Rendu — `captures/vol55_photopea.png`

Landing **~95 % fidèle** : hero sombre + gradient violet/bleu
correct, logo Photopea, nav (Learn/Tutorials/Templates/API),
titres et corps typographiés proprement, image hero (ordinateur
portable) décodée et placée, boutons « Start using Photopea »
pleine largeur rendus à 5 endroits. À `scroll_y=1600`
(`captures/vol55_photopea_scroll.png`) : sections « State-of-the-art
AI / All Essential Features / Adjustments / Vector Graphics »
correctes — **sauf un cadre image noir en haut à gauche**
(~520×240 px) : image absente/undécodée (promo screen WebP ?),
la seconde image (mockup 3D) étant correctement peinte.

### §B-31 (proposé) — clic `<button onclick=…>` → `ignored`

- `<button>` « Start using Photopea » = `onclick="addPP()"`
  (DOM ligne 595), boîte `[27.32 344.4 1311.36x59.39]` — le clic
  `{"type":"click","x":683,"y":374}` (intérieur strict de la boîte,
  `scroll_y=0`) → **`{"result":"ignored"}`**.
- Le même vol prouve que le hit-test fonctionne : clic sur
  `<a href="//www.photopea.com/tuts" target="_blank">` (x=300,
  y=42) → **`{"result":"navigated"}`** avec création d'un nouvel
  onglet (id=2, `can_back:true` après navigation suivante) —
  comportement `_blank` correct, navigations `location`/lien OK.
- Diagnostic : les `<button>` sans `<a>` ne sont pas dans
  l'ensemble hit-test « interactif », ou l'événement `click` n'est
  pas dispatché vers les handlers JS `onclick`. Conséquence :
  **le point d'entrée de l'éditeur Photopea est injoignable** —
  toute app dont l'action primaire est un `<button onclick>` /
  `addEventListener('click')` est bloquée (à distinguer de §B-13
  RÉSOLU qui concernait des liens sous overlays, et de §B-17/18
  « élément sous overlay » : ici aucun overlay dans le layout).
  Crate suspecté : `explorer-core` (dispatch événement) /
  `explorer-layout` (hit-test). **NOUVEAU — `[V4-main]`**.

### Note JS

`js_heap` 527 Ko seulement : la page d'accueil est peu
scriptée ; `addPP()` (injection de l'app + `pp.js` ~10 Mo) n'a
jamais été déclenché. `NotSupportedError: no supported media` =
feature-detect media probable, rejet proprement tracé.

## 2. app.diagrams.net — scripts externes jamais évalués, silence total

- `POST navigate` → titre « Flowchart Maker & Online Diagram
  Software », `doc_h` **900,0** (=viewport), URL
  `https://app.diagrams.net/`.
- `/console` → **`pas de journal`** (ring vide).
- `/mem` à +15 s puis +45 s : 77 nœuds DOM, 4 boîtes, 6 display
  items, `js_heap` **13 364 129 o figé à l'octet près** entre les
  deux relevés, `journal_applies:1`, `journal_fallbacks:1`
  (WARN daemon `NoBaseline` — recouvert).
- Rendu (`captures/vol55_diagrams.png`) : **logo drawio SVG
  72×72 centré en haut sur fond gris clair — c'est tout**. Layout :
  `html` 1366×0 → `body` → `div` 1366×900 → `svg`.
- Scroll ×2 → `scroll_y:0` (doc_h=900, attendu sur splash fixe).

### §B-4(a) confirmé — drains scripts inertes `[V4-main]`

Le DOM référence **3 scripts externes** : `js/app.min.js`
(`defer`), `js/bootstrap.js`, `js/main.js`. Le journal daemon
(118 k lignes) ne contient **aucun** `script: eval` pour un
fichier >13 Ko sur tab=2 — seulement 4 évals inline minuscules :

```
script: eval… tab=2 bytes=829    → ok=true
script: eval… tab=2 bytes=288    → ok=true
script: eval… tab=2 bytes=12982  → ok=true
script: eval… tab=2 bytes=447    → ok=false   ← aucune entrée console
```

Aucune warn fetch script (le traçage FIX-3 couvre les *échecs
réseau*, pas l'absence de déclenchement). Même signature que
« defer Next non exécutés » (GFG §B-7) et « pinterest ~70
scripts 0 éval » : **les `<script src>` externes sont présents
dans le DOM mais jamais passés à l'éval**, sans erreur. Le cas
`defer` (app.min.js) + non-defer (bootstrap/main) mélangés suggère
que ce n'est pas le drain defer seul mais la collecte/ordonnancement
des scripts externes qui saute.

**Trou d'observabilité associé** : l'éval `ok=false` (447 o) n'a
produit **aucune** entrée `/console` — une éval qui échoue sans
remonter au ring est exactement le cas aveugle que §B-4 cherche à
éliminer. À rattacher à §B-4 ou à ouvrir comme micro-anomalie
(« `ok=false` non tracé »).

## 3. regex101.com — SPA montée, `Response.json` manquant, rasters pixelisés

- `POST navigate` → titre « regex101: Online Regex Tester, Builder,
  Converter & Debugger », `doc_h` 900,0 (app viewport-fixe —
  conforme).
- `/console` verbatim (`logs/vol55_regex101_console.json`) :
  ```
  {"console":["unhandledrejection: TypeError: `json` is not a function (on Object)"]}
  ```
- `/mem` : **977 nœuds DOM, 728 boîtes, 551 display items**,
  `js_heap` 29 Mo, `journal_fallbacks:3` (×2 `UnmappedKey` WARN —
  recouverts).
- `/dom` 2972 lignes, `/layout` 728 lignes.

### Rendu — `captures/vol55_regex101.png`

**L'application est montée** : sidebar complète (Manage Workspace,
Save & Share, Publish to Library, Editor — Flavor: PCRE2 (PHP),
Match ✓, Substitution, Extraction, Unit Tests, Tools, Code
Generator, Regex Debugger/Converter/Export/Benchmark/Format avec
badges « New »), zone centrale « Regular Expression » avec champ
de saisie et flags `/…/gm`, panneau droit « Explanation » avec
tout le texte descriptif et la liste des flavors supportés
(PCRE2, JavaScript, Python, Golang, Java, .NET 10.0, Rust,
POSIX ERE/BRE, PCRE Legacy). Scroll ×2 → `scroll_y:0`
(app fixed-height, normal).

### §B-14 — `json is not a function` (fetch `Response.json`)

`unhandledrejection: TypeError: 'json' is not a function (on Object)`
— un `response.json()` (ou équivalent) rejeté : la méthode `json`
de `Response`/objet fetch n'existe pas ou l'objet reçu n'est pas
une Response. → recoupe la matrice §B-14 (APIs restantes,
`explorer-jscore`). Non bloquant ici (l'app a monté — rejet dans
une branche secondaire, probablement fetch config/analytics).

### §B-32 (proposé) — rasters/icônes rendus en pixel-blocs basse résolution

Sur la frame : le logo en haut à droite, l'icône engrenage, et
surtout la **zone éditeur centrale** apparaissent comme des formes
de **pixels noirs grossiers** (rond crénelé ~200 px, rectangles
blocky) au lieu de glyphs/images nets. Les petites icônes sidebar
sont partiellement reconnaissables mais crénelées. Hypothèses :
image décodée à résolution intrinsèque minuscule puis étirée
(`image-rendering`/`width` CSS), glyphe icon-font rasterisé trop
petit, ou canvas/svg fallback. Distinct de §B-27 (mosaïque
répétée plein écran) — ici ce sont des **blocs pixelisés
localisés**. Crate `explorer-render`/`layout`.
**NOUVEAU — `[V4-main]`, certitude moyenne** (à confirmer en
comparant à une référence Chrome — `compare_render.ps1`).

### À nuancer — thème

`<html data-theme="light">` → rendu clair. Le défaut regex101
réel est foncé, mais le thème est persisté côté client
(localStorage) — **non retenu comme anomalie** : sur session
fraîche, clair peut être le défaut légitime.

## Statuts proposés (§8)

| §B | Bras | Verdict ce vol |
|---|---|---|
| §B-31 (proposé) | **clic `<button onclick>` → `ignored`** — dispatch click/handlers JS absent du hit-test, bloque l'entrée des apps bouton-primaires | **NOUVEAU `[V4-main]`** — photopea, crate `explorer-core`/`layout` |
| §B-4(a) | drains scripts inertes | **confirmé `[V4-main]`** — diagrams.net : 3 scripts externes 0 éval, js_heap figé 13,3 Mo, 0 console ; + trou associé : éval `ok=false` non remontée au ring |
| §B-14 | matrice APIs | +1 verbatim — `unhandledrejection: TypeError: 'json' is not a function (on Object)` (regex101, fetch `Response.json`) |
| §B-32 (proposé) | rasters/icônes pixelisés basse résolution | **NOUVEAU `[V4-main]`** — regex101 (certitude moyenne, à confronter référence) |
| §B-30 | fuite post-DELETE | **NON REPRODUIT** — 908 → 408 Mo stable après DELETE ×3, tous les navigate ayant retourné (renforce l'hypothèse « tâche navigate orpheline ») |
| §B-16 | canvas lourd | **non testé** — l'éditeur Photopea exige le clic bouton (§B-31) ; landing SSR seul vue |

## Hygiène de vol

- Onglets 1 (photopea), 2 (diagrams — créé par le clic `_blank`
  puis réutilisé), 3 (créé puis supprimé vide), 4 (regex101)
  fermés par `DELETE /api/tabs/{id}` → `{"tabs":[]}`.
- Daemon tué par son **PID 17436** ; daemons voisins (PID 3444,
  PID 18264, autres vols parallèles) non touchés.
- Artefacts : `captures/vol55_{photopea,photopea_scroll,diagrams,
  regex101}.png` ; `logs/vol55_{photopea,diagrams,regex101}_
  {console.json,mem.txt,dom.txt,layout.txt}` ;
  `logs/daemons/_vol55_daemon.log`.
- NB : PID 16108 (~4,3 Go, relique d'un vol antérieur — famille
  §B-30) présent au lancement, disparu en cours de vol ; non
  touché par ce vol.

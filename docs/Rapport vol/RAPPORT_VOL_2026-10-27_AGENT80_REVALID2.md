<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 80 : photopea · developer.chrome · speedtest `[V7-main]`

- **Binaire** : `[V7-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (18:30, contient les fix post-`[V6-main]` dont le hit-test
  `<button onclick>` et les APIs navigateur).
- **Mission** : 3 cibles de re-validation — photopea.com (§B-31 :
  clic `<button onclick>` accepté ? éditeur joignable ?) ·
  developer.chrome.com (§B-38 : item flex 5 839 px corrigé ?) ·
  speedtest.net (§B-14 : `userAgentData` présent ? sonde serveur ?).
- **Daemon** : PID 18708, port 9463, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol80_daemon.log` (racine dépôt). **RAM max ~1 486 Mo**
  — juste sous le seuil de prudence 1,5 Go (3 onglets SPA/JS
  vivants : photopea app, devsite, speedtest app).
- **Fenêtre** : 17:31:37 → 17:42:40 UTC (~11 min).
- **Verdict global** : **§B-31 RÉSOLU** — le clic bouton `onclick`
  est accepté (`edited`, handler exécuté) et **l'éditeur Photopea
  monte entièrement** (menus, barre latérale, zone de drop, badges
  formats). **§B-38 TOUJOURS PRÉSENT à l'identique** (flex 5 839 px,
  nav expulsée x=5 879, logo-ellipse) + **§B-40 reproduit** sur la
  même cible. **speedtest : `userAgentData`/`getHighEntropyValues`
  désormais implémenté** (rejet console disparu), sonde serveur
  toujours bloquée.

---

## 1. photopea.com — §B-31 RÉSOLU : clic accepté, éditeur complet

`POST /navigate` → **0,6 s** — titre « Photopea | Online Photo
Editor », `doc_h=2 958`, 0 nav auto.

**Rendu accueil** (`captures/vol80_photopea.png`) : landing complète
et fidèle — header (logo, Learn/Tuturials/Templates/API), hero
« Free Online Photo Editor », bouton pleine largeur « Start using
Photopea », sections Fully Local / Cost-Effective / Convenient
Editor / Runs Everywhere, illustration laptop peinte.

**Clic §B-31 — le test** : `<button onclick="addPP()">` layouté
`block<button> [27.32 344.40 1311.36×59.39]` (layout l.32) — clic
centré **(683,374)** → **`edited`** (au vol 55 `[V4-main]` :
`ignored` avec boîte identique 1311×59). **Le handler `addPP()`
s'est exécuté** : après ~6 s la frame
(`captures/vol80_photopea_click.png`) montre **l'application
Photopea montée de bout en bout** — barre de menus
File/Edit/Image/Layer/Select/Filter/View, sous-barre
Window/More/Account, rail latéral complet (Home, This Device,
PeaDrive, Dropbox, OneDrive, Google Drive, Pick any File, PeaGames,
Photopea, Vectorpea, Jampea — icônes dessinées), accueil éditeur
avec « New Project / Open From Computer / Templates / Generate /
Video? », zone « Drop any files here », badges de formats
(.PSD .AI .XD .FIG .sketch .PDF RAW ANY). `dom_nodes` 394→537,
`journal_applies` 1→6. **L'éditeur est joignable — §B-31 RÉSOLU.**

**Clic profond bonus** : « New Project » (287,228) → `edited`,
frame inchangée (pas de boîte de dialogue visible — probablement
géré en canvas/overlays internes de l'app ; livré au DOM).

**`/console` verbatim** (`logs/vol80_photopea_console2.json`) —
les feature-detect de l'app, cascades honnêtes :
- `unhandledrejection: NotSupportedError: no supported media`
  (×25) — `decodeURI`/media probes de l'éditeur ;
- `unhandledrejection: CompileError: WebAssembly unsupported`
  (×4) — pas de WASM dans jscore, attendu.
L'app boote **malgré** ces absences — dégradation gracieuse côté
site.

**Télémétrie** (`logs/vol80_photopea_mem2.json`) :
`dom_nodes=537`, `layout_boxes=166`, `display_items=164`,
`images_bytes=43 524 004`, `journal_fallbacks=2` (1 re-parse
`UnmappedKey` journalisé 17:33:18, récupéré — apply `fin ms=119`
juste avant).

## 2. developer.chrome.com — §B-38 reproduit verbatim + §B-40

`POST /navigate` → **4,2 s** — « Chrome for Developers »,
`doc_h=8 188`.

**§B-38 — signature identique au vol 58** (`logs/vol80_chrome_dev_layout.txt`
l.14-15) : l'item flex logo `block<div> [24 6 5839×36]` toujours
layouté à **5 839 px**, la nav `block<div> [5879 0 519.57×48]`
expulsée à **x=5 879** — hors viewport. Frame
(`captures/vol80_chrome_dev.png`) : logo Chrome en **ellipse
aplatie** multicolore en haut à gauche, nav invisible (deux
bavures clippées à droite). Le reste de la page est correct :
hero « A Powerful Web. Made Easier. », carte I/O 2026, sections
suivantes au scroll (`vol80_chrome_dev_scroll.png`, `scroll_y=800`
sain, doc 8 188 px). **Statut : TOUJOURS PRÉSENT `[V7-main]`** —
le motif est le même qu'au vol 58 ; le fix §B-49 (`741f810`,
enfants `absolute` exclus des items flex) n'a pas couvert ce cas
(ici l'item fautif n'est pas positionné : c'est la contribution
intrinsèque/« base » du premier item qui gonfle — à confirmer par
repro minimal).

**§B-40 reproduit en bonus** : ancre « Find out more »
`block<a> [32 1137.21 94.08×20]` (layout l.120). À `scroll_y=800`
→ position viewport (75,345) **dans la boîte** → **`ignored`**.
Retour `scroll_y=0` : clic carte I/O (540,660) ∈ `block<a>
[32 383.2 1077×606]` → `edited` **puis navigation effective**
vers `https://developer.chrome.com/blog/chrome-at-io26`
(`/api/tabs` : `can_back:true`, titre article complet — la
réponse `edited` précède la nav async). Frame post-nav
`vol80_chrome_io26.png` : article rendu intégral (auteurs avec
avatars photo, corps de texte — superbe). Même signature §B-40
que trainline vol 60 : **ancre réelle `ignored` en position
scrollée, navigation OK à scroll_y=0** — décalage scroll dans le
hit-test confirmé sur un 2ᵉ site.

**`/console` verbatim** : `pas de journal` — zéro erreur JS sur
devsite (SSR complet, scripts inertes).

**Télémétrie** (`logs/vol80_chrome_dev_mem.json`) :
`dom_nodes=2 466`, `layout_boxes=691`, `display_items=559`,
`js_heap=3 694 440`, `journal_fallbacks=3`.

**Détail transitoire** : `/frame` a répondu 204 une fois juste
après le scroll (apply en vol — alternance documentée, retenté →
200).

## 3. speedtest.net — `userAgentData` arrivé ; sonde serveur toujours bloquée

`POST /navigate` → **6,4 s** — « Speedtest by Ookla — The Global
Broadband Speed Test », `doc_h=2 266`.

**Rendu** (`captures/vol80_speedtest.png`) : app montée comme au
vol 78 — header SPEEDTEST, bascule thème, onglets Results/Settings,
état « Finding provider / Finding optimal server… », GO dégradé
cyan→bleu, Connections. Scroll `scroll_y=800` sain
(`vol80_speedtest_scroll.png` : cartes Desktop Apps / Troubleshoot
Wifi / Outages / ISP / QR mobile apps / Global Index / Performance
Directory / Ookla Research — images riches nettes).

**`userAgentData` — signature disparue** : `/console` verbatim
(`logs/vol80_speedtest_console.json`) = **2 entrées seulement**,
contre 4 signatures au vol 78 :
- `unhandledrejection: TypeError: \`push\` is not a function (on
  Num(1.0))` — apstag.js (persistant) ;
- `uncaught: TypeError: cannot set \`amazonStatus\` of Undefined`
  (×2).
- **ABSENT : `'getHighEntropyValues' is not a function (on
  Object)`** ×2 du vol 78 — `navigator.userAgentData` et sa méthode
  existent désormais dans jscore : la feature-detect UA-CH d'Amazon
  s'exécute sans jeter. **Sous-entrée §B-14 → résolue terrain
  `[V7-main]`** (retrait possible de la matrice).

**Sonde serveur — inchangée** : « Finding provider / Finding
optimal server… » persiste **≥35 s** (DOM re-dumpé : état identique
— `logs/vol80_speedtest_dom2.txt` vs `_dom3.txt` : diff 0 ligne).
L'XHR temps réel Ookla n'aboutit toujours pas — même constat qu'au
vol 78, famille fetch/réseau-jscore (à rapprocher N2 timeouts et
géoloc §B-14).

**Clic GO (612,541)** → `edited` : l'app répond — les options
**« Multi | Single » deviennent visibles** en tête au scroll
(`vol80_speedtest_scroll.png`, peintes sous la barre — état React
muté, DOM de vue inchangé car les nœuds existaient déjà masqués).
Interactivité confirmée.

**Observabilité — nouveautés V7** :
- les entrées `/console` portent désormais des **traces de pile**
  complètes (`at <anonymous> (…apstag.js:3:18597)` — progrès net
  vs vol 78, lignes nues) ;
- **résidu §B-47** : le suffixe dédup `×2` est émis **`Ã—2` dans la
  charge JSON elle-même** (`(\u00c3\u20142)` dans le fichier brut) —
  le fix `8ddd26d` (charset déclaré) ne peut pas corriger un texte
  corrompu à la source : la chaîne dedup est double-encodée côté
  serveur. À compléter dans §B-47 (correctif partiel).

**Borne §B-26 tirée ×1** : `cycle de report cassé dropped=1
path=[0,0,0,0,0,0]` à 17:37:51 — même chemin qu'au vol 78, filet
nominal, apply `fin ms=190` juste après.

**Télémétrie** (`logs/vol80_speedtest_mem.json`) :
`dom_nodes=1 120`, `layout_boxes=609`, `display_items=264`,
`js_heap=36 172 717` (~34 Mio — app vivante),
`images_bytes=23 829 108`, `journal_fallbacks=3`.

---

## Synthèse des constats

| Cible | Résultat | IDs §B | Crate |
|---|---|---|---|
| photopea.com | **§B-31 RÉSOLU** — clic `onclick` accepté (`edited`), handler exécuté, **éditeur complet monté et joignable** | §B-31 → RÉSOLU `[V7-main]` ; §B-14 terrain (media/WASM absents, app dégrade proprement) | `explorer-core`/hit-test ; `explorer-jscore` |
| developer.chrome.com | §B-38 **verbatim** (flex 5 839, nav x=5 879, logo-ellipse) ; §B-40 **reproduit** (`ignored` ancre réelle à scroll_y=800 vs nav OK à 0) | §B-38 TOUJOURS PRÉSENT ; §B-40 2ᵉ porteur | `explorer-layout`/`css` ; `explorer-core`/hit-test |
| speedtest.net | `userAgentData`/`getHighEntropyValues` **présent** (rejet disparu) ; sonde serveur bloquée ≥35 s ; GO → Multi/Single révélés ; traces de pile console ; suffixe `Ã—2` résiduel | §B-14 (sous-entrée UA-CH résolue ; sonde → N2/géoloc) ; §B-47 résiduel (dedup corrompu à la source) | `explorer-jscore` ; `explorer-core`/`api` |

### Mises à jour de statut suggérées (SYNTHESE.md)

- **§B-31** : `OUVERT` → **RÉSOLU `[V7-main]` vol 80** — clic
  `<button onclick="addPP()">` → `edited`, handler exécuté, app
  Photopea complète montée (preuve `vol80_photopea_click.png`).
- **§B-38** : annoter « reproduit verbatim `[V7-main]` vol 80 —
  le fix §B-49 `741f810` ne couvre pas ce motif (item fautif non
  positionné : contribution intrinsèque/base flex gonflée, nav
  poussée hors viewport) ».
- **§B-40** : annoter « 2ᵉ porteur `[V7-main]` vol 80 —
  developer.chrome.com « Find out more » `[32 1137 94×20]` →
  `ignored` à scroll_y=800, nav OK à scroll_y=0 ».
- **§B-14** : rayer `navigator.userAgentData`/`getHighEntropyValues`
  (présent `[V7-main]` vol 80 — rejet console disparu) ; laisser
  « sonde serveur speedtest » dans la famille N2/géoloc.
- **§B-47** : annoter « résiduel `[V7-main]` vol 80 : charset OK
  mais le suffixe dédup `×` est émis `Ã—` **dans la charge JSON**
  — corruption à la source dans la génération du suffixe dedup,
  pas au transport ».
- **§B-26** : +1 tirage nominal (`dropped=1 path=[0,0,0,0,0,0]`
  speedtest — même path que vol 78).
- **Observabilité console** : stack traces désormais jointes aux
  entrées — acquis `[V7-main]` à noter.

## Artifacts

- Captures : `captures/vol80_photopea.png`,
  `vol80_photopea_click.png` (éditeur monté — preuve §B-31),
  `vol80_photopea_newproj.png`, `vol80_chrome_dev.png`,
  `vol80_chrome_dev_scroll.png`, `vol80_chrome_io26.png`,
  `vol80_speedtest.png`, `vol80_speedtest_go.png`,
  `vol80_speedtest_scroll.png`.
- Logs : `logs/vol80_{photopea,chrome_dev,speedtest}_{console.json,
  dom.txt,layout.txt,mem.json}` (+`_dom2/_dom3`, `_console2`,
  `_mem2` photopea post-clic ; speedtest pré/post-GO) ; journal
  daemon `_vol80_daemon.log` (racine dépôt — ~40 k lignes
  `sync_profile`).
- Arrêt propre : `DELETE` onglets 1-3 puis kill du PID 18708.

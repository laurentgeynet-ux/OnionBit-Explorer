<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 72 : voyage & metabookers `[V6-main]`

- **Binaire** : `[V6-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (contient le fix §B-26 `1e24942`, borne `emit_scoped_at` — non tirée
  sur ce vol).
- **Mission** : 3 cibles voyage — expedia.fr, lastminute.com,
  **kayak.fr en re-vol** du wedge historique (vol 24 : éval ~90 s
  avant l'existence du disjoncteur). Cibles prudence
  §B-26/§B-30/§B-41/§B-43/§B-46 évitées.
- **Daemon** : PID 15348, port 9454, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol72_daemon.log`. **RAM max ~1,55 Go** — seuil de
  prudence 1,5 Go franchi pendant lastminute → protocole appliqué
  (DELETE des 2 onglets, **purge complète à 440 Mo**, daemon sain
  conservé pour la 3ᵉ cible). RAM finale ~440 Mo.
- **Fenêtre** : 14:33 → 14:45 UTC (~12 min).
- **Verdict global** : **3/3 cibles servies, 0 wedge, 0 fuite** —
  expedia : rendu correct, scroll OK, clic = vraie navigation
  (/trips, réponse HTTP 429 externe) ; lastminute : rendu partiel
  (~50 %), JS tiers en échec, **RAM cumulative >1,5 Go** → garde-fou
  puis purge saine ; kayak : **le wedge historique est contenu par
  le disjoncteur** — une micro-éval de 285 o a tourné 60 s →
  `ok=false`, disjoncteur ouvert, `navigate` retourné en 65,3 s,
  page SSR + modale d'erreur *du site* peinte (**proposition §B-47**).

---

## 1. expedia.fr — rendu ~65 %, navigation réelle validée

`POST /navigate` → **200 en 5,6 s** (title « Expedia.fr Voyages:
Hôtels, Locations de Voiture, Activités & Vols pas chers », `doc_h`
2 212, non clampé). Console **vide** (« pas de journal »).
Télémétrie : 652 nœuds, 402 boîtes, js_heap 36 Mio
(`logs/vol72_expedia_mem.json`).

**Rendu** (`captures/vol72_expedia.png`) : header avec logo Expedia,
nav, carte de recherche complète — onglets Séjours/Vols/Voitures/
Formules/Activités avec icônes peintes, champ destination, dates
« mar. 3 nov. - jeu. 5 nov. », « 2 personnes, 1 chambre », case
« Réservez plus, payez moins* ». Le hero (image) est absent — fond
blanc (hydratation/lazy → famille §B-4/§B-20).

**Anomalies** :

- **Runs inline superposés dans la nav** : « Réductions voyages » et
  « Référencier votre annonce » peints l'un sur l'autre en haut de
  page → **recoupe §B-18** (`explorer-layout`, même signature que
  lemonde/ouest-france vol 48).
- **Contenu scrollé = squelettes gris** : à scroll_y=1 312, la frame
  (`captures/vol72_expedia_scroll.png`) ne montre que des barres
  shimmer (skeleton loaders) — le contenu réel sous le fold n'a
  jamais hydraté → famille §B-4 (ou lazy §B-20).

**Interactions** : scroll ×2 → `scroll_y` 800 puis **1 312,5**
(butée doc_h). **Clic « Assistance » (700,30) → `navigated` vers
`/trips`** — vraie navigation, `bfcache_pages=1`. La destination
répond **HTTP 429** (rate-limit serveur — la rafale de
sous-ressources du chargement a probablement épuisé le quota
anti-bot) : page d'erreur moteur propre « Page inaccessible — Le
serveur a répondu « HTTP 429 » » (`captures/vol72_expedia_click.png`,
`logs` : `{"console":["HTTP 429"]}`). Clic et dégradation
**nominaux** — hors moteur pour le 429.

Journal : ~10 évals tab=1, une `ok=false` à 198 ms (script 507→
non — script court, échec silencieux non tracé en console → §B-4).

## 2. lastminute.com — rendu ~50 %, **RAM >1,5 Go → garde-fou, purge saine**

`POST /navigate` → **200 en 8,0 s** (title correct, `doc_h` 6 916,
non clampé). Télémétrie archivée **en cours d'apply** :
`dom_nodes=0`, `js_heap=101 Mio`, `journal_fallbacks=1` (WARN
`journal DOM rejeté → repli re-parse (ADR-0025) why=UnmappedKey` —
repli propre) ; `/layout` → 404 transitoire (documenté
vol-agent §3 : DOM détaché pendant apply — non archivé).

**Console** (`logs/vol72_lastminute_console.json`) — signatures
**§B-14** (matrice APIs DOM) + réseau :

```
unhandledrejection: TypeError: `includes` is not a function (on Object)
  (hub-static.wayndata.com script-chunk) ×3
unhandledrejection: Error: script load problem: googletagmanager.com/gtm.js
unhandledrejection: TypeError: `json` is not a function (on Object)
unhandledrejection: fetch: network error (×2, ×4, ×6, ×8)
unhandledrejection: Error: Request failed with status code 400:
  api-view.wayndata.com/.../microsites ×3
```

`` `json` is not a function `` = même signature que regex101
`'json' is not a function` (§B-14, vol 55 — `Response.json`
absent) ; `includes` idem famille. Les `fetch: network error` en
escalade (×2→×8) suggèrent des rejets en cascade (CSP/CORS ou
fetch partielle) — tracés, donc pas §B-4 silence.

**Rendu** (`captures/vol72_lastminute.png`) : header magenta fidèle
(logo, « Flight + Hotel », « Call to book »), hero quasi vide
(widget de recherche = 3 boîtes vides + tiret rose), **vraies
photos peintes** en bas de viewport (plage, couple, portrait) —
boot JS partiel, contenu SSR partiel (~50 %).

**Incident garde-fou** : pendant le grâce 9 s post-nav, la RAM
daemon est passée ~870 → **1 540 → 1 547 → 1 551 Mo** (croissance
lente ~4 Mo/8 s) — API toujours responsive, apply complets jusqu'à
`repaint` (pas de signature §B-26/§B-33/§B-41). Protocole appliqué :
**DELETE tab 1 + tab 2 → purge complète et immédiate à ~440 Mo,
stable 20 s** → second cas de purge saine post-DELETE après orsay
vol 67 — **signal §B-30 favorable** (la variante in-flight purge
bien ; le driver était le churn lastminute : js_heap 101 Mio +
rasters doc 6 916 px + fallback UnmappedKey).

Scroll/clic **non testés** sur cette cible (garde-fou déclenché
avant cette phase — vol écourté par prudence, pas par panne).

## 3. kayak.fr — **wedge historique contenu par le disjoncteur** (re-vol vol 24)

`POST /navigate` → **retourné en 65,3 s** (title « Recherchez des
vols, hôtels & voitures de location | KAYAK », `doc_h` **19 251 —
clamped**). Historique : vol 24 = wedge ~90 s (moteur figé,
navigate sans retour). Aujourd'hui le journal montre le
**déclencheur identique, contenu** :

```
14:38:39  script: eval fin tab=3 ms=21 ok=true   (×5 petits scripts,
          38 876/4 664/1 346/726/5 121 o — boot partiel sain)
14:39:39  script: eval fin tab=3 ms=60002 ok=false  (bytes=285 !)
14:39:39  WARN moteur figé — disjoncteur ouvert,
          appels JS court-circuités tab=3
14:39:39  WARN script sauté — disjoncteur moteur ouvert bytes=431
```

**Un script de 285 octets a tourné ≥60 s dans l'éval** — le
disjoncteur l'a tuée, les scripts suivants sautés, `navigate` a
retourné ~5 s plus tard. La signature du wedge kayak = **boucle
infinie sur micro-script** (probablement un busy-wait dont la
condition ne sort jamais sous jscore — coercion/feature-detect) →
**PROPOSITION §B-47** : « micro-script (285 o) en boucle ≥60 s
dans jscore, tuée par le disjoncteur — boot mort, navigate rend en
~65 s » (`explorer-jscore`, bras §B-1 contenu par J1/JS-C164).
Distinction §B-1 : l'API reste vivante et la page sert.

**Constat observabilité associé** : `/console` = **« pas de journal
»** alors que l'éval a été tuée — le kill du disjoncteur n'arrive
que dans le log daemon (WARN), jamais dans le ring per-tab → la
console affiche un faux « tout va bien ». À rattacher à §B-47 (ou
bras §B-4 : un onglet disjoncté devrait pousser l'événement dans
son ring console).

**Rendu** (`captures/vol72_kayak.png`) : logo KAYAK + nav,
**modale « Erreur inattendue — Nous n'avons pas pu charger cette
page »** avec boutons « Recharger la page » / « Contacter le
service à la clientèle » — vérifié dans le DOM : c'est le
**dialog d'erreur d'hydratation propre à kayak** (bootstrap inline
`APP_LOADING_ERROR` → `insertAdjacentHTML`, lignes
`logs/vol72_kayak_dom.txt` ~263) — le site a correctement détecté
la mort de son app et affiché sa surface d'erreur : comportement
**fidèle** du moteur (le bootstrap inline a tourné, le bundle a été
tué). Derrière : hero grisé avec **avion pixelisé en blocs** (→
**recoupe §B-32**, 2ᵉ cible confirmée après regex101), titre SSR
« Recherchez des vols, des hôtels et des locations de voiture »,
liste d'hôtels Montréal peinte (Samesun 30€+, Chrome 44€+…).

Télémétrie : 4 306 nœuds, 3 799 boîtes, 2 981 items, images 18,4 Mo,
js_heap 18,8 Mio, `journal_applies=17`, `journal_fallbacks=1`
(`logs/vol72_kayak_mem.json`).

**Interactions** : scroll ×2 → `scroll_y` 800 puis 1 600, frame
repeinte (`captures/vol72_kayak_scroll.png` — modale `fixed` qui
suit, liste hôtels « VOITURES • VOLS • HÔTELS » derrière).
**Clic « Vols » nav latérale (30,110, ancre `[8 90 53x40]`) →
`edited`** (« click livré au DOM — aucune action par défaut ») :
le routeur SPA a capté le clic mais l'app est morte → pas de
navigation, frame inchangée. **Comportement attendu** sous app
disjonctée — pas un bug hit-test.

## 4. Tableau de synthèse

| Cible | Navigate | Rendu | Scroll | Clic | Anomalies |
|---|---|---|---|---|---|
| expedia.fr | 200 / 5,6 s | ~65 % (hero absent) | OK ×2 (800/1 312) | **navigué** → `/trips` (HTTP 429 externe) | §B-18 runs superposés nav, §B-4/§B-20 squelettes, 429 hors moteur |
| lastminute.com | 200 / 8,0 s | ~50 % (header fidèle, photos OK) | — (garde-fou) | — (garde-fou) | §B-14 (`includes`/`json` not a function), fetch ×20, **RAM >1,5 Go → purge saine** (signal §B-30 favorable) |
| kayak.fr | 200 / **65,3 s** | SSR ~60 % + modale erreur *du site* | OK ×2 (800/1 600) | `edited` (app morte, nominal) | **§B-47 proposé** (micro-éval 285 o boucle ≥60 s → disjoncteur) + §B-32 avion pixelisé + kill muet dans `/console` |

## 5. Observations annexes

- **Disjoncteur validé sur le porteur historique** : kayak n'a
  plus de wedge — la même pathologie (script infini) est bornée à
  60 s + scripts suivants sautés. Le vol 24 est officiellement
  clos côté « blocage » ; le résidu est la racine jscore (§B-47).
- **Purge post-DELETE saine** : 1,55 Go → 440 Mo instantané, API
  vivante — deuxième confirmation post-fix que la fuite §B-30 ne
  se déclenche pas sur ces variantes.
- **`/layout` 404 transitoire** sur lastminute pendant apply —
  conforme au protocole documenté (vol-agent §3), retente possible.
- **`doc_h_clamped`** sur kayak (19 251 > plafond) — métrique
  honnête (T4).
- **Aucun `cycle de report cassé` (§B-26)** sur les 3 cibles ;
  `journal_fallbacks` : 1 (lastminute `UnmappedKey`), 1 (kayak) —
  re-parse ADR-0025 propre à chaque fois. Aucun panic (§B-37 non
  reproduit).
- **HTTP 429 expedia** rejoint la signature externe anti-bot
  (rate-limit déclenché par le volume de sous-ressources — la home
  elle-même avait chargé en 200).
- Expedia montre une **double lecture du DOM dans le layout** : les
  ancres de nav existent à des coords correctes — le chevauchement
  visuel est un vrai bug de peinture des runs (§B-18), pas de
  positionnement.

## 6. Arrêt

DELETE tabs 1/2 (garde-fou RAM) puis 3 → `{"tabs":[]}`. RAM finale
~440 Mo — retour baseline complet. Daemon terminé par **PID 15348**
(`taskkill /PID`, jamais `/IM`) — port 9454 libéré, vérifié
`netstat` (le daemon parallèle PID 19812 :9453, autre vol, non
touché).

**Fichiers** : `captures/vol72_{expedia,expedia_scroll,expedia_click,
lastminute,kayak,kayak_scroll}.png` (+ `kayak_click` identique à la
base — clic `edited` sans navigation),
`logs/vol72_{expedia,lastminute,kayak}_{dom.txt,console.json,mem.json}`,
`logs/vol72_expedia_layout.txt`, `logs/vol72_kayak_layout.txt`
(lastminute : layout 404 transitoire non archivé), journal
`_vol72_daemon.log`.

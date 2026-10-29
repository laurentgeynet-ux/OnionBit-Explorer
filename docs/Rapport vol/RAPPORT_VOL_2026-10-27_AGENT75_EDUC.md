<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 75 : éducation & formation `[V6-main]`

- **Binaire** : `[V6-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (contient le fix §B-26 `1e24942`, borne `emit_scoped_at` — non tirée
  sur ce vol ; disjoncteur B-41 observé à l'œuvre sur openclassrooms).
- **Mission** : 3 cibles éducation — lumni.fr, calameo.com,
  openclassrooms.com. Aucune n'avait été volée (terrain nouveau).
  Cibles prudence §B-26/§B-30/§B-41/§B-43/§B-46/§B-48 évitées.
- **Daemon** : PID 8896, port 9457, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol75_daemon.log`. **RAM max ~1 106 Mo** — sous le seuil
  de prudence 1,5 Go ; purge saine post-DELETE (→ ~406 Mo stable).
- **Fenêtre** : 14:52 → 15:02 UTC (~10 min).
- **Verdict global** : **1/3 cible fonctionnelle** — lumni.fr sert du
  contenu réel, scroll sain, **clic → navigation réelle** + bfcache ;
  calameo refusé à la couche transport (fingerprint, externe
  probable) ; openclassrooms = shell SPA non hydraté, boot mort sur
  `u is not defined` tracé, disjoncteur B-41 armé. 0 wedge, 0 crash,
  0 fuite.

---

## 1. lumni.fr — servi, dégradé §B-17/§B-18, navigation réelle

`POST /navigate` → 200 en **7,4 s**, `doc_h=7 997`, titre complet et
accentué sain (« Lumni | Des vidéos, quiz et jeux gratuits pour
réviser… » — pas de mojibake §B-47). `/console` : **« pas de
journal »** — 0 erreur JS, 0 warn sous-ressource.

**Télémétrie** (`logs/vol75_lumni_mem.json`) : `dom_nodes=1 076`,
`layout_boxes=917`, `display_items=637`, `js_heap=29,1 Mio`,
`images=6,2 Mo`, `journal_fallbacks=1` puis 2
(`journal DOM rejeté → repli re-parse ADR-0025 why=UnmappedKey` ×2 —
absorbé sans symptôme).

**Rendu** (`captures/vol75_lumni.png`) : hero « Bienvenue sur Lumni »
fidèlement peint (fond bleu, illustrations, sous-titre), barre
sociale latérale rose complète (tiktok/instagram/youtube/facebook —
icônes SVG nettes, pas de tofu PUA). **Deux anomalies visuelles** :

- **Menus peints déployés** : le méga-menu Collège étale
  « Sixième · Cinquième · Quatrième · Troisième » en ligne, Lycée
  déroule Seconde/Première/Terminale en colonne, « Primaire » peint
  son chevron ouvert — sur la page interne, la barre entière est
  re-peinte ouverte → **§B-17** (`explorer-jscore`+css : menus
  repliés par JS restent ouverts ; console vide = déplie-t-on par
  CSS `:hover`/JS absent — même famille que ratp/santemagazine).
- **Runs inline collés/superposés** : « MatemellesCPCE1CE2CM1CM2 »,
  « Mon historiqueparamètres », et sur la page interne
  « CollègeSixièmeCinquièmeQuatrièmeTroisième
  LycéeSecondePremièreTerminale » sur une seule ligne → **§B-18**
  (`explorer-layout`/`render` — chevauchements runs inline confirmés
  terrain V6).

**Interactions** : `scroll dy=800` ×2 → `scroll_y` 800 → 1 600,
**frame non blanche au scroll** (§B-8 non reproduit) — section
« Révise et prépare à fond tes examens » avec 3 cartes DNB 3e /
Bac 1re / Bac terminale correctement peintes. `click (683,679)` sur
« Je révise pour le bac » → **`{"result":"navigated"}`** : vraie
navigation vers `lumni.fr/lycee/premiere/bac` (page « Réviser Bac
Français 2027 » servie, rubriques Réviser le français /
mathématiques / sujets corrigés), `can_back=true`, **bfcache
fonctionnel** (page 1 conservée : 1 076 nœuds, heap 24 Mio).

**Classification** : **dégradé mais navigable** — SSR + pipeline
sains ; les deux anomalies visuelles recoupent le catalogue (§B-17,
§B-18), aucune proposition §B nouvelle.

## 2. calameo.com — connexion fermée pré-en-têtes — **externe probable**

`POST /navigate` → `{"error":{"handled":true,"message":"fetch :
réponse HTTP invalide : connexion fermée avant la fin des
en-têtes"}}` en **875 ms**, reproduit **2/2** (~0,9 s). `/console`
verbatim : `{"console":["fetch : réponse HTTP invalide : connexion
fermée avant la fin des en-têtes"]}`. `/dom`/`/layout` → 404
(« pas de DOM ») ; `/mem` : `layout_boxes=18`, `display_items=37`
(`logs/vol75_calameo_mem.json`).

**Rendu** (`captures/vol75_calameo.png`) : page d'erreur interne
propre — « Page inaccessible — La page https://www.calameo.com/ n'a
pas pu être chargée. Impossible de joindre le serveur (réponse HTTP
invalide : connexion fermée avant la fin des en-têtes). »

**Sondes de comparaison** : `curl --http1.1` (schannel) → **HTTP 200
en 0,1 s**, y compris avec `User-Agent: Explorer/0.1` — le refus ne
vise donc **pas l'UA** mais le fingerprint transport (rustls et/ou
H2 — Explorer négocie H2 par défaut ; curl schannel H2 non testable
sur ce poste).

**Classification** : **externe probable — refus fingerprint
TLS/H2** (`explorer-net` non mis en cause : la connexion est coupée
par le pair, signature distincte du « 403 sec » — ici aucun statut
HTTP n'est jamais reçu, famille urssaf RST mais sans repro curl).
Un bug H2 côté `explorer-net` n'est pas exclu à 100 % (une trace
de la trame déciderait), mais le serveur servant normalement en
H1.1, la classification anti-bot fingerprint est la plus parcimonie
use → **entrée externe à ajouter** : « calameo — connexion fermée
avant fin d'en-têtes (fingerprint TLS/H2), curl H1.1 OK ».

**Interactions** (page d'erreur) : `scroll dy=800` ×2 →
`scroll_y=0` (butée nominale) ; `click (683,450)` → `ignored`
(aucune cible — nominal).

## 3. openclassrooms.com — shell SPA non hydraté — §B-4/§B-14

`POST /navigate` → 200 en **3,1 s**, redirection `→ /fr/` honorée,
`doc_h=900`. `/console` verbatim :

```
unhandledrejection: ReferenceError: u is not defined
    at <anonymous> (https://cdn.oc-static.com/production/modules/sdzv4-main/latest/3000.9bb5bcfbfcc0b10c18df.js:1:12335)
    at <anonymous> (https://cdn.oc-static.com/production/modules/sdzv4-main/latest/3000.9bb5bcfbfcc0b10c18df.js:1:9303)
    at <anonymous> (https://cdn.oc-static.com/production/js/index.js?v=3:1:19162) (×2)
```

**Télémétrie** (`logs/vol75_openclassrooms_mem.json`) :
`dom_nodes=193`, `layout_boxes=13`, `display_items=4`,
`js_heap=26,8 Mio` (le runtime a réellement tourné),
`journal_fallbacks=6`.

**Rendu** (`captures/vol75_openclassrooms.png`) : **frame blanche
totale**. Le layout (`logs/vol75_openclassrooms_layout.txt`) montre
le shell : `<div>` chaînés plein viewport + boîte SVG 100×100 centrée
à (633,400) — le **spinner de chargement** de l'app (non peint ou
blanc sur blanc) — plus un `iframe` 300×150 nu à y=900. La page est
une SPA (chunk webpack `sdzv4-main`) : le boot JS est mort au rejet
`u is not defined` ×2 dans le chunk `3000.*.js` → l'hydratation ne
s'est jamais produite, le loader tourne à vide.

**Signal positif** : l'erreur est **tracée** via `unhandledrejection`
(sortie du silence §B-4 bras « exceptions post-instanciation » —
instrumentation JS-C154 fonctionnelle) — il y a deux ans de vols,
cette mort aurait été muette.

**Signal de contention** : journal daemon —
`WARN explorer_core::session::js_glue: tempête de pompes —
disjoncteur armé (B-41) tab=3 streak=64` — l'app morte retente en
rafale ; le **disjoncteur §B-41 a contenu la tempête** (API restée
responsive, RAM stable, navigate retourné en 3,1 s). Les 6
`journal_fallbacks` incluent `why=Fingerprint` ×1 + `UnmappedKey` ×5
— rappel : une rafale `Fingerprint` précédait le panic §B-37
(caniuse) ; ici contenu proprement, **0 panic**.

**Interactions** : `scroll dy=800` ×2 → `scroll_y=150` en butée
(hauteur document réelle ~1 050 px — ce n'est **pas** §B-45, le
shell est réellement court ; `doc_h`=900 rapporté vs 1 050 layout =
écart métrique mineur déjà documenté « doc_h trompeur »). `click
(683,450)` sur le spinner → `edited` (« livré au DOM ») — aucune
cible réelle, comportement attendu.

**Classification** : **§B-4 croisé §B-14** (`explorer-jscore`/
`explorer-core`) — mort de boot SPA après rejet tracé ;
`u is not defined` = référence au runtime webpack non matérialisée
(chunk exécuté avant le runtime, ou global manquant — cascade à
nommer, même famille que « scripts définisseurs jamais exécutés »).
La frame blanche recoupe en apparence §B-7 mais la cause est
différente : ici le SSR ne contient qu'un loader — pas de
`visibility:hidden` à lever (1 occurrence dans le DOM, non
concluante), l'échec est en amont (hydratation).

## 4. Tableau de synthèse

| Cible | Navigate | Rendu | Scroll | Clic | Anomalies |
|---|---|---|---|---|---|
| lumni.fr | 200 / 7,4 s | SSR réel peint, hero fidèle | 800→1 600 sain | `navigated` → `/lycee/premiere/bac`, bfcache | §B-17 menus déployés + §B-18 runs collés |
| calameo.com | **refus transport** / 0,9 s ×2 | page d'erreur propre | butée 0 | `ignored` (nominal) | externe — close pré-en-têtes, fingerprint TLS/H2 |
| openclassrooms.com | 200 / 3,1 s (→/fr/) | **frame blanche** — loader SVG seul | butée 150 (doc réel) | `edited` (aucune cible) | §B-4/§B-14 — `u is not defined` tracé ; disjoncteur B-41 armé (streak=64) |

## 5. Observations annexes

- **Stabilité exemplaire** : 0 wedge, 0 deadlock DELETE (§B-39 non
  reproduit), 0 panic (§B-37 : le `Fingerprint` OC est resté un
  WARN absorbé), purge mémoire post-DELETE complète
  1 106 → 406 Mo stable 15 s (**§B-30 non reproduit** — second
  signal favorable après orsay vol 67).
- **Disjoncteur B-41 validé terrain** : première observation de
  `tempête de pompes — disjoncteur armé` sur cible vivante —
  l'API est restée responsive pendant et après la tempête OC.
- **`UnmappedKey` re-parse** sur les 2 vraies pages (lumni ×2,
  OC ×5) — bruit ADR-0025 absorbé, déjà familier, non corrélé à
  un symptôme visible.
- **Console discriminant §B-4 vs §B-14** : OC montre qu'un rejet
  de chunk webpack **est** tracé — les morts silencieuses
  résiduelles du catalogue (pinterest, twitch) restent donc bien
  des bras distincts (absence d'erreur rapportée), non un trou
  d'instrumentation générique.
- **Titre onglet sain** : accents UTF-8 corrects dans
  `/api/tabs` sur les deux pages lumni (« Réviser Bac Français
  2027 : méthodologie, quiz et fiches de révision | Lumni ») —
  **§B-47 non reproduit** sur ce vol (fix `e268ec7`/`8ddd26d`
  dans ce binaire — signal favorable, non conclusif : le clapotis
  d'origine dépendait du cycle re-parse).
- **Proposition catalogue** : ajouter « calameo — connexion fermée
  avant fin d'en-têtes, curl H1.1 OK (fingerprint TLS/H2) » à la
  section externe (signature distincte du 403 sec, à rapprocher
  d'urssaf RST et des timeouts jprobe).

## 6. Arrêt

DELETE tabs 3/2/1 → `204` ×3, `{"tabs":[]}`, API jamais bloquée.
RAM 1 106 → **406 Mo** post-purge, stable 15 s. Daemon terminé par
**PID 8896** (`Stop-Process -Id`, jamais `taskkill /IM`) — port
9457 libéré, vérifié.

**Fichiers** : `captures/vol75_{lumni,calameo,openclassrooms}.png`,
`logs/vol75_{lumni,calameo,openclassrooms}_{dom.txt,layout.txt,
mem.json}` (calameo dom/layout = « pas de DOM »/« pas de layout » —
archivés pour traçabilité), journal `_vol75_daemon.log`.

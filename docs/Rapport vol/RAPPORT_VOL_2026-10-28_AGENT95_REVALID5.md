<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 95 : REVALID5 `[V9-main]`

- **Binaire** : `[V9-main]` —
  `C:/OnionBit-Explorer/target/release/explorer-daemon.exe`
  (horodaté 28/10 19:40, HEAD `8260b6fa` — post-Étape 148,
  post R-127). **Attention** : Étape 149 (`data-srcset`),
  Étape 150 (marqueurs NavJob), Étape 151 (bornes `event.*`
  §B-54), R-128 (bilinéaire), JS-C173/174 sont committés
  **après** ce binaire (20:40+) → leurs re-vols sont reportés
  au vol 96 (binaire frais en cours de build).
- **Mission** : REVALID5 — balayage de cibles mixtes :
  §B-12/§B-5/§B-20 (marmiton), §B-12 (bookshop), §B-6
  (reddit), §B-5 (letterboxd), §B-51 (micromania, airfrance),
  §B-23 (cryptpad), puis cibles à risque §B-41/§B-30
  (credit-agricole, manomano) et §B-46 (frandroid).
- **Daemon** : PID **18456**, port 9512,
  `EXPLORER_VIEWPORT=1366x900`, journal `_vol95_daemon.log`
  → `logs/daemons/`. Vol exécuté **sans sous-agent**
  (consigne opérateur).
- **RAM** : base ~483 Mo post-purge (tous DELETE propres
  jusqu'au wedge final — signal §B-30 favorable ×7).
- **Verdict global** : **§B-54 reproduit sur un 2ᵉ site**
  (credit-agricole : `event.poll_net` **110,8 s**, navigate
  jamais retourné, **API entière morte**, arrêt forcé par
  PID) — le wedge n'est plus spécifique à flixbus ; Étape 151
  à re-voler d'urgence au vol 96. **§B-23 reproduit sur
  cryptpad** (2ᵉ site) : `eval deadline exceeded` fige le
  boot, disjoncteur §B-41 armé en bonus (streak=64).
  **§B-12 persistant** : bookshop ×9 occurrences identiques
  de `heap memory limit exceeded` — la déduplication
  `83916a6` ne tue pas les répétitions terrain ; js_heap
  542 Mo résiduel. **§B-51 non reproduit** ×2 (micromania =
  échec unique nommé sur page Incapsula ; airfrance =
  échecs nommés à ms réels). **§B-20 favorable** : images
  marmiton peintes au scroll (repli `840a33c`) — verdict
  data-srcset réservé au binaire post-É149.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| marmiton.org | **§B-20 favorable** — photos de recettes peintes au scroll (repli `data-src` livré) ; doc_h=11 098 | `module non chargé` ×2 (→§B-5) | js_heap 51,4 Mo sain ; É149 `data-srcset` hors binaire → re-vol 96 |
| bookshop.org → uk.bookshop.org | **§B-12 PERSISTANT** — `heap memory limit exceeded (1 087 980 cells, 540 750 652 B > 536 870 912 B)` **×9 identiques** ; page néanmoins entièrement peinte | TypeError `left` nommée ; **`import() → fetch échoué : …/null` (404) ×2** | dedup `emit_pipeline_error` (`83916a6`) inopérante terrain — signature stable, répétitions non tuées ; spécificateur `import()` résolu « null » → recoupe J3 ; js_heap 542 Mo |
| reddit.com | **§B-6 non reproduit** — challenge `js_challenge=1` exécuté, homepage déconnectée peinte | `unhandledrejection: SML - Failed to load script` ×2 nommées | aucune trace `import()` sur `data:` |
| letterboxd.com | **§B-5 favorable** — modules `homepage-*.js` évalués (TypeError nommées `cssRules`, `createContext`) ; squelette peint | 229 entrées : `fetch: network error` ×13+ (compteur dedup), exceptions nommées | **anomalie rendu : texte « Wildwood (2026) » peint miroir/inversé** ; backdrops absents |
| micromania.fr | **§B-51 NON REPRODUIT** — page **Incapsula** (`noindex,nofollow` + `/_Incapsula_Resource`) ; 1 éval 103 Ko → ok=false ms=666, `TypeError: push is not a function` **nommée** | 1 entrée | page blanche = challenge anti-bot mort, pas de ré-éval silencieuse ms=0 |
| airfrance.fr → wwws.airfrance.fr | **§B-51 NON REPRODUIT** — 4 évals ok=false à ms réels (104-670) sur bundles 467-599 Ko, toutes erreurs **nommées** | `SyntaxError: invalid regex /…\p{Pat_Syn}\p{Pat_WS}…/` + regex octale `\141\164` ; TypeErrors ; `import() 404` cookie-banner | **échappements Unicode `\p{…}` non supportés par le moteur regex** — gap catalogue ? ; rendu partiel (squelettes) |
| cryptpad.fr | **§B-23 REPRODUIT** (2ᵉ site après lemonde) — `unhandledrejection: RangeError: eval deadline exceeded` → boot figé au splash (logo + « End-to-end encrypted » peints) ; **disjoncteur §B-41 armé streak=64** (2ᵉ tir terrain) | 1 entrée nommée | journal DOM rejeté ×3 (Fingerprint, UnmappedKey) ; DELETE propre, purge 483 Mo |
| credit-agricole.fr | **§B-54 REPRODUIT — 2ᵉ site** — `event.poll_net us=110 783 422` (110,8 s) ; `POST navigate` jamais retourné (>150 s) ; **API morte** (`/api/tabs`, `DELETE` sans réponse >15 s) ; log actif = livelock | — | évals pré-wedge ok (bundle 1 Mo ms=321) ; DELETE impossible → **arrêt forcé PID 18456** ; Étape 151 (`poll_net_budget` 2 s + `event_tail_budget` 15 s) à re-voler vol 96 |
| manomano.fr | **non volé** — daemon wedged par credit-agricole | — | reporté vol 96 (binaire frais) |
| frandroid.com | **non volé** — idem | — | reporté vol 96 |

## Preuves marquantes

### §B-54 — credit-agricole, wedge `event.*` non bornées

```
2026-10-28T20:08:16Z  phase sync tab=8 phase="event.poll_net"    us=110783422   (110,8 s)
2026-10-28T20:08:16Z  phase sync tab=8 phase="event.dispatch"    us=16271
2026-10-28T20:08:16Z  phase sync tab=8 phase="event.mirror_sync" us=3732
/api/tabs   → réponse vide (×2, timeouts 10-12 s)
DELETE tab8 → réponse vide (15 s) — arrêt forcé par PID
```

Signature identique au flixbus vol 93 (phases `event.*`
longues, verrou session global, livelock) → **§B-54 n'est
pas un cas isolé** : tout site à flux réseau continu peut
déclencher le wedge. Le disjoncteur §B-41 ne couvre pas ce
chemin (phases longues, pas fréquence) — la correction
Étape 151 (`poll_net_budget` 2 s, `event_tail_budget` 15 s)
est la seule borne ; re-vol prioritaire vol 96.

### §B-12 — bookshop, rafale heap non dédupliquée

```
uncaught     : RangeError: heap memory limit exceeded (1087980 cells, 540750652 B live > 536870912 B)
exception js : idem ×8   → 9 occurrences strictement identiques
js_heap_bytes = 542 750 370  (persiste après les échecs)
```

La déduplication `emit_pipeline_error` (`83916a6`, vol-ratée
J4 « une signature stable, répétitions tuées ») ne fonctionne
pas sur le terrain : le heap reste à 540 Mo et chaque éval
suivante re-tripote immédiatement avec le même message → 9
entrées. Amélioration vs ×14 historique non significative
(variance de comptage de scripts). **§B-12 OUVERT**.

Complément §B-5/J3 : `import() → fetch échoué :
https://uk.bookshop.org/null (statut 404) ×2` — le
spécificateur dynamique s'est résolu en la chaîne « null »,
confirmant le bras « spec non extrait » de J3.

### §B-23 — cryptpad, deadline au boot

Splash CryptPad peint (« Loading… », logo, « End-to-end
encrypted ») puis `unhandledrejection: RangeError: eval
deadline exceeded` — la chaîne de boot (requirejs/less)
dépasse la deadline canal ; page figée, doc_h=8. Le correctif
lemonde (canal 65 s) a re-validé vol 94 mais cryptpad montre
que **la borne reste insuffisante sur les apps à boot
multi-chaînes**. Bonus : `tempête de pompes — disjoncteur
armé (B-41) tab=7 streak=64` — le disjoncteur a contenu la
rafale, API restée vivante, DELETE propre.

### §B-20 — marmiton, images peintes au scroll

Cartes recettes et bannière entièrement photographiées au
scroll (capture `vol95_marmiton_scroll.png`) — le repli
`data-src` (`840a33c`, V8) fonctionne sur cette page.
`images_bytes` 29,9 Mo, `img_bodies` 1,3 Mo, js_heap 51,4 Mo
— aucun débordement. Verdict `data-srcset` (É149, hors
binaire) reporté au vol 96.

### §B-51 — deux non-reproductions

- **micromania** : page Incapsula (`meta robots
  noindex,nofollow`, script `/_Incapsula_Resource`) — un seul
  éval 103 Ko, échoué ms=666 avec `TypeError` nommée remontée
  à `/console`. Page blanche = anti-bot, signature §B-51
  absente.
- **airfrance** : 4 évals ok=false mais **nommés** (ms 104-670
  réels, SyntaxError/TypeError à la console) — pas de
  ré-évaluation silencieuse `ms=0`. Constat annexe : le moteur
  regex rejette les échappements de classes Unicode
  `\p{Pat_Syn}`/`\p{Pat_WS}` et la notation octale `\141\164`
  — SyntaxError nommées, dégradation propre.

### Anomalie de rendu — letterboxd

Texte d'affiche « Wildwood (2026) » peint **en miroir /
retourné** (haut de page, capture `vol95_letterboxd.png`) —
glitch de transform raster (probable `scale(-1)`/rotation
180° non composée ou flip de matrice). À rapprocher de
l'écart raster/layout déjà noté vol 93 (mégamenu à x=-344).

## Arrêt du daemon

Wedge §B-54 sur tab=8 : `/api/tabs` et `DELETE` sans réponse
(≥15 s chacun) → `Stop-Process -Id 18456` ciblé. Session
voisine :9510 (PID 12052, binaire `onionbit-vol`) intacte.
Journal archivé `logs/daemons/_vol95_daemon.log` (gitignoré,
convention `_vol*`).

## Re-vols reportés au vol 96 (binaire post-É151)

- credit-agricole + flixbus → validation `poll_net_budget`/
  `event_tail_budget` (§B-54)
- marmiton → `data-srcset`/`max_images` (§B-20, É149)
- manomano (§B-30), frandroid (§B-46)
- bookshop → compter les occurrences heap sur binaire frais
  (§B-12, J4)

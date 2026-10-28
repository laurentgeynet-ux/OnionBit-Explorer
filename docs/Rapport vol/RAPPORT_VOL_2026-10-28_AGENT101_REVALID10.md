<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 101 : REVALID10 `[V10-main]`

- **Binaire** : `[V10-main]` —
  `C:/vol100-wt/target/release/explorer-daemon.exe` (build
  `Finished`, HEAD `039f71ca` — inclut JS-C180 §B-56 bras 2,
  Étape 155 §B-51 + É154/153/JS-C179/178/177). Worktree
  propre à HEAD.
- **Commits post-binaire** : `dbd88407` (R-131 §B-58 transform
  boîtes anonymes) + sa doc `9d7a6616` — arrivés en fin de
  vol → priorité 1 vol 102 (letterboxd).
- **Mission** : REVALID10 — re-vols : §B-56 bras 2 airfrance
  (JS-C180), §B-51 twitch/sosh/edf (Étape 155), §B-32
  regex101 (R-128), §B-38 developer.chrome.com (`1c9102e`),
  §B-49 paris.fr (`741f810`), suivi §B-23 cryptpad.
- **Daemon** : PID **10308**, port 9631, viewport 1366×900,
  journal `_vol101_daemon.log` — vol entier sur un daemon,
  arrêt propre (DELETE propres ×7, RAM 444 Mo).
- **Verdict global** : **2 RÉSOLU** — §B-38 (chrome.dev :
  0 boîte expulsée, nav dans viewport) et §B-49 (paris.fr :
  page peinte vs frame blanche). **§B-51 en forte
  décroissance** : silence éliminé ×3 (sosh/edf erreurs
  nommées, twitch détecté par WARN `evals_sans_mutation
  evals=44` — bras §B-4(c) instrumenté, défaut fonctionnel
  restant). **§B-56 PARTIEL** — bras `\p{}` nié fermé
  (airfrance : classe dépréciée acceptée), bras **octal
  `\141\164`/`\100` PERSISTANT**. §B-32 persistant (regex101
  logo flou, SPA morte `Response.json` → §B-14). §B-23
  stable ×2 (cryptpad chaîne dyn non évaluée).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| airfrance.fr | **§B-56 PARTIEL** — `[^\p{Pat_Syn}\p{Pat_WS}]` **acceptée** (bras JS-C180 fermé) ; octal `\141\164[…]` + `\100` toujours rejetés | 9 entrées nommées (regex, `T is not a function`, dynatrace, `unregister token`, import 404, recaptcha iframe) | i18n non résolue (`???label???`), coquille vide — §B-51 hors bras (erreurs tracées) ; doc_h=1 730 |
| twitch.tv | **§B-51 détecté** — silence levé : WARN `evals_sans_mutation — boot possiblement muet evals=44` (É155) ; coquille blanche identique | **0 entrée** (`pas de journal`) — WARN daemon seulement, pas routée au ring | 44 évals `ok=true` (bundle 918 Ko, 1 417 ms) **0 mutation DOM** — §B-4(c) confirmé : React jamais monté ; js_heap 7,3 Mo |
| sosh.fr | **§B-51 RÉSOLU (bras silence)** — éval 863 Ko `ok=false ms=132` **nommée** (`value is not a function: undefined` hptu.….js) vs silence vol 85 | 1 entrée nommée avec src verbatim | coquille vide persistante doc_h=101 — défaut fonctionnel §B-14 (`value is not a function` webpack) |
| particulier.edf.fr | **favorable** — page ~85 % rendue (nav, hero, cartes) ; 4 échecs `ok=false` **tous nommés** | addEventListener ×2, `document of Null` (boomerang), clientlib | §B-51 bras silence fermé ×2ᵉ site ; résidu §B-14 `addEventListener` sur Object |
| regex101.com | **§B-32 PERSISTANT** — logo flou/bloc (bilinéaire R-128 insuffisant — src basse rés ?) ; SPA non montée, menus empilés (§B-18 adjacent) | `unhandledrejection: json is not a function` | **`Response.json` absent → §B-14/J5** — cause montée SPA |
| developer.chrome.com | **§B-38 RÉSOLU** — 0 item expulsé (audit layout : aucune boîte >1366 px), nav/logo/hero ~98 % fidèle | — | — |
| paris.fr | **§B-49 RÉSOLU** — wrapper dans viewport, page peinte (actu, suggestions, cartes) vs frame blanche vol 73 | — | résidu : overlay menu/search pixelisé géant peint ouvert → famille §B-17 |
| cryptpad.fr | **§B-23 PERSISTANT ×2** — reproduit à l'identique : 1 éval, chaîne dyn jamais évaluée, doc_h=8, disjoncteur §B-41 armé | « pas de journal » | signature stable — confirme le bras « chaîne dynamique hors canal éval » du vol 100 |

## Preuves marquantes

### §B-51 → bras « silence » fermé, défaut déplacé

```
twitch : 44/44 evals ok=true (918 845 o en 1 417 ms)
         WARN evals_sans_mutation — boot possiblement muet evals=44
         /console : « pas de journal » ; frame : coquille blanche
sosh   : eval fin ms=132 ok=false →
         TypeError: value is not a function: undefined (hptu.*.js)
         + src verbatim du bundle dans l'entrée
edf    : 4× ok=false → toutes nommées (addEventListener,
         document of Null boomerang)
```

Le bras « échec invisible » de §B-51 est fermé sur les 3
sites ; le défaut résiduel est fonctionnel (§B-4(c) twitch :
évals ok mais 0 mutation — instrumenté, reste à nommer la
cause). Note : le WARN `evals_sans_mutation` reste **dans le
journal daemon**, non routé au ring `/console` — trou
d'observabilité résiduel à remonter.

### §B-56 — octal reste le dernier bras

```
invalid regex /(?:^|\141\164[\f\u1680-…]{1,})…[\v…\r\100]\x28)…
— classes niées \p{} acceptées (JS-C180 OK),
  \141/\100 octal Annex B toujours rejetés (script anti-bot
  obfusqué airfrance 22J9uZT8f)
```

## Arrêt du daemon

DELETE propres ×7 ; `Stop-Process -Id 10308` ; RAM 444 Mo à
`{"tabs":[]}` — purge saine.

## Re-vols reportés au vol 102

- **letterboxd** → §B-58 : CORRECTIF R-131 `dbd88407`
  (transform rejoué par boîtes anonymes) — post-binaire →
  **priorité 1**.
- **airfrance** → §B-56 bras octal `\141/\100` (pas de
  correctif — à remonter en file NAV-JS).
- **twitch** → §B-4(c) : 44 évals sans mutation — cause à
  nommer (warn posé).
- **flixbus / manomano** → §B-54/§B-30 bras É154 +
  §B-57 chemin `ToObject` (reportés vol 100).
- **cryptpad / bookshop** → §B-23 chaîne dyn, §B-12 cap
  cross-engine (reportés vol 100, sans correctif).
- regex101 → §B-32 (logo) + §B-14 `Response.json`.

<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 98 : REVALID8 `[V10-main]`

- **Binaire** : `[V10-main]` —
  `C:/vol96-wt/target/release/explorer-daemon.exe` (build
  `Finished` ~22:28, HEAD `f4c9ec25` — post-JS-C177 §B-56
  `{n,m}`/Unicode). Worktree repositionné sur main après le
  contrôle de fraîcheur de fin de vol 97 (fix committé en
  cours de vol → rebuild, conformément à la règle).
- **Mission** : REVALID8 — re-vol §B-56 (frandroid,
  airfrance), confirmation de l'anomalie miroir letterboxd,
  suivi js_heap marmiton, exploration nouvelle : leboncoin
  (e-commerce FR absent du corpus) et twitch.tv
  (SPA/streaming non couvert).
- **Daemon** : PID **17604**, port 9616,
  `EXPLORER_VIEWPORT=1366x900`, journal
  `logs/daemons/_vol98_daemon.log` — vol entier sur un
  daemon, arrêt propre (DELETE propres).
- **Verdict global** : **§B-56 PARTIEL** — bras `{n,m}`
  RÉSOLU terrain (frandroid : 0 `invalid regex` vs ×1) ;
  bras `\p{}`/octal **PERSISTANT** (airfrance : classe niée
  `[^\p{Pat_Syn}\p{Pat_WS}]` et `\141\164` en contexte
  rejetés — le verrou JS-C177 ne couvrait que la classe
  positive). **§B-12 cap confirmé** (frandroid ×0, 2ᵉ vol
  consécutif). **§B-58 nouveau** : texte peint miroir/retourné
  (letterboxd « Wildwood (2026) », reproduit ×2 vols). js_heap
  marmiton revenu à 49 Mo (hausse vol 96 transitoire).
  **leboncoin : mur Datadome 403** (refus au fetch) ;
  **twitch : coquille blanche à console vide** — silence
  total, variante §B-51.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| frandroid.com | **§B-56 `{n,m}` RÉSOLU** — 0 `invalid regex` ; §B-12 ×0 (cap confirmé) ; §B-46 non reproduit ×3 | `Element missing` ; `moteur js inerte` ×16+ | js inerte post-mortem tracé (heap_dead) — le cap fonctionne |
| airfrance.fr | **§B-56 PARTIEL** — `[^\p{Pat_Syn}\p{Pat_WS}]` + `\141\164` rejetés ; §B-51 non reproduit (erreurs nommées) | 6 entrées nommées (TypeError, SyntaxError regex ×2, import 404) | classe **niée** `\p{}` hors du verrou JS-C177 ; doc_h=1 730 |
| letterboxd.com | **→ §B-58 NOUVEAU** — « Wildwood (2026) » peint **miroir/retourné** (×2 vols : 95 `[V9]` + 98 `[V10]`) | — | glitch transform raster (`scale(-1)`/rotation 180° non composée ?) — recoupe écart raster vol 93 |
| marmiton.org | **favorable** — js_heap **49 Mo** (vs 343 Mo vol 96 : transitoire, pas de régression) | 1 TypeError nommée | doc_h=11 118 ; DELETE propre |
| leboncoin.fr | **mur anti-bot** — `HTTP 403` au fetch (Datadome) | `HTTP 403` | refus réseau, pas un défaut moteur — constat de couverture |
| twitch.tv | **coquille blanche silencieuse** — 246 nœuds, js_heap 7,3 Mo, doc_h=900, logo seul peint | **0 entrée** — silence total | variante §B-51 (évals silencieuses ou scripts jamais évalués — trou d'observabilité) |

## Preuves marquantes

### §B-56 — frandroid corrigé, airfrance restant

```
frandroid : 0 « invalid regex » (vol 97 : /^[\w:-]{32,512}$/
            rejetée — {n,m} déroulait ~30 Mio d'états > 10 Mio)
airfrance : invalid regex /(?:(?:\{\s*[^\p{Pat_Syn}\p{Pat_WS}]+
            \s*))/ — classe NIÉE à props dépréciées, hors verrou
            JS-C177 (classe positive seule)
            + syntaxe js : invalid regex /\141\164[…]{1,}/ —
            octal en contexte rejeté
```

`DELEGATE_SIZE_LIMIT` 64 Mio a débloqué le bras `{n,m}` ; le
bras `\p{}`/octal reste ouvert — **PARTIEL**.

### §B-58 — letterboxd : texte miroir ×2 vols

Texte d'affiche « Wildwood (2026) » peint **retourné 180°**
(haut droite de la hero, `captures/vol98_letterboxd.png`) —
reproduction du constat vol 95 `[V9-main]`. Signature stable
sur deux binaires → **entrée §B-58** (`explorer-render` :
transform raster — rotation 180°/flip matrice non composée,
ou backdrop `transform` non appliqué au run de texte).

### §B-51 — twitch : silence total

```
dom_nodes=246, layout_boxes=22, js_heap=7,3 Mo, doc_h=900
/console : 0 entrée — ni éval échouée, ni erreur nommée
frame : logo Twitch + barres squelette, rien d'autre
```

L'app React ne s'est jamais montée et **rien n'a été tracé**
— soit les bundles n'ont pas été évalués (fetch/plan), soit
l'échec est invisible. Trou d'observabilité §B-51-adjacent :
à instrumenter (compter les évals reçus vs scripts du DOM).

### leboncoin — Datadome

`POST navigate → {"error":{"handled":true,"message":"HTTP 403"}}`
en 1 s — le mur anti-bot refuse en amont du moteur. Non
diagnostiquable moteur ; constat de couverture terrain.

## Arrêt du daemon

DELETE propres ×6 ; `Stop-Process -Id 17604`. Sessions
voisines intactes. Journal `logs/daemons/_vol98_daemon.log`.

## Re-vols reportés au vol 99

- airfrance → §B-56 bras `\p{}` nié + octal en contexte
- bookshop → §B-12 cap 1/moteur (×3 résiduel)
- cryptpad → §B-23 (chemin less.min.js hors JS-C176)
- flixbus → bras post-DELETE §B-54/§B-30 (pompe orpheline —
  pas de correctif posé)
- twitch → §B-51 silence : instrumenter évals vs scripts
- §B-58 → letterboxd backdrop (isoler le transform fautif)

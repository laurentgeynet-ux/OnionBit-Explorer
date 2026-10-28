<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 97 : REVALID7 `[V10-main]`

- **Binaire** : `[V10-main]` —
  `C:/vol96-wt/target/release/explorer-daemon.exe` (build
  `Finished` 22:17, HEAD `5c168546` — post-R-129/R-130,
  JS-C175/176, Étape 152). Worktree éphémère repositionné
  sur HEAD — la règle de fraîcheur a porté : les fixes du
  WIP ont été committés sur main pendant le vol 96.
- **Mission** : REVALID7 — re-vols des correctifs fraîchement
  committés : ouest-france (§B-27/R-129), ici.fr (§B-55/R-130),
  caniuse (§B-37/JS-C175), cryptpad (§B-23/JS-C176),
  bookshop + frandroid (§B-12/É152), regex101 (§B-32/R-128).
- **Daemon** : PID **12248**, port 9615,
  `EXPLORER_VIEWPORT=1366x900`, journal
  `logs/daemons/_vol97_daemon.log` — vol entier sur un seul
  daemon, arrêt propre (7/7 DELETE propres).
- **Verdict global** : **4 RÉSOLU / 3 OUVERT-PARTIEL** —
  §B-27 RÉSOLU (ouest-france : plus de mosaïque, colonnes
  normales) ; §B-55 RÉSOLU (ici.fr : panic resvg confiné,
  pompe reprend à la µs suivante) ; §B-37 RÉSOLU (caniuse :
  0 `index out of bounds`) ; §B-46 confirmé non reproduit ×2
  (frandroid) ; **§B-12 PARTIEL** (frandroid ×0 — É152
  efficace ; bookshop ×3 — mieux que ×7/×9 mais pas le 1
  par moteur promis) ; **§B-23 PERSISTANT** (cryptpad :
  `eval deadline` sur `less.min.js` malgré JS-C176) ;
  §B-32 PARTIEL (regex101 : logo encore flou/bloc) ;
  **§B-57 confirmé 2ᵉ site** (caniuse bundle.js —
  `deeply nested` sur bundle réel).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| bookshop.org → uk | **§B-12 PARTIEL** — heap limit **×3** (vs ×7 vol 96, ×9 vol 95) | `TypeError: left` ; `import()→/null` 404 ×2 | É152 en-dessous du cap 1/moteur promis — ré-armement à la nav ? |
| frandroid.com | **§B-12 RÉSOLU** — heap limit **×0** ; §B-46 non reproduit ×2 | `invalid regex /^[\w:-]{32,512}$/` (→§B-56) ; `Element missing` | aucune rafale heap ; doc_h=17 348 ; DELETE propre |
| ouest-france.fr | **§B-27 RÉSOLU** — plus de fils flex 13 387 px, cartes en colonnes | — | vignettes grises non chargées (lazy) ; chevauchement léger en-tête |
| ici.fr | **§B-55 RÉSOLU** — panic `resvg composite.rs:25` confiné par `catch_unwind`, pompe reprend us=149 après | — | preuve : ligne panic puis `pump.offlock`/`commit.*` reprennent, API vivante |
| caniuse.com | **§B-37 RÉSOLU** — 0 `index out of bounds`/`dom.rs` | `RangeError: expression too deeply nested` bundle.js ×N (**§B-57 2ᵉ site**) | squelette SPA ; doc_h=1 669 |
| cryptpad.fr | **§B-23 PERSISTANT** — `unhandledrejection: eval deadline exceeded` sur `less.min.js` ; boot figé, doc_h=8 | 1 entrée nommée | disjoncteur §B-41 armé ×1 ; JS-C176 insuffisant sur ce chemin |
| regex101.com | **§B-32 PARTIEL** — logo raster encore flou/bloc | — | SPA non montée : menu vertical empilé (lettres superposées), `doc_h`=900 |

## Preuves marquantes

### §B-55 — ici.fr : panic confiné, apply continue

```
panicked at resvg-0.45.1/filter/composite.rs:25:5:
assertion failed: src1.width == src2.width && src1.width == dest.width
→ phase suivante : pump.offlock us=149, commit.dl_net us=0,
  deliver us=90 — pompe reprend immédiatement
/api/tabs : 200, frame capturée, DELETE 204
```

`catch_unwind` (R-130) fait exactement ce qui était promis :
panic de la dep rattrapé, dégradation `None`, vie du process.

### §B-27 — ouest-france : mosaïque disparue

Vol 48 `[V4-main]` : fils `_b#focus_1_4_6` à 13 387 px dans une
colonne 1 260 → image géante plein viewport. Vol 97 : page
d'accueil structurée (cartes édito, rubriques, liens) — les
deux défauts flex (split `calc` paren-aware + cb de référence)
sont corrigés (`captures/vol97_ouestfrance_scroll.png`).

### §B-12 — É152 efficace ×1, partiel ×1

```
frandroid : 0 occurrence heap limit (vs ×6+ vol 96)
bookshop  : 3 occurrences identiques (vs ×7 vol 96)
           (1 079 661 cells, 541 244 657 B)
```

Le cap « 1 occurrence par document » n'est pas atteint sur
bookshop (×3) — variantes de piles passent encore le filtre
`heap_dead_reported` ? **EN DÉCROISSANCE**, pas RÉSOLU.

### §B-23 — cryptpad : deadline toujours fatale au boot

`unhandledrejection: RangeError: eval deadline exceeded at
/lib/less.min.js` — JS-C176 (drain microtasks partage la
deadline boot) ne couvre pas ce chemin (l'éval less elle-même
dépasse ?). Splash peint puis figé, doc_h=8 ; disjoncteur
§B-41 armé ×1 — API restée vivante. **OUVERT**.

### §B-37 — caniuse : zéro panic dom

`grep index out of bounds|dom.rs → 0`. Le site qui paniquait
`dom.rs:2752` ×3 au vol 94 (intermittent) passe propre —
requêtes d'arbre sur `NodeId` obsolète dégradées (JS-C175).
`deeply nested` sur `bundle.js` = **2ᵉ site §B-57** après
manomano/polyfill.

## Arrêt du daemon

7/7 DELETE propres (0,15-0,49 s) ; `Stop-Process -Id 12248`.
Sessions voisines intactes. Journal
`logs/daemons/_vol97_daemon.log`.

## Re-vols reportés au vol 98

- bookshop → §B-12 caper à 1/moteur (É152 partiel)
- cryptpad → §B-23 (chemin less.min.js hors JS-C176)
- flixbus → bras post-DELETE §B-54/§B-30 (pompe orpheline —
  pas de correctif posé à ce jour)
- §B-56 regex : rejouer `{n,m}`/`\p{}`/octal (WIP regexp.rs
  observé en session — probe5 : delegate_size_limit)
- marmiton → js_heap 343 Mo (É148 ?)
- letterboxd → anomalie rendu texte miroir (vol 95, non
  numérotée — à cataloguer si reproduite)

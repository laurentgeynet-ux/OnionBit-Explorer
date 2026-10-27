<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Vol 64 — VALIDATION `[V5-main]` — 2026-10-27

**Agent 64 · mission VALIDATION** — re-vols ciblés sur les fixes du
binaire `[V5-main]` : **JS-C169** (`Array.isArray` sur proxies),
**JS-C170** (`heap_dead`), **JS-C171** (spread dans optional-chain),
**B-35** (colonne `block` effondrée à largeur 0).

- Binaire : `C:/obe-vol-target/release/explorer-daemon.exe`
  (20 782 080 o, 2026-10-27 13:10)
- Daemon dédié : port **9446**, `EXPLORER_VIEWPORT=1366x900`,
  PID 5540, journal racine `_vol64_daemon.log`
- Protocole : `docs/vol-agent.md` — traçabilité §8 (crate + ID §B,
  tag binaire)

**Santé globale du vol** : 3/3 cibles servies, API responsive en
permanence, RAM max ~1,45 Go (< seuil 2 Go), retombée à **413 Mo**
après DELETE des 3 onglets — **pas de fuite §B-30**. Aucun crash,
aucun wedge. Daemon tué par PID à la clôture.

---

## Cible 1 — https://alpinejs.dev → §B-3 (stack overflow `explorer-jscore`)

### Verdict : **RÉSOLU** `[V5-main]`

| Mesure | `[V4-main]` vol 46 | `[V5-main]` vol 64 |
|---|---|---|
| Process | **mort** — `overflowed its stack` ~110 ms dans éval script 55 935 o, abort `STATUS_STACK_OVERFLOW` | **vivant** — nav retournée, 519 Mo, stable au scroll |
| Console | n/a (process tué) | **vide** — `pas de journal` |
| `doc_h` | n/a | 4 677 |
| DOM / layout | n/a | 1 173 nœuds / 526 boîtes |

**Preuve fonctionnelle Alpine bootée** (`logs/vol64_alpinejs_dom.txt`
l.436+) : le `<template x-for="([number, name], key) in steps">` a
été **expansé** — les `<button>` clonés existent dans le DOM avec
`x-text` **évalué** (`<strong>"15"</strong>`, `<span>"Attributes"</span>`,
`"6"`/`"Properties"`, `"2"`…) et `:class` lié (`bg-gray-800` sur
l'étape courante). Alpine utilise des `Proxy` pour sa réactivité :
un `x-for` fonctionnel valide en même temps **JS-C169**
(`Array.isArray` traversant un proxy) en conditions réelles.

Captures : `captures/vol64_alpinejs.png` (page complète : logo,
nav, code-block) + `vol64_alpinejs_scroll.png` (docs des directives
x-show/x-transition/x-for/x-if/x-init — rendu propre à scroll_y=2400).

### Cible 2 — https://www.bbcgoodfood.com → §B-9 / JS-C171 (spread optional-chain)

### Verdict JS-C171 : **RÉSOLU** — boot Next : **DÉGRADÉ EN MIEUX**

| Mesure | `[V3]` vol 34 | `[V5-main]` vol 64 |
|---|---|---|
| `_app` Next | **mort au parse** — `` `...` `` « outside a call or array literal » | **32/32 évals `ok=true`** (chunks jusqu'à 1,5 s : 1 255 / 1 340 / 1 525 ms) |
| Console | erreur parse | **vide** — `pas de journal` |
| DOM | page morte | 2 855 nœuds, `__NEXT_DATA__` présent (hero-slider, tabbed-card-carousel — `logs/vol64_bbcgoodfood_dom.txt` l.162) |
| `doc_h` | — | 12 608, titre « Good Food \| Recipes… » |

Le spread en position optional-chain n'est plus un tueur : le parse
passe, tous les scripts s'évaluent sans erreur.

**Restes** (autres anomalies, pas JS-C171) :
- `tempête de pompes — disjoncteur armé (B-41) tab=2 streak=64`
  à 12:12:43 — microtâches churn tronquées par le coupe-circuit ;
  `js_heap_bytes=0` au `/mem` (moteur stoppé proprement —
  cohérent avec **JS-C170** `heap_dead`, plus de wedge).
- Hero-slider `block<img> [683 356 0x0]` — composant React non
  hydraté (hydratation tronquée par le disjoncteur ?).
- Méga-menus rendus **dépliés** : la page s'affiche en liste
  textuelle (`captures/vol64_bbcgoodfood.png` et `_scroll.png`) —
  famille §B-17 (menus fermés par JS) / hydratation incomplète.
- Cartes recettes en bas layoutées sainement
  (`img 218×197` ×8+, `logs/vol64_bbcgoodfood_layout.txt` l.914+).
- RAM onglet : ~0,5 Go net ; daemon 1,04 Go pic. API responsive.

## Cible 3 — https://welcometothejungle.com → §B-35 (colonne `block` w=0)

### Verdict : **RÉSOLU** `[V5-main]` (symptôme d'origine disparu)

| Mesure | `[V4-main]` vol 57 | `[V5-main]` vol 64 |
|---|---|---|
| Colonne h2/p | `0x876` — texte **mot-par-ligne** | `block<h2> [40 254 1261×115]`, runs « Stop looking for any job. » 636 px ; `block<p>` 252 → 1 286 px (`logs/vol64_wttj_layout.txt` l.48-53) |
| `0x` résiduels | colonnes entières | **6 occurrences bénignes** (`<br>`, divs vides) |
| Contenu peint | colonne vide | « What if recruiters came to you? », carte « Your Profile » (photo, « Dean Charles / Product Manager », badge « 2 new recruiter requests ») — `captures/vol64_wttj_scroll.png` |

**Restes** (nouveaux/résiduels — ≠ §B-35) :
- **Hero occlus** : la section d'accueil peint tout jaune, seul le
  mot rotatif « tech. » s'est montré un instant (capture t0) puis
  tout est recouvert (`captures/vol64_wttj.png` puis `_top.png`
  uniformes). Suspect : backdrop `block [0 -5 1366x1114]` peint
  **par-dessus** le texte (ordre d'empilement) ou animation reveal
  JS jamais déclenchée → variante §B-7/§B-17 ou nouvelle entrée §B.
- **Colonne à x négatif** : `block<div> [-40 2375 302×454]` /
  `block<p> [-15 2473 252×115]` — texte clippé au bord gauche
  (« ow they can… », 14 boîtes x<0) — famille §B-18/§B-38
  (positionnement flex/grid).
- **Runs nav superposés** : « I'm a recruiter » griffé/chevauché
  en haut à droite (`vol64_wttj_scroll.png`) — §B-18 confirmé.
- 4 évals `ok=true`, 0 erreur console ; `tempête de pompes`
  disjoncteur armé ×3 (streak=64) sans wedge — API restée
  responsive (comportement §B-41 borné, pas de blocage).

---

## Synthèse des verdicts

| Fix | Cible | Verdict `[V5-main]` |
|---|---|---|
| §B-3 stack overflow (C134/C166/stack_guard + relais V5) | alpinejs.dev | **RÉSOLU** — process survit, Alpine boot, `x-for` expansé |
| JS-C169 `Array.isArray` proxies | alpinejs.dev (implicite) | **RÉSOLU** — réactivité Proxy Alpine fonctionnelle |
| JS-C171 spread optional-chain | bbcgoodfood `_app` | **RÉSOLU** — 32/32 évals ok, zéro erreur parse |
| Boot Next complet | bbcgoodfood | **DÉGRADÉ EN MIEUX** — scripts verts mais hydratation tronquée (disjoncteur B-41), menus dépliés, hero 0×0 |
| JS-C170 `heap_dead` | bbcgoodfood + wttj | cohérent — disjoncteur arrête le moteur proprement (`js_heap=0`, API vivante), aucun wedge |
| §B-35 colonne w=0 | welcometothejungle | **RÉSOLU** — largeurs réelles 252-1286 px partout |

## Propositions §B (trous nouveaux / à rattacher)

- **Hero occlus par backdrop plein-viewport** (wttj) : texte
  layouté mais jamais peint, couvert par `block [0 -5 1366x1114]` —
  ordre d'empilement ou reveal JS non déclenché. Rattacher §B-17
  (overlays résiduels) ou §B-7, ou numéro suivant si distinct.
- **Colonne flex/grid positionnée à x<0** (wttj `-40`/`-15`) —
  famille §B-38 (item layouté hors position). À recouper.

## Fichiers produits

```
captures/vol64_alpinejs.png          captures/vol64_alpinejs_scroll.png
captures/vol64_bbcgoodfood.png       captures/vol64_bbcgoodfood_scroll.png
captures/vol64_wttj.png              captures/vol64_wttj_scroll.png
captures/vol64_wttj_top.png          captures/vol64_wttj_hero_zoom.png
logs/vol64_alpinejs_{dom,layout,mem}.txt
logs/vol64_bbcgoodfood_{dom,layout,mem}.txt
logs/vol64_wttj_{dom,layout,mem}.txt
_vol64_daemon.log (racine)
```

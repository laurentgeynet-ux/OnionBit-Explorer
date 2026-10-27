<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 67 : validation du fix §B-26 `[V6-main]`

- **Binaire** : `[V6-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (build 2026-10-27 14:37 local, post-`79d25825`) — contient le fix
  **`1e24942`** : borne `MAX_EMIT_PER_PATH=4` / `MAX_EMITS_TOTAL=100_000`
  sur le cycle de report `emit_scoped_at` du paint, log
  `cycle de report cassé dropped=N path=…` + `dropped_emits`/
  `first_cycle_path` remontés (`explorer-render`, journalisé par
  `explorer-core`).
- **Mission** : re-vol des trois porteurs de wedge §B-26 — meteofrance
  (mort process 2,9 Go/75 s en V5), musee-orsay (variante fuite
  post-DELETE → recoupe §B-30), lefigaro (variante fuite libérée au
  DELETE).
- **Daemon** : PID 20340, port 9449, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol67_daemon.log`.
- **Fenêtre** : 13:39:58 → 13:52 UTC (~12 min).
- **Verdict global** : **§B-26 RÉSOLU ×3** — les trois variantes
  produisent le cycle de report, la borne le casse à chaque apply
  (`dropped=1`/`2`), tous les apply atteignent `raster`/`repaint`,
  aucune fuite non bornée, process vivant et API responsive tout du
  long. **§B-30 recoupé favorablement** : purge post-DELETE immédiate
  et stable.

---

## 1. meteofrance.com — RÉSOLU `[V6-main]` (était : mort process `[V5-main]`)

`POST /navigate` → **200 en 7,8 s** (title « PREVISIONS METEO FRANCE »,
`doc_h` 2 783). `apply: fin tab=1 ms=371` à 13:40:14.

Signature du fix, répétée à **chaque apply** (~1,5–2 Hz, pump JS
continu — 994 occurrences en ~9 min) :

```
apply: display_list items=2782
WARN  apply: cycle de report cassé (§B-26) dropped=1 path=Some([0, 0, 0, 0])
apply: raster regions=1
apply: repaint ms=306–348
```

Le chemin coupable est exactement celui mesuré en vol 65
(`[0,0,0,0]`) — le cycle se produit toujours, la borne l'étouffe
à chaque frame au lieu de laisser `build_display_list` tourner
sans fin.

| Instant | RAM daemon |
|---|---|
| baseline (0 onglet) | 362 Mo |
| fin nav | 743 Mo |
| +60 s de pump continu | 743→746 Mo (Δ +3 Mo) |
| +9 min, onglet seul | 846→847 Mo (Δ +1 Mo/30 s) |

**Fuite résiduelle : ~0 Mo/s** (vs +38 Mo/s en accélération en V5).
Télémétrie : `/frame` 200 (PNG 1,96 Mo, `captures/vol67_meteofrance.png`
— page peinte, contenu textuel cohérent, styles partiels),
`/dom` 200 (368 Ko, 3 047 nœuds), `/mem` 200 (~101 Mo alloués,
`journal_fallbacks=2`), `/console` 200 (erreurs JS : `SyntaxError
unexpected token in JSON` sur bundle Drupal + `unhandledrejection:
invalid token ×N` — `logs/vol67_meteofrance_console.json`).

## 2. musee-orsay.fr — RÉSOLU `[V6-main]` + recoupe §B-30 favorable

`POST /navigate` → **200 en 17,2 s** (→ `/fr`, « Musée d'Orsay »,
`doc_h` 10 051). `apply: fin tab=2 ms=978` à 13:43:04 — contre un apply
mort post-`layout ms=968` en vol 62/65.

```
apply: display_list items=2300
WARN  apply: cycle de report cassé (§B-26) dropped=2 path=Some([0, 6, 0])
apply: raster regions=1
apply: repaint ms=1087
```

Nouveau chemin cyclique `[0,6,0]` (×2) — borne efficace aussi.
`/frame` 200 (PNG 5,4 Mo, `captures/vol67_orsay.png` — page réellement
rendue : logo M'O, navigation, modale consentement « Cher visiteur »,
section « Les expositions »). `/dom` 200 (538 Ko, 4 061 nœuds),
`/mem` 200, `/layout` 200. `/console` → 404 « pas de journal »
(aucune erreur JS enregistrée — endpoint 404 quand le ring est vide).

**Test §B-30** : RAM 1 015→1 021 Mo pré-DELETE (+6 Mo/25 s,
bruit de pump — pas la fuite +4,4 Mo/s de vol 65) ; **DELETE →
793 Mo immédiat puis 780 Mo stable sur 65 s**. La fuite post-DELETE
de vol 62 (976→2 350 Mo, +28 Mo/s) est **absente**.

## 3. lefigaro.fr — RÉSOLU `[V6-main]` (rendu blanc résiduel, hors §B-26)

`POST /navigate` → **200 en 15,8 s** (« Le Figaro », `doc_h` 71 992
`clamped=true`). Séquence : `journal DOM rejeté → re-parse
(ADR-0025) why=UnmappedKey` à 13:45:59, puis applies complets :

```
apply: display_list items=10650–10968
WARN  apply: cycle de report cassé (§B-26) dropped=1 path=Some([0, 1])
apply: repaint ms=1044–1117
```

Chemin cyclique `[0,1]` (×6). `/frame` : 204 transitoire pendant le
re-parse puis **200** (PNG 1,84 Mo, `captures/vol67_figaro.png`) —
conforme au 404/204 transitoire documenté (vol-agent §3). `/dom`
200 (1,48 Mo, 10 028 nœuds), `/mem` 200, `/console` 200
(`unhandledrejection hasWebVitals of Undefined` marfeel-sdk,
`` `delete` is not a function`` appconsent, `` `addEventListener`
is not a function`` — `logs/vol67_figaro_console.json`,
`journal_fallbacks=4`).

| Instant | RAM |
|---|---|
| post-nav | 1 380 Mo |
| +40 s | 1 548→1 552 Mo (Δ +4 Mo — quasi stable) |
| DELETE | 1 409 Mo → **848 Mo en 30 s** — libérée |

**Réserve** : la frame servie est **blanche** (PNG 1,84 Mo uniforme)
malgré 10 796 display_items — le wedge est mort mais la page ne
peint pas son contenu (JS de boot en erreur : `addEventListener`,
`hasWebVitals` — probablement shell SPA vide). **Non catalogué tel
quel** : distinct de §B-26 (plus de wedge ni fuite), plutôt un
« peint blanc sous erreurs JS » — à recouper avec les entrées §B
de rendu vide (cf. §B-27 famille « DOM riche / frame vide »).
Fuite résiduelle : **~0,1 Mo/s** (vs +29 Mo/s en vol 49).

## 4. Tableau de synthèse

| Cible | `[V5-main]` vol 65 | `[V6-main]` vol 67 | Verdict |
|---|---|---|---|
| meteofrance | apply mort post-layout, +38 Mo/s, mort 4,9 Go, API morte | `dropped=1 [0,0,0,0]` ×994, `apply: fin ms=371`, ~0 Mo/s, vivant 9 min | **RÉSOLU** |
| musee-orsay | apply mort post-layout, +4,4 Mo/s, fuite post-DELETE | `dropped=2 [0,6,0]` ×2, `apply: fin ms=978`, purge DELETE propre | **RÉSOLU** (§B-30 confirmé fixé sur cette variante) |
| lefigaro | apply mort (wedge §B-26 reproduit) | `dropped=1 [0,1]` ×6, repaint ms≈1 s, RAM stable, libérée au DELETE | **RÉSOLU** — réserve frame blanche |

## 5. Observations annexes

- Le WARN `cycle de report cassé` se déclenche à **chaque apply** sur
  meteofrance (~994 fois) : la condition de report est permanente
  (le porteur re-reporte la même cible à chaque frame). La borne
  fonctionne, mais le coût d'un `display_list` recomputé par pump
  (~300–350 ms de repaint à ~2 Hz) persiste — piste perf, pas
  stabilité : `explorer-render`/`explorer-core`, sévérité basse
  (CPU soutenu, pas de croissance).
- Pas de `apply: fin` sur le chemin `journal rejeté → re-parse`
  (figaro tab 3) — le jalon `fin` n'est émis que sur l'apply
  nominal ; repaints observés malgré tout. Cosmétique de
  journalisation.
- `/console` 404 « pas de journal » quand le ring est vide
  (orsay) — comportement à documenter ou uniformiser en 200 vide ;
  cosmétique API.

## 6. Arrêt

DELETE tab 1 → `{"tabs":[]}`, RAM 458 Mo (retour quasi-baseline).
Daemon tué par PID 20340 (`taskkill /PID`) — jamais `/IM`.

**Fichiers** : `captures/vol67_{meteofrance,orsay,figaro}.png`,
`logs/vol67_{meteofrance,orsay,figaro}_{dom.txt,mem.json,console.json}`,
journal daemon `_vol67_daemon.log` (994+6+2 WARN `cycle de report`).

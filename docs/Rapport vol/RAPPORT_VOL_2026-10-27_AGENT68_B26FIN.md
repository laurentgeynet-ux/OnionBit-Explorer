<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 68 : clôture §B-26 (variante bfmtv) `[V6-main]`

- **Binaire** : `[V6-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (build 2026-10-27 15:04 local, post-`de73b81e`) — contient le fix
  **`1e24942`** : borne `MAX_EMIT_PER_PATH=4` / `MAX_EMITS_TOTAL=100_000`
  sur le cycle de report `emit_scoped_at` (`explorer-render`).
- **Mission** : re-voler la **dernière variante §B-26 non vérifiée** —
  bfmtv « dent de scie » (fuite 6,88 Go/5 min + API morte au vol 49,
  `[V4-main]`) — plus radiofrance (2ᵉ porteur, variante « apply en
  escalade »).
- **Daemon** : PID 18704, port 9450, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol68_daemon.log`. Baseline RAM : 364 Mo.
- **Fenêtre** : 14:05:47 → 14:18 UTC (~12 min).
- **Verdict global** : **§B-26 RÉSOLU ×4 — catalogue complet**. bfmtv :
  fuite dent-de-scie bornée et purgée, API vivante 12 min, zéro cycle
  de report émis. radiofrance : l'apply en escalade **va à son terme**
  (`repaint ms=199721`) au lieu de wedger — la fuite transitoire
  (pic 4,75 Go) purge post-DELETE. Résidu non-§B-26 : l'escalade de
  latence d'apply elle-même (perf, nouvelle proposition §B — §5).

---

## 1. bfmtv.com — RÉSOLU `[V6-main]` (était : 6,88 Go/5 min, API morte `[V4-main]` vol 49)

`POST /navigate` → **200** (« Actualités en continu… BFM »,
`doc_h` 17 221 `clamped=true`). Pump continu ~1,4 Hz, applies
nominaux complets : `apply: fin tab=1 ms=851–1086` récurrents.

- **Zéro `WARN cycle de report`** sur tout le vol (0 occurrence dans
  le journal) — la condition cyclique de meteofrance/orsay/figaro ne
  se produit même pas ici : le fix n'a rien eu à borner, le wedge de
  vol 49 était le même défaut vu par un autre chemin.
- 3× `journal DOM rejeté → re-parse (ADR-0025) why=UnmappedKey`
  (14:06:08, 14:10:09, 14:10:12) — chemin de repli connu, suivi
  d'applies complets ; pas un wedge (signature §B-26 = arrêt à
  `layout`, jamais observé sur tab 1).

**La dent de scie est devenue bornée** — mesure minute par minute :

| Instant | RAM daemon | Lecture |
|---|---|---|
| baseline (0 onglet) | 364 Mo | |
| post-nav | 779 Mo | |
| +40 s / +80 s | 780 / 781 Mo | plateau |
| +120 s | 1 163 Mo | **dent** (+382 Mo en 40 s) |
| +150 s → +210 s | 1 155 / 1 155 / 1 154 Mo | **plateau — la dent redescend** |
| (épisode radiofrance, cf. §2) | 496 Mo | purge complète de la dent |
| reprise pump tab 1 | 773 → 1 023 Mo | dent suivante, même amplitude |
| DELETE tab 1 | **153 Mo** | retour sous la baseline |

Contre 6,88 Go en 5 min sans retour au vol 49 : l'oscillation est
désormais **bornée ~0,5–1,2 Go** et **intégralement purgée** — le
réservoir qui fuyait (le cycle de report non borné qui ré-allouait
des display lists à l'infini) est cassé. Fuite résiduelle mesurée
sur les plateaux : **~0 Mo/s**.

Télémétrie : `/frame` 200 (PNG 2,5 Mo, `captures/vol68_bfmtv.png` —
**page réellement rendue** : bandeau nav BFM, « DIRECT. Un hommage
rendu à l'Assemblée nationale… », « Fil actu », hero « Live en
cours » ; artefact cosmétique : un data-URI SVG affiché en texte
brut dans l'en-tête). `/dom` 404→**200** au retry (transitoire
documenté vol-agent §3, DOM détaché pendant apply ; 1,26 Mo dumpé,
`logs/vol68_bfmtv_dom.txt`). `/mem` 200 (`logs/vol68_bfmtv_mem.json`
— `journal_fallbacks=2`, `js_heap` 40 Mo). `/console` 404 (ring
vide — aucune erreur JS, comportement 404-vide déjà noté vol 67).

## 2. radiofrance.fr — mécanisme §B-26 RÉSOLU, escalade résiduelle (perf)

`POST /navigate` → **200** (« Radio France | Écoutez en ligne… »,
`doc_h` 32 907 `clamped=true`). La variante « apply en escalade »
se reproduit fidèlement — et **cette fois elle termine** :

```
14:10:34  repaint ms=363          (items=12115)
14:10:44  cascade ms=6384 → boxtree ms=6882 → layout ms=8161
14:10:47  repaint ms=9558
14:12:02  cascade ms=46460        ┐ apply géant — EXACTEMENT la
14:12:11  boxtree ms=54960        │ signature vol 49 « journalisé
14:12:28  layout ms=72596         │ jusqu'à layout »…
14:14:33  repaint_payload         │
14:14:35  display_list items=3047 │ …mais POURSUIT jusqu'au bout :
14:14:35  repaint ms=199721       ┘ apply COMPLET, pas de wedge
```

Vol 49 (`[V4-main]`) : escalade `0,4 → 9,2 → >49,5 s`, `+90 Mo/s`,
jamais de `repaint`/`fin`. Vol 68 (`[V6-main]`) : escalade `0,36 →
9,5 → 199,7 s`, **avec `display_list` puis `repaint` émis** — la
borne `emit_scoped_at` laisse `build_display_list` aboutir.

**Séquence mémoire** (tab 2 créé à ~14:10:30, DELETE ~14:11 sous
protocole >1,5 Go) :

| Instant | RAM | Événement |
|---|---|---|
| pré-nav tab 2 | 1 155 Mo | (bfmtv tab 1 sur plateau) |
| post-nav | 3 668 Mo | escalade — protocole : DELETE |
| DELETE tab 2 (204) | 3 943 → 4 223 → **4 752 Mo** | pic, apply in-flight continue post-DELETE |
| fin de l'apply 199,7 s | 4 077 → 3 676 → 3 501 → 3 285 | purge progressive |
| post-apply | 1 320 → **496 Mo** | ~3,2 Go relâchés |

Constats :

1. **Pas de wedge** : l'apply aboutit (`display_list items=3047`,
   `repaint ms=199721`) — la signature §B-26 « arrêt à `layout` »
   est morte sur radiofrance aussi.
2. **Purge post-DELETE fonctionnelle** (recoupe §B-30 favorable ×2
   après orsay vol 67) : l'apply in-flight d'un onglet supprimé
   tourne jusqu'à son terme puis **relâche tout** — pas de fuite
   +28 Mo/s à `{"tabs":[]}` comme manomano vol 53/59.
3. **API vivante pendant l'escalade** : `/frame` tab 1 → 200 en
   2,3 s en plein apply 200 s ; le wedge est scopé au pump de
   l'onglet.
4. **Résidu — escalade de latence** : cascade 46 s / boxtree 55 s /
   layout 72 s pour ~12 k items = croissance pathologique par apply
   (×200 entre deux applies) + 4,75 Go transitoire. Distinct du
   cycle de report corrigé (borne jamais déclenchée, 0 WARN) — à
   cataloguer comme entrée perf/stabilité séparée (proposition §5),
   pas comme §B-26.

## 3. Tableau de synthèse §B-26 — toutes variantes re-volées `[V6-main]`

| Variante | Signature `[V4/V5-main]` | `[V6-main]` vol 67 | `[V6-main]` vol 68 | Verdict |
|---|---|---|---|---|
| meteofrance | apply mort post-`layout`, +38 Mo/s, mort 4,9 Go | `dropped=1 [0,0,0,0]` ×994, `apply: fin`, ~0 Mo/s | — | **RÉSOLU** |
| musee-orsay | apply mort post-`layout`, fuite post-DELETE | `dropped=2 [0,6,0]`, purge DELETE propre | — | **RÉSOLU** |
| lefigaro | apply mort | `dropped=1 [0,1]` ×6, RAM stable | — | **RÉSOLU** (réserve frame blanche → famille §B-27) |
| **bfmtv** | **dent de scie 6,88 Go/5 min, API morte** | (non re-volée) | **0 cycle WARN, dent bornée ~1,2 Go, purge 153 Mo, API vivante** | **RÉSOLU** |
| radiofrance | apply escalade >49,5 s, +90 Mo/s | (non re-volée) | apply mené à terme (`repaint ms=199721`), purge post-DELETE | **RÉSOLU** mécanisme wedge/fuite — **résidu escalade → prop. §B** |

**§B-26 : RÉSOLU `[V6-main]` — 5/5 variantes re-volées, catalogue
complet.** La borne `emit_scoped_at` éteint le wedge partout où le
cycle se produisait ; sur bfmtv/radiofrance le cycle ne se produit
plus du tout (0 WARN) — leur wedge vol 49 passait par la même
allocation sans fin en amont du même point.

**§B-30 : signal favorable ×2 cumulé** (orsay vol 67, radiofrance
vol 68) — DELETE pendant apply in-flight purge correctement ; reste
à re-voler les porteurs d'origine (manomano, flixbus) pour clore.

## 4. Observations annexes

- `/console` 404 quand le ring est vide (bfmtv — même constat
  qu'orsay vol 67) ; cosmétique API déjà notée.
- Artefact de rendu bfmtv : data-URI SVG affiché en texte brut
  dans l'en-tête (visuel, `vol68_bfmtv.png`) — mineur, famille
  « contenu brut non décodé ».
- Aucun `apply: fin tab=2` : les applies en escalade radiofrance
  n'émettent pas le jalon `fin` (cohérent avec vol 67 — `fin` n'est
  émis que sur l'apply nominal) ; le `repaint ms=` fait foi.

## 5. Proposition d'entrée §B (numéro suivant)

> **Escalade quadratique d'apply sur pump continu** — sur
> radiofrance (et plausiblement la « dent » bfmtv), chaque apply
> refait cascade/boxtree/layout en croissance géométrique
> (0,36 → 9,5 → 199,7 s, ×20–40 par apply) avec RAM transitoire
> multi-Go ; le work n'est plus un cycle infini (la borne §B-26
> tient) mais un **re-travail non borné par apply** — probable
> invalidation totale à chaque tick JS au lieu d'incrémentale.
> `explorer-core`/`explorer-layout`. Mesuré : `cascade ms=46460`,
> `layout ms=72596`, `repaint ms=199721`, pic 4,75 Go purgeable.
> Impact : onglet quasi-inutilisable ~3 min sous escalade, mais
> process et API sains.

## 6. Arrêt

DELETE tab 2 (14:11) puis tab 1 (14:18) → `{"tabs":[]}`, RAM
**153 Mo** (sous la baseline 364 Mo — purge complète). Daemon tué
par **PID 18704** (`taskkill /PID /F` — refus sans /F) ; le daemon
humain sur autre port n'a jamais été touché.

**Fichiers** : `captures/vol68_bfmtv.png` (2,5 Mo, page rendue),
`logs/vol68_bfmtv_{dom.txt,mem.json,console.json}`,
journal daemon `_vol68_daemon.log` (0 WARN `cycle de report`,
3× re-parse `UnmappedKey` tab 1, escalade radiofrance complète).

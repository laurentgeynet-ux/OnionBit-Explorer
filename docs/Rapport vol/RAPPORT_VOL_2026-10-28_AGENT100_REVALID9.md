<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 100 : REVALID9 `[V10-main]`

- **Binaire** : `[V10-main]` —
  `C:/vol100-wt/target/release/explorer-daemon.exe` (build
  `Finished`, HEAD `c52e1512` — inclut JS-C179 §B-23,
  Étape 153 §B-12, **Étape 154 `ee6008fa` §B-30/§B-54**).
  Worktree propre à HEAD (arbre principal WIP).
- **Commits post-binaire** : `897f523e` (JS-C180 §B-56 bras 2),
  `83d12755` (Étape 155 §B-51 observabilité) — arrivés en fin
  de vol → re-sélection vol 101.
- **Mission** : REVALID9 — re-vols des correctifs dans le
  binaire : §B-23 cryptpad (JS-C179), §B-12 bookshop+frandroid
  (Étape 153), §B-57 manomano+caniuse (JS-C178),
  §B-30/§B-54 flixbus (Étape 154), §B-7 geeksforgeeks
  (JS-C174), §B-45 clubic (`34a019b`).
- **Daemon** : 2 process — PID 4340 port 9620 (wedged par
  flixbus, arrêt forcé PID), PID 12168 port 9630 (fin de vol,
  arrêt propre) ; `EXPLORER_VIEWPORT=1366x900` ; journaux
  `logs/daemons/_vol100_daemon_wedged.log` +
  `_vol100_daemon2.log`. Note : port 9621 squatté par une
  session voisine (sonde §B-58) — collision évitée.
- **Verdict global** : **§B-23 PERSISTANT** (cryptpad : boot
  figé au splash, chaîne dynamique jamais évaluée — variante
  sans `deadline`). **§B-12 PERSISTANT** (bookshop ×5
  occurrences heap identiques — cap Étape 153 inopérant).
  **§B-57 PERSISTANT ×2** (manomano `ToObject`, caniuse
  `bundle.js` — trampoline JS-C178 hors chemin). **§B-54
  REPRODUIT sur flixbus** (wedge pendant vie de l'onglet, API
  morte — bras É154 non atteignable). **§B-45 RÉSOLU**
  (clubic : doc_h 11 940, scroll effectif). **§B-7 PERSISTANT
  réattribué** (GFG : hydrate mort `getInitialProps`, pas
  cssText → bras §B-4). 6 DELETE propres, RAM ~400 Mo.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| cryptpad.fr | **§B-23 PERSISTANT** — splash « Loading… », doc_h=8, **1 seul éval** (2 217 o) sur 55 `<script>` ; disjoncteur §B-41 armé ×6 | « pas de journal » (recorder vide) | chaîne requirejs/less **jamais évaluée** — scripts injectés absents du journal : variante §B-23 sans `deadline exceeded`, recoupe §B-4/§B-19 ; heap 52 Mo |
| bookshop.org | **§B-12 PERSISTANT** — 5 entrées `heap memory limit exceeded` signature identique (1 078 884 cells, 540,9 Mo) dont ×11 et ×3 dédup | beacon.min.js + klaviyo + inline | cap 1/moteur Étape 153 **inopérant terrain** ; page néanmoins peinte, doc_h=9 738 |
| frandroid.com | **§B-12 ×0** (cap confirmé ×3ᵉ vol) ; **§B-46 non reproduit ×4** — navigate 82,4 s, doc_h=17 348, DELETE 0,24 s | `Element missing` ; `moteur js inerte` ×16+ | rendu dégradé : zone centrale vide, cartes sans images (moteurs inertes post-heap) |
| manomano.fr | **§B-57 PERSISTANT** — `expression too deeply nested` **at ToObject** sur `polyfill.min.js` cdnjs → redirect `/navigateur-incompatible`, doc_h=0 | 1 entrée nommée | JS-C178 a dépilé l'épine `eval_expr` ; la récursion `ToObject` (autre chemin) reste au-dessus de la borne |
| caniuse.com | **§B-57 ×2ᵉ site** — `deeply nested` sur `bundle.js` ; page **rendue** (SSR, ~95 % fidèle) ; **§B-37 tient** (0 panic dom.rs) | 2 entrées nommées + `osdlfm` iframe | — |
| flixbus.fr | **§B-54 REPRODUIT** — navigate retourné 24,6 s, **puis wedge pendant vie de l'onglet** : `event.poll_net` ×51, `event.mirror_sync` 9,9 s, `event.dispatch` 5 s, API morte >60 s (tabs + frame + DELETE timeout), livelock (log actif) → arrêt forcé PID | 200 ms=0 puis API morte | Étape 154 (bras post-DELETE) **non mesurable** — wedge pré-DELETE ; le budget `event.*` défère la queue mais les phases longues continuent |
| geeksforgeeks.org | **§B-7 PERSISTANT** — frame blanche, `visibility:hidden` ×1 survivant | `unhandledrejection: getInitialProps of Undefined` (main-*.js, **tracé**) | **réattribution** : hydrate Next mort en amont (§B-4), pas `cssText` — JS-C174 non testable sans boot vivant ; dom_nodes=614, js_heap 42,7 Mo |
| clubic.com | **§B-45 RÉSOLU** — doc_h=11 940 (vs 900 verrouillé), `scroll_y=1600` effectif, contenu scrollé peint | `currentScript` invariant Next.js ; `moteur figé — éval dépassée` ; inerte ×3 | résidu layout : chevauchement de texte colonne droite (famille §B-18) |

## Preuves marquantes

### §B-23 — cryptpad : variante sans deadline

```
script: eval… tab=1 bytes=2217  → eval fin ms=0 ok=true
(plus aucun eval ensuite — 55 <script> au DOM, require×116)
journal DOM rejeté → repli re-parse  ×3 (Fingerprint, UnmappedKey)
tempête de pompes — disjoncteur armé (B-41) streak=64 ×6
doc_h=8, frame = splash « Loading… » (vol100_cryptpad.png)
```

La fenêtre boot JS-C179 couvre les pompes pendant l'éval boot —
ici l'éval du script de boot **n'a jamais lieu** : la chaîne
d'injection dynamique (requirejs → less.min.js) n'atteint pas le
canal. Distinct du bras « deadline » — nouvelle frange §B-23.

### §B-12 — bookshop : cap 1/moteur inopérant

```
RangeError: heap memory limit exceeded
  (1078884 cells, 540871055 B live > 536870912 B)
  at <anonymous> (inline:15:1:6394)
  at beacon.min.js:1:697          ×11 (dedup)
  at klaviyo vendors~reviews…js   ×2 entrées
  at beacon.min.js:1:697          ×3 (dedup)
```

→ 5 entrées au ring malgré `heap_dead` + routage Étape 153 :
les occurrences émanent probablement de **moteurs multiples**
(cap = 1 *par moteur* — la rafale cross-engine n'est pas
plafonnée) ou le drain post-`heap_dead` réarme avant émission.

### §B-54 — flixbus : wedge migré « pendant vie »

```
22:13:49 event.* au-delà du budget — queue différée (B-54) elapsed=15 099
22:13:54 event.dispatch    us=5 009 516
22:14:04 event.mirror_sync us=9 948 820
GET /api/tabs  → 000 ×3 (10 s), /frame → 000, DELETE → impossible
kill PID 4340 — livelock (log actif jusqu'au bout)
```

Bras « navigate » borné (retour 24,6 s), bras « post-DELETE »
(É154) non testable : la starvation survient **avant** le
DELETE — les phases `event.*` longues reprennent après chaque
defer. §B-54 reste OUVERT.

## Arrêt des daemons

- PID 4340 (9620) : wedged flixbus → `Stop-Process -Force`
  (PID ciblé).
- PID 12168 (9630) : DELETE propres ×3 (gfg, clubic + onglet),
  `Stop-Process`, RAM 404 Mo stable, `{"tabs":[]}`.
- 6 DELETE propres sur l'ensemble du vol (purge ~400 Mo).

## Re-vols reportés au vol 101

- **airfrance** → §B-56 bras 2 : CORRECTIF JS-C180 `897f523e`
  (`\p` sloppy en classe = identité) — post-binaire →
  **priorité 1**.
- **twitch / sosh / edf** → §B-51 : Étape 155 `83d12755`
  (scripts vus vs évals observables) — post-binaire →
  **priorité 1**.
- **cryptpad** → §B-23 bras « chaîne dynamique non évaluée »
  (pas de correctif posé — à remonter en file).
- **bookshop** → §B-12 : cap par-moteur insuffisant (rafale
  cross-engine / drain post-mort) — à remonter en file.
- **manomano** → §B-57 : chemin `ToObject` hors trampoline.
- **flixbus** → §B-54 : wedge « pendant vie » post-É154 —
  borne `event.*` à appliquer au *contenu* des phases.
- geeksforgeeks → §B-7/§B-4 : hydrate `getInitialProps` mort.

<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 81 : cultura · micromania · franceculture `[V7-main]`

- **Binaire** : `[V7-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`.
- **Mission** : 3 cibles fraîches « culture/médias » — cultura.com,
  micromania.fr, franceculture.fr.
- **Daemon** : PID 21856, port 9464, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol81_daemon.log` → `logs/daemons/_vol81_daemon.log`
  (~9 267 lignes).
- **RAM** : base ~356 Mo → **pic 4 794 Mo** pendant l'apply radiofrance
  (franceculture redirige dessus — cible liste prudence) → purge
  complète **434 Mo** post-DELETE. Seuil 1,5 Go franchi : arrêt
  appliqué après collecte télémétrie sommaire.
- **Fenêtre** : ~17:31 → 17:34 UTC (vol court — 2 cibles anti-bot,
  1 wedge prudence).
- **Verdict global** : **0 site réellement visité** — cultura = 403
  sec (externe), micromania = challenge **Incapsula/Imperva** (externe,
  nouvelle signature catalogue), franceculture = redirect
  radiofrance → **§B-44 reproduit verbatim** (escalade quadratique
  apply : cascade 35 s / boxtree 39 s / layout 47 s / repaint 63 s,
  RAM +4,4 Go) avec purge saine post-DELETE.

---

## 1. cultura.com — 403 sec, page d'erreur moteur propre

`POST /navigate` → **234 ms** — réponse
`{"error":{"handled":true,"message":"HTTP 403"}}`. Aucune page servie.

**Rendu** (`captures/vol81_cultura.png`) : page d'erreur interne
peinte correctement — « Page inaccessible / La page
`https://www.cultura.com/` n'a pas pu être chargée. / **Le serveur a
répondu « HTTP 403 ».** / Réessayez plus tard… » — mise en page propre,
`display_items=29`, `layout_boxes=18`.

**`/console` verbatim** (`logs/vol81_cultura_console.json`) :
`{"console":["HTTP 403"]}` — erreur remontée.

**Télémétrie** (`logs/vol81_cultura_mem.json`) : `dom_nodes=0`,
`js_heap=0` (aucun script exécuté — 403 au fetch document).
`/dom` → « pas de DOM », `/layout` → « pas de layout ».

Scroll ×1 → `scrolled scroll_y=0` (page d'erreur = viewport) ;
clic (300,400) → `ignored` (hit=false, attendu).

**Constat** : anti-bot 403 — **ajouter cultura.com à la liste 403
externe** (§B « Externe » : ebay, etsy, stackoverflow… — signature
cumulée UA/fingerprint rustls déjà documentée).

## 2. micromania.fr — challenge Incapsula/Imperva, éval `ok=false` silencieuse

`POST /navigate` → **1,0 s** — `doc_h=8,0`, `title:null`, URL inchangée.

**DOM** (`logs/vol81_micromania_dom.txt`) : coquille anti-bot —
`<meta name="robots" content="noindex,nofollow">` + unique
`<script src="/_Incapsula_Resource?SWJIYLWA=5074a744e2e3d891814e9a2dace20bd4,…">`.
11 nœuds, 2 boîtes (`block<html>` 1366×0, `block<body>` 1350×0),
`display_items=0` → frame **blanche** (`captures/vol81_micromania.png`).

**Journal daemon — constat §B-51** : le script de challenge
**103 006 octets** est évalué : `script: eval… tab=2 bytes=103006` →
`eval fin ms=703 ok=false` — et **`/console` retourne
« pas de journal »** : l'échec d'éval n'atteint ni la console ni un
WARN. Même signature que messenger vol 78 (§B-51) mais sur éval
**initiale** (pas post-re-parse) et en `ms=703` (échouée en cours
d'exécution, pas instantanée) → la famille §B-51 couvre donc aussi
les évals du premier parse.

**Télémétrie** (`logs/vol81_micromania_mem.json`) : `js_heap=975 438`
(le challenge a tourné ~700 ms avant de jeter), `dom_nodes=11`,
`journal_fallbacks=0`.

Scroll ×2 → `scroll_y=0` ×2 (doc=8 px — légitime) ; clic (683,450) →
`ignored` (hit=false).

**Constats** : **anti-bot Incapsula/Imperva — nouvelle signature
externe** pour le catalogue (ni Datadome ni Akamai ni Turnstile) ;
§B-51 élargi (éval initiale `ok=false ms>0` sans trace console).

## 3. franceculture.fr → radiofrance — §B-44 reproduit verbatim `[V7-main]`

`POST /navigate` → **13,8 s** — redirection suivie vers
`https://www.radiofrance.fr/franceculture` (cible liste prudence,
porteur §B-44 vol 68), titre complet « France Culture – Écouter la
radio en direct et podcasts gratuitement », `doc_h=25 315,3`
(`clamped:true` — dépasse le clamp 16 384 ? non : `doc_h` brut
mesuré à 25 315 mais `clamped:true` indique la limite atteinte côté
métrique).

**Premier apply sain** (17:32:27) : `cascade ms=75`, `boxtree ms=146`,
`layout ms=349`, `display_list items=12 774`, `repaint ms=389` —
la page SSR complète a existé.

**Puis re-parse (ADR-0025) → escalade §B-44 verbatim** (17:32:40 →
17:34:08) : `parse ms=318` → `cascade ms=34 983` → `boxtree
ms=38 904` → `layout ms=47 318` → `display_list items=3 795` →
**`repaint ms=63 468`**. Même signature que le porteur initial
(radiofrance vol 68 : cascade 46 s / boxtree 55 s / layout 72 s /
repaint 363→9 558→199 721 sur le même site) — ici repaint total
~64 s sur l'apply re-parse vs 389 ms sur l'apply initial. **La borne
§B-26 `emit_scoped_at` n'a pas tiré** (0 `dropped` dans le journal) —
mécanisme distinct confirmé, comme documenté.

**RAM** : 395 Mo (2 onglets) → 4 794 Mo pendant l'apply —
croissance continue pendant les phases cascade/boxtree/layout
(+~90 Mo/s mesuré, cohérent avec la note §B-26 radiofrance).

**API restée responsive pendant tout le wedge** : `/console`,
`/frame` (204 — apply en vol, DOM détaché transitoire documenté),
`/mem` (`dom_nodes=0`, `js_heap=6 415 427`) servis en <15 s —
distinct de §B-33 (journal non muet) et §B-46 (phases journalisées,
fin atteinte).

**Évals** : 5 scripts — 443 o `ok=true`, 2 818 o `ok=true`, 599 o
`ok=true`, **4 224 o `ok=false ms=0` silencieux** (même signature
§B-51), 807 836 o `ok=true ms=38`. `/console` : « pas de journal ».

**Arrêt prudence** : DELETE des 3 onglets (204, <200 ms) pendant
l'apply en vol → l'apply a continué jusqu'à `repaint ms=63 468`
(fin journalisée), puis **purge mémoire complète** :
4 794 → 3 737 → 3 371 → 3 205 → 2 853 → … → **434 Mo stable**,
`{"tabs":[]}`, API responsive. **Signal §B-30 favorable de plus**
(comme orsay vol 67 : l'in-flight se purge une fois l'apply terminé).

Pas de scroll/clic tentés : onglet supprimé sur seuil prudence avant
interaction (frame 204 pendant tout le wedge — rien à viser).

**Statut §B-44 : TOUJOURS PRÉSENT `[V7-main]`** — reproduction à
l'identique sur le même site (radiofrance via franceculture) ;
stabilisation confirmée (API vivante, purge complète), perf à
profiler (à planifier : profilage cascade 35 s + repaint 63 s).

---

## Synthèse des constats

| Cible | Résultat | IDs §B | Crate |
|---|---|---|---|
| cultura.com | 403 sec, page d'erreur moteur peinte proprement | Externe — +1 entrée liste 403 | — (externe) |
| micromania.fr | Challenge **Incapsula/Imperva** : coquille 11 nœuds, frame blanche ; éval 103 Ko `ok=false ms=703` **sans trace console** | Externe — **nouvelle signature anti-bot** ; §B-51 élargi (éval initiale, ms>0) | `explorer-jscore`/`core` (traçage) |
| franceculture.fr → radiofrance | Redirect suivi ; 1er apply sain ; **re-parse : escalade §B-44 verbatim** (repaint 63 s, RAM +4,4 Go) ; API vivante ; purge post-DELETE complète | §B-44 **confirmé V7** ; §B-30 signal favorable ; §B-51 (eval 4 224 o `ok=false` muette) | `explorer-core`/`layout` |

### Mises à jour de statut suggérées (SYNTHESE.md)

- **§B-44** : annoter « reproduit verbatim `[V7-main]` vol 81 sur
  radiofrance (via franceculture.fr) — cascade 34 983 / boxtree
  38 904 / layout 47 318 / repaint 63 468 ms, RAM +4,4 Go, borne
  §B-26 non déclenchée, API vivante, purge post-DELETE complète ».
- **§B-51** : élargir — l'absence de trace couvre aussi les évals
  **initiales** `ms>0` (micromania 103 Ko `ok=false ms=703`),
  pas seulement les `ms=0` post-re-parse.
- **§B-30** : +1 signal favorable (vol 81 : DELETE pendant apply
  radiofrance en vol → fin d'apply → purge 4 794→434 Mo).
- **Externe** : ajouter **Incapsula/Imperva** (micromania) comme
  signature anti-bot distincte ; ajouter **cultura.com** à la liste
  403 sec.
- **Liste prudence** : franceculture.fr à ajouter (redirect
  systématique vers radiofrance.fr, porteur §B-44).

## Artifacts

- Captures : `captures/vol81_cultura.png` (page erreur 403),
  `captures/vol81_micromania.png` (blanche — coquille Incapsula),
  `captures/vol81_franceculture.png` (vide — frame 204 pendant wedge).
- Logs : `logs/vol81_cultura_{console.json,dom.txt,layout.txt,mem.json}`,
  `logs/vol81_micromania_{console.json,dom.txt,layout.txt,mem.json}`,
  `logs/vol81_franceculture_{console.json,mem.json}` ;
  journal daemon `logs/daemons/_vol81_daemon.log`.
- Arrêt propre : DELETE onglets 1-3 (204 ×3) puis kill du PID 21856.

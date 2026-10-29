<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Vol 79 — REVALIDATION `[V7-main]` — 2026-10-27

**Agent 79 · mission REVALID** — re-vols ciblés sur les fixes du
binaire `[V7-main]` : **§B-50** (`3e91c1dd` — EOF clos une déclaration
à parenthèse non fermée), **§B-44** (`e34208db` — plafond DOM
miroir), **§B-51** (`3ff1da64` — `emit_pipeline_error` n'avale plus),
**`userAgentData`** (`8dcd96ed`), **pompes bornées** (`17f5cfb5`,
`b93107c5`, `b9189d5e`).

- Binaire : `C:/obe-vol-target/release/explorer-daemon.exe`
  (20 861 440 o, 2026-10-27 18:30 — postérieur à tous les commits
  code ; seul `d6519c0d` docs est plus récent)
- Daemon dédié : port **9462**, `EXPLORER_VIEWPORT=1366x900`,
  PID 7184, journal racine `_vol79_daemon.log`
- Protocole : `docs/vol-agent.md` — traçabilité §8 (crate + ID §B,
  tag binaire)

**Santé du vol** : API responsive en permanence, aucun wedge, aucun
crash. **Alerte RAM franchie** : 2,01 Go pendant la pompe continue
de messenger (seuil mission 1,5 Go) → DELETE des 4 onglets → retombée
à **493 Mo**, daemon tué par PID. La montée est portée par la pompe
applicative messenger (~2 apply/s en continu), pas par une fuite —
purge complète post-DELETE.

---

## Cible 1 — https://www.bordeaux.fr → §B-50 (DOM jamais réattaché)

### Verdict : **RÉSOLU** `[V7-main]`

| Mesure | `[V6-main]` vol 73 | `[V7-main]` vol 79 |
|---|---|---|
| Repli `UnmappedKey` | re-parse → `dom` 404 / `frame` 204 >7 min | rejet à 17:31:52.596 → `parse ms=14` → `cascade ms=370` → `boxtree 421` → `layout 539` → `repaint ms=726` — **commit complet** |
| `/dom` `/layout` `/mem` `/frame` | 404 persistant | **200 / 200 / 200 / 200** |
| `dom_nodes` | 0 | **7 187** (21 343 lignes dump) |
| `/console` | — | `pas de journal` |
| Scroll | bloqué | `scroll_y` 800 → 1 600 OK |

La séquence de guérison promise par le fix est verbatim dans le
journal daemon : le re-parse n'est plus piégé dans la cascade —
`cascade` est journalisé puis `layout` puis commit (`journal_fallbacks=1`
au `/mem`). La page se peint : hero « La ville avec et pour vous »,
barre de recherche, tuiles « Accès rapide », bandeau cookies
(`captures/vol79_bordeaux.png`).

**Restes** (≠ §B-50) : menu de navigation peint **déplié** — liens
superposés en colonne (captures t0) — famille §B-17 (menus fermés
par JS non appliqués) ; un WARN `cycle de report cassé (§B-26)
dropped=1` isolé, non bloquant.

## Cible 2 — https://lumiplan.com → §B-50 variante oscillante

### Verdict : **RÉSOLU** `[V7-main]` — fin de la boucle re-parse

| Mesure | `[V6-main]` vol 76 | `[V7-main]` vol 79 |
|---|---|---|
| Oscillation re-parse | boucle sans fin | `journal_fallbacks=2` puis **stable** — `/dom` 200 confirmé à +5 s, `/frame` 200 idem |
| `dom_nodes` | clapotait 404↔200 | **1 591**, stable |
| Repaints | — | stables 388–498 ms (pas d'escalade) |
| Scroll | — | `scroll_y` 800 → 1 600 OK |

Un rejet `UnmappedKey` à 17:33:10 suivi d'un re-parse complet, puis
le DOM reste attaché : sondes répétées 200. Page peinte (cartes
« Mairies / Acteurs de mobilité / Offices de Tourisme / Domaines de
montagne », compteurs, onglet CONTACT latéral —
`captures/vol79_lumiplan.png`).

**Conjoint** : 1 éval `ok=false ms=3` à 17:33:05 (boot) — voir §B-51
ci-dessous.

## Cible 3 — https://messenger.com → §B-51 (évals `ok=false` silencieuses)

### Verdict §B-51 : **TOUJOURS PRÉSENT** `[V7-main]` — mécanisme affiné

| Mesure | `[V6-main]` vol 78 | `[V7-main]` vol 79 |
|---|---|---|
| Évals `ok=false` | 7 invisibles | **1** (`ms=0`, script 1 202 o, 17:34:03.218 — journal daemon) |
| `/console` | vide | **vide** — `pas de journal` |
| Repli `UnmappedKey` | oui | 1× à 17:34:05.7 (`journal_fallbacks=3`) |

Le `PipelineError` est bien **émis** désormais — preuve par
contre-expérience : nav d'un onglet témoin (tab 5) vers
`https://nonexistent-domain-xyz.invalid/` → `/console` =
`["fetch : connexion à … Hôte inconnu. (os error 11001)"]`. Le ring,
la dédup `×N` et `emit_pipeline_error` (js_glue.rs:495-522)
fonctionnent.

**Mécanisme résiduel suspect** (`explorer-api`/`explorer-core`) :
les évals de scripts de **boot** s'exécutent *pendant* le commit
initial, **avant** le `Navigated` émis en queue de commit
(`net.rs:341`). Or `record_console_event` **vide le ring sur
`Navigated`** (`explorer-api/src/lib.rs:108-117`) — toute erreur
d'éval émise pendant le boot est enregistrée puis **purgée** par le
`Navigated` qui la suit à ~200 ms. L'échec fetch, lui, arrive après
`Navigated` → survit. Conséquence : le trou d'observabilité §B-51 est
réduit mais pas fermé — les erreurs de boot (les plus précieuses)
restent avalées par l'ordre « erreur → Navigated → clear ». Piste :
ne vider le ring que sur `Navigated` **cross-document** (ou horodater
et ne purger que les entrées antérieures au début de la nav).

**`userAgentData` : RÉSOLU / présent** `[V7-main]` — sondé en direct
via `POST /eval` sur l'onglet messenger :
`typeof navigator.userAgentData` → `"object"`,
`.mobile` → `false`, `.platform` → `"Windows"`,
`typeof .getHighEntropyValues` → `"function"`.

**Pompes bornées / §B-44** : messenger tient une pompe applicative
continue (~2 apply/s, `display_items` oscillant 481↔532) mais les
coûts sont **plats** — repaint 388–485 ms en régime, max observé
1 074 ms sur tout le vol, cascade ≤ 418 ms. **Aucune escalade
quadratique** sur les trois cibles (le symptôme §B-44 reste
radiofrance-spécifique — déjà reproduit verbatim `[V7-main]` par
vol 81, cf. §B-44 ligne catalogue). Scroll messenger : `scroll_y`
reste 0 — `doc_h`=554 < viewport, rien à scroller (normal, login
page). Page de login peinte proprement (logo, formulaire, badges
stores, footer — `captures/vol79_messenger.png`).

---

## Synthèse des verdicts

| ID | Fix binaire | Cible | Verdict `[V7-main]` |
|---|---|---|---|
| §B-50 | `3e91c1dd` css | bordeaux.fr | **RÉSOLU** — re-parse complet, DOM réattaché (2ᵉ confirmation) |
| §B-50 var. | `3e91c1dd` css | lumiplan.com | **RÉSOLU** — oscillation terminée, DOM stable |
| §B-51 | `3ff1da64` core | messenger.com | **TOUJOURS PRÉSENT** — `emit_pipeline_error` émet (preuve fetch) mais erreurs de **boot** avalées par le clear-on-`Navigated` du ring |
| `userAgentData` | `8dcd96ed` jscore | messenger.com | **RÉSOLU** — objet complet + `getHighEntropyValues` en live |
| §B-44 / pompes | `e34208db` + `17f5cfb5`… | les 3 cibles | non reproduit ici — repaints plats ≤1,1 s (radiofrance reste le porteur §B-44, vol 81) |

## Proposition §B (trou résiduel à préciser)

- **§B-51 — préciser le périmètre** : le fix `emit_pipeline_error`
  fonctionne post-`Navigated` ; le résidu = erreurs **pendant le
  commit de boot** purgées par le clear du ring sur `Navigated`.
  Suggestion d'entrée ou de note §B-51 : « erreurs d'éval boot
  effacées par `Navigated` — ring `/console` vidé après
  enregistrement ». Crate : `explorer-api` (record_console_event)
  / `explorer-core` (ordre d'émission).

## Fichiers produits

```
captures/vol79_bordeaux.png          captures/vol79_bordeaux_scroll.png
captures/vol79_lumiplan.png          captures/vol79_lumiplan_scroll.png
captures/vol79_messenger.png         captures/vol79_messenger_scroll.png
logs/vol79_bordeaux_{dom,layout,mem,console}.txt
logs/vol79_lumiplan_{dom,layout,mem,console}.txt
logs/vol79_messenger_{dom,layout,mem,console}.txt
_vol79_daemon.log (racine)
```

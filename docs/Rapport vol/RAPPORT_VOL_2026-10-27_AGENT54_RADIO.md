<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 54 — Radios (Radio France ×3) (2026-10-27)

- **Binaire** : `[V4-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (WIP `depth_limit` embarqué, §B-3 confirmé ouvert), `--port 9436`,
  `EXPLORER_VIEWPORT=1366x900`, journal `_vol54_daemon.log` (racine)
- **Cibles** : www.radiofrance.fr · www.rfi.fr · www.fip.fr —
  **1/3 tentée : vol interrompu par reproduction §B-26** (consigne
  « >2 Go → DELETE les onglets et termine » appliquée). rfi.fr et
  fip.fr **non volés** — les trois appartiennent au groupe Radio
  France (socle front probablement partagé → même risque de wedge).
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de
  `SYNTHESE.md`
- **RAM daemon (PID 16108)** : 375 Mo au boot → **4,16 Go à T+~80 s**
  → **5,78 Go à T+~95 s** (~90 Mo/s) → DELETE des 2 onglets → 4,4 Go
  → 4,66 Go résiduel (libération partielle ~1,4 Go seulement, le
  reste resté résident) → kill par PID.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| www.radiofrance.fr | **Wedge §B-26 — fuite 5,8 Go en ~95 s, apply sans fin** | `pas de journal` (0 erreur tracée) | **§B-26 confirmé `[V4-main]`** + nuance « API restée responsive » |
| www.rfi.fr | non volé (vol interrompu) | — | — |
| www.fip.fr | non volé (vol interrompu) | — | — |

Fait marquant : **la page a rendu** (premier apply complet en 435 ms,
doc_h=28 626), puis **deux applies post-JS en escalade exponentielle**
(0,4 s → 9,2 s → >49,5 s, mort entre `layout` et `repaint`) pendant
que la RAM doublait par paliers de ~1,5 Go / 15 s — **et zéro entrée
console**.

---

## 1. www.radiofrance.fr — reproduction §B-26 avec escalade d'apply

- `POST navigate` → réponse saine : `doc_h=28 626` (**clamped**),
  titre « Radio France | Écoutez en ligne vos radios », URL finale
  `https://www.radiofrance.fr/`.
- Journal daemon — **apply initial sain** : `cascade 91 ms → boxtree
  177 ms → layout 392 ms → repaint 435 ms` (une frame a existé).
- Évals JS : série de petits scripts `ok=true`, un script de 4 225 o
  **`ok=false` non remonté dans `/console`** (bras §B-4 — échec
  d'éval silencieux), puis le bundle applicatif de **899 377 o
  évalué `ok=true` en 65 ms**. Rafale d'`event.dispatch` ensuite
  (timers/microtasks applicatifs — famille §B-1).
- **Escalade des applies** (ms cumulés dans le journal) :

  | Apply | cascade | boxtree | layout | repaint |
  |---|---|---|---|---|
  | #1 (09:22:56) | 91 | 177 | 392 | **435 — terminé** |
  | #2 (09:23:07) | 6 261 | 6 687 | 7 773 | **9 234 — terminé** |
  | #3 (09:24:16) | 37 783 | 41 651 | **49 513** | **jamais** |

  → signature §B-26 au mot près : « apply journalisé jusqu'à
  `layout`, jamais `repaint`/`fin` » — et l'apply n°3 était encore
  en vol quand le daemon a été tué (~60 s après son début).

- À T+~80 s : `/console` → `pas de journal` ; `/dom` → **404** ;
  `/frame` → **204** (0 octet) ; `/mem` → `dom_nodes=0`,
  `layout_boxes=0`, `journal_applies=0`, `js_heap=6,4 Mo` — **DOM
  détaché terminal**, la fuite est hors tas JS (catalogue §B-26 :
  « fuite mémoire sans borne », ici dans le pipeline apply/layout
  ou les rasters en vol).
- **RAM** : 375 Mo → 4,16 Go → 5,78 Go en ~95 s. Le DELETE a rendu
  ~1,4 Go (5,78→4,4) puis remontée à 4,66 Go — **libération
  partielle**, à distinguer de linuxmint (vol 51 : retombée propre
  à 510 Mo) ; résidu proche des constats caf/figaro vol 47.
- **Nuance vs vol 49 (bfmtv)** : l'API REST est restée **entièrement
  responsive** pendant toute la fuite (chaque `curl` servi) — le
  wedge est confiné au flux tab, l'exécuteur ne sature pas un cœur
  au point de muer l'API. Variante §B-26 à consigner (vol 49 disait
  « API entière morte » ; ici non — peut-être parce que l'apply n°3
  n'avait pas atteint le stade de saturation observé chez bfmtv).
- Aucune capture : `/frame` 204 avant tout PNG —
  `captures/vol54_radiofrance.png` = 0 octet (conservé comme trace
  du 204). Pas de scroll/clic possible : la page était déjà en
  wedge au moment de la grâce.

## Statuts proposés (§8)

| §B | Bras | Verdict ce vol |
|---|---|---|
| §B-26 | wedge DOM détaché terminal post-apply + fuite non bornée | **confirmé `[V4-main]`** — radiofrance.fr : 5,78 Go/95 s, apply #3 mort après `layout` (49,5 s), DELETE libération partielle ; **variante : API responsive** |
| §B-1 | starvation post-apply (rafale événements) | recoupement — escalade d'apply 0,4→9,2→49,5 s corrélée aux `event.dispatch` en rafale ; même famille « pas de budget post-apply » |
| §B-4 | morts silencieuses | recoupement — `script eval ok=false` (4 225 o) absent de `/console` ; wedge entier sans aucune entrée |

Pas de nouvelle entrée §B proposée : le constat s'insère dans §B-26
existant (preuve terrain supplémentaire + nuance « API vivante » +
escalade d'apply mesurée, à reporter dans la cellule Preuve).

## Hygiène de vol

- Onglets 1 (vide, créé par accident de sondage du format `id`) et
  2 (radiofrance) fermés par `DELETE /api/tabs/{id}` — DELETE
  effectué dès dépassement du seuil 2 Go (RAM 5,78 Go).
- Daemon tué par son **PID 16108** — `taskkill //PID` (jamais
  `/IM`) ; les daemons voisins (PID 3444, 17436) n'ont pas été
  touchés.
- Artefacts : `captures/vol54_radiofrance.png` (0 o — trace 204),
  `logs/vol54_radiofrance_{console.json,dom.txt,mem.txt}`,
  journal `_vol54_daemon.log` (racine). Pas de `layout.txt` —
  l'endpoint n'a jamais quitté l'état détaché.

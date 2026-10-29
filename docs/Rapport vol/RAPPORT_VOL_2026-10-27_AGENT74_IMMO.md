<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 74 : immobilier & petites annonces `[V6-main]`

- **Binaire** : `[V6-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (contient le fix §B-26 `1e24942`, borne `emit_scoped_at` — non tirée
  sur ce vol).
- **Mission** : 3 cibles annonces — logic-immo.com, lacentrale.fr,
  **leboncoin.fr en re-vol Datadome** (403 documenté vol 37 `[V4-main]`
  et mission équipe 6). Cibles prudence §B-26/§B-30/§B-41/§B-43/§B-46/
  §B-48 évitées.
- **Daemon** : PID 19696, port 9456, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol74_daemon.log`. **RAM max ~414 Mo** — très loin du
  seuil de prudence 1,5 Go (le vol s'est déroulé entièrement en
  erreur HTTP, aucune page réelle servie). Daemon parallèle
  PID 16800 (autre vol) non touché.
- **Fenêtre** : 14:46 → 14:48 UTC (~2 min).
- **Verdict global** : **3/3 cibles refusées par l'edge — HTTP 403
  instantané** (191-582 ms), page d'erreur interne propre peinte sur
  les trois, **0 constat moteur, 0 wedge, 0 fuite**. leboncoin :
  Datadome confirmé pour la 3ᵉ fois ; logic-immo et lacentrale
  rejoignent la famille « 403 sec » du catalogue externe.

---

## 1. logic-immo.com — HTTP 403 externe

`POST /navigate` → `{"error":{"handled":true,"message":"HTTP 403"}}`
en **582 ms**. `/console` verbatim : `{"console":["HTTP 403"]}`.
`/dom` → « pas de DOM », `/layout` → « pas de layout » ; `/mem` :
`layout_boxes=18`, `display_items=30`, `frame_bytes=4 917 600`,
`js_heap=0` (`logs/vol74_logicimmo_mem.json`).

**Rendu** (`captures/vol74_logicimmo.png`) : page d'erreur interne
fidèle — « Page inaccessible — La page https://www.logic-immo.com/
n'a pas pu être chargée. Le serveur a répondu « HTTP 403 ». »

**Classification** : **externe** — refus sec en ~0,6 s, aucune
interstitielle challenge. Même signature que les refus edge
catalogués (ebay, etsy, seloger…) ; logic-immo n'était pas encore
listé → **entrée externe à ajouter** (« 403 sec — logic-immo »,
probablement Datadome comme leboncoin, mais refus sec non
caractérisé ici).

## 2. lacentrale.fr — HTTP 403 externe

`POST /navigate` → `{"error":{"handled":true,"message":"HTTP 403"}}`
en **206 ms** — refus quasi instantané. `/console` :
`{"console":["HTTP 403"]}`. `/dom`/`/layout` → « pas de DOM » /
« pas de layout » ; `/mem` : `layout_boxes=18`, `display_items=29`
(`logs/vol74_lacentrale_mem.json`).

**Rendu** (`captures/vol74_lacentrale.png`) : page d'erreur interne
identique (URL substituée).

**Classification** : **externe** — 403 sec ~200 ms, edge sans
challenge → **entrée externe à ajouter** (« 403 sec —
lacentrale »).

## 3. leboncoin.fr — HTTP 403 externe — **Datadome confirmé 3ᵉ fois**

`POST /navigate` → `{"error":{"handled":true,"message":"HTTP 403"}}`
en **191 ms** — le refus le plus rapide des trois. `/console` :
`{"console":["HTTP 403"]}`. `/dom`/`/layout` → « pas de DOM » /
« pas de layout » ; `/mem` : `layout_boxes=18`, `display_items=29`
(`logs/vol74_leboncoin_mem.json`).

**Rendu** (`captures/vol74_leboncoin.png`) : page d'erreur interne
identique.

**Classification** : **externe — Datadome (catalogue à jour)** —
3ᵉ confirmation (mission équipe 6 vol 1 : 403 + END_STREAM ~10 ms ;
vol 37 `[V4-main]` : 403 + page d'erreur propre). Contrairement au
vol 1 équipe 6 qui recevait un corps d'interdiction, le refus est
ici **sec** (aucun challenge servi, aucun cookie `datadome` à
jouer) : la signature cumulée (`UA: Explorer/0.1`, rustls, pas de
`sec-ch-ua`/`sec-fetch-*`) est probablement classée bot à la
première requête.

**Interactions testées malgré tout** (sondage de la page d'erreur) :
`scroll dy=800` → `{"result":"scrolled","scroll_y":0.0}` (doc_h =
viewport, butée nominale) ; `click (683,450)` → `{"result":"ignored"}`
journalisé « aucun contrôle/lien actionnable sous le point tab=3
hit=false » — comportement **nominal** : la page d'erreur interne
ne porte aucune cible.

## 4. Tableau de synthèse

| Cible | Navigate | Rendu | Scroll | Clic | Anomalies |
|---|---|---|---|---|---|
| logic-immo.com | **HTTP 403** / 582 ms | page d'erreur propre | — | — | externe 403 sec — à cataloguer |
| lacentrale.fr | **HTTP 403** / 206 ms | page d'erreur propre | — | — | externe 403 sec — à cataloguer |
| leboncoin.fr | **HTTP 403** / 191 ms | page d'erreur propre | `scrolled` scroll_y=0 (butée, nominal) | `ignored` (aucune cible, nominal) | externe Datadome confirmé ×3 — refus sec |

## 5. Observations annexes

- **Dégradation externe exemplaire** : les trois refus produisent
  une page d'erreur interne peinte correctement (18 boîtes,
  ~29-30 items, repaint 9-335 ms), `/console` porte « HTTP 403 »,
  titre onglet « Erreur de chargement » propre en JSON (**pas de
  mojibake §B-47** — mais le titre ne contient aucun accent, test
  non discriminant).
- **Curiosité observabilité mineure** : `/mem` rapporte
  `layout_boxes=18` / `display_items=29-30` alors que `/dom` et
  `/layout` répondent littéralement « pas de DOM » / « pas de
  layout » — la page d'erreur est peinte via un payload sans
  document attaché. Cohérent avec la note équipe 6
  (« layout+repaint exécutés sur corps d'erreur 403 » — piste
  d'optimisation déjà répertoriée, hors §B).
- **Aucune activité moteur au-delà de la page d'erreur** : 0 éval
  JS, 0 fallback journal, 0 cycle §B-26, 0 nav auto (§B-2 tient —
  pas de re-navigation sur interstitiel puisqu'aucun n'est servi),
  RAM stable 375→414 Mo.
- **Signature externe cumulée confirmée** : les trois sites
  filtrent à la première requête, sans challenge — Datadome
  (leboncoin) est caractérisé ; logic-immo et lacentrale partagent
  le même profil de refus sec. Proposition de mise à jour du
  §B-externe : ajouter « lacentrale, logic-immo » à la liste 403.
- **Vol le plus court de la série** — aucune cible n'a franchi
  l'edge ; aucun constat moteur nouveau, aucune proposition §B.

## 6. Arrêt

DELETE tabs 1/2/3 → `{"tabs":[]}`. RAM finale ~414 Mo (aucune
purge nécessaire — rien n'avait chargé). Daemon terminé par
**PID 19696** (`taskkill /PID`, jamais `/IM`) — port 9456 libéré,
vérifié (le daemon parallèle PID 16800, autre vol, non touché).

**Fichiers** : `captures/vol74_{logicimmo,lacentrale,leboncoin}.png`,
`logs/vol74_{logicimmo,lacentrale,leboncoin}_{dom.txt,layout.txt,
mem.json}` (dom/layout = « pas de DOM »/« pas de layout » —
archivés pour traçabilité), journal `_vol74_daemon.log`.

<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 60 — Mobilité (2026-10-27)

- **Binaire** : `[V4-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (binaire WIP depth_limit fourni par l'opérateur, timestamp 09:32)
- **Daemon** : `--port 9442`, `EXPLORER_VIEWPORT=1366x900`,
  journaux `_vol60_daemon.log` (PID 7060) et `_vol60_daemon2.log`
  (PID 11072 — redémarrage après wedge flixbus n°1)
- **Cibles** : thetrainline.com · blablacar.fr (re-vol vol 39, §B-18) ·
  flixbus.fr (tenté ×2)
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de
  `SYNTHESE.md`

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| thetrainline.com | **SSR réel, bon rendu** | vide | §B-18 (cosmétique), **§B-37 proposé** : ancre réelle `ignored` en position scrollée ; iframe YouTube peinte noire ; placeholders lazy non chargés (§B-20) |
| flixbus.fr | **WEDGE — reproduit ×2** | inaccessible | **§B-38 proposé** : escalade des phases `event.*` (poll_net 24-31 s, mirror_sync 10-13 s), POST navigate jamais retourné, API affamée — famille §B-26/§B-33, signature nouvelle |
| blablacar.fr | **SSR réel, layout dégradé** | vide | **§B-18 confirmé** (chevauchements + serif fallback, identique vol 39) ; clic → nav réelle → 403 externe |

Acquis : 2/3 cibles servies avec contenu réel ; 2 navigations réelles
par clic (Railcards trainline, Dunkerque blablacar) ; zéro panic ;
la fuite RAM du wedge n'est **pas** reproduite ici (RAM stable
~0,5-0,9 Go, décroissante) — le wedge flixbus est une starvation
pure, pas une fuite.

---

## 1. thetrainline.com — SSR réel, rendu correct `[V4-main]`

- `POST navigate` → retour propre : `doc_h=4793.26`, titre réel
  « Trainline : Search, Compare & Buy Cheap Train Tickets ».
- `/console` : **« pas de journal »** — aucune erreur JS.
- `/mem` : `dom_nodes=1642`, `layout_boxes=655`, `display_items=569`,
  `js_heap=47,8 Mo`, `journal_fallbacks=0`, `total=78,3 Mo`.

### Rendu (`captures/vol60_trainline.png` + `_scroll.png`)

- Page d'accueil complète et fidèle : header (sélecteur locale UK
  avec drapeau, Railcards, Business, Basket, My Bookings, Register,
  burger), formulaire de réservation (From/To, One-way/Return
  segmenté, date « Wed 21 Oct • 09:30 », passagers « 1 Adult
  (26-59) », « Add a voucher code », CTA « Find cheap tickets »),
  hero « Explore Europe effortlessly by train and bus / Save 61%… »,
  bandeau 4 arguments. Meilleur rendu de la session.
- Cosmétique : zone hero droite grande et vide (image hero absente ?),
  petit artefact barre grise sous le header à gauche, icône loupe/
  compte isolée en haut à droite — résidus mineurs → §B-18.
- Au scroll (`vol60_trainline_scroll.png`, scroll_y=1600) : grande
  boîte bleu pâle vide (~1120×240 px) = image lazy jamais chargée,
  puis section « We're here to help » — la page est structurée mais
  les médias différés n'arrivent pas → recoupe **§B-20** (famille
  lazy-load/`data-src`, JS d'hydratation probablement mort sans
  trace → §B-4).

### Input — NOUVEAU constat §B-37 : ancre réelle `ignored` en scroll

- `scroll` ×2 → `scroll_y=1600` réel.
- Clic sur « FAQs » (viewport 540,645 → doc ~[540,2245]) →
  **`{"result":"ignored"}`**.
  - Vérifié dans `logs/vol60_trainline_layout.txt` : l'ancre est
    layoutée — `inline<a> [510.29 2233.51 45.06x26.6]` contenant
    `text "FAQs"` — et dans le DOM : `<a class="_iuwv49"
    href="/en/help/">`, ancre réelle avec href.
  - Le point cliqué (540, 2245) est **strictement à l'intérieur** de
    la boîte [510,2233 → 555,2260]. Rien de visible ne la recouvre
    (frame nette).
- Contre-test : retour `scroll_y=0`, clic « Railcards » header
  (632,32) → **`{"result":"navigated"}`** → page
  `/trains/great-britain/railcards` réelle (titre « UK Digital
  Railcards for £35… », `can_back=true`).
- **Constat** : le hit-test fonctionne à `scroll_y=0` mais ignore une
  ancre réelle et visible à `scroll_y=1600`. Deux hypothèses :
  (a) décalage scroll dans le hit-test (viewport↔document),
  (b) overlay transparent couvrant la zone cliquée (banniere app /
  consentement `position:fixed` non peint mais hit-testé — variante
  §B-18/§B-17 « élément sous overlay »). À bissecter ; `explorer-
  core`/hit-test ou `explorer-layout`.
- Sur la page railcards (`vol60_trainline_railcards.png`) : iframe
  `inline<iframe> [312 1049 483x400]` (embed YouTube
  `youtube.com/embed/pKTYIpj7wSc`, confirmé dans le DOM) peinte en
  **rectangle noir plein** — sous-frame non chargée ou peinte noir.
  Cosmétique, joint à §B-18 ; à surveiller si les sous-frames
  échouent silencieusement (recoupe §B-4a sur les fetches iframe —
  ici aucun warn console).

## 2. flixbus.fr — WEDGE déterministe ×2 `[V4-main]` — §B-38 proposé

Tenté sur **deux daemons distincts** (PID 7060 puis PID 11072,
même port après kill), même signature les deux fois :

- `POST /api/tabs/2/navigate {flixbus.fr}` → **jamais retourné**
  (timeout curl 90 s puis 60 s).
- `GET /api/tabs` → **muet** (`http=000`, timeouts 8-15 s) pendant
  toute la durée du wedge, avec **une fenêtre transitoire** de
  réponse (`http=200` en 6,35 s à ~10:03) entre deux phases.
- Journal daemon (les deux fois, signature identique) : boucle de
  phases `event.*` en escalade multi-secondes sur le tab flixbus —
  ```
  event.dispatch    us=5.0-5.8 M   (~5 s)
  event.mirror_sync us=10.4-13.3 M (~10-13 s)
  event.poll_net    us=5.0-31.4 M  (~5-31 s)
  ```
  soit ~25-50 s par cycle de pompe, en continu. Aucune phase
  `apply:`/`repaint`/`fin` observée pour le tab — le blocage est
  **en amont** du pipeline d'apply.
- WARN corrélé **sur les deux runs** : `journal DOM rejeté → repli
  re-parse (ADR-0025) why=UnmappedKey` sur le tab wedged — le
  re-parse journal pourrait entretenir la boucle (à investiguer).
- RAM : 986 → 501 → 928 Mo, **stable voire décroissante** — ce
  n'est PAS une fuite ; CPU occupé mais API affamée.
- Le navigate n'a jamais produit de document consultable (`/console`
  a répondu « pas de journal » dans une fenêtre, `/frame` timeout).

**Attribution** : famille §B-26 (« wedge, apply jamais terminé »)
et §B-33 (« `take_nav_paint` busy, navigate en vol ») mais
**signature nouvelle** : ici le navigate reste en vol **pendant**
que la pompe d'événements tourne en phases géantes — ni un apply
mort (§B-26 : dernier log `apply: layout`), ni une boucle `Busy →
sleep(50ms)` silencieuse (§B-33 : `journal_fallbacks=0`, 0 WARN).
Chez flixbus le journal **parle** (phases `event.*` géantes +
fallback UnmappedKey). Proposition **§B-38** : « starvation de la
pompe par phases event.* non bornées (poll_net/mirror_sync) » —
`explorer-core`. Nouveau site déclencheur fiable : flixbus.fr 2/2.

## 3. blablacar.fr — §B-18 confirmé à l'identique `[V4-main]`

Re-vol de la cible vol 39 — résultat **quasi identique** :

- `POST navigate` → `doc_h=15161.0` (identique au vol 39), titre réel.
- `/console` : vide. `/mem` : `dom_nodes=3007`, `layout_boxes=2541`,
  `js_heap=86,5 Mo`, `journal_fallbacks=1` (WARN daemon :
  `journal DOM rejeté → repli re-parse why=Fingerprint`, tab=1).
- `vol60_blablacar.png` : formulaire De/Vers/Départ/Retour/Passagers
  peint en texte plat sans boîtiers, **carrousel d'itinéraires
  (Lille, Amsterdam, Strasbourg, Metz) superposé sur la photo hero**,
  boîte blanche vide chevauchant la photo — mêmes chevauchements de
  runs inline que vol 39 → **§B-18 confirmé, non régressé mais non
  corrigé**.
- `vol60_blablacar_scroll.png` (scroll_y=1600) : le texte du header
  (« Covoiturage Bus Train / Proposer un trajet / Connexion /
  Inscription ») est **peint par-dessus la photo** — header fixed/
  sticky répercuté sur le contenu → §B-18 encore. Police repli
  serif toujours présente (→ §B-18, webfont non appliquée).
- Clic « Dunkerque » (viewport 625,272 → doc [592,1860,135×24]) →
  **`{"result":"navigated"}`** → `/ride-sharing/dunkerque/lille`
  → **HTTP 403** (Datadome, page « Erreur de chargement ») —
  identique au vol 39 : **§B-13 confirmé résolu** sur ancres
  standard, la 403 est externe (⚪).

## 4. Télémétrie / observabilité

- `_vol60_daemon.log` (daemon 1) : flot `INFO sync_profile`,
  1 WARN (`UnmappedKey`, tab flixbus), 0 ERROR — puis silence
  relatif hors phases `event.*` du tab wedged.
- `_vol60_daemon2.log` : 2 WARN (`Fingerprint` tab blablacar,
  `UnmappedKey` tab flixbus), 0 ERROR.
- `/dom`, `/layout`, `/frame`, `/mem`, `/console` : 200 fiables sur
  les tabs sains ; totalement affamés pendant le wedge flixbus.
- Le fallback journal ADR-0025 `UnmappedKey` apparaît **les deux
  fois** sur le tab wedged — corrélation à creuser (le re-parse
  pourrait être à la fois symptôme et carburant du wedge).

## 5. Actions proposées

| # | Proposition | Cible |
|---|---|---|
| **§B-37 (NOUVEAU)** | Hit-test `ignored` sur ancre réelle visible en position scrollée (trainline FAQs, scroll_y=1600) alors que le même mécanisme navigue à scroll_y=0 — décalage scroll hit-test OU overlay transparent | `explorer-core`/hit-test, `explorer-layout` |
| **§B-38 (NOUVEAU)** | Wedge par escalade des phases `event.*` (poll_net 24-31 s, mirror_sync 10-13 s, dispatch ~5 s, en boucle) — `POST navigate` jamais retourné, API affamée, RAM stable ; flixbus.fr reproduit 2/2 sur deux daemons ; corrélé au fallback `UnmappedKey` | `explorer-core` — famille §B-26/§B-33, **bornes par phase event.* + abandon de nav** |
| §B-18 | Confirmé à l'identique : chevauchements carrousel + header peint sur contenu + serif fallback (blablacar) ; + iframe YouTube embed peinte noire (trainline railcards) | `explorer-layout`/`render`/`css` |
| §B-20 | Placeholders d'images lazy non résolus au scroll (trainline, grande boîte bleue vide ~1120×240) | `explorer-core`/`net` |
| §B-13 | Toujours RÉSOLU : ancres standard naviguent (Railcards, Dunkerque) — mais voir §B-37 pour la variante scroll | — |
| ⚪ externe | blablacar interne `/ride-sharing/dunkerque/lille` → 403 Datadome (déjà catalogué vol 39) | externe |

**Statuts** : §B-13 `RÉSOLU confirmé [V4-main]` (2 navigations
par clic) · §B-18 `PARTIEL — confirmé [V4-main]` · §B-20
`recoupe` · §B-37/§B-38 `OUVERT — NOUVEAU proposé`.

### Fichiers produits

- `captures/vol60_trainline.png` · `vol60_trainline_scroll.png` ·
  `vol60_trainline_railcards.png` · `vol60_blablacar.png` ·
  `vol60_blablacar_scroll.png`
- `logs/vol60_trainline_{dom,layout}.txt` ·
  `vol60_trainline_railcards_{dom,layout}.txt` ·
  `vol60_blablacar_{dom,layout}.txt`
- `_vol60_daemon.log` · `_vol60_daemon2.log` (racine)

<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 87 : airfrance · sncf-voyages · camping-cars `[V7-main]`

- **Binaire** : `[V7-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (horodaté 27/10 18:30).
- **Mission** : 3 cibles « voyage » — www.airfrance.fr ·
  www.sncf-voyages.com (re-vol : successeur de sncf-connect, externe §B) ·
  www.camping-cars.info. Prudence §B respectée (aucune cible liste
  26/30/33/41/43/46/48/50/51… en vol — §B-51 rencontré *de facto* sur
  airfrance, non provoqué).
- **Daemon** : PID **2372**, port 9470, `EXPLORER_VIEWPORT=1366x900`,
  journal `logs/daemons/_vol87_daemon.log` (13 093 lignes).
- **RAM** : base ~371 Mo → pic **852 Mo** (airfrance) → 401/405 Mo après
  chaque DELETE — **seuil 1,5 Go jamais franchi**, purge complète
  (signal favorable §B-30 ×3).
- **Verdict global** : **1/3 cible réellement volée — les 2 autres sont
  externes** (qualifiées curl + double résolveur). **airfrance.fr =
  meilleur vol SPA Angular du corpus** : rendu ~80 % (hero photo peint,
  footer complet, icônes réseaux, badges stores), scroll sain, **clic
  footer à scroll_y=1035 → vraie navigation `/contact`**. Mais 2 taches :
  (a) **§B-51 ×4 bundles applicatifs** (467-614 Ko) `ok=false`
  silencieux à chaque nav — la page d'accueil survit grâce au SSR, la
  page interne `/contact` reste coquille vide (§B-4 conséquence) ;
  (b) **§B-18 nav superposée** quantifiée au layout (items `li` 112 px,
  `bwc-menu-action` débordant à x négatif — runs mangés/chevauchés).
  sncf-voyages = **timeout TCP connect** (Incapsula 165.160.x.x, curl
  schannel idem → externe, *nouveau blocage distinct* du DataDome 403 de
  sncf-connect). camping-cars.info = **NXDOMAIN** confirmé Quad9 +
  Google DNS (domaine mort).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|---|
| www.airfrance.fr | Rendu ~80 % — nav/hero/footer complets ; scroll peint ; **clic → `/contact` réel** | `pas de journal` — **mensonger §B-51** | §B-51 ×4 bundles (608/467/483/614 Ko `ok=false`) · §B-18 (nav overlap, x négatif) · §B-26 borne tirée · dyn import 404 tracé WARN |
| www.sncf-voyages.com | **Externe** — timeout connect 10 s ; page « Page inaccessible » gracieuse | `fetch : timeout connect après 10000 ms` | ⚪ externe — Incapsula 165.160.13/15.20, curl schannel timeout idem (cf. sncf-connect = DataDome 403, externe aussi) |
| www.camping-cars.info | **Externe** — NXDOMAIN ; page d'erreur gracieuse | `connexion… Hôte inconnu (os error 11001)` | ⚪ externe — domaine mort (NXDOMAIN Quad9 + 8.8.8.8) |

Points marquants :

- **§B-51 s'impose comme le défaut majeur des sites React/Angular** :
  4 bundles applicatifs distincts échouent à chaque navigation sur
  airfrance (`ok=false ms=104→4679`), **zéro entrée `/console`** — la
  seule signature est `eval fin … ok=false` dans le journal daemon.
  Conséquence asymétrique : accueil SSR sauvé (~80 % peint), `/contact`
  = coquille (header + bandeau, corps blanc, dom 686 nœuds,
  `js_heap` 61 Mo) — §B-4 bras (c) « mort avec heap chargé ».
- **§B-18 quantifié** (preuve `logs/vol87_airfrance_layout.txt`) :
  `<nav><ul>` = 6 `li` ~112,46 px à x∈{0,112,225,337,450,585} mais les
  `bwc-menu-action` enfants sont **plus larges et décentrés** —
  « Acheter un billet » `[-17,29 40 147x26]` déborde à x négatif et
  chevauche « S'enregistrer » `[114,7 …]` ; « Mes Réservations »
  `[207 40 148x26]` chevauche « Flying Blue » `[346 …]` → rendu
  « cheter un billet / enregistrem / es Réservation / lying Blue ».
  Motif : item flex/grid 112 px contenant un enfant centré plus large —
  débordement bilatéral non clippé.
- **Clic scrollé → navigation réelle** : à scroll_y=1035, clic
  « Nous contacter » (232,156) dans le footer → `navigated` →
  `wwws.airfrance.fr/contact` (titre correct). **Contre-exemple §B-40**
  (le défaut trainline/chrome.dev n'est pas universel).
- **Externes confirmés out-of-band** : sncf-voyages.com résout
  165.160.13.20/15.20 (Incapsula), TCP connect timeout **identique sous
  curl/schannel** → pas un bug moteur. sncf-connect.com répond 403
  **DataDome** sous curl (cookie `datadome=`) → externe confirmé inchangé.
  camping-cars.info = NXDOMAIN (Quad9 + Google) → domaine expiré.
- **Pages d'erreur gracieuses** : les 2 échecs externes produisent la
  page « Page inaccessible » moteur avec message typé — tolérance §C
  validée.
- **Exit spontané du daemon** (observation, faible confiance) : après
  le DELETE final + `{"tabs":[]}` + mesure RAM 405 Mo, le PID 2372
  n'existait plus au `taskkill` (~10 s plus tard) — sortie code 1,
  **sans** « arrêt propre » ni erreur dans le journal (dernière ligne
  = `click ignored` 18:34:23). Aucun chemin `exit(1)` du code ne
  correspond (les deux loggueraient) → mort externe ou abort muet.
  RAM 405 Mo, API vivante 30 s avant — non reproduit, à surveiller.
- Garde-fou §B-26 tiré ×1 (`dropped=1 path=[0,1]`, airfrance) —
  borné, aucun wedge.

---

## 1. www.airfrance.fr — Angular SSR fonctionnel, JS applicatif partiellement mort

`POST /navigate` → `200`, `doc_h=1 934` (non clampé), titre « Billets
d'avion, réservation de vols pas chers | Site officiel Air France® »,
URL finale `https://wwws.airfrance.fr/` (redirect `wwws.` absorbée).

**`/console` verbatim** : `pas de journal` — **mensonger §B-51** : le
journal montre 28 évals tab=1, **8 `ok=false` dont 4 bundles
applicatifs** répétés à chaque nav — `bytes=608361` (ms=692/764),
`bytes=467528` (ms=3908/4679), `bytes=483519` (ms=128/104),
`bytes=613991` (ms=214/205). Aucun rejet tracé côté ring.
WARN légitime tracée : `dyn import : fetch échoué — rejet livré …
554-…-cookie-banner.js seq=0 error=statut 404` (404 serveur réel).

**Rendu** (`captures/vol87_airfrance.png`) : logo AIRFRANCE, bandeau
« Situation au Proche et Moyen-Orient » (icône + croix), **hero photo
cabine peint** avec texte « L'Option Siège Economy Comfort », bouton
« Acheter un billet », encart recherche (cadre vide — champs
JS-driven absents), bouton « Recherch… les vols » (texte tronqué,
même famille §B-18), placeholders gris images lazy. **Nav header
superposée** : libellés mangés/chevauchés (preuve layout ci-dessus).

**Télémétrie** (`logs/vol87_airfrance_mem.json`) : `dom_nodes=1 623`,
`layout_boxes=680`, `display_items=432`, `frame_bytes=10,6 Mo`,
`images_bytes=150 Mo`, `js_heap=61,2 Mo`, `journal_fallbacks=0`.
DOM = Angular Material custom elements (`mat-sidenav-container`,
`bwc-sticky-header`, `bwc-top-task`, `mat-icon`).

**Scroll ×2** → `scroll_y=800` puis `1 035` (fin doc) — tout peint
(`captures/vol87_airfrance_scroll.png`) : icônes réseaux (Facebook, X,
LinkedIn, Instagram, Messenger, YouTube, WhatsApp), 5 colonnes footer,
badges App Store/Google Play/AppGallery, liens « Vols au départ de /
en France / Monde », mentions légales. **Pas de §B-8.**

**Clic** « Nous contacter » (232,156) à scroll_y=1035 → `navigated` →
`https://wwws.airfrance.fr/contact` — vraie navigation interne
(`captures/vol87_airfrance_click.png`).

**Page `/contact`** = coquille : header + bandeau peints, **corps
blanc total** (`captures/vol87_airfrance_contact.png`, +16 s de
grâce) — `dom_nodes=686`, `layout_boxes=442`, `js_heap=61,1 Mo`
identique (scripts rechargés, mêmes 4 bundles `ok=false` ms=764/4679/
104/205), console toujours « pas de journal ». L'app ne monte pas :
**§B-4 bras (c) / conséquence §B-51** — le SSR livre une coquille que
le JS mort ne remplit jamais.

**Garde-fou** : `apply: cycle de report cassé (§B-26) dropped=1
path=Some([0, 1])` ×1 — borné, process vivant, API responsive.

## 2. www.sncf-voyages.com — externe : timeout TCP connect (Incapsula)

`POST /navigate` → erreur typée `fetch : timeout connect après
10000 ms` (retour propre, pas de hang).

**Qualification externe** : `www.sncf-voyages.com` → A
165.160.13.20 / 165.160.15.20 (**plage Incapsula/Imperva**) ;
`curl -svI` schannel : même timeout 15 s sur les deux IP — le blocage
est **en amont du moteur** (WAF Incapsula refusant le SYN ou le
réseau). Comparaison : `www.sncf-connect.com` (l'ancien domaine,
catalogué §B-externe « 403 sec ») → Cloudflare + **DataDome 403**
sous curl (`Set-Cookie: datadome=…`) — deux domaines, deux anti-bots
différents, même verdict : **injoignable, hors moteur**.

**Rendu** (`captures/vol87_sncf.png`) : page « Page inaccessible »
moteur — « Impossible de joindre le serveur (timeout connect après
10000 ms) ». Dégradation gracieuse correcte. Télémétrie :
`dom_nodes=0`, `js_heap=0`, scroll_y=0, clic `ignored` — comportement
API sain sur page d'erreur.

## 3. www.camping-cars.info — externe : domaine mort (NXDOMAIN)

`POST /navigate` → erreur typée `connexion à
https://www.camping-cars.info:443 impossible : Hôte inconnu.
(os error 11001)` — retour immédiat.

**Qualification externe** : `NXDOMAIN` confirmé par **deux résolveurs
publics** — Quad9 (9.9.9.9 : « Non-existent domain ») et Google DNS
(8.8.8.8 : idem), avec et sans `www`. Le domaine est expiré/absent —
rien à voler.

**Rendu** (`captures/vol87_campingcars.png`) : page « Page
inaccessible » correcte ; `dom_nodes=0`, clic `ignored`, scroll
`scroll_y=0` — API saine.

## Statuts proposés (§8)

| §B | Bras | Verdict ce vol |
|---|---|---|---|
| §B-51 | évals `ok=false` silencieuses | **confirmé `[V7-main]` — 4 bundles distincts ×2 navs** : airfrance 608 361 / 613 991 / 483 519 / 467 528 octets, `ms=104→4679`, console vierge aux deux instants ; conséquence terminale visible sur `/contact` (coquille vide) — second cas à effet observable total après sosh vol 85 |
| §B-18 | chevauchements runs inline | **+1 instance quantifiée `[V7-main]`** — nav airfrance : `li` 112,46 px contenant `bwc-menu-action` 95-148 px **décentré** (x=−17,29 sur le 1er) → débordement bilatéral non clippé, 3 chevauchements de runs (« cheter un billet », « Recherch… les vols ») |
| §B-4 | morts silencieuses bras (c) | **+1** — `/contact` : heap 61 Mo, DOM 686, app jamais montée, zéro trace (même cause §B-51) |
| §B-40 | hit-test ancre scrollée | **contre-exemple ×1** — clic footer à scroll_y=1 035 → `navigated` `/contact` correcte |
| §B-26 | borne cycle report | tirée ×1 (`dropped=1 path=[0,1]`) — borné, aucun wedge |
| §B-30 | fuite post-DELETE | **signal favorable ×3** — purge 852→401-446 Mo à chaque DELETE, `{"tabs":[]}` final |
| ⚪ externe | anti-bot / domaines | **catalogue** : `sncf-voyages.com` Incapsula (timeout TCP, curl idem) — **nouvelle signature** distincte du DataDome 403 de `sncf-connect.com` ; `camping-cars.info` NXDOMAIN (domaine mort) |

**Observation hors catalogue** (confiance faible, non reproduit) :
sortie spontanée du daemon ~10 s après le dernier DELETE — exit 1
sans « arrêt propre » ni erreur journalisée. Aucun chemin `exit(1)`
de `main.rs` ne logguerait silencieusement → mort externe (infra
session ?) ou abort muet. À surveiller sur prochains vols ; si
reproduit → proposition §B-54.

## Hygiène de vol

- Onglets 1 (airfrance), 2 (sncf-voyages), 3 (camping-cars) — tous
  `DELETE`, `{"tabs":[]}` final, purge mémoire à chaque fois.
- Daemon PID **2372** — disparu avant `taskkill` (exit spontané
  ci-dessus) ; port 9470 libéré confirmé ; **jamais `/IM`**, daemons
  voisins intacts.
- Artefacts : `captures/vol87_{airfrance,airfrance_scroll,
  airfrance_click,airfrance_contact,sncf,campingcars}.png` ;
  `logs/vol87_*_{console,dom,layout,mem}.{txt,json}` ;
  `logs/daemons/_vol87_daemon.log`.

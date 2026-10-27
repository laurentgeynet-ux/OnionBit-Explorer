<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 51 — Forums (SSR + JS léger) (2026-10-27)

- **Binaire** : `[V4-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (WIP `depth_limit` embarqué, §B-3 confirmé ouvert), `--port 9433`,
  `EXPLORER_VIEWPORT=1366x900`, journal `_vol51_daemon.log` (racine)
- **Cibles** : discourse.mozilla.org · forum.affinity.serif.com ·
  www.hardware.fr/forum/ — 2/3 bloquées 403 → substitution
  forums.linuxmint.com (phpBB SSR, même famille de terrain)
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de
  `SYNTHESE.md`
- **RAM daemon (PID 12776)** : 374 Mo au boot → pic 1 238 Mo pendant
  l'activité AdSense linuxmint → **510 Mo après DELETE onglet fautif**,
  ~436 Mo à la fin — sous le seuil 2,5 Go en permanence, libération
  post-DELETE correcte (contraste §B-26).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| discourse.mozilla.org | **Rendu fidèle — crawler SSR** | `pas de journal` | §B-cosm. (0 `<img>` : avatars/logos absents — connu) ; §B-8 non reproduit |
| forum.affinity.serif.com | **HTTP 403 — page d'erreur moteur** | `HTTP 403` | ⚪ externe anti-bot — **nouvelle entrée 403** (Invision) |
| www.hardware.fr/forum/ | **HTTP 403 — page d'erreur moteur** | `HTTP 403` | ⚪ externe anti-bot — **nouvelle entrée 403** |
| forums.linuxmint.com (subst.) | **phpBB ~95 % fidèle, AdSense exécuté** | `ReferenceError: osdlfm is not defined — iframe dynamique` | **proposition §B-28** — boucle d'iframes `sodar` non bornée : DOM +200 nœuds/min, js_heap 53→160 Mo |

Acquis : 2/4 pages servies avec contenu réel et fidèle ; **zéro panic,
zéro freeze, API responsive en continu** ; scrolls réels repeints sans
blanc. Fait marquant : **`show_ads_impl.js` AdSense s'exécute** — le
moteur crée le slot `aswift_0`, fetch et peint la créa publicitaire
dans l'iframe (premier cas terrain d'ad-tech fonctionnel)… au prix
d'une boucle d'iframes de télémétrie non bornée.

---

## 1. discourse.mozilla.org — crawler SSR, rendu fidèle

- `POST navigate` → `doc_h=1 886` (non clampé), titre « Mozilla
  Discourse », URL finale `https://discourse.mozilla.org/`.
- `/console` verbatim : `pas de journal` — **0 erreur**.
- `/mem` : 845 nœuds DOM, 428 boîtes, 495 display items, js_heap=0,
  images 295 Ko, total 10,6 Mo.
- Le serveur a livré la **version crawler** (`<body class="crawler ">`) :
  un seul `<script>` (non exécuté — heap 0), 0 `<img>`.
- Frame (`captures/vol51_discourse.png`) : en-tête « Mozilla Discourse »
  + sous-titre, liste des catégories complète — pastille couleur par
  catégorie (vert/violet/cyan/rouge), titre bleu-gris, description grise,
  compteur de sujets aligné à droite (Add-ons 945 … Common Voice 1210).
  **~95 % fidèle** à la version crawler réelle.
- Scroll dy=800 → `scroll_y=800` effectif, repeinte complète
  (`vol51_discourse_scroll.png` : MDN 920 … Off-Topic 13) — §B-8
  non reproduit.
- Cosmétique connu : aucun `<img>` dans le dump (avatars/logos
  catégories absents du HTML crawler lui-même → fidélité source,
  catalogue §B-cosm. « avatars discourse absents »).

## 2. forum.affinity.serif.com — 403 externe, dégradation propre

- `POST navigate` → `{"error":{"handled":true,"message":"HTTP 403"}}`.
- `/console` verbatim : `{"console":["HTTP 403"]}`.
- `/dom` → **404** (page synthétisée, pas de DOM livré) ; `/mem` :
  18 boîtes / 29 items — la page d'erreur interne existe en layout.
- Frame (`vol51_affinity.png`) : page d'erreur **moteur localisée**
  « Page inaccessible — Le serveur a répondu « HTTP 403 ». Réessayez
  plus tard… » peinte proprement — dégradation gracieuse conforme
  (AGENTS §1 « jamais crash »).
- Attribution **⚪ externe** : forum Invision Community derrière
  filtrage anti-bot ; signature connue du catalogue (UA `Explorer/0.1`,
  pas de `sec-ch-ua`/`sec-fetch-*`, fingerprint rustls).
  → **nouvelle entrée pour la liste 403** (§B ⚪).

## 3. www.hardware.fr/forum/ — 403 externe, même signature

- `POST navigate` → `{"error":{"handled":true,"message":"HTTP 403"}}`.
- `/console` verbatim : `{"console":["HTTP 403"]}` ; `/dom` 404 ;
  18 boîtes / 30 items ; même page d'erreur moteur peinte
  (`vol51_hardware.png`).
- Attribution **⚪ externe** : hardware.fr filtre son forum en amont.
  → **nouvelle entrée pour la liste 403** (§B ⚪).

## 4. forums.linuxmint.com — phpBB fidèle + AdSense exécuté + boucle sodar

*(substitution pour la cible 403 — même famille « forum SSR »)*

- `POST navigate` → `doc_h=2 972`, titre « Linux Mint Forums - Index
  page ».
- `/console` verbatim (stable sur tout le vol) :
  `{"console":["exception js : ReferenceError: osdlfm is not defined — iframe dynamique"]}`
- `/mem` (fenêtre stable) : 3 313→3 537 nœuds DOM, 2 523 boîtes,
  images 15,2 Mo, **js_heap 53→160,5 Mo en ~2,5 min**.
- `/layout` et `/mem` : alternance 200/404 mesurée (DOM détaché
  pendant apply jobs — **pattern transitoire connu** vol-agent §3).

### Rendu — ~95 % fidèle

- `vol51_linuxmint.png` : header vert + logo Mint, recherche,
  « Quick links / Unanswered topics / Active topics / Search »,
  tableaux forums complets (Important Notices, Main Edition Support)
  avec compteurs et « Last post » datés.
- `vol51_linuxmint_scroll.png` (scroll_y=900) : sections Debian
  Edition / Interests / Customization — tableaux nets, sous-forums
  en liens, aucun blanc (§B-8 non reproduit ×2).

### AdSense s'exécute — cas terrain notable

- `show_ads_impl.js` (m202610230101) **exécuté** : le DOM contient le
  slot `aswift_0` + `data-adsbygoogle-status="done"` + wrapper
  `aswift_0_host` — le script a transformé le `<ins>` en iframe pub.
- La créa est **peinte** : bouton « TÉLÉCHARGER » visible dans le
  bandeau 728×90 — fetch + rendu du document iframe fonctionnels.

### Proposition §B-28 — boucle d'iframes `sodar` non bornée

- Le dump DOM contient **419 occurrences « sodar »** — des iframes
  `https://ep2.adtrafficquality.google/sodar/sodar2/255/runner.html`
  empilées en série (créées par la télémétrie AdSense).
- Croissance mesurée continue : nœuds DOM 3 313 → 3 377 → 3 537
  (+~90/min) ; js_heap 53 → 98,9 → 117,7 → 160,5 Mo (~40 Mo/min) ;
  cycles `js_run_sync` + `apply` toutes les ~1,3 s au journal
  (`apply: fin ms=416`).
- En navigateur réel, sodar crée une poignée de runners ; ici la
  boucle de retry/création n'est **pas bornée** → DOM et heap ne
  convergent pas. Signature distincte de §B-21 (`×N` console sans
  borne — là la source est muette, c'est le DOM qui gonfle) et de
  §B-1 (pas de saturation cœur constatée, API restée responsive) —
  mais même famille « boucle applicative post-apply sans budget ».
- Effet de bord mesuré : la note « heap JS gelé ~50 Mio/page » du
  catalogue **ne tient pas** sur page à pubs — heap non gelé,
  croissance continue. RAM daemon : 1 238 Mo au pic → **retombée
  à 510 Mo au DELETE** — libération saine (contrairement à la fuite
  §B-26).
- `osdlfm is not defined` : fonction de `osd.js` (listener on-screen
  AdSense) appelée par le script inline d'un iframe dynamique — le
  définisseur n'a pas tourné dans ce contexte (scope iframe ou script
  mort) — cascade secondaire de la boucle.

---

## Statuts proposés (§8)

| §B | Bras | Verdict ce vol |
|---|---|---|
| §B-cosm. | avatars/`<img>` absents (discourse crawler) | inchangé — fidélité source `[V4-main]` |
| §B-8 | frame blanche au scroll | non reproduit ×2 `[V4-main]` |
| §B ⚪ | liste 403 anti-bot | +2 entrées terrain : `forum.affinity.serif.com`, `www.hardware.fr/forum/` |
| §B-21/§B-1 | boucles applicatives non bornées | recoupement — nouvelle signature DOM/heap (voir §B-28) |
| §B-28 (proposé) | boucle iframes `sodar` AdSense non bornée — DOM +~90 nœuds/min, heap +~40 Mo/min, muette console | **NOUVEAU — `[V4-main]`** (linuxmint, crate `explorer-jscore`/`core` — budget/borne à poser) |

## Hygiène de vol

- Onglets 1-4 fermés par `DELETE /api/tabs/{id}` (onglet 4 clos tôt
  pour contenir la boucle sodar — RAM 1 238→510 Mo).
- Daemon tué par son PID (12776) — `taskkill //PID` (jamais `/IM`) ;
  les daemons tiers voisins n'ont pas été touchés.
- Artefacts : `captures/vol51_{discourse,discourse_scroll,affinity,
  hardware,linuxmint,linuxmint_scroll}.png`,
  `logs/vol51_{discourse,linuxmint}_{dom,mem}.txt`,
  `logs/vol51_discourse_layout.txt`,
  `logs/vol51_{affinity,hardware}_{dom,mem}.txt`,
  `logs/vol51_*_console.json`, journal `_vol51_daemon.log` (racine).
  NB : `vol51_linuxmint_layout.txt` contient le stub « pas de layout »
  — les 5 retries ont tous tapé des fenêtres 404 après un premier 200
  écrasé par la boucle ; dump layout perdu, DOM/mem intacts.

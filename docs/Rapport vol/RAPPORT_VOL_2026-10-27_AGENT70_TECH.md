<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 70 : presse tech grand public `[V6-main]`

- **Binaire** : `[V6-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (contient le fix §B-26 `1e24942`, borne `emit_scoped_at` —
  signature `cycle de report cassé` observée ×1).
- **Mission** : vol libre sur trois cibles presse tech —
  clubic.com, zdnet.fr, frandroid.com. Cibles prudence
  §B-26/§B-30/§B-41/§B-43 évitées.
- **Daemon** : PID 1600, port 9452, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol70_daemon.log`. **RAM max ~2,71 Go** — seuil de
  prudence 1,5 Go franchi sur frandroid → protocole appliqué
  (DELETE tenté, API morte, arrêt par PID).
- **Fenêtre** : 14:19 → 14:30 UTC (~11 min).
- **Verdict global** : **2/3 cibles servies, 1 wedge critique** —
  clubic : DOM/layout complets mais **page verrouillée au viewport**
  (`doc_h`=900 alors que le contenu descend à ~13 450 px — scroll
  impossible, contenu sous la ligne de flottaison injoignable ;
  **proposition §B-44**). zdnet : navigable, vraies images et
  articles peints, hero non hydraté (famille §B-4). frandroid :
  **wedge post-apply silencieux** — navigate jamais retourné, API
  entière morte, ~2 cœurs saturés, RAM 2,7 Go (**proposition
  §B-45**, recoupe §B-33/§B-41) → arrêt d'urgence par PID.

---

## 1. clubic.com — contenu présent mais **scroll verrouillé à 900 px**

`POST /navigate` → **200 en 13,6 s** (title « Clubic - La meilleure
source d'information Tech », `doc_h` **900.0**, `doc_h_clamped=false`).
Console **vide** (« pas de journal »). Télémétrie : 6 689 nœuds DOM,
2 300 boîtes, 2 616 items, 36,5 Mo d'images, js_heap 25,6 Mo
(`logs/vol70_clubic_mem.json`).

**Rendu** (`captures/vol70_clubic.png`) : header complet (logo Clubic,
nav Actus/Tests/Guides/Tutos/Bons plans/Télécharger/Comparer, sous-nav
Cybersécurité→Maison connectée), `<details>` « Top Logiciels » déplié,
bloc « Suivez-nous » (icônes FB/X/YT/LinkedIn/Instagram peintes).
**Aucun article visible** — le site réel affiche un flux d'actus.

**Preuve du verrou** (`logs/vol70_clubic_layout.txt`) : le vrai contenu
existe — `block<main> [0 1119 1200x11750.6]` avec ~20 `block<article>`
réels dès y=1466, `block<footer> [0 13016 1366x428]`. Mais
`block<body> [0 0 1366x900]` : la boîte body est figée à la hauteur
viewport et `doc_h`=900 → contenu au-delà de y=900 **jamais peint ni
atteignable**.

**Mécanisme suspect** (`logs/vol70_clubic_dom.txt`) :
`html,body{width:100%;height:100%}` + `body{…;overflow-x:hidden}`.
La seule règle `overflow:hidden` plein axe est
`html.no-event,html.no-event body{overflow:hidden}` — or `<html>` ne
porte **pas** `no-event` (il porte `data-theme-ready="true"`, posé
par le script inline via double-rAF → le moteur JS exécute bien les
scripts inline et rAF). Hypothèse : `doc_h` calculé sur la hauteur
de la boîte body (100 % = 900) **sans tenir compte du débordement**
des descendants (~13 450 px), et/ou `overflow-x:hidden` propagé au
viewport traité comme un clip sur les deux axes. `explorer-layout`/
`explorer-core` — **PROPOSITION §B-44** : « `doc_h` plafonné à
`body{height:100%}` → scroll verrouillé, contenu sous le fold
injoignable ».

**JS** : site Next.js (`self.__next_f.push` présent) — hydratation
jamais réalisée, console vide malgré scripts inline exécutés →
recoupe **§B-4** (bras « defer Next non exécutés sans erreur », déjà
vu sur GFG vol 36).

**Interactions** : scroll ×2 → `scroll_y` reste **0.0** (frame
byte-identique 2 072 590 o). **Clic « Actus » (320,21) → `navigated`
vers `/actualites-informatique/`** — vraie navigation, `bfcache_pages=1`,
mais nouvelle page soumise au même verrou (2 713 nœuds, même rendu
header-only, `captures/vol70_clubic_click.png`).

## 2. zdnet.fr — navigable, ~55 %, hero fantôme

`POST /navigate` → **200 en 5,8 s** (title correct, `doc_h` 9 918,
non clampé). Console **vide**. Télémétrie : 2 370 nœuds, 1 088 boîtes,
58 Mo d'images, js_heap 29,6 Mo (`logs/vol70_zdnet_mem.json`).

**Rendu** (`captures/vol70_zdnet.png`) : header lime ZDNET correct
(nav complète, tagline, « Mon Compte » tronqué en « Mon Com » par la
largeur de boîte), puis **deux bandes vides** (noire ~110 px + grise
~300 px = le hero, dont le layout montre des conteneurs à hauteur
nulle `block<div> [0 135 1366x0]` — contenu non injecté/non dimensionné,
famille §B-4 hydratation), puis bandeau sombre avec **vraies photos
d'articles peintes**.

**Scroll** ×2 → `scroll_y` 800 puis 1 600, frame repeinte
(`captures/vol70_zdnet_scroll.png`) : article vedette « Bill Gates
alerte : l'IA pourrait provoquer le plus grand bouleversement de
l'histoire de l'humanité » (photo peinte, auteur/date), « Toutes les
tribunes » + cartes portrait. Un **logo lime cassé** (« IVE ! »
tronqué) flotte en haut de section — texte/SVG partiellement peint,
famille §B-36/§B-32 cosmétique.

**Clic « Actualités » (365,103) → `navigated` vers `/actualites/`** —
breadcrumb « ZDNET › Actualités » + liste d'articles réels peints
(« J'ai testé le menu contextuel personnalisable de Windows 11 »,
photo laptop, `captures/vol70_zdnet_click.png`). Les bandes vides du
hero persistent sur la section — même cause §B-4.

## 3. frandroid.com — **wedge post-apply silencieux, arrêt d'urgence**

`POST /navigate` → **jamais retourné** (timeout curl 90 s). Journal
tab=3 (`_vol70_daemon.log` l.64918-64971) :

- 14:24:51 — rafale de **44 évals JS, toutes `ok=true`** (~12 ms
  cumulées — scripts courts) ;
- 14:25:31 → 14:25:34 — `apply` complet `journal=false`, `fin
  ms=2480`, avec `cycle de report cassé (§B-26) dropped=2
  path=[0,14]` — la borne a tiré, l'apply a **fini** ;
- **après 14:25:34 : silence total** — aucune ligne journal.

État constaté : **API entière morte** (`GET /api/tabs` et
`DELETE /api/tabs/3` → HTTP 000 timeout), CPU ~200 %
(280,3 → 296,5 s en 8 s ≈ 2 cœurs saturés), RAM **2 635 →
2 713 Mo** et croissante — seuil 1,5 Go largement franchi →
protocole prudence appliqué : DELETE impossible (API morte) →
**kill par PID 1600**, port 9452 libéré.

**Signature** : wedge **post-`apply: fin`** avec journal muet —
distinct de §B-26 (l'apply termine, `dropped=2` borné), de §B-33
(`nav_paint` Busy en boucle : ici le journal est silencieux, pas de
spam de phases) et de §B-41 (starvation avec phases `event.*`
loguées en continu : ici rien ne sort). Le point commun avec
§B-33 est le `navigate` jamais retourné ; avec §B-41 l'API affamée
et le cœur saturé. **PROPOSITION §B-45** : « wedge post-apply
silencieux — journal stoppé à `apply: fin`, API morte, ~2 cœurs
saturés, RAM >2 Go » (`explorer-core`). Repro : navigate frandroid
sur binaire `[V6-main]` — **cible à ajouter à la liste prudence**.

## 4. Tableau de synthèse

| Cible | Navigate | Rendu | Scroll | Clic | Anomalies |
|---|---|---|---|---|---|
| clubic.com | 200 / 13,6 s | header seul (~25 %) | **verrouillé** (`scroll_y`=0, `doc_h`=900) | navigué → `/actualites-informatique/` | **§B-44 proposé** (doc_h plafonné) + §B-4 Next.js muet |
| zdnet.fr | 200 / 5,8 s | ~55 % | OK ×2 (800/1 600) | navigué → `/actualites/` | hero fantôme §B-4, logo cassé §B-36 cosm., texte tronqué |
| frandroid.com | **jamais retourné** | — | — | — | **§B-45 proposé** : wedge post-apply silencieux, API morte, ~2 cœurs, RAM 2,7 Go |

## 5. Observations annexes

- La borne §B-26 (`emit_scoped_at`) a tiré **×1** sur frandroid
  (`dropped=2 path=[0,14]`) — apply complet jusqu'à `fin`, le
  cycle de report reste contenu ; le wedge §B-45 est **ailleurs**
  (post-apply). ×2 cibles nouvelles saines avant le wedge.
- `journal_fallbacks=0` partout — aucun re-parse ADR-0025, aucun
  panic (§B-37 non reproduit).
- clubic : preuve que **rAF + scripts inline s'exécutent**
  (`data-theme-ready="true"` posé) alors que l'hydratation Next.js
  ne démarre pas sans erreur — bras §B-4 précisé sur une seconde
  cible Next.js « App Router » (après GFG vol 36).
- RAM : clubic+zdnet ≈ 945 Mo → frandroid pousse à 2,71 Go ; la
  croissance continuait lentement après la mort de l'API
  (~80 Mo/4 min) — composante fuite possible en plus du spin CPU.
- Clics = vraies navigations ×2/×2 tentés sur cibles vivantes —
  §B-13 tient confirmé sain (quand la page n'est pas sous overlay).

## 6. Arrêt

DELETE tab 3 tenté → API morte (HTTP 000). Daemon terminé par
**PID 1600** (`taskkill /PID`, jamais `/IM`) — port 9452 libéré,
vérifié `netstat` (seuls TIME_WAIT clients). Aucun autre daemon
touché.

**Fichiers** : `captures/vol70_{clubic,clubic_scroll,clubic_click,
zdnet,zdnet_scroll,zdnet_click}.png`,
`logs/vol70_{clubic,zdnet}_{console.json,dom.txt,layout.txt,mem.json}`,
journal `_vol70_daemon.log`. frandroid : aucune capture possible
(API morte avant tout `/frame`).

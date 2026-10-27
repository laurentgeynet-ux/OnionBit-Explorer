<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 63 — Banque/assurance FR (2026-10-27)

- **Binaire** : `[V4-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (binaire WIP depth_limit fourni par l'opérateur, timestamp 09:32)
- **Daemon** : `--port 9445`, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol63_daemon.log` (PID 17544) — arrêt propre par PID
- **Cibles** : credit-agricole.fr · banquepopulaire.fr · maif.fr
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de
  `SYNTHESE.md`

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| credit-agricole.fr | **WEDGE — starvation pompe** (page servie puis API affamée ~25 min) | iframeResizer `TypeError: cannot read 'iframe' of Undefined` ×3, 2 images 404 | **§B-41 confirmé, nouveau déclencheur** : `commit.drains` 60→152→**185 s** + `event.poll_net` 201→**217 s**, corrélé `UnmappedKey` ; §B-17 (méga-menu déployé) ; §B-18 (bannière dégradée superposée + skip-links peints) |
| banquepopulaire.fr | **SSR réel, nav clic réelle** | `TypeError: cannot read 'querySelector' of Null` (app.js webpack) | §B-17 confirmé (méga-menu peint déployé, `header` 1366×8395) ; §B-14 (`querySelector` of Null) ; 404 DOM transitoires |
| maif.fr | **Bon rendu, modale CMP correcte** | vide | §B-17 confirmé ; **§B-31 recoupé** (clic `<button>` « Fermer » → `ignored`) ; clic `<a>` → navigation réelle (§B-13 tient) ; `doc_h` clamped |

Acquis : 3/3 cibles servies avec contenu réel ; **2 navigations
réelles par clic** (banquepopulaire → page PEA, maif → /cookies) ;
zéro panic ; zéro fuite RAM — le wedge crédit-agricole est une
**starvation pure scopée à l'onglet** : DELETE a libéré ~820 Mo en
une fois et rendu l'API instantanée. RAM daemon : pic ~1,05 Go
(seuil 2 Go jamais approché).

---

## 1. credit-agricole.fr — wedge par phases géantes `[V4-main]` — §B-41 confirmé

- `POST navigate` : retour curl à timeout 90 s, **mais l'onglet a
  été servi** — titre réel « Banque & Assurances - Crédit Agricole »,
  `apply: fin tab=1 ms=1064` à 10:19:18.
- À partir de ~10:19:40, la pompe entre dans des **phases géantes
  en boucle** (`_vol63_daemon.log`) :
  ```
  commit.drains   us= 60 105 173   (10:20:39)
  commit.drains   us=152 342 236   (10:25:24) → apply: fin ms=1789
  commit.drains   us=185 340 464   (10:31:01)
  event.poll_net  us=201 794 504   (10:34:23)
  event.dispatch  us=  8 289 929   (10:34:31)
  event.poll_net  us=217 615 140   (10:38:09)
  ```
  soit des drains JS de **60 → 152 → 185 s** sous `pump.commit_lock`,
  puis `poll_net` de **202 et 218 s**. WARN corrélé à 10:20:39 :
  `journal DOM rejeté → repli re-parse (ADR-0025) why=UnmappedKey`
  (1 fallback, `journal_fallbacks=1`).
- **API affamée par fenêtres** : `/frame` servi une fois (19,7 s,
  3,15 Mo) entre deux timeouts 25 s ; `/mem` servi en 52 s ;
  `/console` et `/api/tabs` servis par intermittence ; `/dom`
  **jamais servi** (4×40 s + 1×180 s timeouts) ; scroll ×2 et clic
  expirés (45-60 s, probablement exécutés en différé).
- **RAM stable ~0,86-1,05 Go, CPU ~1,2 cœur en continu** — pas de
  fuite : starvation pure. `apply` *termine* (`fin` journalisé) —
  le blocage est dans la **pompe** (`commit.drains` /
  `event.poll_net`), pas dans l'apply : signature §B-41, distincte
  de §B-26 (apply mort après `layout`) et §B-33 (`take_nav_paint`
  Busy muet, journal vide — ici le journal **parle**).
- **DELETE difficile** : 5 tentatives (~60 s chacune) avant
  `http=204` à ~10:41 — la requête attend une fenêtre de pompe.
  Après DELETE : `{"tabs":[]}`, RAM 939 → **51 Mo**, `/api/tabs`
  en 1,7 ms. **Le wedge est intégralement scopé à l'onglet** —
  ni §B-30 (pas de fuite post-DELETE), ni §B-39 (pas de deadlock :
  le DELETE finit par passer).
- `/mem` à 10:27 : `dom_nodes=4186`, `layout_boxes=1884`,
  `display_items=2392`, `js_heap=59,8 Mo`, `total=112 Mo`.

### Rendu (`captures/vol63_credit-agricole.png`)

- Contenu réel peint : header « Vous êtes un particulier / Choisir
  une caisse régionale / Ouvrir un compte / Me connecter », hero
  « Le bon taux pour votre 1er toit », simulateur, « Besoin d'une
  assurance ? », devis Habitation/Auto.
- Bannière de dégradation du site **superposée** au contenu en
  grand corps : « Malheureusement, votre configuration de
  navigation actuelle ne vous permet pas de naviguer dans de bonnes
  conditions… ni accéder à votre espace client » — feature-detect
  côté site déclenché (constat honnête : le moteur est détecté) +
  chevauchement → §B-18.
- Méga-menu « Comptes & Cartes » peint déployé avec boîtes
  superposées en haut → §B-17. Skip-links « Aller au Menu / Aller
  au Contenu / Aller au Pied de page » peints → §B-cosmétique.
- Console : `TypeError: cannot read 'iframe' of Undefined` ×3 dans
  `iframeResizer.min.js` (AEM forms) → §B-14 ; 2 badges stores 404
  (URL `%5C` backslash encodé — source fautive ou résolution).

## 2. banquepopulaire.fr — SSR réel, méga-menu déployé `[V4-main]`

- `POST navigate` → retour propre en **38,4 s** : `doc_h=6400.55`,
  titre « Banque Populaire : La réussite est en vous ».
- `/console` : « pas de journal » (vide à l'accueil).
- `/mem` (instantané pendant un apply) : `dom_nodes=0`,
  `journal_applies=1`, `journal_fallbacks=2` — DOM détaché
  transitoire ; WARN `Fingerprint` ×2 dans le journal daemon.
- `/dom` oscillant 200/404/200 — 404 transitoire conforme
  `vol-agent.md` §3 (appli jobs), dump 580 Ko obtenu.
- `/layout` : `block<header> [0 0 1366x8394.9]` — le header fait
  **91 % de la hauteur du document** (doc 9 164 px) : tout le
  méga-menu y est layouté déployé.

### Rendu (`captures/vol63_banquepopulaire.png` + `_scroll.png`)

- Logo BP (+X) et liens d'accès rapide peints, mais **tout le
  méga-menu est déployé en colonne** : « Comptes / Choisir une
  carte bancaire » (Visa Classic/Premier/Platinum/Infinite),
  « Choisir une formule bancaire » (Cristal…), « Réaliser des
  achats en ligne », « Gérer son budget »… — sur la page entière
  (`_scroll.png` à scroll_y=1600 : « Épargne / Constituer son
  épargne » Livret A, LDDS, LEP, PEA, Cryptoactifs, ETF…).
  → **§B-17 confirmé** : menu fermé par JS reste ouvert.
- Scroll ×2 : `scroll_y` 800 → 1600 réels.

### Input — clic = navigation réelle (§B-13 tient)

- Clic (140,370) sur un lien du menu → `{"result":"navigated"}` →
  `https://www.banquepopulaire.fr/epargner/le-plan-depargne-en-actions-pea/`,
  titre réel « Plan d'Epargne en Actions (PEA) ».
- Page PEA : console = **1 erreur JS** —
  `TypeError: cannot read 'querySelector' of Null` dans
  `bp-child/js/app.js` (webpack chunk) → §B-14.
- `vol63_bp_pea.png` : méga-menu toujours déployé sur la page
  interne → §B-17. `/mem` : `dom_nodes=6538`, `js_heap=22 Mo`,
  `journal_applies=16`, `journal_fallbacks=3`.

## 3. maif.fr — modale CMP peinte, `<button>` incliquable `[V4-main]`

- `POST navigate` → retour propre en **7,2 s** : `doc_h=17886`
  (**`doc_h_clamped=true`** — clamp 16 384 documenté §B-perf),
  titre « MAIF - Assurance des particuliers… ».
- `/console` : **vide** — zéro erreur JS.
- `/mem` : `dom_nodes=4093`, `layout_boxes=2894`, `js_heap=47 Mo`,
  `journal_fallbacks=3` (WARN `UnmappedKey` ×4 au fil de la
  session sur ce tab).

### Rendu (`captures/vol63_maif.png` + `_scroll.png`)

- Page réelle : logo MAIF, « Prendre RDV / Assistance et sinistre /
  Espace personnel », sections Habitation/Prêt/Projets/Quotidien.
- **Modale de consentement TrustCommander correctement peinte** :
  boîte blanche « Choisissez l'usage de vos données », backdrop
  `rgba(0,0,0,.5)` grisant la page, **fixe au scroll** (position
  fixed suivie correctement à scroll_y=1600). Boutons layoutés :
  « Personnaliser mes choix » [713,952], « Refuser » [713,995],
  « Accepter » [841,995] — **sous le pli du viewport** (modale plus
  haute que 900 px, zone boutons inaccessible).
- Bandeau « Un sinistre lié à la tempête en France ? » peint ;
  méga-menu latéral déployé (« Vous avez un projet ? / Au
  quotidien »…) → §B-17. Petit bloc rouge occluant des débuts de
  runs (« ■s partenaires Mobilité ») → §B-18 mineur.

### Input — §B-31 recoupé sur `<button>`, §B-13 tient sur `<a>`

- Clic hors modale sur « Assurance décès » (66,670 à scroll 1600)
  → `ignored` — **légitime** (modale ouverte, backdrop intercepte).
- Clic « Fermer » (X du bandeau cookies, `block<button> [1317 357
  24x24]`, point 1329,369 strictement dedans) → **`ignored`** :
  bouton layouté, visible, mais hit-test ne le trouve pas —
  **recoupe §B-31** (photopea : clic `<button>` refusé alors que
  `<a>` fonctionne ; suspect handler/onclick non branché ou
  hit-test boutons).
- Clic lien « Connaître notre politique cookies » (900,924)
  → **`navigated`** → `https://www.maif.fr/cookies` (titre
  « Cookies - MAIF ») — §B-13 toujours résolu sur ancres.
- Page /cookies : console vide, `journal_fallbacks=7`, bfcache
  peuplé (1 page, 4093 nœuds).

## 4. Télémétrie / observabilité

- `_vol63_daemon.log` : 6 WARN `journal DOM rejeté → repli
  re-parse (ADR-0025)` (UnmappedKey ×4 tab 1+3, Fingerprint ×2
  tab 2), 0 ERROR. Phases `sync_profile` précieuses : ce sont
  elles qui font la preuve du wedge (`commit.drains`/`poll_net`
  en µs journalisés une fois la phase terminée).
- Pendant la starvation tab=1, **l'API répond par fenêtres** entre
  deux phases géantes — exploitable mais lent (attendre une
  fenêtre, retry agressif). `/dom` est le endpoint le plus
  affamé (jamais servi en 220 s cumulés).
- RAM daemon mesurée : 384 Mo (boot) → 1,05 Go (tab CA) → 51 Mo
  (post-DELETE) → 790 Mo (BP+MAIF) → stabilité. **Aucune fuite**.

## 5. Actions proposées

| # | Verdict | Cible |
|---|---|---|
| **§B-41** | **CONFIRMÉ — 2ᵉ déclencheur fiable : credit-agricole.fr.** Signature enrichie : outre `event.poll_net` 202-218 s et `dispatch` 8,3 s (flixbus), la phase dominante ici est **`commit.drains` sous `commit_lock` : 60/152/185 s**. Corrélé `UnmappedKey` (re-parse ADR-0025). Navigate retourné, apply `fin` journalisé, RAM stable, DELETE libère tout → starvation pompe pure. Renforce la demande : bornes par phase + abandon de nav. | `explorer-core` |
| §B-17 | Confirmé ×2 : méga-menus BP (header 1366×8395 = 91 % du doc) et MAIF peints déployés ; site CA idem + bannière « configuration de navigation » superposée | `explorer-jscore`/boots |
| §B-31 | Recoupé : `<button>` « Fermer » (maif, 24×24 layouté, point strictement dedans) → `ignored` ; `<a>` voisin navigue | `explorer-core`/hit-test |
| §B-14 | +2 entrées matrice : `iframe` of Undefined (CA iframeResizer), `querySelector` of Null (BP app.js webpack) | `explorer-jscore` |
| §B-18 | CA : bannière dégradée superposée au hero + nav boîtes chevauchées ; MAIF : bloc rouge occluant des débuts de runs | `explorer-layout`/`render` |
| §B-13 | Toujours RÉSOLU `[V4-main]` : 2 navigations réelles par clic (BP→PEA, MAIF→/cookies) ; `ignored` hors modale = comportement correct | — |
| §B-30/39 | NON reproduits : DELETE long (~5 min, fenêtres de pompe) mais aboutit et libère toute la RAM — ni fuite ni deadlock | — |
| ⚪ externe | CA détecte le moteur (« configuration de navigation… ») — feature-detect honnête, pas un blocage dur | externe |

**Statuts** : §B-13 `RÉSOLU confirmé [V4-main]` · §B-17
`PARTIEL — confirmé [V4-main]` ×3 · §B-31 `OUVERT — recoupé
[V4-main]` · §B-41 `CRITIQUE — confirmé [V4-main], déclencheur
credit-agricole.fr` · §B-14/§B-18 `recoupe`.

### Fichiers produits

- `captures/vol63_credit-agricole.png` · `vol63_banquepopulaire.png`
  · `vol63_banquepopulaire_scroll.png` · `vol63_bp_pea.png` ·
  `vol63_maif.png` · `vol63_maif_scroll.png` · `vol63_maif_cookies.png`
- `logs/vol63_ca_{console,mem}.txt` (dom/layout non servis — wedge)
  · `vol63_bp_{console,dom,layout,mem,pea_mem}.txt` ·
  `vol63_maif_{console,dom,layout,mem,cookies_mem}.txt`
- `_vol63_daemon.log` (racine)

<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 71 — Santé / magazines féminins (santemagazine · doctissimo · femmeactuelle) (2026-10-27)

- **Binaire** : `[V6-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (release du 2026-10-27 15:04), `--port 9453`, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol71_daemon.log` (PID 19812, tué par PID en fin de vol)
- **Cibles** : www.santemagazine.fr · www.doctissimo.fr ·
  www.femmeactuelle.fr — **3/3 volées**
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de `SYNTHESE.md`
- **RAM daemon** : ~374 Mo boot → 776 Mo (santemag, 2 onglets) →
  **1 407 Mo pic sous doctissimo** (`js_heap=537 Mo`) → purge 429 Mo
  → 1 032 Mo (femmeactuelle) → purge 616 Mo → kill à ~615 Mo.
  **Seuil 1,5 Go jamais franchi ; purges post-DELETE saines ×3**
  (signal contraire §B-30).
- **Aucun wedge** : API 100 % responsive tout du long, `navigate`
  toujours retourné, DELETE toujours honorés.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| www.santemagazine.fr | Dégradé — vrai contenu masqué par le tiroir burger | `pas de journal` | **§B-17 +1 porteur** : drawer `menu-burger` (`inert`, `aria-hidden`) peint ouvert [0 70 384×900] ; **§B-18** runs superposés ; §B-13 sain (clics → nav réelles, `target=_blank`) ; **→ §B-45 nouveau** : mojibake `title` |
| www.doctissimo.fr | Correct au-dessus du fold, hero JS vide, churn massif | `pas de journal` (muet malgré ~2 000 évals) | §B-4 famille (hero carousel jamais rempli, zéro erreur) ; **124 rejets journal ADR-0025** (58 `TooManyOps`, 65 `UnmappedKey`) ; `js_heap=537 Mo` ; fail-cap §B-21 validé terrain (`id=79 streak=8`) ; images sous le fold absentes ; → §B-45 |
| www.femmeactuelle.fr | Partiel → page article **vidée par son propre loader** | `unhandledrejection: Fallback is mandatory ×3` (pmc-starter Prisma) | 240+ rejets journal en rafale continue ; clic `ignored` **mais navigation effective** (sous-cas §B-24/§B-13) ; article = `Failed to execute packing script` → DOM 7 nœuds remplacé par iframe erreur fullscreen → frame blanche ; → §B-45 (titre clapote correct↔mojibake) |

Fait marquant : **femmeactuelle matérialise l'échec moteur dans son propre
DOM** — son loader/packer anti-erreur détecte l'échec d'exécution
(`Error: Failed to execute packing script`, base64 dans l'URL
`report.error-report.com`) et remplace tout le `<body>` par un iframe
télémétrie plein écran. C'est une **preuve terrain directe** qu'un
bootstrap packé ne s'exécute pas sous explorer-jscore, lisible sans
bissection — le §B-4 « mort silencieuse » devient visible.

---

## 1. www.santemagazine.fr — §B-17 nouveau porteur + §B-18 + §B-45

- `POST navigate` → `doc_h=26 409` (`doc_h_clamped=true`), titre
  « Santé Magazine, le féminin qui fait du bien ! … », retour < 10 s.
- **Frame** (`captures/vol71_santemag.png`) : header correct (burger,
  logo jaune sur teal, « Je m'abonne », icône compte), mais le corps est
  entièrement occupé par le **tiroir burger déployé** : champ
  « Rechercher sur SanteMagazine.fr », arbre de rubriques
  (Contraception → Psycho-Sexo) avec **runs superposés** (« Tous les
  cancers »/« Rubriques »/« Pollution »/« Régimes » chevauchent les
  items suivants — `vol71_santemag.png`, `vol71_santemag_scroll.png`).
- **Layout** (`logs/vol71_santemag_layout.txt` l.27-45) :
  `block<div> [0 70 384x900]` = le drawer, contenant `form` recherche
  et `<nav>` de `<li>` 384×24-48. Dans le DOM
  (`logs/vol71_santemag_dom.txt` l.7540+) : `<div id="menu-burger"
  inert aria-hidden="true" role="dialog" class="menu-burger …">` —
  **le tiroir est censé être hors-champ/inerte** (Stimulus
  `data-controller="menu-burger"`, classe `menu-burger` à transform
  off-canvas). Il est layouté et **peint** à [0,70] → recoupe §B-17
  (« ceux fermés par JS restent ouverts ») ; `inert` non honoré est
  un détail nouveau à noter dans l'entrée.
- **Console** : `pas de journal` (404 transitoires aux retry — cycle
  apply documenté §vol-agent §3). Zéro erreur JS malgré ~3 600 évals
  `tab=1` journalisées → le cacher du menu dépend probablement d'un
  transform CSS que le JS aurait posé… ou d'une classe jamais togglée
  (boot Stimulus muet → §B-4 bras (c)).
- **Clics — §B-13 confirmé sain `[V6-main]`** : (100,500) sur item
  menu → `{"result":"navigated"}` → `/recettes-sante` ; (1100,35) sur
  « Je m'abonne » → `navigated` → **nouvel onglet** `target=_blank`
  (`/recettes-sante#`).
- **Scroll** ×2 → `scroll_y=1 600`, repaint cohérent (même arbre menu,
  runs toujours superposés).
- **Télémétrie** : `dom_nodes=6 537`, `layout_boxes=2 547`,
  `display_items=2 223`, `images_bytes=92 Mo`, `js_heap` 40 Mo→0
  (oscille avec le détachement), `journal_fallbacks=9` — repli ADR-0025
  bénin, chaque apply sain.
- **→ §B-45 (proposition d'entrée) — mojibake `title` dans
  `/api/tabs`** : après les navigations par clic, le titre remonte
  `Recettes healthy\u00c2\u00a0: id\u00c3\u00a9es de recettes
  sant\u00c3\u00a9` (= « healthyÂ : idÃ©es… santÃ© », UTF-8 relu en
  Latin-1) alors que le dump DOM `<title>` est **correct** (« idées
  de recettes santé », `logs/vol71_santemag_recettes_dom.txt` l.45).
  Le titre correct existait dans la réponse `navigate` initiale — le
  champ stocké est décodé par une voie différente (re-parse/snapshot)
  que le DOM. Crate suspect : `explorer-core` (métadonnées onglet).

## 2. www.doctissimo.fr — rendu présent, hero JS vide, churn ADR-0025

- `POST navigate` → `doc_h=12 636` (non clampé), titre correct au retour.
- **Frame** (`captures/vol71_doctissimo.png`) : header fidèle (icônes
  sociales peintes, logo Doctissimo en dégradé violet, « Rechercher »),
  bande lavande + « En ce moment », puis **~450 px de blanc** (zone
  hero/carrousel réservée mais jamais remplie — JS-driven → famille
  §B-4), puis « Top articles » avec **2 vraies photos peintes**
  (soupe, modèle anatomique poumon).
- **Scroll** ×2 → `scroll_y=1 600` (`captures/vol71_doctissimo_scroll.png`) :
  textes d'articles fidèlement rendus et accentués (« Alexandra
  Rosenfeld… maladie de Behçet », « Christina Applegate… sclérose en
  plaques », « Le virus Shamonda franchit la barrière d'espèce »),
  barre rubriques « Addictions / Analyses médicales / Cholestérol… »,
  bandeau « Octobre rose » — mais **aucune image sur les cartes sous
  le fold** (lazy `data-src` jamais résolu → voisin §B-20) et cartes
  de droite tronquées au bord viewport (carrousel plus large que la
  fenêtre, comportement plausible du site).
- **Console : muette** (`pas de journal`) malgré **~2 000 évals**
  `tab=3` — le site tourne, échoue sans bruit (hero, lazy images) →
  bras §B-4(c).
- **Churn ADR-0025 massif** : `journal_fallbacks=124` en ~4 min —
  `why=TooManyOps` ×58 + `UnmappedKey` ×65 + `Fingerprint` ×1.
  La raison `TooManyOps` (journal au-delà de la borne d'ops → re-parse
  complet) domine : mutations DOM continues (pubs/trackers/carrousel)
  → re-parse à répétition. Coût : `js_heap=537 Mo`, `total=647 Mo`,
  `frame_bytes=69 Mo`, `images_bytes=39 Mo`, RAM process 1,41 Go.
  **Pas de wedge** : apply journalisés jusqu'à `fin` (ex.
  `apply: fin tab=3 ms=904` à 14:26:24).
- **§B-21 validé terrain** : `WARN explorer_jscore::timers:
  intervalle en boucle d'échecs tué (fail cap) id=79 streak=8` —
  la borne d'intervalles en échec fonctionne en conditions réelles.
- **Clic** (700,450) → `edited` (zone hero vide, pas d'action par
  défaut — cohérent).
- **→ §B-45 ×2** : titre onglet `Doctissimo : toutes les
  actualit\u00c3\u00a9 sant\u00c3\u00a9` (mojibake) alors que le
  titre retourné par `navigate` et le DOM sont corrects.
- **Purge saine** : DELETE → RAM 1 407 → 429 Mo stable (§B-30
  non reproduit).

## 3. www.femmeactuelle.fr — churn infini + « packing script » échoue visible

- `POST navigate` → `doc_h=29 682` (`doc_h_clamped=true`).
- **Frame accueil** (`captures/vol71_femmeact.png`) : header partiel
  (icône recherche, « En kiosque › », logo script, bouton rouge
  « S'abonner »), grand blanc, puis « À la une » + titre article
  Miss France en bas — SPA clairsemée.
- **Console** (`logs/vol71_femmeact_console.json`) :
  `unhandledrejection: Error: Fallback is mandatory ×3` — pile
  `tra.scds.pmdstatic.net/pmc-starter/6.1.0/esm.index.js` (stack
  Prisma Media) depuis le bundle `3537.*.js`. Échec **tracé**, pas
  silencieux — bras §B-14 (API/feature manquante côté consentement/
  tracking) ; le suffixe de dédup `(×3)` est lui-même sérialisé
  `Ã—3` dans le JSON → → §B-45.
- **Churn ADR-0025 en rafale continue** : `journal_fallbacks`
  132 → 161 → 209 → **240+** en ~8 min (~30/min, ne s'arrête pas),
  `why` mêlés `UnmappedKey`/`Fingerprint` ; `/layout` alterne 404 ↔
  200 (dump 52 o squelette `html/body [0 0 1366x0]` vs 185 Ko complet
  — `vol71_femmeact_layout.txt` vs `layout2.txt`) ; `/mem` clapote
  `dom_nodes=0↔5 820`. `journal_applies=22`, ~6 900 évals `tab=4`.
- **Scroll** ×2 → `scroll_y=800` **deux fois** — la seconde lecture
  n'a pas cumulé : entre les deux, l'onglet a **navigué** (reset).
- **Clic (680,820) → `{"result":"ignored"}`… puis l'onglet est sur
  l'article** : `url=/actu/news-actu/une-presence-irremplacable-…
  -2212743`, titre devenu celui de l'article. Deux lectures :
  (a) le hit-test refuse (carte hero « À la une » = stretched-link
  probable → §B-24) **mais le handler JS délégué a navigué** —
  verdict API mensonger, sous-cas nouveau de §B-13/§B-24 ;
  (b) navigation `location` spontanée (§B-25) coïncidant. Le détail
  exact (URL = article cliqué) penche pour (a).
- **Page article = preuve §B-4 rendue visible** : DOM = **7 nœuds**
  (`vol71_femmeact_art_dom.txt`, 787 o) — `<body style="overflow:
  hidden">` contenant uniquement `<iframe
  src="https://report.error-report.com/modal?error=
  RXJyb3I6IEZhaWxlZCB0byBleGVjdXRlIHBhY2tpbmcgc2NyaXB0…">` =
  **« Error: Failed to execute packing script »** (base64),
  `position:fixed` 100 vw×100 vh z-index max. Le packer du site
  détecte son propre échec sous Explorer et affiche son rapport
  d'erreur ; l'iframe externe n'est pas peint → **frame blanche
  stable** (`vol71_femmeact_article.png`, 1 844 526 o identique
  sur deux captures à 10 s d'écart). bfcache retient la home
  (5 827 nœuds).
- **Frame servie obsolète en pleine nav** : la capture « scroll »
  post-clic (`vol71_femmeact_scroll.png`) est **byte-identique**
  (`cmp`) à la frame d'accueil alors que l'onglet avait déjà changé
  d'URL/titre — `/frame` peut livrer le PNG de l'ancien document
  pendant la fenêtre de commit (constat d'invalidation, mineur).
- **Titre clapote** : correct (« “Une présence irremplaçable”…
  décédée ») à t+20 s, mojibake (`\u00e2\u20ac\u0153Une
  pr\u00c3\u00a9sence…`) juste après le clic → §B-45 **oscillant**,
  corrélé au cycle de re-parse.
- **RAM** : 1 032 Mo sous churn → DELETE → **616 Mo stable**
  (§B-30 non reproduit ×3). `journal_fallbacks` cessent avec
  l'onglet.

## Statuts proposés (§8)

| §B | Bras | Verdict ce vol `[V6-main]` |
|---|---|---|
| §B-17 | menus/overlays fermés par JS restent ouverts | **+1 porteur santemagazine** — drawer `inert`+`aria-hidden` peint [0 70 384×900] ; `inert` non honoré à noter |
| §B-18 | runs inline superposés | confirmé santemag — rubriques/menu items chevauchés sur 2 captures |
| §B-13 | hit-test | **sain santemag** — 2/2 clics → nav réelle (dont `target=_blank` → onglet) ; **verdict `ignored` mensonger femmeactuelle** (nav effective) → sous-cas à attacher §B-24 |
| §B-24 | stretched-link incliquable | **variante nouvelle** : clic `ignored` sur hero « À la une » mais JS délégué navigue — le refus hit-test ne reflète plus le résultat |
| §B-4 | morts JS (bras c : exécution sans effet) | doctissimo hero/lazy images vides à console muette ; **femmeactuelle : le site affiche lui-même son échec** (`Failed to execute packing script`, iframe rapport) — le silence est percé côté site |
| §B-14 | APIs manquantes | `unhandledrejection: Fallback is mandatory ×3` pmc-starter (Prisma) femmeactuelle — tracé |
| §B-20/§B-4 | images lazy | cartes doctissimo sous le fold sans image peinte |
| §B-21 | boucles applicatives | **borne validée terrain** — `fail cap` tue l'intervalle `id=79 streak=8` (doctissimo) |
| §B-29/§B-41 | churn DOM/budget | rejets journal ADR-0025 en rafale : doctissimo ×124 (dont `TooManyOps` ×58 — seuil d'ops dépassé par les mutations), femmeactuelle ×240+ non borné sur page morte ; `js_heap` 537 Mo ; API toujours vivante |
| §B-30 | fuite post-DELETE | **signal contraire ×3 `[V6-main]`** — purges 1 407→429 et 1 032→616 Mo stables |
| **§B-45 (proposé)** | **mojibake UTF-8→Latin-1 des champs JSON** | `title` `/api/tabs` garbled sur 3 sites (`idÃ©es`, `actualitÃ©`, `â€œ…â€`), suffixe dédup `/console` `Ã—3` ; DOM et rendu toujours corrects ; **oscille correct↔garbled** selon le cycle de parse — suspect : snapshot titre sur re-parse à charset perdu / double-décodage sérialisation. Crate : `explorer-core`/`api` |

## Hygiène de vol

- Daemon PID **19812** — le second `explorer-daemon.exe` (PID 1600,
  port autre) **n'a pas été touché** ; kill par `taskkill //PID
  19812 //F`, jamais `/IM`. `{"tabs":[]}` confirmé avant arrêt ;
  port 9453 mort après kill (vérifié).
- Onglets : tab 1-2 (santemag + fils target=_blank), 3 (doctissimo),
  4 (femmeactuelle) — tous DELETE proprement, RAM libérée à chaque
  fois.
- Artefacts : `captures/vol71_{santemag,santemag_scroll,doctissimo,
  doctissimo_scroll,femmeact,femmeact_scroll,femmeact_article,
  femmeact_article2}.png` ; `logs/vol71_{santemag,santemag_recettes,
  doctissimo,femmeact,femmeact_art}_{dom,layout,console,mem}*.{txt,json}`
  ; journal `_vol71_daemon.log` (racine).
- Limites honnêtes : attribution clic-femmeactuelle non tranchée
  (handler JS vs nav spontanée) ; la voie exacte du mojibake titre
  n'est pas bissectée (re-parse vs sérialisation) ; le rôle exact de
  `inert` vs transform manquant dans le drawer santemag reste à
  confirmer en fixture.

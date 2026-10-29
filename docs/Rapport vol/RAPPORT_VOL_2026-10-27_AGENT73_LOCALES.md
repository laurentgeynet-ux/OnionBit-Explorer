<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 73 — Collectivités / DINUM (paris.fr · bordeaux.fr · numerique.gouv.fr) (2026-10-27)

- **Binaire** : `[V6-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (release du 2026-10-27 15:04), `--port 9455`, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol73_daemon.log` (PID 16800, tué par PID en fin de vol)
- **Cibles** : www.ville.paris.fr (→ **www.paris.fr**, voir note DNS) ·
  www.bordeaux.fr · www.numerique.gouv.fr — **3/3 volées**
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de `SYNTHESE.md`
- **RAM daemon** : ~683 Mo (paris + bordeaux) → **1 109 Mo pic** (bordeaux
  détaché + numerique) → purge **549 Mo stable** post-DELETE×3.
  **Seuil 1,5 Go jamais franchi ; purge post-DELETE saine** (signal
  contraire §B-30, zéro log post-DELETE).
- **API** : 100 % responsive tout du long — `navigate`, `input`, `DELETE`
  toujours honorés ; seules les vues DOM de bordeaux restent détachées
  (§B-26 variante).
- **Note cible** : `www.ville.paris.fr` est **NXDOMAIN** (externe —
  nslookup quad9 « Non-existent domain », curl idem). Remplacé par
  `https://www.paris.fr` (site officiel, Fastly 151.101.x).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| www.paris.fr | **Page entière invisible** — layout complet mais tout le contenu hors-champ | `pas de journal` | **→ §B-48 (proposition)** : `.paris-main-wrapper` collapsé `w=0` à `x=1366` (bord droit viewport), descendants à x≥1366 ; seul un motif voilier quasi-transparent peint ; borne §B-26 tirée bénigne (`dropped=5`) ; §B-45 sain |
| www.bordeaux.fr | Peint une fois (display_list 6 837) puis **DOM détaché terminal** | `pas de journal` | **§B-26 variante nouvelle** : apply complet (`repaint ms=670`, `dropped=1 [0,1,0,0]`) → journal rejeté (`UnmappedKey`) → re-parse → **jamais réattaché** >7 min, flag « paint en vol » bloqué (`input différé → ignored`), pump phases vivantes ~30-70 s |
| www.numerique.gouv.fr | **Correct+** — vrai site DSFR peint, nav→clic→nav réelle, scroll propre | `pas de journal` | §B-17 +1 porteur DSFR (`fr-collapse`/menu mobile peint déplié → liste verticale) ; boutons tarteaucitron blanc-sur-clair ; clic consentement `edited` sans effet (§B-4 bras c mineur) ; §B-13 sain ; §B-45 sain |

Fait marquant : **deux sites institutionnels sur trois échouent sans
aucune erreur console** — paris.fr par un bug layout géométrique pur
(wrapper expulsé hors viewport), bordeaux.fr par un détachement DOM
post-re-parse qui ne se résout jamais. À l'inverse numerique.gouv.fr
(DSFR + tarteaucitron, JS réellement exécuté) est le site le plus sain
volé récemment : header Marianne, nav, fil d'Ariane, hero, `position:
fixed` honoré, clic → navigation réelle.

---

## 1. www.paris.fr — → §B-48 (proposé) : wrapper expulsé à x=viewport, w=0

- `POST navigate` → `doc_h=9 927` (non clampé), titre correct « Paris.fr,
  site officiel de la Ville de Paris », retour < 15 s.
- **Frame** (`captures/vol73_paris.png`) : quasi-blanche — seul un motif
  **voilier** (emblème de Paris) peint en quasi-transparent centré-haut ;
  `vol73_paris_scroll.png` (scroll_y=1 600) : blanche intégrale.
- **Layout** (`logs/vol73_paris_layout.txt` l.1-12) : la structure est
  complète et saine *partout sauf un point* :
  `block<body> [0 0 1366x9927]` contient `block<ul> [0 0 1366x0]` (skip-
  links) puis **`block<div> [1366 0 0x9927.348]`** — c'est
  `<div class="paris-main-wrapper">` (`logs/vol73_paris_dom.txt` l.188) :
  **largeur 0, posée à x=1366**, et tous ses descendants layoutés à
  x≥1366 (`header` `[1366 0 1366x80]`, `<h1>` logo `[1381 8.5]`, nav,
  contenu à x=3 497-3 790…). **Tout le site est peint hors viewport** →
  la frame blanche n'est ni §B-7 (`visibility`) ni un boot mort : c'est
  une **expulsion géométrique**.
- Lecture mécanique probable : le motif `[0 0 1366x0]` + `[1366 0 0xH]`
  ressemble à un **flex row nowrap dont l'item 2 est compressé à 0 et
  poussé au bord droit** — famille §B-35 (colonne effondrée, RÉSOLU V5)
  transposée à la racine. Les panels méga-menu `position:absolute`
  `[0 80 1366x740]` (« Dans l'actualité », « Accès rapides », recherche)
  sont, eux, correctement ancrés à x=0 — mais restent non peints
  (état replié, plausiblement correct).
- **Borne §B-26 validée en passant** : `WARN apply: cycle de report
  cassé (§B-26) dropped=5 path=Some([0,1,2,0,1,13,1,0,0])` (14:38:49)
  pendant le premier apply — l'apply a **terminé** (`repaint ms=263`),
  process vivant. La mitigation fait son travail.
- **Console** : `pas de journal` — zéro erreur ; scroll ×2 →
  `scroll_y=1 600` accepté ; clic (300,400) → `edited` (aucune ancre
  visible n'existe à ces coords — cohérent avec l'expulsion).
- **Télémétrie** : `dom_nodes=2 598`, `layout_boxes=1 295`,
  `display_items=3 944`, `images_bytes=39 Mo`, `js_heap` 32 Mo→0,
  `journal_fallbacks=1`, `frame_bytes=54 Mo`.
- **→ §B-48 (proposition d'entrée, §8)** — *« Conteneur racine
  collapsé à largeur 0 et expulsé à x=largeur viewport — page complète
  peinte hors-champ, frame blanche, aucune erreur »* — crate
  `explorer-layout`/`explorer-css`. Preuve : layout l.12
  `block<div> [1366 0 0x9927]` = `.paris-main-wrapper` ; frame quasi-
  blanche. Famille §B-35 (effondrement de boîte, résolu V5) /
  symptôme §B-7 (page blanche à layout complet) mais **mécanisme
  nouveau** : ce n'est pas un masque, c'est une translation
  géométrique de tout le sous-arbre.

## 2. www.bordeaux.fr — §B-26 variante : détachement terminal post-re-parse

- `POST navigate` → `doc_h=7 290`, titre correct « Site officiel de la
  ville de Bordeaux | Bordeaux », retour < 15 s.
- **Premier apply sain** (14:40:22) : `cascade 322 / boxtree 374 /
  layout 518 / display_list items=6 837 / repaint ms=670`, avec
  `cycle de report cassé (§B-26) dropped=1 path=[0,1,0,0]` — la borne
  tirée, l'apply **complété**.
- **Puis** : `journal DOM rejeté → repli re-parse (ADR-0025) tab=2
  why=UnmappedKey` (14:40:22.72) — et **plus jamais de `apply:`** :
  `/dom` 404, `/layout` 404, `/frame` 204, `dom_nodes=0`
  **continuellement pendant >7 min** (retentes à +12 s, +45 s, +3 min,
  +7 min — jamais réattaché).
- **Flag peinture bloqué** : mon clic (14:43:49) journalisé
  `input différé — paint en vol, rejeté comme ignored tab=2` — un
  paint est cru « en vol » depuis l'apply de 14:40:22 → les inputs
  sont systématiquement refusés `ignored`.
- **Pompe vivante** : `pump.js_run_sync` + `apply_queued` + `drains`
  toutes les ~30-70 s (14:41:32, 14:42:44, 14:43:33, 14:47:09),
  `journal_fallbacks=3`, `journal_applies=2`, `js_heap` ~11 Mo stable,
  API parfaitement responsive — **pas de spin, pas de fuite
  intra-onglet visible** (RAM tab imputable ~+100-400 Mo).
- Distinction avec les signatures connues : **pas §B-26 canonique**
  (l'apply a dépassé `layout` et `repaint`) ; **pas §B-33**
  (`navigate` retourné, scroll accepté `scroll_y=1 600`) ; **pas
  §B-41** (pas de boucle de phases). Signature nouvelle : **repli
  re-parse ADR-0025 qui ne produit jamais de nouveau commit** —
  document détaché + `paint en vol` jamais libéré. À raccrocher à
  §B-26 comme variante post-re-parse (le fix `emit_scoped_at` couvre
  le cycle de report, pas ce chemin) ou à ouvrir en §B-49 si la
  distinction se confirme sur un 2ᵉ porteur.
- Console : `pas de journal` — aucun rejet tracé.
- Aucune capture possible (`/frame` 204) — preuve = journal daemon +
  mémo (`logs/vol73_bordeaux_mem.json`, `vol73_bordeaux_dom.txt`
  = « pas de DOM »).

## 3. www.numerique.gouv.fr — correct+ ; §B-17 porteur DSFR

- `POST navigate` → `doc_h=7 736`, titre correct « À la une —
  numerique.gouv.fr ».
- **Frame** (`captures/vol73_numerique.png`) : **site réellement
  rendu** — Marianne « Gouvernement », bloc-marque
  « numerique.gouv.fr », tagline, champ « Rechercher » (clippé au bord
  droit), nav 6 rubriques (dernier item « La transformation numérique
  des territoires » clippé → débordement horizontal mineur), bande
  FranceConnect, hero « À la une » + illustrations peintes.
- **§B-17 +1 porteur DSFR** : liste verticale « Offre d'accompagnement
  · Services numériques · Actualités · Dernières informations · Espace
  presse · Blog · Postuler » peinte en pleine page — DOM : `nav.fr-nav
  data-fr-js-navigation` + `class="fr-collapse fr-menu fr-collapsing"`
  (`logs/vol73_numerique_dom.txt` l.533/563) — le panneau DSFR
  `fr-collapse` (repli dépendant du JS `disclose`) est layouté/peint
  déplié, exactement le bras « ceux fermés par JS restent ouverts ».
- **Consentement tarteaucitron vivant** : la carte `position:fixed`
  `[403 596 640x264]` (« Ce site utilise des cookies… » + Personnaliser
  / Tout refuser / Tout accepter — `logs/vol73_numerique_layout.txt`
  l.831-847) est correctement centrée et **suit le scroll** (fixed
  honoré). JS exécuté : tarteaucitron a injecté tout son arbre
  (`dom l.4316+`). Défaut cosmétique : les boutons
  « Tout refuser »/« Tout accepter » sont **texte blanc sur fond clair**
  (fond bouton non peint — CSS tarteaucitron `useExternalCss`),
  quasi-illisibles.
- **Clic « Tout accepter » (870,808) → `edited`** — livré au DOM,
  carte toujours peinte après : handler incomplet (§B-4 bras c,
  mineur — `set cookie + remove` n'a pas abouti).
- **Clic nav « La DINUM » (596,143) → `navigated`** : navigation
  réelle `/numerique-etat/dinum/`, titre correct, `apply: fin ms=347`
  après re-parse (`Fingerprint`). **§B-13 sain `[V6-main]`**.
  Frame `vol73_numerique_dinum.png` : fil d'Ariane, hero « La DINUM —
  Cheffe de file du numérique de l'État », état actif souligné —
  fidèle.
- **Scroll** ×2 → `scroll_y=1 600` (`vol73_numerique_scroll.png`) :
  repaint cohérent, sections « Un État plus simple… » et « Une
  stratégie collective » peintes, carte fixed toujours centrée.
- **Télémétrie** : `dom_nodes=1 823`, `layout_boxes=848`,
  `display_items=987`, `images_bytes=64 Mo`, `js_heap` 5 Mo,
  `journal_fallbacks=2`. Aucun churn.
- **§B-45 sain ×3** : titres `/api/tabs` corrects UTF-8 sur les trois
  cibles (pas de mojibake observé ce vol).

## Statuts proposés (§8)

| §B | Bras | Verdict ce vol `[V6-main]` |
|---|---|---|
| **§B-48 (proposé)** | **wrapper racine collapsé `w=0` expulsé à x=viewport** | paris.fr : `.paris-main-wrapper [1366 0 0x9927]`, tout le contenu hors-champ → frame blanche à layout complet ; suspect flex row/compression d'item ; famille §B-35, symptôme §B-7 ; crate `explorer-layout`/`css` |
| §B-26 | wedge DOM détaché | **variante post-re-parse** : apply complet (`repaint ms=670`) + `dropped=1`, puis `journal rejeté UnmappedKey` → re-parse → **jamais réattaché >7 min** + `paint en vol` bloqué (`input différé → ignored`) — le fix couvre le cycle de report, pas le repli qui n'aboutit pas ; à raccrocher §B-26 ou §B-49 si 2ᵉ porteur |
| §B-26 | borne `emit_scoped_at` | **validée terrain ×2** : `dropped=5 [0,1,2,0,1,13,1,0,0]` (paris), `dropped=1 [0,1,0,0]` (bordeaux) — apply terminés, process vivants |
| §B-17 | menus fermés par JS restent ouverts | **+1 porteur DSFR** — `fr-collapse fr-menu fr-collapsing` peint déplié (numerique.gouv.fr) : la disclose JS n'a jamais replié le panneau |
| §B-13 | hit-test | **sain** — clic nav → `navigated` réelle `/dinum/` ; clic consentement `<button>` → `edited` attendu (pas d'action par défaut) |
| §B-4(c) | JS exécuté sans effet | tarteaucitron `Tout accepter` livré mais carte non fermée — handler partiellement exécuté, console muette |
| §B-30 | fuite post-DELETE | **signal contraire** — 1 109 → 549 Mo stable, `{"tabs":[]}`, zéro log post-DELETE |
| §B-45 | mojibake titres JSON | **signal contraire ×3** — titres corrects UTF-8 paris/bordeaux/numerique+dinum |
| cosmétique | boutons consentement | tarteaucitron : texte blanc sur fond clair (fond bouton non peint) — quasi-invisible |

## Hygiène de vol

- Daemon PID **16800**, port 9455 — kill par `taskkill //PID 16800 //F`
  (jamais `/IM`) ; `{"tabs":[]}` confirmé avant arrêt ; port libéré.
- Onglets : 1 (paris.fr), 2 (bordeaux), 3 (numerique.gouv.fr) — DELETE
  204 ×3, purge 1 109 → 549 Mo.
- Artefacts : `captures/vol73_{paris,paris_scroll,bordeaux(vide—204),
  numerique,numerique_accept,numerique_scroll,numerique_dinum}.png` ;
  `logs/vol73_{paris,bordeaux,numerique}_{dom,layout}.txt`,
  `vol73_{paris,bordeaux,numerique}_mem.json`,
  `vol73_{bordeaux,numerique}_console.txt` ; journal `_vol73_daemon.log`
  (racine).
- Limites honnêtes : la cause exacte de l'expulsion paris (flex row
  suspectée, non prouvée — pas d'introspection du style calculé) ; la
  carte bordeaux jamais vue peinte (frame 204 depuis le premier accès
  — le repaint ms=670 journalisé n'a pas produit de PNG servi) ;
  l'échec du clic consentement numerique n'est pas bissecté (handler
  JS vs hit-test interne).

<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 56 — Wikis / SSR (Wikipédia · wikiless · cjoint) (2026-10-27)

- **Binaire** : `[V4-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (WIP `depth_limit` embarqué, §B-3 confirmé ouvert), `--port 9438`,
  `EXPLORER_VIEWPORT=1366x900`, journal `_vol56_daemon.log` (racine, PID 18264)
- **Cibles** : fr.wikipedia.org · wikiless.org/wiki/Main_Page?lang=fr ·
  www.cjoint.com — **3/3 volées**, terrain favorable (SSR/wiki), aucun wedge
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de `SYNTHESE.md`
- **RAM daemon (PID 18264)** : 375 Mo boot → 469 Mo (wiki) → 667 Mo
  (article) → 906 Mo (cjoint) → **969 Mo final** — croissance linéaire
  saine, loin du seuil 2 Go ; aucune rafale §B-26.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| fr.wikipedia.org | **Excellent** — peau Vector quasi fidèle, clic→article réelle, bfcache OK | `pas de journal` (×2 navs) | runs inline superposés dans « Éphéméride » → **§B-18 confirmé** ; clic `ignored` isolé (visée bord de viewport) ; carrés noirs d'icônes |
| wikiless.org | **Cible morte — domaine squatté** : `/wiki/Main_Page?lang=fr` → HTTP 404 (handled), la racine sert un site de jeu « KINGBET188 » | `pas de journal` | ⚪ **externe** : le frontend wiki n'existe plus (domaine ré-enregistré) ; le site parasite rend correctement |
| www.cjoint.com | **Contenu juste, layout images cassé** — catalogue notices Samsung rendu mais `<img>` `max-h-full object-contain` à taille intrinsèque | `pas de journal` | **nouveau → §B-31 proposé** (`max-height`% ignorée sur remplacé) ; `doc_h=22 672` clampée, `images_bytes` 80 Mo |

Fait marquant : **zéro entrée console sur les 3 sites, zéro erreur JS**,
navigations/clics/bfcache/scroll tous fonctionnels — le terrain SSR
confirme la santé du pipeline. Le seul trou moteur du vol est un bug
de sizing d'élément remplacé (cjoint, §B-31 proposé).

---

## 1. fr.wikipedia.org — référence SSR, presque irréprochable

- `POST navigate` → `doc_h=3 640` (non clampée), titre « Wikipédia,
  l'encyclopédie libre », redirection `→ /wiki/Wikipédia:Accueil_principal`.
- Apply initial : `cascade 237 ms → repaint 1 378 ms` (premier paint ~1,4 s) ;
  applies scroll/post-nav 5-46 ms. Évals JS : petits scripts (214 o–4,6 Ko)
  tous `ok=true`, aucun rejet.
- **Frame** (`captures/vol56_wikipedia.png`) : logo, champ recherche,
  onglets Accueil/Discussion/Lire, « Article labellisé du jour » avec
  image, colonne Présentation — rendu très proche de la référence.
- **Télémétrie** : `dom_nodes=3 653`, `layout_boxes=1 782`,
  `js_heap=8,7 Mo`, `total≈35 Mo`.
- **Scroll ×2** → `scroll_y=1 600`, repaint correct
  (`captures/vol56_wikipedia_scroll.png`).
- **Clic** « Éphéméride détaillée du 27 octobre » → `{"result":"navigated"}`
  → `fr.wikipedia.org/wiki/27_octobre`, `can_back=true`.
- **Article** (`captures/vol56_wikipedia_article.png`) : sommaire latéral,
  fil d'Ariane, infobox « Éphémérides » avec calendrier d'octobre en
  table stylée (« 27 » en gras) — très fidèle. `dom_nodes=10 589`,
  `bfcache_pages=1` (accueil retenue, `js_heap` bfcache 8,7 Mo).

### Constats

- **§B-18 confirmé `[V4-main]`** — `explorer-layout`/`render` : dans la
  colonne « Éphéméride du 27 octobre » (capture scroll), les runs inline
  des entrées **1856 et 1956 se superposent** (« 1956 : le traité
  européen de la Sarre… » peint par-dessus le texte voisin vers
  y≈320-430 viewport). Même signature que lemonde/ouest-france vol 48.
  Repro disponible : `logs/vol56_wikipedia_layout.txt` + capture.
- **Un `WARN` benign** : `journal DOM rejeté → repli re-parse (ADR-0025)
  tab=1 why=UnmappedKey` à 09:28:20 (nav accueil → article) —
  `journal_fallbacks=1` sur les `/mem` suivants ; le repli a fonctionné,
  aucun impact visible. À rattacher à la famille des rejets de journal
  ADR-0025 (recoupement §B-26 : ici UnmappedKey **sans** wedge).
- **Clic `ignored` isolé** : premier clic à (88, 848) sur « Allan Blair »
  (lien tout en bas de viewport à scroll_y=1600) → `ignored` ; le clic
  suivant (820, 553) a navigué normalement. Bord de lien probablement
  tronqué au bas du viewport — observation marginale, **pas** de retour
  §B-13 (les vrais clics naviguent).
- **Cosmétique** : petits carrés noirs pleins à la place d'icônes du
  chrome Vector (puce menu en haut à gauche, après « Voir l'historique »,
  avant « 349 langues ») — probablement icônes CSS-mask/SVG inline non
  rendues → famille §B-cosm. (masques/pictos) ; encadré « Apparence /
  masquer » peint en haut à droite (peut être un panneau légitime de
  l'accueil fr.wikipedia — à vérifier, non tranché).

## 2. wikiless.org — cible morte : domaine ré-enregistré (externe)

- `POST navigate /wiki/Main_Page?lang=fr` → **`{"error":{"handled":true,
  "message":"HTTP 404"}`** — erreur typée propre, pas de crash.
- `navigate /` (racine) → charge une page titre
  « KINGBET188 Platform Game Online Terbaik Terpercaya 2026 » —
  **le domaine wikiless.org a été ré-enregistré par un site de jeu
  indonésien** (slot). Le frontend wiki a cessé d'exister.
- Frame `captures/vol56_wikiless_root.png` : le site parasite est rendu
  correctement (colonne centrée ~370 px, dégradés or/noir, images peintes,
  DOM 193 nœuds, `js_heap` 0,6 Mo, console vide).
- **Verdict** : ⚪ hors moteur — à retirer des cibles terrain
  « wiki/SSR favorable » (analogue aux entrées anti-bot §B-externe :
  c'est le terrain qui a changé, pas le moteur). Aucune anomalie moteur.

## 3. www.cjoint.com — `max-height:%` ignorée sur `<img>` (§B-31 proposé)

- `POST navigate` → `doc_h=22 672,8` **`doc_h_clamped=true`**, titre
  « Accueil de Cjoint.com » — le site sert désormais un **catalogue de
  notices PDF** (« Mode d'emploi et notice des produits Samsung »).
- **Frame** (`captures/vol56_cjoint.png`) : grille 3 colonnes correcte
  (Tailwind `grid lg:grid-cols-3`, cartes `border rounded-lg
  overflow-hidden` appliquées, titres/liens bleus OK) **mais chaque
  vignette `<img>` est peinte à taille intrinsèque** (couvertures PDF
  ~700×1 000 px) et tronquée par le clip `overflow:hidden` de la carte —
  « MODE D'EMPLOI » géant illisible, dépassement de l'espace prévu.
- **Layout** (`logs/vol56_cjoint_layout.txt`) — signature exacte :
  conteneur `block<div> [9 178,99 366x192]` (la classe `h-48` = 192 px
  **est** honorée) contenant `block<img> [-159,5 -222,0 703x994]` :
  la boîte image garde la **taille intrinsèque** du webp, centrée à
  offsets négatifs (x ≈ (366−703)/2 + bordure) par le `flex
  items-center justify-center` du parent. La classe `max-h-full`
  (`max-height:100%`, base définie 192 px) **n'a pas contraint la boîte
  remplacée** ; `object-fit:contain` (implémenté R-34, `display.rs
  object_fit_dst`) ne joue que sur la destination *de peinture* dans la
  boîte — la boîte elle-même reste 703×994.
- DOM verbatim (`logs/vol56_cjoint_dom.txt`) :
  `<div class="h-48 bg-gray-100 flex items-center justify-center">
  <img class="max-h-full object-contain" src="…/page_001.webp">`.
  188 cartes → empilement de boîtes ~994 px → `doc_h` 22 672.
- **Coût mesuré** : `images_bytes=80,6 Mo` décodés pour
  `img_bodies_bytes=288 Ko` transférés ; `total_bytes≈86 Mo` ;
  `journal_fallbacks=1` (même WARN UnmappedKey bénin, repli OK).
- **Scroll ×2** → `scroll_y=1 600`, repaint OK
  (`captures/vol56_cjoint_scroll.png` : mêmes vignettes géantes clippées,
  jagged tops = sommets des glyphes « SAMSUNG »).
- **Clic** (934, 655) → `{"result":"navigated"}` →
  `/mode-d-emploi/samsung/galaxy-watch4`, page détail rendue
  (`captures/vol56_cjoint_article.png` : fil d'Ariane, titre, méta, puis
  couverture PDF géante ~900 px — même signature de taille intrinsèque,
  mais acceptable ici). `bfcache_pages=1`, `can_back=true`.
- **Console** : `pas de journal` sur les deux navs — aucun échec
  sous-ressource tracé.

## Statuts proposés (§8)

| §B | Bras | Verdict ce vol |
|---|---|---|
| **§B-31 (NOUVEAU)** | `explorer-css`/`explorer-layout` — **`max-height` (%) non appliquée aux éléments remplacés** : `<img>` dans conteneur à hauteur définie garde sa taille intrinsèque (offsets négatifs via centrage flex), déborde puis est clippée par `overflow:hidden` | **proposé** — cjoint.com `[V4-main]` : 188 vignettes ×~994 px, `doc_h` 22 672 (clamped), `images_bytes` 80 Mo ; layout verbatim en preuve. Voisin de §B-28 (`height:100%` effondrée à 0) : **même famille « % de hauteur mal résolu », bras opposé (intrinsèque au lieu de 0)** |
| §B-18 | runs inline superposés | **confirmé `[V4-main]`** — éphéméride accueil fr.wikipedia, scroll_y=1 600 |
| §B-cosm. | icônes CSS-mask/SVG = carrés noirs | recoupement cosmétique — chrome Vector (wiki) |
| ⚪ externe | cible terrain | wikiless.org **squatté** (jeu d'argent) — à rayer des listes de cibles SSR |
| ADR-0025 | rejet journal `UnmappedKey` | recoupement — 2 occurrences (tabs 1 & 3), repli re-parse sain, **sans** wedge (contraste §B-26) |

## Hygiène de vol

- Onglets 1 (wikipedia), 2 (wikiless), 3 (cjoint) fermés par
  `DELETE /api/tabs/{id}`.
- Daemon tué par son **PID 18264** (`taskkill //PID`, jamais `/IM`) ;
  les daemons voisins (PID 3444, 17436) n'ont pas été touchés.
- RAM max observée **969 Mo** — sous le seuil de prudence 2 Go, aucune
  action d'urgence §B-26 nécessaire.
- Artefacts : `captures/vol56_{wikipedia,wikipedia_scroll,
  wikipedia_article,wikiless_root,cjoint,cjoint_scroll,
  cjoint_article}.png` ; `logs/vol56_{wikipedia_console.json,
  wikipedia_dom.txt,wikipedia_layout.txt,wikipedia_mem.txt,
  wikipedia_article_console.json,wikipedia_article_dom.txt,
  wikipedia_article_mem.txt,wikiless_dom.txt,wikiless_layout.txt,
  wikiless_mem.txt,cjoint_console.json,cjoint_dom.txt,cjoint_layout.txt,
  cjoint_mem.txt,cjoint_article_console.json,cjoint_article_dom.txt,
  cjoint_article_mem.txt}` ; journal `_vol56_daemon.log` (racine).

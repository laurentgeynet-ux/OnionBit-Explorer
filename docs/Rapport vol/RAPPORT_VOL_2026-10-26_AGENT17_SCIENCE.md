<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 17 — Sites scientifiques / documentaires

Date : 2026-10-26. Daemon : `C:/obe-vol-head/target/release/explorer-daemon.exe`
(release), port 9401, `EXPLORER_VIEWPORT=1366x900`.
Captures : `captures/vol17_*.png` — logs : `logs/vol17_*`.

## Synthèse

Les quatre cibles rendent et restent **lisibles**. Aucun crash, aucune
page blanche. Trois défauts transverses dominent :

1. **`</#comment>` rendu comme texte** sur 3 sites sur 4 (arXiv,
   Wikipedia, Wikidata) — balises de fin de commentaire (ou commentaires
   conditionnels) fuient dans le flot de texte.
2. **Cascade jQuery en échec sur arXiv** : `toLowerCase is not a
   function` puis `ReferenceError: $ is not defined` ×2.
3. **Superpositions inline** sur Wikipedia : figures et éléments
   encadrés (`http:`/`https:`) recouvrent le texte du paragraphe.

---

## Cible 1 — arXiv listings

- URL demandée : `https://arxiv.org/list/cs/new`
- URL finale : `https://arxiv.org/list/cs/new` puis, après clic,
  `https://arxiv.org/abs/2610.31678`
- `doc_h = 287893.47`, **`doc_h_clamped = true`** — le listing
  (642 entrées annoncées par la page) dépasse le plafond de document.
- Console (verbatim) après navigation vers /abs :
  - `exception js : TypeError: \`toLowerCase\` is not a function (on Object) at <anonymous> (https://code.jquery.com/jquery-latest.min.js:4:13185) …`
  - `exception js : ReferenceError: $ is not defined — https://arxiv.org/static/browse/0.3.4/js/toggle-labs.js?20241022`
  - `exception js : ReferenceError: $ is not defined — https://arxiv.org/static/browse/0.3.4/js/cite.js`

Constats mesurés (`vol17_arxiv.png`, `vol17_arxiv_scroll.png`,
`vol17_arxiv_click.png`) :

- Listing complet et lisible : titre « Computer Science », liens
  « New submissions / Cross-lists / Replacements », « Showing new
  listings for Monday, 26 October 2026 », « Total of 1093 entries »,
  entrées numérotées `[1] arXiv:2610.31678 [pdf, html, other]`,
  titres en gras, auteurs en liens, Comments/Subjects et abstracts
  justifiés. Logo arXiv texte stylé rendu ; champ Search en place.
- Le TeX brut des abstracts (`$F_1$`, `$C(v,k,t)$`, `$k$`, `$v$`,
  `$(v-1,k-1,t-1)$`, `$\in\{3,4,5\}$`, `$C(39,24,3)=9$`) reste
  littéral — attendu sans MathJax (jQuery mort → MathJax non
  déclenché non plus ; le lien « Disable MathJax » est visible en
  bas de page /abs).
- Page /abs : breadcrumb, abstract, submission history, panneau
  latéral « Access Paper » (View PDF / HTML / TeX Source),
  « References & Citations », onglets « Bibliographic Tools /
  Code, Data, Media / Demos / Related Papers / About arXivLabs »
  tous rendus et alignés.
- **`</#comment>` littéral** rendu au moins 6 fois sur la page /abs
  (avant le breadcrumb, dans la colonne droite, avant Comments,
  avant les onglets, dans le pied).
- Clic sur `arXiv:2610.31678` → navigation correcte vers la page
  /abs (`result: navigated`, `can_back: true`).

Classification : **rendu correct avec artéfacts**. Sévérité :
moyenne (fuite de commentaires = pollution visuelle récurrente ;
jQuery mort = fonctionnalités interactives arXiv absentes ;
clamp document à surveiller sur les listings).

## Cible 2 — Wikipedia « Web browser » (check régression)

- URL : `https://en.wikipedia.org/wiki/Web_browser`
- `doc_h = 19310.46`, **`doc_h_clamped = true`** (marqueur posé
  même à ~19k px — à vérifier : sur Wikidata, `doc_h` 3227 →
  `clamped: false` ; le seuil n'est donc pas systématique).
- Console : `pas de journal` (aucune erreur).

Constats mesurés (`vol17_wikipedia.png`, `vol17_wikipedia_scroll.png`,
`vol17_wikipedia_click.png`) :

- Chrome Wikipedia complet : logo, champ de recherche avec bouton
  « Search », Donate / Create account / Log in, boîte « Contents »
  avec liens d'ancrage fonctionnels (clic « Features » → navigation
  `#Features` confirmée par l'URL).
- `</#comment>` littéral centré en haut de page.
- **Recouvrement du paragraphe d'introduction** : « A web browser,
  often abbreviated » est rendu deux fois en surimpression (effet
  barré/doublé mesuré visuellement), et la boîte figure « A web
  browser (Safari) on a MacBook displaying the Wikipedia web page »
  est dessinée en pleine largeur (≈ 730 px de large, bordure double)
  et **chevauche le texte** du lead au lieu de flotter à droite.
- Idem plus bas : la figure « Navigating to English Wikipedia using
  a web browser (Firefox) » recouvre la colonne de texte ; les spans
  `http:` et `https:` sont peints avec bordure et **par-dessus** la
  ligne « start with either … which means they are retrieved with
  the » (texte raturé visible).
- Table sortable « Web browser / Market share / Reference » : **seul
  l'en-tête est rendu** avec `[show]` — les lignes de données sont
  absentes (table repliable restée repliée faute de JS, ou lignes
  masquées non recouvertes).
- Sections « Function », « Browser market », « Market share by type
  of device », « Security », « Privacy » rendues correctement ;
  références `[n]` en exposant présentes ; liens bleus OK.
- Scroll ×3 sans défaut ; clic sur ancre Contents → navigation.

Classification : **régression partielle vs historiquement bon** —
page lisible mais deux classes de défauts de layout (figures non
flottantes + inline box chevauchants) dégradent le lead. Sévérité :
moyenne-haute pour le recouvrement de texte.

## Cible 3 — Wikidata

- URL : `https://www.wikidata.org` → `https://www.wikidata.org/wiki/Wikidata:Main_Page`
- `doc_h = 3227.37`, `doc_h_clamped = false`. Console : `pas de journal`.

Constats mesurés (`vol17_wikidata.png`, `vol17_wikidata_scroll.png`,
`vol17_wikidata_click.png`) :

- Page d'accueil quasi complète : logo réseau (lignes/nœuds SVG
  visibles mais partiellement — le rendu semble fil de fer sans les
  nœuds colorés), « Welcome to Wikidata », « 123,759,067 data
  entities », grilles de panneaux colorés (Welcome! vert, Learn
  about data bleu, Get involved rouge, News bleu, Related to
  current events vert, Discover rouge, Contact bleu), listes
  d'entités `Alaatin Cagil (Q104777195)`, `Douglas Adams (Q42)`…,
  portraits (image Claude Cohen-Tannoudji), symbole « Mind the
  gap », bandeau famille Wikimedia en bas (toutes les icônes des
  projets sœurs rendues en ligne).
- `</#comment>` littéral ×4 en pied de page (`</#comment>
  </#comment> </#comment> </#comment>`) + un en haut.
- Panneau « Appearance hide » visible en haut à droite — le
  panneau de préférences Vector 2022 devrait être replié (JS).
- Clic sur l'image SVG → navigation vers
  `File:Wikidata_nodes_in_white.svg` : page de description de
  fichier rendue intégralement (métadonnées, licensing, file
  history tabulaire, file usage, global file usage) — dense mais
  structurée.

Classification : **bon rendu**. Sévérité : basse (artéfacts
commentaires + panneau non replié).

## Cible 4 — developer.chrome.com/docs

- URL : `https://developer.chrome.com/docs`
- `doc_h = 7574.0`, `doc_h_clamped = false`. Console :
  `pas de journal`.

Constats mesurés (`vol17_devdocs.png`, `vol17_devdocs_scroll.png`,
`vol17_devdocs_click.png`) :

- Thème sombre complet, grille de cartes rendue : « Build with
  Chrome », « Productivity made easier » (DevTools, Lighthouse,
  Chrome UX Report, PageSpeed Insights, Workbox, ChromeDriver,
  Puppeteer — icônes circulaires colorées OK), « Powerful platform
  capabilities », « Solutions for quality experience », « Build an
  open web », « More to learn », pied de page.
- Les descriptions de cartes et liens de sous-sections sont rendus
  mais en très petite taille dans le PNG plein-document — le
  `/frame` a retourné le **document entier 1366×9094 px (53 Mo)**
  et non le viewport 1366×900 attendu : comportement de l'endpoint
  à documenter (comparé aux frames viewport des autres cibles).
- Clics : deux `input click` consécutifs ont retourné
  `{"result":"ignored"}` puis le 3e a navigué vers
  `https://developer.chrome.com/docs/devtools` — page DevTools
  rendue proprement (breadcrumb « Docs > Chrome DevTools », onglets
  Get started / AI assistance / Panels / Settings / Accessibility /
  For agents, sections « Debug with AI », « AI innovations »,
  « DevTools for agents », liens bleus). Hit-testing fonctionnel
  mais flaky sur cette page (2 ignorés / 1 navigated) — ou le
  navigateur ignorait les clics pendant un travail de pipeline.

Classification : **bon rendu**, hit-test intermittent. Sévérité :
basse-moyenne.

## Constats actionnables

1. **Fuite de `</#comment>` dans le rendu** (arXiv /abs ×6+,
   Wikipedia ×1, Wikidata ×5) — le tree-builder HTML émet des nœuds
   texte pour des marqueurs de fin de commentaire / commentaires
   conditionnels MediaWiki & arXiv. Repro : tout `<!…>` mal ouvert
   sur ces pages. Priorité haute : pollution transverse.
2. **jQuery : `toLowerCase` is not a function (on Object)**
   (jquery-1.11.1, arXiv) — probablement appel sur un objet non
   string dans le boot jQuery → casse en chaîne tout le JS du site
   (`$ is not defined` ×2). Bissecter l'appel fautif dans
   explorer-jscore.
3. **Figures Wikipedia non flottantes / recouvrantes** — les
   `figure`/`div.thumb` (ou `class="thumb tright"`) sont posés en
   pleine largeur avec bordure par-dessus le texte ; mêmes
   symptômes sur les `<code>`/spans `http:`/`https:`. Suspect :
   float:right ou positionnement des blocs avec `mw-halign` non
   résolu → boîte absolue à pleine largeur.
4. **Double peinture du lead Wikipedia** — « A web browser, often
   abbreviated » rendu deux fois en surimpression (texte
   strikethrough-artifact) : possible double enregistrement du run
   de texte dans la display list (peinture incrémentale ?).
5. **Tableaux repliables Wikipedia vides** — tables `collapsible`/
   `sortable` n'affichent que l'en-tête + `[show]` : soit les lignes
   `display:none` ne sont jamais révélées (normal sans JS), soit le
   parser perd `<tbody>`. À distinguer par le dump DOM
   (`logs/vol17_wikipedia_dom.txt`).
6. **`doc_h_clamped` posé à 19 310 px sur Wikipedia** — vérifier le
   seuil : vrai clamp (perte de contenu) ou drapeau mal armé.
7. **`/frame` retourne le document entier sur devdocs**
   (1366×9094, 53 Mo) vs viewport 1366×900 ailleurs — préciser le
   contrat de l'endpoint (et son poids).
8. **Clics intermittents `ignored` sur devdocs** — 2/3 clics
   ignorés avant navigation réussie ; hit-test ou busy-gate à
   instrumenter.
9. **MathJax jamais déclenché sur arXiv** — conséquence de (2) ;
   re-tester après fix jQuery.

## Arrêt

Onglets fermés, daemon tué par son PID. Aucun code modifié.

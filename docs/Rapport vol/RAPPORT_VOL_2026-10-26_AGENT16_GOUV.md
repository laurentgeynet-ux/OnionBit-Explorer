<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 16 — Sites gouvernementaux

Date : 2026-10-26
Daemon : `explorer-daemon.exe --port 9400` (release, `EXPLORER_VIEWPORT=1366x900`,
binaire `C:/obe-vol-head/target/release/`).
Méthode : API REST (tabs/navigate/input/frame/dom/layout/console),
captures PNG plein-document (frame = 1366 × doc_h), dumps DOM/layout
dans `logs/vol16_*`, images dans `captures/vol16_*`.

Cibles : www.gov.uk, www.canada.ca, www.legifrance.gouv.fr, www.usa.gov.
Quatre cibles sur quatre visitées — pas de fallback nécessaire.

---

## 1. https://www.gov.uk (GDS)

- URL finale : `https://www.gov.uk/` puis `https://www.gov.uk/browse/benefits`
- Titre : « Welcome to GOV.UK » / « Benefits - GOV.UK »
- doc_h mesuré : 4645 px (accueil), 2779 px (benefits)
- Console : **vierge** — `pas de journal`, zéro exception JS sur
  les deux pages.
- Captures : `vol16_govuk.png`, `vol16_govuk_scroll.png`,
  `vol16_govuk_click.png` (page Benefits)
- Logs : `vol16_govuk_dom.txt` (153 Ko), `vol16_govuk_layout.txt`

### Constats mesurés

- Rendu global **très proche** de la référence : bannière cookies
  complète (3 boutons), bandeau bleu GOV.UK avec titre et champ
  Search, « Popular on GOV.UK », cartes « Featured » avec images,
  « Government activity », « More on GOV.UK », pied de page complet
  avec les trois colonnes de liens et la mention OGL.
- Skip-link présent dans le DOM (`gem-c-skip-link govuk-skip-link`,
  « Skip to main content », `logs/vol16_govuk_dom.txt:364-367`) et
  correctement **non peint** au chargement — comportement fidèle.
- **Défaut** : le bloc « Services and information » de la page
  d'accueil n'affiche **que les filets horizontaux** des 16 items —
  aucun libellé de lien peint (zone gauche entièrement vide entre
  les traits). Les `<a>` existent dans le DOM
  (`logs/vol16_govuk_dom.txt:574+`, classe
  `gem-c-layout-super-navigation-header__navigation-second-item-link`)
  : c'est le **menu déroulant de la super-navigation** qui devrait
  être replié par défaut — ici il est rendu ouvert, et ses liens ne
  sont pas peints (double faute : visibilité non masquée + texte
  invisible).
- Sur `/browse/benefits` : page quasi parfaite — fil d'Ariane
  « Home », « Most viewed », liste « Topics » complète avec
  descriptions. Pastilles de liste rendues en **disques noirs
  pleins** (marqueurs génériques, style GDS non reproduit) —
  cosmétique.
- Interaction : `click` sur un lien de la zone « Popular » renvoyé
  `{"result":"ignored"}` (zone probablement hors hit-test des liens
  masqués) ; navigation directe `/browse/benefits` OK, `back`
  disponible.

Classification : **excellent** — benchmark historique tenu.
Sévérité : **moyenne** (un composant majeur de la home — le menu
services — est vide de texte).

---

## 2. https://www.canada.ca (GCWeb / WET-BOEW)

- URL finale : `https://www.canada.ca/` (splash bilingue) puis
  `https://www.canada.ca/en.html`
- Titre : « Canada.ca » / « Home - Canada.ca »
- doc_h mesuré : 1350 px (splash), 2577 px (en)
- Captures : `vol16_canada.png` (splash), `vol16_canada_fr.png`
  (page en), `vol16_canada_scroll.png`
- Logs : `vol16_canada_dom.txt`, `vol16_canada_en_dom.txt` (84 Ko),
  `vol16_canada_en_layout.txt`

### Erreurs console (verbatim)

```
exception js : TypeError: cannot read `document` of Null
    at <anonymous> (1:316) — src «!function(a){var e="https://s.go-mpulse.net/boomerang/"…»
exception js : TypeError: `toLowerCase` is not a function (on Object)
    at <anonymous> (…/jquery/2.1.1/jquery.min.js:4:1493) …
exception js : ReferenceError: jQuery is not defined — wet-boew.min.js
exception js : ReferenceError: jQuery is not defined — theme.min.js
exception js : TypeError: `toLowerCase` is not a function (on Object)
    at <anonymous> (…/jquery/2.2.4/jquery.min.js:4:665) …
exception js : TypeError: cannot read `open` of Undefined — ep-pp.min.js:6:399
exception js : TypeError: cannot read `startsWith` of Undefined — suggestions.min.js:6:2
exception js : ReferenceError: jQuery is not defined — gc-thématique.min.js
```

### Constats mesurés

- **Splash bilingue correct** : marque « Government of Canada /
  Gouvernement du Canada », boutons « English » / « Français »
  (liens `/en.html`, `/fr.html`), « Terms & conditions / Avis »,
  mot-symbole Canada.
- **Fuite de nœud texte `<#comment>`** peint en haut à gauche du
  splash, et en répétition dans la section « Features » de la page
  en (7 occurrences : `<#comment> <#comment> <#comment>…`).
  Dans le dump DOM : `"<#comment>"` est un **nœud texte** juste
  avant un vrai `<!-- #comment -->`
  (`logs/vol16_canada_dom.txt:21-22`) — le HTML source contient
  vraisemblablement une syntaxe de gabarit/commentaire que le
  tree-builder expose comme texte.
- **Éléments de `<head>` dans `<body>`** : le dump montre
  `<title>`, `<meta>`, `<link>` enfants de `<body>`
  (`logs/vol16_canada_dom.txt:10-32`) — le tree-builder semble
  avoir basculé en « in body » prématurément (corrélé à la fuite
  de commentaire ci-dessus ?).
- **jQuery entièrement cassé** (`toLowerCase` is not a function) →
  toute la pile WET/GCWeb morte. Conséquence mesurée : **aucune
  barre de recherche ni bouton Menu dans l'en-tête** — le DOM
  contient bien `type="search"` (`vol16_canada_en_dom.txt:586`) et
  les skip-links, mais rien de tout cela n'est peint dans le header
  (seuls logo + titre).
- Page en par ailleurs lisible : « Most requested » (8 liens),
  bandeau « Website outages », tuiles de thèmes en 3 colonnes
  (grille OK ici), photo du Parlement, section « Features »
  parasitée par les `<#comment>`, méga-pied-de-page complet.
- Clic « Français » sur le splash : `{"result":"ignored"}` —
  bouton non cliquable au hit-test (malgré `<a href>` présent).
- Skip-links présents dans le DOM (« Skip to main content »,
  « Skip to "About government" », `vol16_canada_en_dom.txt:355-364`)
  et non peints — correct.

Classification : **correct en statique, fonctionnellement
dégradé** — recherche et menu absents (JS + header).
Sévérité : **majeure** (search = fonction n°1 du portail ;
fuite `<#comment>` visible = faute de parsing).

---

## 3. https://www.legifrance.gouv.fr (DSFR)

- URL finale : `https://www.legifrance.gouv.fr/` puis
  `https://www.legifrance.gouv.fr/codes/article_lc/LEGIARTI000032040777/`
- Titre : « Légifrance » / « Article 1103 - Code civil - Légifrance »
- doc_h mesuré : **8747 px** (accueil — éclaté, réf. ≈ 3500),
  7820 px (article)
- Captures : `vol16_legifrance.png`, `vol16_legifrance_article.png`
- Logs : `vol16_legifrance_dom.txt` (287 Ko),
  `vol16_legifrance_layout.txt`

### Erreurs console (verbatim, extraits)

```
exception js : TypeError: `toLowerCase` is not a function (on Object)
    at <anonymous> (…/webjars/jquery/jquery.min.js:4:665) …
exception js : TypeError: Bootstrap's JavaScript requires jQuery…
exception js : ReferenceError: jQuery is not defined
    at <anonymous> (…/jquery-accessible-accordion-aria.js:7:2)
exception js : ReferenceError: jQuery is not defined — jquery-accessible-tabs.js
exception js : Error: Datepicker's JavaScript requires jQuery — datepicker.min.js:10:2806
exception js : TypeError: value is not a function: undefined
    at <anonymous> (…/dsfr/core.module.min.js:2:11016)
exception js : TypeError: cannot read `internals` of Undefined
    — module «…/dsfr/scheme|accordion|navigation|breadcrumb|sidemenu|modal|header|display|tag|checkbox|tooltip.module.min.js»
exception js : TypeError: cannot read `core` of Undefined
    — module «…/dsfr/card|tile|toggle.module.min.js»
exception js : ReferenceError: $ is not defined
    at initCollapsibleRegions (…/legifrance.js:44:1)
```

### Constats mesurés

- **Mise en page effondrée** : toute la page (accueil et article)
  est empilée en **colonne unique étroite** (~150 px de texte à
  gauche) — la grille DSFR (fr-container / fr-grid-row / fr-col-*)
  n'est pas appliquée. Hauteur document ×2,5 vs référence.
- **Le méga-menu d'en-tête est entièrement déplié** et injecté
  dans le flux : « Accès … », « Codes », « Jurisprudence »,
  « Conventions et accords », « Documentation », toutes les
  entrées listées verticalement — normalement masqué par
  `fr-collapse` + JS d'accordéon (cassé : `internals`/`$` absents).
  Il occupe ~6000 px sur les 8747 de la page.
- Contenu de l'article **lisible malgré tout** : « Code civil »,
  sommaire (Article 1100 à …), encadré « ChoisirLég / Copier le
  texte de l'article », « Article 1103 » avec versions et
  « Informations pratiques » — le texte légal survit, la forme non.
- Date-picker / recherche JORF : champs présents dans le flux mais
  sans mise en forme (calendrier empilé « JANV FEV … JUIL AOUT »
  en deux colonnes serrées).
- Formulaire de recherche visible (« Rechercher sur Légifrance »,
  radios Article présenté / Article complet) — structure OK,
  style absent.
- Pied de page « République Française » présent avec bloc marque.

Classification : **cassé en mise en page** — contenu intégral mais
illisible dans l'état (méga-menu déplié = ~70 % de la page).
Sévérité : **critique** pour l'usage, probable cause racine
double : grille DSFR non calculée + composants collapse jamais
repliés (JS DSFR mort sur `internals`).

---

## 4. https://www.usa.gov (USWDS)

- URL finale : `https://www.usa.gov/` puis `https://www.usa.gov/housing-help`
- Titre : « Making government services easier to find | USAGov » /
  « Housing help | USAGov »
- doc_h mesuré : 4850 px (accueil)
- Console : **vierge** — `pas de journal` sur les deux pages.
- Captures : `vol16_usagov.png`, `vol16_usagov_scroll.png`,
  `vol16_usagov_click.png` (housing-help)
- Logs : `vol16_usagov_dom.txt` (83 Ko), `vol16_usagov_layout.txt`

### Constats mesurés

- **Même fuite `<#comment>`** que canada.ca : rangée de 7
  `<#comment>` littéraux en haut de page (mélangés à « Here's how
  you know ») et 2 en bas avant le pied de page + 5 tout en bas.
  Présent aussi sur housing-help — faute de parsing systématique
  sur les commentaires conditionnels/template.
- **Banner USWDS rendue** (« An official website of the United
  States government » + « Here's how you know »), logo USA.gov,
  bouton « Español » — mais **aucun menu ni recherche dans
  l'en-tête** (absents à la peinture).
- Hero « Making government services easier to find » rendu en
  petit bloc centré — l'image de fond et la **boîte de recherche
  héro sont absentes** ; chevron « Jump to all topics and
  services » peint.
- Cartes « All topics » : 19 tuiles listées, titre + description
  corrects, bordures cyan — mais **empilées à pleine largeur au
  lieu de la grille** (réf. : 2-3 colonnes).
- Clic **fonctionnel ici** : `click(700,300)` →
  `{"result":"navigated"}` → `/housing-help`. Fil d'Ariane
  « Home › Housing help », H1 rouge, texte d'intro, « Call us at
  1-844-USAGOV1 » — mais grand **bloc gris vide** (cartes/images
  absentes, espace ~450 px) et boutons « share » ok (icônes
  Facebook/X/mail présentes).
- Skip-link DOM OK (`usa-skipnav usa-sr-only`, `href="#skip-to-h1"`,
  cible `id="skip-to-h1"` présente, `vol16_usagov_dom.txt:365,1528`)
  et non peint — correct.
- Pied de page officiel complet (mention Executive Order 14241,
  liens Accessibility policy / Privacy / FOIA).

Classification : **bon en structure, dégradé en forme** — contenu
navigable, clics OK, mais fuites `<#comment>` visibles et grilles/
hero incomplets.
Sévérité : **moyenne**.

---

## Synthèse

| Cible | Console JS | Rendu | Navigation | Sévérité |
|---|---|---|---|---|
| gov.uk | vierge | quasi-parfait (1 composant vide) | nav OK, 1 clic ignoré | moyenne |
| canada.ca | 10+ exceptions | bon (hors header/features) | clic splash ignoré | majeure |
| legifrance | 20+ exceptions | effondré (colonne unique) | nav OK | critique |
| usa.gov | vierge | bon (cartes empilées) | clic OK | moyenne |

## Constats actionnables

1. **Fuite de commentaires en texte peint** : la séquence
   `<#comment>` (et voisines) apparaît comme nœud texte rendu sur
   canada.ca et usa.gov — vraisemblablement des commentaires
   conditionnels / gabarits serveur (`<!--[if`, `<#…>`) mal
   tokenisés par le tokenizer HTML. Reproduit sur 2/4 cibles —
   **bug n°1 à traiter côté explorer-html**.
2. **Tree-builder « after head » précoce** : `<title>`, `<meta>`,
   `<link>` observés enfants de `<body>` dans le dump canada.ca —
   à corréler avec le constat 1.
3. **jQuery cassé** (`toLowerCase is not a function (on Object)`)
   sur canada.ca et legifrance — en cascade : tout WET/GCWeb et
   tout l'outillage accessibilité (accordéons ARIA, tabs,
   datepicker) meurt. Trou jscore.
4. **Modules DSFR morts** : `dsfr/core.module.min.js` échoue
   (`value is not a function`), puis `cannot read 'internals'
   of Undefined` en cascade sur 13 modules — tout le système de
   composants DSFR (collapse, navigation, modal…) est inerte,
   d'où le méga-menu déplié de legifrance. Trou jscore (ES
   modules / custom elements ?).
5. **Grilles non appliquées** : conteneurs DSFR (`fr-grid-row`)
   et cartes USWDS empilés pleine largeur — CSS grid (ou
   flex-basis multi-colonnes) incomplet côté explorer-css/layout.
6. **Composants « hidden jusqu'à JS » rendus visibles** :
   super-nav de gov.uk (liens non peints mais filets visibles)
   et méga-menu legifrance — la cascade `hidden`/`[hidden]`/
   classe-collapse n'est pas honorée sans JS.
7. **Hit-test de clic partiellement aveugle** : clics sur des
   `<a href>` réels renvoyés `ignored` (splash canada.ca, zone
   gov.uk) alors que le même geste navigue sur usa.gov.
8. **Champs de recherche absents à la peinture** (canada.ca,
   usa.gov hero) alors que le `<input type="search">` est dans le
   DOM — masquage CSS non résolu ou input non peint.
9. Point positif à verrouiller : **skip-links correctement
   invisibles** au chargement sur les 4 sites (DOM présent,
   non peint) — l'accessibilité statique de base tient.

## Annexes

- Aucun code modifié. Onglets fermés, daemon port 9400 arrêté
  (PID Windows 15292 — le PID bash 7169 était le wrapper).
- Les deux autres daemons `explorer-daemon.exe` actifs
  (PID 7396, 17744, autres vols) n'ont pas été touchés.
- Journal daemon : `C:/OnionBit-Explorer/_vol16_daemon.log`.

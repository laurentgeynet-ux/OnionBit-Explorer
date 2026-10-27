<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol AGENT7 — reddit (approfondi)

- Date : 2026-10-26
- Binaire : `C:/obe-vol-head/target/release/explorer-daemon.exe` (release, head propre)
- Port : 9391, viewport 1366×900 (`EXPLORER_VIEWPORT`)
- Captures : `docs/Rapport vol/captures/vol7_*.png` ; dumps : `logs/vol7_*_{dom,layout}.txt`, `logs/vol7_*_console.json`
- **Erreurs console : AUCUNE sur les 4 cibles** (`/console` → `pas de journal` à chaque fois). Les défauts constatés ne remontent donc pas en erreurs JS — soit les scripts n'ont jamais tourné, soit ils échouent silencieusement.

## Cible 1 — https://www.reddit.com (home)

- URL finale : `https://www.reddit.com/?solution=51874e6353c51ee251874e6353c51ee2&js_challenge=1&jsc_token=2824be10929bdc604753c70a67a1c331551abfe8e1d73acb56ed195701c67df2&jsc_orig_r=`
  → **Reddit sert un défi JS anti-bot** (paramètres `solution`/`js_challenge`/`jsc_token`). Le contenu SSR est néanmoins livré et peint.
- Titre : « Reddit - The heart of the internet », `doc_h` = 942 (viewport 900 → à peine un écran ; le feed réel fait des dizaines de milliers de px).
- Console : `pas de journal`.
- Capture `vol7_www_reddit.png` :
  - **Page NON blanche** : feed SSR rendu (titres de posts « Man compares work… », « Odyssée 100 % IA… », sidebar « Popular Communities » avec r/AskReddit 59 983 818 members etc., footer légal).
  - `<#comment>` peint comme texte en haut à gauche (y=66) et dans la colonne login gauche (y=713, 931).
  - **Menu pays/« Everywhere » rendu déroulé et superposé** au-dessus du feed : liste Argentina→Iceland empilée verticalement sur toute la colonne centrale, recouvrant les posts. C'est un sous-menu `faceplate-*`/`hidden`-jusqu'à-définition qui reste visible.
  - Icônes de la barre nav (gauche) réduites à des **pastilles noires écrasées** — SVG d'icônes non dessinés (viewBox/aspect ?).
  - Bloc login gauche « Join the most real place on the internet » + boutons « Continue with Phone Number / Email » rendus (texte superposé sur les boutons, overflow visible).

## Cible 2 — https://www.reddit.com/r/rust/

- URL finale inchangée, titre « Reddit - The heart of the internet », `doc_h` = 1100 (contenu très court : ~3 posts visibles alors que le DOM compte **12 `<shreddit-post>`** — `grep -c "<shreddit-post" vol7_rust_dom.txt` = 12).
- Console : `pas de journal`.
- Capture `vol7_rust.png` : en-tête r/rust (avatar + titre), « Best / Open sort options / Change post / Card » **rendu déroulé** (menu de tri ouvert au lieu de fermé), titres « How to fund Rust contributors », « PSA: If your program does HTTP requests… », « Ochre: a deterministic voxel world… », sidebar Community Bookmarks (Megathreads/Official Resources/Learn Rust/Discussion Platforms), image du post Ochre peinte correctement, règles du sub visibles.
- Scroll : `scroll_y` bloque à **200.0** après 3×800px — cohérent avec doc_h=1100, mais révèle que **~9 posts du DOM ne produisent pas de hauteur** (leurs sous-arbres à contenu shadow/slots restent 0×0, cf. layout).
- Le clic sur le titre « How to fund Rust contributors » (760,122) **navigue correctement** → `/r/rust/comments/1xfu4nt/how_to_fund_rust_contributors/` (`{"result":"navigated"}`).

## Cible 3 — https://old.reddit.com/r/programming/ (bissection « page blanche »)

### Constat

- **URL finale : `https://old.reddit.com/login/?reason=lor2&dest=…`** — Reddit **redirige tout old.reddit logged-out vers la page de login shreddit** (`reason=lor2`). Même résultat sur `https://old.reddit.com/` : la « vieille » UI n'est plus servie du tout. La page blanche n'est donc **pas** un bug de rendu du vieux HTML — c'est la page de **login shreddit** qui rend blanche.
- doc_h = 941, titre « Welcome to Reddit ». Capture `vol7_oldreddit.png` : fond gris clair plein écran, seul l'en-tête (logo reddit écrasé, 616×57 d'après layout ligne 60) et deux `<#comment>` textuels (y=66 et y=933).

### Diagnostic de la page blanche (preuves dans les dumps)

Le DOM (`vol7_oldreddit_dom.txt`, 1234 lignes) est complet et **bien formé** : `<auth-flow-manager step-name="login">` (l.789) contient des `<span slot="login|register|phone_auth|magic_link_*">`, chacun abritant un couple `<faceplate-loader>` + `<faceplate-partial loading="programmatic" src="/svc/shreddit/partial/<hash>/login-step?query=…">`.

- **Tous les `faceplate-partial`/`faceplate-loader` sont VIDES** (0 enfant, 28 occurrences de `loading="programmatic|eager"`). Le formulaire de login n'est **pas** dans le HTML : il arrive par `fetch('/svc/shreddit/partial/…')` déclenché côté client quand le custom element est *défini* et connecté.
- Layout (`vol7_oldreddit_layout.txt`) : tous ces nœuds sont `inline<faceplate-*> [0 0 0x0]` (l.52-186) — vides, donc 0×0. **Pas un `display:none` résiduel** : les boîtes sont absentes parce que les sous-arbres sont vides, pas masqués par du CSS.
- Les règles `*:not(:defined){display:none}` présentes dans la feuille (`.nd:hidden:not(:defined)`, `faceplate-hovercard:not(:defined)>[slot=content]`…) **ne s'appliquent pas** ici : il n'y a rien à masquer.
- Pas de `<noscript>` de secours pertinent ; le seul contenu « dur » est l'en-tête et le skip-link (`text "Skip to main content"` à y=-891, positionné hors-écran volontairement).

**Conclusion : la page blanche = hydratation JS jamais venue.** Hypothèses ordonnées :

1. Les scripts `type="module"` de boot shreddit (fin du dom, `src="https://www.redditstatic.com/js/concat?…chunks=…"`) ne s'exécutent pas ou échouent sans erreur remontée à `/console` (silence suspect : même un `import()` en échec devrait logger).
2. `customElements` existe (`dom_host.rs:2249`) et `attachShadow` est un **stub documenté** (`dom_host.rs:1776-2039`, « issue #25 ») : même définis, les custom elements ne peuvent pas peupler leur shadow DOM — le modèle shreddit (light DOM + slots + shadow) est structurellement hors de portée aujourd'hui.
3. Même avec les scripts, `fetch` de `/svc/shreddit/partial/*` + swap innerHTML est requis pour le moindre contenu : dépendances profondes NAV/JS.

## Cible 4 — page commentaires (clic depuis r/rust)

- URL : `https://www.reddit.com/r/rust/comments/1xfu4nt/how_to_fund_rust_contributors/`, `doc_h` = 11277 (layout : `block<main> [339 76 756x11277]`, `block<shreddit-comment-tree> [339 602 756x10703]`).
- Console : `pas de journal`.
- **Les commentaires existent et sont layoutés** : 19 `<shreddit-comment>` dans le DOM, boîtes 756×134, 756×10478 etc. dans le layout.
- **Bug de dimensionnement catastrophique** (zoom `vol7_comments_zoom.png` sur région x=320,y=640) : le texte des commentaires wrappe dans une **colonne de 32 px** — chaque mot tronqué à 4-5 caractères (« repr », « Do », « wor », « Trac », « zulip »…). Le document est donc une colonne étroite de ~11 000 px de haut.
- Cause visible dans le layout : le commentaire racine est `756x134`, mais les `<shreddit-comment>` imbriqués collapsent à **`32x6608` / `32x5284` / `32x1828`** (lignes 547, 632, 775, 847). 32 px = exactement la largeur de la colonne avatar (`block<a> 32x32`, `var(--spacer-xl)`). La grille `.comment-main-grid` (`display:grid`, `grid-template-columns: <avatar> minmax(0,1fr)`) n'est pas résolue pour les enfants imbriqués : le contenu est placé dans la **première piste seule** — `grid-column: 1/-1`, `minmax(0,1fr)` ou l'héritage des pistes dans `#comment-children` non supportés.
- `faceplate-loader[name^=CreateComment]{display:none}` est bien appliqué (absent du layout) — le sélecteur d'attribut préfixe fonctionne.

## Constats actionnables

1. **Commentaires HTML peints en texte** — `<#comment>` rasterisé à y=66/713/931 (www) et dans `vol7_rust_layout.txt` lignes 44/1067/1246 (`text "…<#comment>"` avec boîte 86×21). Les nœuds Comment du DOM fuient dans la display list comme runs de texte. → fix PIPELINE (`explorer-render`/`explorer-layout` : exclure les nœuds commentaire du flux inline). Priorité haute : pollue chaque page.
2. **Page blanche old.reddit = hydratation manquante, pas CSS** — tout le contenu est derrière `<faceplate-partial loading="programmatic" src="/svc/shreddit/partial/…">` vides (28 occurrences, toutes 0×0). → prérequis NAV/JS : fetch + insertion HTML déclenchés par custom element ; shadow DOM (`attachShadow` stub issue #25). Sans cela, aucune page login shreddit ne s'affichera. Vérifier aussi pourquoi `/console` reste vide — les erreurs de module/fetch ne sont pas remontées.
3. **old.reddit.com est mort côté serveur** — redirect `login/?reason=lor2` sur toute URL. Retirer old.reddit des cibles de vol : plus d'UI « old » à rendre. (Mise à jour du constat du vol précédent : ce n'était pas « shreddit shell 0 erreur » sur old UI, c'était cette page login.)
4. **Grid `minmax(0,1fr)` / `grid-column: 1/-1` non résolu pour les items imbriqués** — commentaires reddit à 32 px de large (`block<shreddit-comment> 32x6608`). Le texte wrappe à la piste avatar. → fix PIPELINE `explorer-layout` (grid : placement explicite des items + pistes `minmax`). Effet spectaculaire : toute l'arborescence de commentaires est illisible.
5. **Sous-menus `faceplate-*` rendus déroulés par défaut** — menu pays « Everywhere/Argentina/…/Iceland » superposé au feed (www), menu de tri « Best/Card » ouvert (r/rust). Dans un navigateur réel ces contenus sont dans un popover/slot masqué jusqu'à interaction. → vérifier support `popover`/`hidden` sur les containers dropdown, ou `:not(:defined)` qui devrait les masquer mais ne matche pas.
6. **SVG d'icônes non peints** — barre nav = pastilles noires écrasées (www, r/rust, login). Les `<svg icon-name="…">` ont des boîtes 10.7×32/16×20 dans le layout mais peignent noir/ovale → probablement `<use href>` ou paths non résolus. → fix PIPELINE `explorer-render` (svg `<use>`/symbols).
7. **`doc_h` sous-estimé** — home 942 px pour un feed infini : ~9/12 posts SSR produisent zéro hauteur (contenu shadow/slots). Conséquence : scroll bloqué à 200 px. Pas un bug de scroll — un symptôme de (2)/(5).
8. **Console toujours vide** sur un site qui échoue massivement — la remontée d'erreurs JS/net doit être auditée : soit explorer-jscore n'exécute pas les modules `concat?chunks=`, soit les erreurs sont avalées sans log (le `data-src` des scripts tiers `googleid-signin-script`/`turnstile-script` suggère un loader JS qu'on n'exécute pas).

## Arrêt

- Onglets 1, 2, 3 ouverts sur le daemon agent :9391 (fermés en fin de vol), daemon tué par son process dédié.

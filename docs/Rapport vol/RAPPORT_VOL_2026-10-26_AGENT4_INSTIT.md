# Rapport de vol — Agent 4 (institutionnel / specs / docs)

- **Date** : 2026-10-26
- **Build** : `C:/obe-vol-head/target/release/explorer-daemon.exe` (HEAD 7a01534), viewport 1366×900, port 9388
- **Méthode** : `/api/tabs` + `navigate`, attente 7-10 s, `console` / `frame` (PNG) / `dom` / `layout`, puis scroll ×2 et clic.
- **Incident infra** : le daemon n°1 (PID 15624) a **crashé silencieusement** (log 0 octet, socket mort) pendant la transition datagouv → ecma (DELETE tab 4 + POST /tabs + navigate ecma). Redémarré (PID 8420), le même chargement d'ecma a réussi. Crash non reproduit — à garder en tête (possible race à la destruction/création d'onglet ou ressource résiduelle du site précédent).

---

## 1. https://www.w3.org/TR/

- **URL finale** : `https://www.w3.org/TR/` (pas de redirection). Titre OK. `doc_h = 320 166 px` (clampé).
- **Console** : vide (« pas de journal ») — **0 erreur JS/CSS remontée**.
- **Visuel** (`captures/vol4_w3tr.png`, `vol4_w3tr_scroll.png`) :
  - Un nœud texte parasite **`<#comment>`** est peint en haut à gauche de la page — confirmé dans le layout : `text "<#comment>" [0 1.78 92.05x24]` (vol4_w3tr_layout.txt:4). Le DOM montre **deux** nœuds par commentaire : un texte `"<#comment>"` **et** le vrai `<!-- #comment -->` (vol4_w3tr_dom.txt:50-51).
  - Le logo W3C SVG (texte circulaire « WORLD WIDE WEB CONSORTIUM ») occupe ~500×500 px au lieu d'un petit badge → dimensionnement SVG non contraint (pas de width/height intrinsèque appliqué ou viewBox ignoré).
  - Les ~8 liens du menu principal sont posés **sur une seule ligne horizontale qui se chevauchent** (« About W3C / web standards / standards & drafts… » avec collisions de glyphs) → probablement flex/inline-block ou `float` mal résolu.
  - Après scroll (capture scroll) : le corps de page est correct — champ de recherche, bouton « Show results », liste des 1243 rapports, dates/liens. Les définitions (`dl` Tags:/Deliverers:) affichent un retrait bizarre du `dd` à droite.
  - Listes sans puces visibles partout (nav + sidebar) → `list-style` ou `::marker` non rendu.
- **Interaction** : scroll ×2 OK (scroll_y 1600). Clic (600,400) → `ignored` (zone sans lien).
- **Classification** : DOM/tree-builder (texte `<#comment>` émis en double), CSS/layout (SVG sans taille, chevauchement nav, puces), sévérité **moyenne** — contenu lisible, page utilisable.

## 2. https://www.rfc-editor.org/rfc/rfc9110

- **URL finale** : `https://www.rfc-editor.org/info/rfc9110/` — **redirection opérée par le site** (la page RFC redirige vers la fiche info). Puis clic → `#section-7.7` : navigation par ancre fonctionnelle. `doc_h = 224 215 px` (clampé).
- **Console** (vol4_rfc_console.json) — 10 entrées :
  1. `exception js : moteur figé — éval dépassée — module «https://www.rfc-editor.org/_nuxt/1.144.1/FnlBmrV-.js»` (×1)
  2. `moteur js inerte` (×9) — le moteur JS se fige définitivement sur un module Nuxt, puis refuse tout le reste.
- **Visuel** (`vol4_rfc9110.png`, `vol4_rfc9110_scroll.png`) : **excellent rendu** malgré le JS mort — header sombre complet, fil d'Ariane, badge « Internet Standard », étoiles de notation, boutons Subscribe/Add to Set, résumé, et **layout deux colonnes correct** (texte à gauche, TOC détaillée à droite, liens bleus).
  - Fil d'Ariane : « Home♥RFC 9110 » — un caractère/icône séparateur s'affiche comme ♥ (icône font / glyphe manquant ou mauvais fallback).
  - Icônes loupe/compte du header minuscules (~10 px) → dimensionnement SVG.
  - « The RFC Series / For / Authors » : retour à la ligne non désiré dans le menu (espace insuffisant ou nowrap manquant).
- **Interaction** : scroll ×2 OK ; clic sur un item de TOC → navigation `#section-7.7` réussie.
- **Classification** : **JS** (gel du moteur sur module Nuxt → site en mode dégradé, mais SSR suffisant), CSS mineur (fonts icônes). Sévérité **faible** pour ce site, mais le « moteur figé » est un pattern dangereux.

## 3. https://www.data.gouv.fr

- **URL finale** : `https://www.data.gouv.fr/`. Titre OK. `doc_h = 4 541 px` (non clampé).
- **Console** : vide — **0 erreur remontée**.
- **Visuel** (`vol4_datagouv.png` = page complète, `vol4_datagouv_scroll.png`) : rendu globalement fidèle — en-tête RF + logo, barre de recherche, cartes de jeux de données avec vignettes, section sombre « La mission data.gouv.fr », grille des portails thématiques, footer complet.
  - Chevauchement en haut à gauche : « 4,6 » superposé à « En ce moment / Données relatives à l'éducation » (bandeau d'actu mal positionné).
  - Textes tronqués dans les cartes footer-thématiques : « météo.data.gouv.fr » coupé à « méte… », « simplifions.data.gouv.fr » coupé → overflow/ellipsis partiel ou largeur de carte sous-évaluée.
  - Boutons « En savoir plus » qui débordent de leur carte.
  - Icônes sociales du footer rendues en petits glyphs génériques.
- **Interaction** : scroll ×2 OK (1600) ; clic (650,250) → `ignored` (probablement zone non-lien ; non concluant).
- **Classification** : CSS/layout (chevauchements mineurs, overflow). Sévérité **faible** — le meilleur des 4 vols.

## 4. https://ecma-international.org

- **URL finale** : `https://ecma-international.org/`. `doc_h = 3 437 px`.
- **Console** (vol4_ecma_console.json) — 3 erreurs distinctes :
  1. `syntaxe js 1:13365 : invalid regex /^[-]?[0-9]+(?:[eE][+-]?[0-9]+)?$/: Parsing error at position 5: Target of repeat operator is invalid` — plugin contact-form-7 `swv/js/index.js`. **Bug du parseur de regex** : `[-]` en tête de classe de caractères rejeté.
  2. `exception js : ReferenceError: recaptcha is not defined — iframe dynamique` (×4+).
- **Visuel** (`vol4_ecma.png`, `vol4_ecma_scroll.png`) : **cassé**.
  - **Des dizaines de `<#comment>` peints** en haut de page (bandeau de ~9 occurrences) et dans le corps — le layout montre `text "\n<#comment>"` répété (vol4_ecma_layout.txt:4-88). Même bug qu'en #1 mais massif : chaque commentaire HTML engendre un nœud texte rendu.
  - Bande orange pleine largeur de ~30 px en haut + grande zone vide de ~350 px sous le hero → un slider/carrousel (Enfold/Avia) dont les images ne chargent pas ou dont les panneaux ont une hauteur non calculée.
  - Menu « Publications and standards » déplié en **liste verticale à puces toujours visible** (sous-menu déroulant non masqué — `display:none`/hover CSS non appliqué).
  - Texte hero avec collisions : « celebrating 65 Years… and continue » superposé aux lignes voisines.
  - Footer : grand bloc orange (colophon) correct mais précédé de `<#comment>` ; en dessous, **une immense zone noire** (~1500 px) — probablement un overlay/modal ou une section à fond noir rendue en pleine page ; lignes « Could not connect to the reCAPTCHA service » répétées.
  - Cookie settings panel visible en plein flux.
- **Interaction** : scroll 800 OK, 2e scroll `ignored` (fin atteinte), clic (420,40) `ignored`.
- **Classification** : DOM/tree-builder (commentaires→texte), CSS (menus, slider, overlay), JS (regex + recaptcha). Sévérité **élevée** — page à peine utilisable.

---

## Constats actionnables

1. **Nœuds texte parasites `<#comment>` rendus** — le bug le plus visible du vol (2 sites/4). Le DOM montre pour chaque commentaire **à la fois** `<!-- #comment -->` **et** un texte `"<#comment>"` (parfois fusionné au whitespace : `"\n\t<#comment>"`). Hypothèse : le tree-builder HTML émet un nœud texte pour le marqueur des *bogus comments* / commentaires conditionnels, ou sérialise le nom des nœuds comment dans le flux texte. À investiguer dans `explorer-html` (tree-builder, états comment/BOGUS COMMENT du tokenizer) — repro minimal : `<html><!-- x -->` puis vérifier qu'aucun texte `#comment` n'apparaît dans le layout.
2. **Regex `[-]` en début de classe de caractères rejetée** — `explorer-jscore` : `/^[-]?[0-9]+…$/` échoue avec « Target of repeat operator is invalid » à la position du `-`. Le `-` en première position d'une `[charclass]` doit être littéral. Cas de conformité test262 probable. (Source : contact-form-7 swv/index.js.)
3. **Gel du moteur JS (« moteur figé — éval dépassée ») sur module Nuxt** — `rfc-editor.org` : un module JS dépasse le budget d'évaluation → le moteur passe « inerte » pour **tout** le reste de la page (×9). À vérifier : (a) le budget est-il trop court pour un bundle Nuxt (~réaliste) ? (b) le gel devrait-il être par-script plutôt que par-onglet ? Le rendu SSR a sauvé le site, mais toute interactivité est morte.
4. **SVG sans dimension intrinsèque appliquée** — logo W3C peint ~500 px au lieu d'un badge ; icônes header rfc-editor minuscules. Vérifier la résolution `width`/`height`/`viewBox` et la taille par défaut des `<svg>` dans `explorer-layout`/render.
5. **Sous-menus déroulants non masqués** — ecma (liste verticale « Standards / Technical Reports… » peinte en permanence) : `display:none` des sous-menus, sélecteurs `:hover`/`:focus-within`, ou `visibility` non honorés. Le même mécanisme explique probablement le chevauchement des liens du nav W3C sur une ligne.
6. **Glyphes de font-icon fallback** — fil d'Ariane rfc-editor affiche « ♥ » (séparateur) et icônes génériques : les fonts d'icônes (woff2 custom) semblent non chargées/non mappées → vérifier `@font-face` + cmap des fonts téléchargées, et le fallback des codepoints PUA.
7. **Overlay/modal peint en plein flux** — ecma : panneau « Cookie and privacy settings » + zone noire géante + messages reCAPTCHA visibles : éléments `position:fixed`/hidden non exclus du layout ou `display:none` JS non exécuté (conséquence possible du JS partiellement mort — recaptcha ReferenceError).
8. **Crash daemon non reproduit** — mort silencieuse (log vide) entre DELETE d'un onglet data.gouv.fr et navigation ecma. Suggestion agent FIX : reproduire la séquence navigate(datagouv) → DELETE → POST /tabs → navigate(ecma) sous le binaire release avec `RUST_BACKTRACE` ; un crash sans log évoque un abort (panic dans un `catch` manquant ou plantage hors runtime tokio).
9. **`doc_h` clampé à ~224k-320k px** — w3.org/TR atteint 320 166 px (`doc_h_clamped:true`) : vérifier que le clamp ne tronque pas la navigation par scroll au-delà (les `<#comment>` en ligne haute laissent penser que la hauteur est réelle, pas un bug — mais le seuil mérite d'être documenté/configuré).
10. **Timeouts `frame` intermittents** — deux `GET /frame` ont retourné un fichier vide (0 octet) sur ecma alors que l'endpoint répond 200 ; le retry a rendu un PNG 9,3 Mo. Possible dépassement du temps de raster sur grandes pages — prévoir retry côté client ou agrandir le timeout interne.

## Artefacts

- Captures : `captures/vol4_{w3tr,w3tr_scroll,rfc9110,rfc9110_scroll,datagouv,datagouv_scroll,ecma,ecma_scroll}.png`
- Logs : `logs/vol4_{site}_{console,dom,layout}.*`
- Daemon : `_vol4_daemon.log` (crash, vide), `_vol4_daemon2.log`

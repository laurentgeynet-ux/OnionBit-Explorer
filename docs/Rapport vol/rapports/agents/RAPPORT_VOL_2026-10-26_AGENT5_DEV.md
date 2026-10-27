<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 5 (périmètre : dev / forums / outils)

- **Date** : 2026-10-26
- **Daemon** : `C:/obe-vol-head/target/release/explorer-daemon.exe` (release, HEAD propre 7a01534), port 9389, `EXPLORER_VIEWPORT=1366x900`.
- **Protocole** : `docs/vol-agent.md`.
- **Captures** : `docs/Rapport vol/captures/vol5_*.png`
- **Télémétrie** : `docs/Rapport vol/logs/vol5_*_{console.json,dom.txt,layout.txt}`

> Incident de session : le premier daemon (PID 3147) a été tué
> quand son shell de fond a expiré — redémarré (PID 3822), vol
> repris sans perte de mesures. **Pas** un crash du moteur.

---

## 1. https://news.ycombinator.com — « Hacker News »

- **URL finale** : `https://news.ycombinator.com/` (HTTP 200, `doc_h=1078.5`, non clampée).
- **Erreurs console** : **0** (« pas de journal »), y compris après scroll ×2 + clic.
- **Constats visuels mesurés** (`captures/vol5_yc.png`, 1366×1078) :
  - Rendu **quasi parfait** : barre d'en-tête orange pleine largeur,
    logo « Y » encadré, liste des 30 articles numérotée 1–30, lignes
    de métadonnées (points/auteur/âge/commentaires) correctes,
    pied de page (YC W2027, liens Guidelines…Contact, champ Search)
    présent.
  - Layout table `1147.5px` centré dans `1350px` (marges ~109px) —
    conforme au rendu attendu.
- **Interactions** : scroll ×2 OK (`scroll_y` 1600). Clic en
  (400,300) → navigation réelle vers la page de commentaires de
  l'article (`{"result":"navigated"}`), rendue intégralement
  (`captures/vol5_yc_click.png`, document complet ~22 MB PNG,
  thread + liens « reply » imbriqués corrects, `add comment`
  visible en tête — le clic avait touché le lien « comments »).
- **Classification** : aucun défaut mesuré.
- **Sévérité** : néant — **référence de bon fonctionnement**.

## 2. https://sr.ht — « sourcehut hub »

- **URL finale** : `https://sr.ht/` (HTTP 200, `doc_h=1575.6`).
- **Erreurs console** : **0**.
- **Constats visuels mesurés** (`captures/vol5_srht.png`,
  1366×900 visible) :
  - Le bouton **« Register here »** est rendu en bloc bleu de
    **~124×539 px** (layout : `inline<a> [128 96 123.6x538.8]`)
    au lieu d'un bouton inline ~34px de haut.
  - Deux **glyphes noirs géants** en forme de double-caret
    (~250×250 px chacun) peints par-dessus la page : ce sont les
    `<svg viewBox="0 0 192 512">` (caret-right) posés à leur
    **taille viewBox en px** : `block<svg> [187.8 120.2 192x512]`,
    `block<svg> [327.2 122.8 192x512]` dans
    `logs/vol5_srht_layout.txt`.
  - Conséquence : le `<p>` parent est étiré à 562.8px de haut et
    repousse tout le contenu (« or Log in to get started… » rendu
    à y≈620).
  - Le bouton **« More projects »** (`btn btn-block`) forme un
    panneau gris ~350×460px en bas à droite avec le même caret
    géant.
  - Le reste (titre « Featured projects », grille 2 colonnes de
    projets avec descriptions et hashtags) est correctement rendu.
- **Interactions** : scroll ×2 OK ; clic (700,570) → `ignored`
  (zone blanche, plausible).
- **Classification** : **layout / dimensionnement des SVG
  remplacés** (taille intrinsèque prise = viewBox brut, taille CSS
  `.icon` non appliquée ou absente du parcours).
- **Sévérité** : **majeure** — corruption visuelle de tout site
  utilisant des icônes SVG inline sans `width`/`height`.

## 3. https://stackoverflow.com/questions

- **URL finale** : idem, **HTTP 403** (refus côté serveur, bot
  protection).
- **Erreurs console** : 1 — `HTTP 403`.
- **Constats** (`captures/vol5_so.png`) : la **page d'erreur
  interne** du navigateur s'affiche correctement et en français
  (« Page inaccessible… Le serveur a répondu « HTTP 403 » ») —
  dégradation gracieuse conforme à la mission.
- **DOM/layout** : « pas de DOM » / « pas de layout » (document
  d'erreur interne).
- **Classification** : **net** (403 externe, pas un défaut du
  moteur) ; le chemin d'erreur fonctionne.
- **Sévérité** : info.

## 4. https://framagit.org/explore (GitLab)

- **URL finale** : `https://framagit.org/explore` (HTTP 200,
  `doc_h=1006.4`).
- **Erreurs console** : **1** — erreur de **syntaxe JS** fatale au
  bundle principal :
  ```
  syntaxe js 25:232950 : invalid regex /[#*0-9]\uFE0F?\u20E3|[\xA9\xAE\u203C\u2049\u2122\u2139\u2194-\u2199…]/:
  Parsing error at position 1253: Invalid codepoint for hex or unicode escape
  — https://framagit.org/assets/webpack/main.02b8bf31.chunk.js
  ```
  (regex emoji Unicode complète dans `logs/vol5_framagit_console.json`).
- **Constats visuels mesurés** (`captures/vol5_framagit.png`) :
  - Page **quasi vide** : fond gris-lavande, titre « Explore
    projects » à y≈855, aucune liste de projets, aucun header
    GitLab — l'app Vue ne s'amorce jamais (bundle mort à la
    syntaxe).
  - Deux **textes « <#comment> » peints à l'écran** (y≈8 et y≈455).
    Layout : `text "<#comment>" [0 0 86.2x21]` ;
    `logs/vol5_framagit_dom.txt` lignes 436-437 et 465-466 : le
    DOM contient un nœud **texte `"<#comment>"` suivi d'un nœud
    commentaire `<!-- #comment -->`** — le tree-builder duplique
    les commentaires en nœuds texte visibles.
- **Interactions** : scroll ×2 + clic → `ignored`, rien ne change
  (page morte, cohérent avec le JS tué).
- **Classification** :
  - **JS** — parseur de regex : échappements `\uXXXX`/`\xXX` rejetés
    (« Invalid codepoint for hex or unicode escape », pos. 1253)
    sur une regex emoji pourtant valide ES — **bloquant total SPA**.
  - **DOM/HTML** — commentaires matérialisés en texte visible.
- **Sévérité** : **critique** (JS) / moyenne (commentaires).

## 5. https://slashdot.org (fallback, 5e site)

- **URL finale** : `https://slashdot.org/` (HTTP 200,
  `doc_h=10343.1`, page complète rendue).
- **Erreurs console** : **3** (puis 4 après interactions) —
  verbatim :
  ```
  1. exception js : TypeError: appendChild: not a node
      at <anonymous> (https://a.fsdn.com/sd/all-minified.js?0ab6e7235c60a5c8:125:70448)
  2. exception js : TypeError: cannot read `appendChild` of Undefined
      at c (1:13)
  3. exception js : TypeError: appendChild: not a node
      at <anonymous> (https://a.fsdn.com/sd/all-minified.js?0ab6e7235c60a5c8:125:70448)
  4. exception js : TypeError: `[object globalThis]` is not a function (on Object)
      at _each_op (https://a.fsdn.com/sd/all-minified.js?0ab6e7235c60a5c8:…)
  ```
- **Constats visuels mesurés** (`captures/vol5_slashdot.png`,
  capture pleine page ~1366×10343) :
  - Structure **correcte** : ~20 articles avec en-têtes teal,
    auteur/date, résumés, colonne latérale droite (boîtes
    grises/orange) — la page est exploitable malgré le JS mort.
  - **Tâche noire géante** ~400×560px en haut à droite : ce sont
    les icônes sociales SVG inline sans `width`/`height` posées à
    taille viewBox : `inline<svg> [834.7 139 1792x1260]` (YouTube)
    et `inline<svg> [858 504 448x512]` (Mastodon) dans
    `logs/vol5_slashdot_layout.txt`. Le SVG Bluesky, lui, a
    `width="20" height="20"` → rendu **20×20 correct**
    (`inline<svg> [881.4 750.5 20x20]`) — confirmation que le bug
    est bien l'absence de dims.
  - Le `<i>` parent est étiré à `1792x1280`, poussant les `<span
    class="sr-only">` hors écran (y≈1398) — même mécanisme que
    sr.ht.
  - Ici aussi un nœud texte `"…<#comment>"` précède le commentaire
    `<!-- #comment -->` dans le DOM (`vol5_slashdot_dom.txt`
    ~ligne 742) — même anomalie que framagit.
- **Interactions** : scroll ×2 OK (`scroll_y` 1600), clic →
  `ignored`; 1 erreur JS supplémentaire en console.
- **Classification** : **layout/SVG** (identique à sr.ht),
  **JS/DOM** (`appendChild` sur non-nœud / `undefined` — API DOM
  incomplète côté `explorer-jscore`), **DOM** (commentaire → texte).
- **Sévérité** : majeure (visuel) / moyenne (JS non fatal).

---

## Constats actionnables

1. **[Layout/SVG — priorité haute]** Un `<svg>` **sans
   `width`/`height`** est dimensionné à sa taille viewBox en px
   (sr.ht : `192×512` ; slashdot : `1792×1260`, `448×512`) au lieu
   de la taille par défaut de remplacé (300×150, ou contrainte CSS
   du parent `.icon`/`font-size`). Cassé sur **2 sites/4** avec
   corruption visuelle massive. Repro minimal : `<i><svg
   viewBox="0 0 192 512">…` vs `<svg width="20" height="20">` (ce
   dernier OK). Vérifier le dimensionnement intrinsèque des
   éléments remplacés SVG dans `explorer-layout`.
2. **[JS — priorité haute]** `explorer-jscore` rejette une regex
   emoji valide : `Parsing error at position 1253: Invalid
   codepoint for hex or unicode escape` sur
   `/…[\xA9\xAE\u203C…]\uFE0F?…/` (framagit, webpack main chunk) —
   le parseur de regex ne gère pas les échappements `\xNN` /
   `\uXXXX` dans les classes de caractères (ou les surrogates dans
   `\uD83C`). **Effet : bundle entier mort → SPA vide.** Couvre
   potentiellement tous les sites emoji-regex/GitLab-like.
3. **[DOM/HTML — moyenne]** Chaque nœud commentaire du DOM est
   accompagné d'un **nœud texte `"<#comment>"` rendu à l'écran**
   (framagit ×2, slashdot ×1 — vérifié dans les dumps
   `*_dom.txt` et peint dans `*_layout.txt`). Le tree-builder
   émet probablement un texte parasite à côté du commentaire.
4. **[JS/DOM — moyenne]** `appendChild` lève
   `TypeError: appendChild: not a node` et
   `cannot read 'appendChild' of Undefined` sur
   `all-minified.js` de slashdot — vérifier la conversion
   nœud/argument et le chaînage `parentNode.insertBefore` /
   `appendChild` dans le mini-DOM de `explorer-jscore` ; aussi
   `` `[object globalThis]` is not a function `` (méthode globale
   attendue absente ou `this` mal lié).
5. **[Net — info]** `stackoverflow.com` répond **HTTP 403** (bot
   protection) — la page d'erreur interne s'affiche proprement en
   français : dégradation conforme. Rien à fixer côté moteur ;
   envisager un UA/en-têtes plus réalistes si on veut mesurer ces
   sites.
6. **[Positif]** `news.ycombinator.com` : rendu complet, 0 erreur
   console, navigation par clic fonctionnelle — bon étalonnage du
   pipeline table-layout.

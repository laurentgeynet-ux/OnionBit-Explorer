<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 14 « Outils / SPA » — 2026-10-26

- **Binaire** : `C:/obe-vol-head/target/release/explorer-daemon.exe` (release),
  daemon dédié port **9398**, viewport `EXPLORER_VIEWPORT=1366x900`.
- **Protocole** : `docs/vol-agent.md` — `/console`, `/frame`, `/dom`, `/layout`,
  `/input` (scroll ×2, clic).
- **Captures** : `docs/Rapport vol/captures/vol14_*.png`
- **Télémétrie** : `docs/Rapport vol/logs/vol14_*_{dom,layout}.txt`

## Synthèse

| Cible | Verdict | Sévérité |
|---|---|---|
| MDN `/docs/Web/CSS` | **fonctionnel** — page lisible, nav interne OK | mineure (`<#comment>` rendus, en-tête chevauché) |
| CodePen `/pen` | **coquille non hydratée** — 3 panneaux vides | majeure (JS boot meurt en chaîne) |
| app.netlify.com | **page blanche** — SPA morte au parse | critique (chunk webpack non parsé) |
| JSFiddle | **coquille partielle** — chrome visible, éditeurs absents | majeure (`appendChild` en boucle) |
| observablehq.com (fallback) | **HTTP 429** — échec réseau, non diagnostiquable | n/a |

---

## 1. MDN — https://developer.mozilla.org/en-US/docs/Web/CSS

- URL finale : `https://developer.mozilla.org/en-US/docs/Web/CSS` puis
  navigation interne `…/CSS_grid_layout` → redirigée `…/CSS/Guides/Grid_layout`
  (`can_back:true`, `doc_h:7340.7`, titre correct).
- Console : **« pas de journal »** — zéro erreur sur les deux pages.
- Frame `vol14_mdn.png` : article complet et lisible (titres, listes, liens,
  encadré « Help improve MDN », pied de page). Deux défauts visibles :
  1. **les nœuds commentaires HTML sont peints comme du texte** :
     littéraux `<#comment>` devant « Beginner's tutorials », « Guides »,
     « Tools », « Reference », « See also » et dans le pied de page ;
  2. l'en-tête/navigation (logo + barre supérieure) se **chevauche** sur
     ~3 lignes en haut de page (liens superposés illisibles).
- Interactions : scroll ×2 → `scroll_y:800→1600` OK (`vol14_mdn_scroll.png`) ;
  navigation lien interne → nouvelle page rendue (`vol14_mdn_grid.png`,
  18 Mpx, contenu présent).
- **Classification** : rendu documentaire fonctionnel — pas de régression
  bloquante ; le leak `<#comment>` est un bug de peinture DOM→display list.
- **Sévérité** : mineure.

## 2. CodePen — https://codepen.io/pen

- URL finale : `https://codepen.io/pen` (titre « CodePen - Create a New Pen »).
- Frame `vol14_codepen.png` : fond sombre du thème twilight, **trois
  panneaux éditeur dessinés** (icônes engrenage + chevrons en haut de chaque
  panneau) mais **contenus vides** — pas de CodeMirror, pas de texte.
- Console (verbatim, ordre chronologique) :
  1. `exception js : TypeError: \`toLowerCase\` is not a function (on Object)`
     — `everypage-b3f204f3….js:3:2170` (première erreur fatale de boot) ;
  2. `exception js : TypeError: cannot define property \`tree\``
     — `vendor-20556d05….js:1:1174435` (defineProperty dans le bundle
     GraphQL/AST) ;
  3. `exception js : ReferenceError: jQuery is not defined`
     — `commonLibs-18d595d9….js:1:2` (conséquence : jQuery jamais initialisé) ;
  4. `syntaxe js 12:8426 : invalid regex /[!\"#$%&'()*+,\-.\/:;…]/:
     Parsing error at position 1776: Invalid codepoint for hex or unicode
     escape` — `codemirror-e628accc….js` (regex Unicode de ponctuation,
     échappement `\uXXXX` sur plage surrogate) → **CodeMirror jamais parsé** ;
  5. `exception js : ReferenceError: $ is not defined` — `pen/index-…js` ;
  6. `TypeError: cannot read \`appendChild\` of Undefined  at c (1:13)`.
- DOM (`vol14_codepen_dom.txt`, 5 028 lignes) : le squelette **existe côté
  serveur** — `class="room-editor editor state-htmlOn-cssOn-jsOn twilight
  preload prelayout"`, `<textarea>` présents, modales settings — mais aucun
  nœud `cm-editor`/`CodeMirror` instancié (0 occurrence de ces classes comme
  éléments actifs). **Coquille construite, jamais hydratée.**
- **Classification** : SPA/éditeur mort en cascade — la première faute est
  un manque d'API (`toLowerCase` sur Object) puis un parse regex Unicode ;
  le rendu HTML statique, lui, est correct.
- **Sévérité** : majeure.

## 3. Netlify — https://app.netlify.com

- URL finale : `https://app.netlify.com/` (titre « Netlify », `doc_h:0.0`).
- Frame `vol14_netlify.png` : **page intégralement blanche**.
- Console (verbatim) :
  1. `syntaxe js 325:1465 : expected \`)\`` —
     `assets/js/30991.bd3f08225ae6212e9de6.js` (chunk webpack, src
     `(self.webpackChunk=self.webpackChunk||[]).push([[30991],{1003(e,t,n){…`)
     — **première faute : un chunk applicatif ne parse pas** ;
  2. `exception js : ReferenceError: recaptcha is not defined — iframe
     dynamique` (conséquence : le boot recaptcha, qui tourne dans un iframe
     injecté, ne trouve pas l'API attendue).
- DOM (`vol14_netlify_dom.txt`, 18 K) : `<body>` contient deux `<div>` vides
  (montures React) — le `<noscript>` « The Netlify dashboard needs
  JavaScript » est présent mais n'est pas affiché (comportement attendu si
  JS « activé ») ; **rien n'a jamais été construit** — squelette absent,
  pas seulement non hydraté.
- Layout : 276 octets — quasi rien à peindre, cohérent avec la page blanche.
- **Classification** : SPA morte au parse (syntaxe ES récente dans un chunk
  webpack — probablement un opérateur non supporté) ; aucune erreur de
  rendu propre, le mur de login n'est même pas atteint.
- **Sévérité** : critique.

## 4. JSFiddle — https://jsfiddle.net

- URL finale : `https://jsfiddle.net/` (titre « JSFiddle - React, Tailwind,
  and code Playground »).
- Frame `vol14_jsfiddle.png` : **chrome de l'éditeur présent** — barre
  d'icônes gauche (panneaux, settings), barre droite (Run, Save, Sign in,
  Go PRO), champ « Untitled fiddle », panneau « Result0px » + mention
  « JSFiddle Console. Turn on/off in Editor settings ». Mais **zones des 4
  éditeurs vides** et nouvelle occurrence des `<#comment>` peints comme
  texte (haut de page et panneau droit).
- Console : **12 occurrences identiques** de
  `exception js : TypeError: cannot read \`appendChild\` of Undefined
  at c (1:13)` — 4 au chargement, puis de nouvelles à chaque input
  (scroll/clic) → un handler JS (probablement l'init CodeMirror ou un
  append sur `document.head`/`body` résolu `Undefined`) se relance et
  replante en boucle.
- DOM (`vol14_jsfiddle_dom.txt`, 355 K) : squelette serveur complet ; les
  conteneurs d'éditeurs existent mais vides → **coquille non hydratée**.
- Interactions : `scroll dy:600` → `scroll_y:0.0` (page non scrollable —
  app plein écran, normal) ; clic `(400,300)` → `result:"ignored"` + 4
  erreurs `appendChild` supplémentaires.
- **Classification** : SPA partiellement vivante (chrome + listeners) mais
  fonctionnalité cœur absente.
- **Sévérité** : majeure.

## 5. ObservableHQ (fallback) — https://observablehq.com

- Réponse navigate : `{"error":{"handled":true,"message":"HTTP 429"}}` ;
  console : `HTTP 429`. Capture `vol14_observable.png` prise (page d'erreur/
  blanche). **Non diagnostiquable** — rate-limit côté serveur, hors moteur.

---

## Constats actionnables

1. **Nœuds commentaires peints comme texte** (`<#comment>` littéral dans le
   rendu) — reproduit sur **deux sites indépendants** (MDN, JSFiddle). Bug
   transverse DOM→display list : les `Comment` nodes semblent émettre un
   run de texte. Repro minimal : tout document avec `<!-- -->` avant un
   titre. **Sévérité élevée** (polue des pages par ailleurs correctes).
2. **Parser regex : échappements `\uXXXX` / plages surrogate refusés** —
   « Invalid codepoint for hex or unicode escape » sur la regex de
   ponctuation Unicode de CodeMirror (CodePen) ; un chunk webpack Netlify
   échoue aussi au parse (`expected )` 325:1465). Deux parseurs JS distincts
   bloquent le boot de deux grandes SPA. Priorité spécialiste JS.
3. **`String.prototype.toLowerCase` manquant sur Object / appel incorrect**
   — `toLowerCase is not a function (on Object)` (CodePen everypage.js) :
   vérifier le binding `toLowerCase` et `Object.defineProperty` sur objets
   gelés (`cannot define property 'tree'` — le même bundle) : probable
   écart de sémantique `defineProperty` sur objet non extensible.
4. **`document.body`/cible `appendChild` résolue `Undefined`** — erreur
   récurrente `cannot read 'appendChild' of Undefined` (CodePen fin de boot,
   JSFiddle ×12 en boucle à chaque input). Hypothèse : API retournant
   `Undefined` là où le DOM devrait donner un élément (`document.body` tôt,
   ou résultat de sélecteur non null-safe). Boucle infinie d'erreurs à
   instrumenter : un seul échec ne devrait pas se relancer par input.
5. **SPA jamais construites vs non hydratées** : Netlify = montures React
   vides (boot tué au parse) ; CodePen/JSFiddle = squelette SSR présent
   mais zéro hydratation (boot tué à l'exécution). Le moteur rend
   fidèlement le HTML statique dans les deux cas — le goulot est
   uniquement JS (parse + API DOM), pas le pipeline de rendu.
6. **En-tête MDN chevauché** (liens superposés sur ~3 lignes) — probable
   positionnement `flex`/`position:sticky` ou `gap` non appliqué dans la
   barre de navigation ; à reproduire sur un cas flex réduit.
7. **Login wall Netlify non atteint** : contrairement à l'hypothèse du
   protocole, ce n'est pas un mur d'auth qui bloque mais la mort du JS —
   documenté pour calibration des attentes « SPA dashboard ».

## Arrêt

Onglets 2-6 fermés (`DELETE /api/tabs/{id}`), daemon PID 5964 tué par son
PID. Aucun code modifié.

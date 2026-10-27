# Rapport de vol — AGENT1 (domaine : apps / JS lourds)

- **Date** : 2026-10-26
- **Daemon** : `explorer-daemon.exe` release (HEAD 7a01534), port 9385, viewport 1366×900
- **Méthode** : API REST `/api/tabs` — navigate → console → frame → dom/layout → scroll ×2 + click → delete
- **Artefacts** : `captures/vol1_*.png`, `logs/vol1_*_console|dom|layout.txt`

---

## 1. https://gitlab.com/explore

- **URL finale** : `https://gitlab.com/explore` (titre « Explore projects · GitLab »)
- **Erreurs console** : 1
  1. `syntaxe js 1:540292 : invalid regex /[#*0-9]\uFE0F?\u20E3|[\xA9\xAE\u203C\u2049\u2122\u2139\u2194-\u2199…` — la regexp emoji Unicode complète du bundle GitLab est rejetée par le parseur de regex (capture verbatim complète dans `logs/vol1_gitlab_console.txt`)
- **Constats visuels mesurés** (`captures/vol1_gitlab.png`) : page quasi blanche. Seuls rendus : un header de 48 px et le titre « Explore projects » en haut à gauche de la zone de contenu. Sidebar (`<aside>` 240 px de large) totalement vide. La liste des projets (contenu JS Vue.js) n'est jamais montée. Layout : `aside [0 48 240x0]` hauteur 0, nombreux `div … x0`.
- **Classification** : **JS** (une seule erreur de syntaxe regex dans le bundle principal → tout le runtime Vue mort → page shell sans contenu).
- **Sévérité** : **critique** — site inutilisable.

## 2. https://codeberg.org

- **URL finale** : `https://codeberg.org/` (titre « Codeberg.org », doc_h = 2868 px)
- **Erreurs console** : 1
  1. `exception js : TypeError: 'createExpression' is not a function (on Object)` at `assets/js/index.js…:12:35200` via `__webpack_require__` — un module du bundle Gitea/Forgejo échoue au chargement.
- **Constats visuels mesurés** (`captures/vol1_codeberg.png`) : rendu quasi complet — hero « Software development, but free! », logo Codeberg, colonnes NON-PROFIT/COMMUNITY/RESPECT, sections membres, footer complet. Défauts :
  - Barre de navigation supérieure : les liens nav sont absents/chevauchés (seuls « Donate », icônes utilisateur et « ‹ » visibles en haut).
  - Zone compteurs : les chiffres « 7461825 0512 1700+ » se **chevauchent mutuellement** et recouvrent le logo Forgejo (trois compteurs empilés sur la même ligne au lieu de trois colonnes).
  - Titre de colonne « e.V. » se superpose à la zone des compteurs.
- **Classification** : **CSS/layout** (chevauchements = probablement float/flex/inline-block des compteurs non respectés) ; trou **JS** mineur (module webpack en échec, sans conséquence visible immédiate — menus déroulants probablement morts).
- **Sévérité** : **moyenne** — page lisible et navigable, défauts localisés.

## 3. https://excalidraw.com

- **URL finale** : `https://excalidraw.com/` (titre « Excalidraw Whiteboard »)
- **Erreurs console** : 1
  1. `exception js : TypeError: 'in' requires an object` at `Dr (assets/index-BboF1hDf.js:56:3506)` — l'opérateur `in` appliqué à un non-objet (probablement `null`/`undefined` retourné par une API navigateur manquante) tue le bootstrap React.
- **Constats visuels mesurés** (`captures/vol1_excalidraw.png`) : **page blanche à 100 %**. Le DOM existe (238 lignes, `header > h1 "Excalidraw"` présent) mais le h1 fait 1×1 px et tout le canvas/app React n'est pas monté.
- **Classification** : **JS/DOM** — exception au bootstrap ; soupçon d'API navigateur manquante (canvas 2D, localStorage ou autre global retournant une valeur inattendue).
- **Sévérité** : **critique** — application inutilisable.

## 4. https://lobste.rs

- **URL finale** : `https://lobste.rs/` puis page domaine après click (navigation interne OK, `captures/vol1_lobsters_after.png`)
- **Erreurs console** : **0** (« pas de journal » — aucune erreur JS/CSS rapportée)
- **Constats visuels mesurés** (`captures/vol1_lobsters.png`) : contenu texte intégral (25 stories, métadonnées, footer « About Tags Filter Moderation Log ») mais **layout vertical cassé** :
  - Chaque story est empilée sur ~5 lignes au lieu du gabarit « score à gauche / titre+tags à droite » : score (ex. « 29 »), boîte upvote, tags (`brennenputh.me`, `java`, `lisp`…) en petites boîtes bordées **qui se chevauchent** verticalement.
  - Header compressé : « Active Recent Comments Search » et « Login » s'affichent mais sans barre colorée de fond attendue sur toute la largeur.
  - Favicon/story icons absents ou en boîtes vides.
- **Classification** : **CSS/layout** — aucune erreur JS ; le site utilise des floats (`<li class="story">` avec voter/checkbox en float) et inline-block pour les tags ; le non-respect des floats produit exactement cet empilement avec chevauchement des tags.
- **Sévérité** : **moyenne/haute** — contenu accessible et navigation fonctionnelle, mais présentation très dégradée.

---

## Constats actionnables

1. **JS — parseur de regex Unicode incomplet** (bloquant GitLab) : la regexp emoji
   `[#*0-9]\uFE0F?\u20E3|[\xA9\xAE…]` (classes de caractères avec ranges `\uXXXX-\uYYYY`, surrogate pairs `\uD83C[…]`) est rejetée en « invalid regex ». Vérifier le support des échappements `\uXXXX` dans les classes `[…]` et des séquences d'alternance longues dans `explorer-jscore`. → `logs/vol1_gitlab_console.txt` ligne 1 (regex complète en verbatim).
2. **JS — `TypeError: 'in' requires an object`** (bloquant Excalidraw) : l'opérateur `in` reçoit une primitive/`undefined`. Investiguer quelle globale/API retourne `undefined` là où le site attend un objet (`navigator`, `localStorage`, `window`-injection). Repro minimal : bundle `index-BboF1hDf.js:56:3506` fonction `Dr`.
3. **JS — `createExpression is not a function`** (Codeberg/webpack) : un export de module n'est pas résolu comme fonction — possible souci de résolution d'exports webpack/destructuring (`const {createExpression} = mod`). Impact : modules UI morts.
4. **CSS/layout — floats non ou mal implémentés** (Lobsters) : les stories `.story` s'empilent verticalement avec tags chevauchés au lieu du layout float-left (voter) + contenu. Cible : support de `float`, clear, et/ou `display:inline-block` sur liens-tag.
5. **CSS/layout — chevauchement de compteurs inline** (Codeberg) : trois compteurs « projects / users / e.V. » et le logo Forgejo se superposent — flexbox/grid probablement non appliqué sur cette section.
6. **Images** : icônes/favicons absents dans les listings Lobsters (boîtes vides bordées) — vérifier le chargement/décodage des `<img>` distants (png/svg).
7. **Constat transversal** : sur 4 sites « JS lourds », 3 échecs sont causés par **un seul point dur JS** (regex, `in`, export webpack) qui tue le bundle entier — le rapport coût/bénéfice des fixes JS est énorme : chaque exception bootstrap = page blanche.

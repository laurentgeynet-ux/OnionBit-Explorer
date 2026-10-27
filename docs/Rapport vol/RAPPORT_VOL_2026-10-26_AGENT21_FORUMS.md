# Rapport de vol — AGENT 21 : Forums / communautés (Discourse)

- **Date** : 2026-10-26
- **Daemon** : `explorer-daemon.exe --port 9405`, viewport `1366x900` (PID 9240)
- **Méthode** : `POST /api/tabs` → `navigate` → grâce 10-12 s → `/console`, `/frame`, `/dom`, `/layout` ; interactions via `/api/tabs/{id}/input` (`scroll`, `click`).
- **Constat transverse majeur** : les 5 cibles sont des instances **Discourse**.
  Toutes ont servi la **version « crawler »** (`<body class="crawler">`)
  — le HTML statique de secours, sans l'application Ember. Le moteur
  l'a rendue correctement partout. Aucun bundle JS n'a été servi →
  **aucune erreur JS n'a pu être déclenchée** ; la bissection
  « même erreur fatale Ember sur N cibles » est donc close : il n'y a
  pas eu d'exécution Ember du tout.

## Synthèse mesurée

| Cible | doc_h | DOM | Console | Verdict |
|---|---|---|---|---|
| meta.discourse.org | 4 331,9 px | 190 Ko | `pas de journal` | Liste de sujets lisible, tableau Topics/Replies/Views/Activity |
| ↳ page sujet (`/t/…/1`) | 2 566,6 px | — | `pas de journal` | Article complet lisible (résumé IA, sections, liens) |
| community.letsencrypt.org | 4 215,8 px | 166 Ko | `pas de journal` | Liste de sujets lisible |
| ↳ page sujet | 3 134,3 px | — | `pas de journal` | OK |
| forums.opensuse.org | 900,0 px (= viewport) | 27 Ko | `pas de journal` | Tableau de catégories par langue, complet mais tient dans le viewport |
| forum.arduino.cc | 939,0 px | 27 Ko | `pas de journal` | Catégories lisibles, bandeau teal rendu |
| discuss.python.org | 1 846,0 px | 50 Ko | `pas de journal` | 20 catégories, lisible |
| ↳ `/c/help/7` (clic réel) | 3 370 px | — | `pas de journal` | Liste de sujets de la catégorie |
| ↳ page sujet (`/t/…/42`, clic réel) | 10 437 px | — | `pas de journal` | Sujet complet : titres, code coloré, sujets liés |

Couleurs dominantes mesurées : 85–96 % de blanc + gris texte + accent
de la charte du site (violet Discourse `#7B5FE2`, teal Arduino
`#008184`, bleu liens `#0088CC`) — le CSS de base Discourse
(tableaux, liens, en-têtes) est appliqué.

## Constats par cible

### 1. meta.discourse.org — lisible, version crawler

- Frame 1366×4332 : liste de sujets en tableau (Topic / Replies /
  Views / Activity) avec titres en violet, extraits gris, dates
  complètes (« October 26, 2026 »).
- Page sujet : rendu propre, hiérarchie de titres correcte, liens
  violets, listes à puces.
- **Écart** : excerpt brut `« :waving_hand: We're so glad… »` —
  le shortcode emoji Discourse reste littéral (le HTML crawler
  contient le texte tel quel — probablement fidèle à la source,
  à vérifier, pas nécessairement un bug moteur).
- **Écart** : les avatars (`<img src="/user_avatar/…">`, présents
  dans le DOM dump) ne produisent qu'**1 seule boîte `img`** dans le
  layout (un placeholder « img - Di… ») — la colonne avatars de la
  liste n'est pas matérialisée. Sévérité : mineure (crawler view les
  affiche peu de toute façon).

### 2. community.letsencrypt.org — lisible, version crawler

- Frame 1366×4216, même gabarit tableau ; titres bleus `#0088CC`.
- Page sujet OK (3 134 px de contenu).
- Aucun écart spécifique.

### 3. forums.opensuse.org — crawler minimal

- `doc_h` exactement 900 px = viewport : la vue crawler « catégories »
  ne contient que le tableau des sections linguistiques (English,
  Nederlands, Deutsch, Русский, Polski, Español, Português…) avec
  compteur « Topics » à 0 partout.
- Rendu correct : filets de tableau, bandeaux colorés, nav
  `crawler-nav` (Home/Categories/Guidelines/Terms/Privacy) en pied.
- **Note** : « 0 Topics » partout — données crawler, pas un trou
  moteur (le serveur sert ce HTML aux agents non-JS).

### 4. forum.arduino.cc — crawler minimal

- `doc_h` 939 px ; en-tête teal rendu, tableau catégories complet
  (Development Tools, Projects, Official Hardware, Other Hardware,
  Community, International, Forum 2005-2010).
- **Écart** : colonne « Topics » en haut à droite partiellement
  tronquée au bord droit du viewport dans la capture
  (`Topi…` visible) — possible débordement du tableau sur 1366 px.
  Sévérité : mineure.

### 5. discuss.python.org — crawler + navigation par clic OK

- 20 catégories en tableau, descriptions multi-lignes.
- **Clic réel** `{"type":"click","x":188,"y":170}` sur « Python
  Help » → `result: navigated`, URL `/c/help/7` : liste de sujets
  rendue (3 370 px).
- **Second clic** sur un titre de sujet → `/t/about-the-python-help-category/42`
  : page complète 10 437 px — corps du sujet, bloc de code
  (`python … me.bake("bagel")`), liste des sujets liés soulignés.
- Interaction complète prouvée : hit-test, navigation, re-layout.

## Bissection JS — résultat

Hypothèse de départ : « Discourse = Ember lourd → même erreur fatale
sur les 3-4 cibles ». **Infirmée par la mesure** : le `class="crawler"`
sur `<body>` des 5 dumps DOM prouve que Discourse a détecté l'agent
et servi son HTML statique (comportement serveur, UA-based). Les
bundles Ember n'ont jamais été livrés → `explorer-jscore` n'a eu
que 1 à 3 `<script>` légers par page (dont
`script.outbound-links.tagged-events.js`) et **zéro erreur** consignée
(`pas de journal` partout).

Conséquence pour la suite : pour mesurer le trou Ember réel, il faut
soit forger un `User-Agent` de navigateur moderne (le crawler view
se déclenche sur sniffing UA), soit viser un site Discourse sans
fallback crawler. Constat reporté, pas un bug moteur en soi.

## Constats actionnables

1. **[Mineure] Colonne droite du tableau catégories tronquée** —
   `forum.arduino.cc`, en-tête « Topics » coupé au bord x=1366 dans
   `vol21_arduino.png`. Vérifier le dimensionnement `table-layout` /
   cellule à `text-align:right` dans la vue crawler.
2. **[Mineure] Avatars `<img>` absents du layout** — DOM complet
   (`/user_avatar/…` × N) mais une seule boîte `img` posée sur
   meta.discourse.org. Vérifier le chargement/pose des `<img>`
   dans les tableaux crawler (lazyload `data-src` non résolu ?).
3. **[Info] Shortcodes emoji Discourse (`:waving_hand:`) rendus
   littéraux** — conforme au HTML servi ; à re-vérifier quand un
   vrai rendu Ember sera testé.
4. **[Info / procédure] Les 5 cibles Discourse servent la vue
   crawler** — aucun JS lourd exécuté, console vide. Pour un test
   Ember réel : empoisonner le `User-Agent` ou choisir une instance
   sans fallback. Le comportement « dégradé gracieux » est en
   revanche **prouvé** : navigation, clic, scroll, pages longues
   (10 437 px) fonctionnels.
5. **[Note outillage] `POST /api/tabs/{id}/scroll` n'existe pas** —
   le scroll passe par `/input` `{"type":"scroll","dy":N}`
   (confirmé : `scroll_y` 800→1600). Adapter le protocole vol-agent.
6. **[Note outillage] `/frame` retourne le document entier**
   (1366×4332), pas le viewport — le « PNG scroll » est identique à
   l'original ; utiliser `?x=&y=&w=&h=` pour des régions.

## Artefacts

- Captures : `captures/vol21_{meta_discourse, meta_discourse_topic,
  meta_discourse_scroll, letsencrypt, letsencrypt_topic, opensuse,
  arduino, python, python_cat, python_topic}.png`
- Télémétrie : `logs/vol21_{meta_discourse, letsencrypt, opensuse,
  arduino, python}_{dom,layout}.json` + `vol21_python_cat_layout.json`
- Log daemon : `_vol21_daemon.log` (vide — aucun diagnostic émis)
- Onglets fermés, daemon arrêté (PID 9240). Aucun code modifié.

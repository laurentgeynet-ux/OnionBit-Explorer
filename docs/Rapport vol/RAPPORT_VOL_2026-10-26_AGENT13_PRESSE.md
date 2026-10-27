# Rapport de vol — AGENT 13 — Sites de presse

- **Date** : 2026-10-26, session « vol 13 »
- **Daemon** : `C:/obe-vol-head/target/release/explorer-daemon.exe --port 9397`,
  `EXPLORER_VIEWPORT=1366x900`, log `C:/OnionBit-Explorer/_vol13_daemon.log`
- **Protocole** : `docs/vol-agent.md` — navigate → grâce ~10 s → console/frame/dom/layout
  → scroll ×3 → clic article.
- **Cibles** : lemonde.fr, theguardian.com/international, bbc.com/news,
  nytimes.com (échec 403), fallback franceinfo.fr.
- **Captures** : `docs/Rapport vol/captures/vol13_*` — **logs** : `logs/vol13_*`.

Note méthodo : les mesures « blanc % / couleur % » sont comptées par
échantillonnage de pixels (1 px / 3-6 en x et y) sur les PNG. « coloré » =
écart > 40 entre canaux R/G/B (proxy « image photo réellement peinte »).

---

## 1. https://www.lemonde.fr — rendu partiel, images absentes, article → anti-bot

- **URL finale home** : `https://www.lemonde.fr/` — titre « Le Monde.fr -
  Actualités et Infos en France et dans le monde », `doc_h=16908` (clamped).
- **Erreurs console (verbatim)** :
  ```
  exception js : TypeError: cannot read `hosts` of Undefined
      at <anonymous> (inline:10:1:67404)
  exception js : TypeError: cannot read `context` of Undefined
      at <anonymous> (inline:8:2:23348) at i (inline:8:2:130991)
  exception js : ReferenceError: lmd is not defined
      at <anonymous> (inline:11:1:2)
  exception js : TypeError: Invalid URL
      at <anonymous> (inline:5:2:20826)
  exception js : TypeError: cannot read `context` of Undefined (inline:16)
  exception js : TypeError: cannot read `hosts` of Undefined (inline:14)
  syntaxe js 1:226510 : expected `)` — https://www.lemonde.fr/_fs-ch-1T1wmsGaOgGaSxcX/assets/init.js
  ```
- **Constats mesurés** :
  - Frame home `vol13_lemonde.png` (1366×900, viewport) : bande sombre
    (24,26,30) sur les ~140 premiers px (header « en continu »), puis
    **95-100 % de blanc** sur tout le reste ; pixels « colorés » (images
    photo) : **0,3 %** à 2,4 % même après scroll ×3 — alors que le DOM
    contient **491 `<img>`** dont **195 attributs `lazy`**.
  - Layout cohérent : titres présents (1 h1, 27 h2, 76 h3, 54 h4),
    boîtes `<picture>/<img>` dimensionnées (ex. `[1066 2919 300x200]`),
    listes d'articles avec « Article réservé à nos abonnés » — le
    **contenu éditorial est parsé et mis en page**, seules les **images
    ne sont jamais peintes** (lazy-load piloté par JS, mort avec les
    exceptions ci-dessus).
  - **Consentement Didomi** : aucun bandeau Didomi dans le DOM/layout
    (injection JS échouée — `lmd is not defined` ; `lmd` est l'objet de
    config inline pourtant présent en `data`). Conséquence inattendue :
    **rien ne bloque**, la page est consultable — mais c'est un échec
    « par accident », pas par conformité.
  - **Clic article** (« La victoire de Lula au Brésil… », lien h2
    `[0 2940 975x21]`) : navigation OK mais aboutit à une page
    **« Client Challenge »** (anti-bot DataDome, `vol13_lemonde_article.png`
    = 99 % blanc) — le challenge JS ne passe évidemment pas.
- **Classification** : rendu dégradé exploitable (texte oui, images non) ;
  article bloqué par anti-bot. **Sévérité : majeure.**

## 2. https://www.theguardian.com/international — meilleur rendu, liens cartes à largeur nulle

- **URL finale** : idem — titre « Latest news, sport and opinion from
  the Guardian », `doc_h=13833` (non clampé).
- **Erreurs console (verbatim)** :
  ```
  exception js : TypeError: object is not iterable
      at <anonymous> (https://assets.guim.co.uk/assets/index.client.web.eb20e68fd805fe3f4b4c.js:1:134012)
      at <anonymous> (.../index.client.web.eb20e68fd805fe3f4b4c.js:1:133681)
  ```
- **Constats mesurés** :
  - `GET frame` renvoie ici le **document entier** (1366×13833, 53 Mo),
    pas le viewport — comportement différent de lemonde/franceinfo
    (viewport 1366×900). Corrélé à `doc_h_clamped` : non clampé →
    frame = doc entier, clampé → frame = viewport. Sémantique de
    `GET /frame` instable — à documenter/fixer.
  - Premier viewport : 46,6 % de pixels colorés — **les images sont
    réellement décodées et peintes** (meilleur site de la session).
    Ensemble du doc : 12,8 % coloré, 62 % blanc (zones texte/listes).
  - **Liens d'articles à largeur nulle** : `block<a> [73 2996.1
    0x236.6]` enveloppant les cartes d'articles (ex. « Cheers, fireworks
    and flags after Vučić… ») — 24 `<a>` sur 247 ont `w=0`. Un clic au
    centre d'une carte/titre → `{"result":"ignored"}` (vérifié :
    clic (500,190) sur légende figcaption, et la hitbox des titres est
    vide). Cliquer un titre d'article Guardian est **impossible**.
  - Débordement horizontal : textes à `x=1333/1363` avec largeurs
    ~145-414 px → contenu peint au-delà de 1366 (colonne de droite
    tronquée, visible dans `vol13_guardian_v3.png`).
  - Clic header (nav `[978 310 178x38]`) : navigation OK →
    `/subscribe/weekly` (`vol13_guardian_nav.png`, doc 1366×1470,
    89 % blanc, 9,3 % coloré) — le chrome du site fonctionne.
- **Classification** : home riche et lisible, navigation article cassée
  (hitbox nulles). **Sévérité : majeure (clic), mineure (débordement).**

## 3. https://www.bbc.com/news — layout sain, images home jamais peintes

- **URL finale** : idem — « BBC News - Breaking news… »,
  `doc_h=5848` non clampé.
- **Erreurs console** : **aucune** (`{"console":[]}` — « pas de journal »).
- **Constats mesurés** :
  - Frame = doc entier (1366×5895). Home : **95 % blanc, 0,0 % coloré**
    alors que les `<img>` ont des boîtes correctes (ex. `[375 538
    616x346.5]`, grille de cartes `[59 940.5 221x124]` ×4) et que 44
    `<img>` existent dans le DOM. Titres et chapôs tous présents
    (« Lula wins fourth term… », « Israel and the US… », etc.) — le
    layout BBC est le plus propre de la session (grille respectée,
    colonnes 221/300/616 px nettes), mais **zéro image peinte sur la home**
    (lazy-loading BBC = `src` injecté par JS, ou images `<img>` sans
    src résolu).
  - Clic sur le titre « Lula wins fourth term » (1100,740) : navigation
    réussie → article complet `…/articles/…` (titre « Brazil election:
    Lula wins fourth term in dramatic comeback over Bolsonaro »),
    frame doc 1366×9649, 75 % blanc / **10,7 % coloré** — les images de
    l'article, elles, **sont** peintes. Donc le décodeur marche ; c'est
    bien le lazy-load de la home qui ne se déclenche pas.
  - Scroll ×3 OK (`scroll_y` 900/1800/2700).
- **Classification** : home « texte seul », article correct.
  **Sévérité : modérée.**

## 4. https://www.nytimes.com — HTTP 403, rien à mesurer

- `POST navigate` → `{"error":{"handled":true,"message":"HTTP 403"}}`,
  console : `["HTTP 403"]`, « pas de DOM ». Frame = page vide blanche
  (99 %). Le WAF NYT refuse le client (UA/TLS fingerprint).
- **Classification** : inaccessible. **Sévérité : bloquante** (cible
  paywall non testable — ni consentement ni paywall observés).

## 5. Fallback : https://www.franceinfo.fr — hero sombre géant, scroll intermittent

- **URL finale** : `https://www.franceinfo.fr/` — « franceinfo -
  Actualités en temps réel… », `doc_h=96532` **clampé** (layout racine
  brut : 277114 px — doc potentiellement ~3× la hauteur clampée).
- **Erreurs console (verbatim, extrait)** :
  ```
  exception js : ReferenceError: writeEmbed is not defined — iframe — src «writeEmbed();»
  exception js : ReferenceError: writeEmbed is not defined — iframe dynamique (×3)
  exception js : TypeError: Invalid scheme
      at <anonymous> (https://timeliner.live.franceinfo.fr/_next/static/chunks/polyfills-42372ed130431b0a.js:1:81519)
      at <anonymous> (…/polyfills-42372ed130431b0a.js:1:90778)
      at <anonymous> (https://www.franceinfo.fr/assets/components/footers/footer-classic/footer-classic-…js:1:892)
  ```
- **Constats mesurés** :
  - Frame viewport : **63 % sombre** — un bloc dark navy occupe
    tout l'écran de y=0 à ~y=520 (header 522,6 px dans le layout :
    `block<header> [0 0 1366x522.6]`) : le hero « live » franceinfo
    remplit plus de la moitié du viewport. Images : 1,9 % coloré
    (absentes, comme lemonde).
  - DOM/layout : la home contient **deux copies du header** (nav à
    y=0 puis répétée à y≈1246 — seconde occurrence du menu
    « contenu principal / Newsletters / Météo / Jeux / Scolarité ») :
    duplication mesurée dans `vol13_franceinfo_layout.txt` (lignes
    ~13-127 vs ~228-320). Probable double rendu (SSR + re-montage JS
    partiel qui ré-appose des nœuds) ou gabarit dupliqué.
  - **Scroll intermittent** : `scroll dy:900` ×3 → `scrolled(900)`,
    puis `ignored`, `ignored` — le scroll se fige à 900 px alors que
    `doc_h` = 96532. Un scroll négatif −1800 depuis 900 est accepté
    (`scroll_y=900` rapporté, valeur incohérente — attendu 0). Le
    défilement est cassé sur cette page (peut-être lié au clamp ou au
    re-layout continu sous JS).
  - Clic sur « Mobilisation lycéenne : l'organisation "Les Lycéens !"… »
    (lien `block<a> [56 1017.5 429x108]`, bien dimensionné) : navigation
    OK → article franceinfo chargé et titré correctement
    (`vol13_franceinfo_article.png` : 60 % sombre, toujours le hero).
- **Classification** : lisible sous un hero envahissant ; scroll cassé ;
  images absentes. **Sévérité : majeure.**

---

## Constats actionnables

1. **[Images lazy-load jamais résolues — presse généraliste]**
   lemonde (491 `<img>`, 195 `lazy`, 0,3 % coloré) et bbc.com/news
   (44 `<img>`, boîtes OK, 0,0 % coloré) ne peignent aucune image de
   home, alors que l'article BBC (src directes) atteint 10,7 % coloré.
   Hypothèse : `src`/`srcset` injectés par JS (IntersectionObserver) ou
   `data-src` non promus — le moteur devrait résoudre `data-src` et/ou
   charger les images visibles sans JS. Équipe : NAVIGATEUR (JS) +
   PIPELINE (fallback `data-src` côté `<img>`). **Prioritaire.**

2. **[Hitbox de lien à largeur nulle — Guardian]** `block<a> [x y
   0x236]` : 24/247 ancres avec `w=0`, enveloppant les cartes
   d'articles → clic « ignored » sur tous les titres. Le `<a>` block
   reçoit la hauteur du contenu mais pas la largeur du parent (stretch
   flex/grid perdu ?). Équipe : PIPELINE (`explorer-layout`).

3. **[Anti-bot / 403]** lemonde article → page « Client Challenge »
   (DataDome) ; nytimes.com → HTTP 403 dès la home. Le challenge JS
   est hors de portée actuelle ; documenter la détection (titre
   « Client Challenge », HTTP 403 propre) plutôt qu'écran blanc muet.
   Équipe : NAVIGATEUR (surface d'erreur API : `handled:true` ok, mais
   la frame d'erreur est blanche — exposer un état « challenge » ?).

4. **[Scroll figé / incohérent — franceinfo]** scroll ×3 : un accepté,
   deux `ignored` à `scroll_y=900` sur doc clampée à 96532 ; un scroll
   −1800 rapporte `scroll_y=900`. Soit le scroll est perdu pendant les
   apply-jobs JS (parallèle au « DOM détaché » 404 connu), soit le
   clamp sur `doc_h` massivement instable (layout brut 277114 px) casse
   le suivi. Équipe : NAVIGATEUR (core/input).

5. **[Duplication de header — franceinfo]** deux navs identiques à y=0
   et y≈1246 dans le layout (le document contient aussi 3 occurrences
   des disclaimers newsletter) : re-montage JS qui duplique des gabarits
   SSR, ou fixture DOM double. À confirmer côté `explorer-dom`/JS.
   Sévérité modérée (visuellement masqué sous le hero).

6. **[Sémantique de `GET /frame` instable]** viewport-only (1366×900)
   quand `doc_h_clamped=true` (lemonde, franceinfo), document entier
   (1366×13833, 53 Mo PNG) sinon (guardian, bbc). Deux comportements,
   même endpoint — fixer un contrat (paramètre explicite `?full=` ?).

7. **[Consentement non testé, par absence]** Didomi (lemonde) ne
   s'injecte pas — `lmd is not defined`, exceptions `hosts/context of
   Undefined` (probablement `window.lmd`/`didomi` non initialisés ou un
   global manquant). Conséquence « heureuse » : rien ne bloque ; mais le
   scénario « bandeau consentement dismissable » reste non couvert —
   aucune cible de la session n'a affiché de CMP. À rejouer quand les
   globals JS manquants seront supportés.

8. **[Débordement horizontal — Guardian]** textes posés à
   x=1333-1363 avec w=145-414 → hors viewport 1366 (colonne droite
   tronquée). Probable conteneur fixed-width/overflow non clippé.
   Équipe : PIPELINE (layout).

9. **[Exceptions JS systématiques]** chaque cible JS-riche échoue tôt :
   `cannot read X of Undefined` (globals de config), `object is not
   iterable` (Guardian — probablement `Symbol.iterator`/destructuring
   manquant), `Invalid URL`, `Invalid scheme` (franceinfo — constructeur
   URL incomplet ?), `writeEmbed is not defined` (iframes sandboxées
   sans API). Ce sont les trous JS-Cxx typiques : itérables, globals
   window, URL/scheme dans iframes. Équipe : NAVIGATEUR / spécialiste JS.

## Hygiène de session

- Onglets 1-4 fermés, daemon propre tué (voir § Arrêt ci-dessous).
- Aucune ligne de code modifiée. Captures et logs sous
  `docs/Rapport vol/{captures,logs}/vol13_*`.

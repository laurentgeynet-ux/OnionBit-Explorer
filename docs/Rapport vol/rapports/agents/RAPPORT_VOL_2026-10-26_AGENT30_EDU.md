# Rapport de vol — Agent 30 — Sites éducatifs / encyclopédiques

- **Date** : 2026-10-26
- **Binaire** : `C:/obe-vol-integ/target/release/explorer-daemon.exe` (intégration V2)
- **Viewport** : 1366×900, port 9414, daemon PID 14056
- **Protocole** : `docs/vol-agent.md` — onglet par cible, grâce 8-12 s,
  console verbatim, frame PNG, dumps DOM/layout, scroll ×2, clic plausible.
- **Note de reprise** : agent précédent mort d'erreur interne ; vol entièrement refait.
- **Régression en investigation** : clics répondant `ignored` — **reproduite
  sur 3 cibles sur 4** (documentée, pas re-diagnostiquée, cf. §5).

---

## 1. W3Schools — https://www.w3schools.com/html/default.asp

- **Titre** : « HTML Tutorial » ; `doc_h` = 4650 px, non clampé.
- **Captures** : `vol30_w3schools.png`, `vol30_w3schools_scroll.png`,
  `vol30_w3schools_click.png` ; logs `vol30_w3schools_dom/layout.json`.

### Console (verbatim, après clic)

```
exception js : TypeError: `7fkQgoC` is not a function (on function upack@242:11)
    at _uic_u_ok (https://www.w3schools.com/lib/uic.js?v=1.0.12:261:15)
    at <anonymous> (https://www.w3schools.com/lib/uic_prov.js?v=1.5.3:5:22)
syntaxe js 778:5 : `debugger` is not supported — https://www.w3schools.com/lib/user-session/main.v1.0.52.js
exception js : ReferenceError: UserSession is not defined
    at <anonymous> (https://www.w3schools.com/lib/my-learning/main.v1.0.107.js:547:40)
    at <anonymous> (https://www.w3schools.com/lib/my-learning/main.v1.0.107.js:567:38)
    at <anonymous> (https://www.w3schools.com/lib/my-learning/main.v1.0.107.js:2378:19)
exception js : ReferenceError: UserSession is not defined
    at <anonymous> (https://www.w3schools.com/lib/my-learning/main.v1.0.107.js:1847:29)
    at <anonymous> (https://www.w3schools.com/lib/w3schools_footer.v1.0.1.js:386:2)
exception js : ReferenceError: UserSession is not defined
    at <anonymous> (https://www.w3schools.com/lib/my-learning/main.v1.0.107.js:929:23)
exception js : TypeError: Object is not a function
    at <anonymous> (https://static.fastcmp.com/fast-cmp-stub.legacy.3945c53a58e3db662989.js:1:184232)
    at <anonymous> (…fast-cmp-stub…:1:290713)
    at <anonymous> (…fast-cmp-stub…:1:96740)
```

### Constats mesurés

- Rendu **quasi parfait** : méga-navbar (4 lignes, ~30 entrées), sidebar
  gauche complète, titre « HTML Tutorial », boutons ‹ Home / Next ›,
  panneau « Adventure Mode » avec illustration, sidebar pub droite.
- Scroll ×2 OK (`scroll_y` 800 → 1600), sections « HTML Exercises »,
  « HTML Quiz Test », « Track Your Progress » correctement peintes.
- **Clic fonctionnel** : clic sidebar → `{"result":"navigated"}` vers
  `html_elements.asp`, page « HTML Elements » repeinte avec coloration
  syntaxique des exemples `<tagname>` (rouge) — le seul site du vol où
  le clic navigue.
- Défaut mineur : micro-texte chevauchant au-dessus du bouton
  « Try Adventure Mode » (≈2 mots superposés illisibles).
- Le refus de `debugger` casse `main.v1.0.52.js` entier → cascade
  `UserSession is not defined` (5 frames mesurées) — un seul token de
  syntaxe bloque tout le module.

**Classification** : navigable. **Sévérité** : faible (défauts JS tolérés,
rendu visuel complet).

---

## 2. GeeksforGeeks — https://www.geeksforgeeks.org

- **Titre** : « GeeksforGeeks | Your All-in-One Learning Portal » ;
  `doc_h` = 4176 px.
- **Captures** : `vol30_gfg.png`, `vol30_gfg_retry.png` (blanche,
  vérifiée pixel : extrema RGBA = 255 partout) ; logs `vol30_gfg_dom.json`,
  `vol30_gfg_layout.json`.
- **Console** : `pas de journal` (zéro erreur JS rapportée).

### Constats mesurés

- **Frame entièrement blanche** alors que DOM (155 Ko) et layout
  (46 Ko, boîtes réelles : « DSA », « Practice Problems », « C++ »,
  « Machine Learning »…) sont pleinement peuplés.
- Cause mesurée dans le DOM : wrapper racine portant
  `style="visibility:hidden;background-color:#ffffff;min-height:100vh"`
  (ligne 173 du dump) — le site masque la page jusqu'à révélation JS ;
  sans exécution du dévoilement, tout reste peint blanc. 22 `opacity:0`
  et 5 `display:none` supplémentaires dans le markup (reveal-on-scroll).
- Scroll exécuté (`scroll_y` 600) — frame toujours 100 % blanche.
- **Clic `ignored`** : clic sur l'entrée nav « Python » (542,31) →
  `{"result":"ignored"}` — cohérent avec le contenu `visibility:hidden`
  (hit-test sur contenu invisible) ; constat joint à la régression en
  investigation.

**Classification** : page blanche (blocage contenu). **Sévérité** : bloquante —
le site est inutilisable bien que fetch/parse/layout réussissent.

---

## 3. Khan Academy — https://www.khanacademy.org

- **Titre** : « Client Challenge » ; `doc_h` = 16 px au chargement.
- **Captures** : `vol30_khan.png` ; logs `vol30_khan_dom.txt`,
  `vol30_khan_layout.txt`.
- **Console** : `pas de journal`.

### Constats mesurés

- Réponse servie = **challenge anti-bot** (`/_fs-ch-1T1wmsGaOgGaSxcX/`,
  CSP stricte `script-src 'self' 'sha256-…'`). Page challenge peinte
  correctement : *« A required part of this site couldn't load. This may
  be due to a browser extension, network issues, or browser settings… »*.
- Le challenge n'est jamais résolu (JS de preuve non exécuté) → contenu
  réel jamais atteint. Attendu pour un moteur maison ; pas un bug de
  rendu — le peu de page servi est peint fidèlement.

**Classification** : bloqué par anti-bot (hors moteur). **Sévérité** : n/a
(limitation connue, le rendu du challenge est correct).

---

## 4. Coursera — https://www.coursera.org

- **Titre** : « Coursera | Online Courses, Certificates, & Degrees » ;
  `doc_h` = 7871 px.
- **Captures** : `vol30_coursera.png`, `vol30_coursera_scroll.png` ;
  logs `vol30_coursera_dom.txt`, `vol30_coursera_layout.txt`.
- **Console** : `pas de journal`.

### Constats mesurés

- Rendu **partiel mais substantiel** : header (logo, Explore, Degrees,
  champ recherche, Log In, Join for Free), deux bannières hero
  (« AI is reshaping Data Analytics », « Unlock 3 months of Google AI
  Pro… ») correctement peintes avec images.
- **Cartes catalogue = squelettes** : la rangée « New and popular »
  n'affiche que des placeholders gris (images/titres jamais hydratés —
  contenu React sans hydrate complet).
- **Boutons trop étroits** : « Get started », « Find your AI Series »,
  « Start 7-day free trial » rendus en colonne ~90 px avec texte
  enroulé mot à mot + flèche — dimensionnement de bouton défaillant
  (flex/inline-size non résolu).
- **Empilement z-order défaillant** : après scroll, les squelettes de
  cartes se peignent **par-dessus** le header fixe (capture scroll) —
  contexte d'empilement du sticky header perdu.
- Barre noire parasite bord droit du viewport (haut).
- Police de fallback serif utilisée pour le contenu (webfonts
  probablement non chargées/substituées).
- **Clics `ignored`** : Explore (90,63), Log In (1181,63), zone
  recherche (575,60) → trois `{"result":"ignored"}` — régression
  documentée (liens React/JS probablement sans `<a href>` hit-testable).

**Classification** : rendu partiel dégradé. **Sévérité** : majeure
(squelettes persistants + boutons cassés + z-order) mais page lisible.

---

## 5. MIT OpenCourseWare (fallback) — https://ocw.mit.edu

- **Titre** : « MIT OpenCourseWare | Free Online Course Materials » ;
  `doc_h` = **25 615 px, `doc_h_clamped: true`** (plus grande page du vol,
  hauteur bornée).
- **Captures** : `vol30_ocw.png`, `vol30_ocw_scroll.png` ; logs
  `vol30_ocw_dom.txt`, `vol30_ocw_layout.txt`.

### Console (verbatim)

```
exception js : Error: CAROUSEL: Option "interval" provided type "false" but expected type "(number|boolean)".
    at <anonymous> (https://ocw.mit.edu/static_shared/js/common.b9daa.js:2:2276)
    at <anonymous> (…common.b9daa.js:2:10079)
    at e (…common.b9daa.js:2:8248)
    at <anonymous> (…common.b9daa.js:2:14240)
    at <anonymous> (…common.b9daa.js:2:65258)
    at <anonymous> (…common.b9daa.js:2:63781)
    at <anonymous> (…common.b9daa.js:2:14211)
    at <anonymous> (…common.b9daa.js:2:15128)
    at <anonymous> (…common.b9daa.js:2:101354)
    at <anonymous> (…common.b9daa.js:2:99893)
```

### Constats mesurés

- Rendu partiel : hero « Unlocking knowledge, Empowering Minds »,
  panneau « Are you new to OCW? », carrousel « MIT Learn » (flèches +
  pagination), bannière « Your Donation Makes a Difference », section
  « OCW News » avec carte rouge MIT et liste d'articles — tous peints.
- **Navigation/header absents** : aucun menu/topbar visible en haut de
  page (logo, liens de nav non rendus) — seul « GIVE NOW » flotte en
  haut-centre.
- **Cartes « Featured Courses » effondrées** : colonnes ~60 px de
  large, titres tronqués à 4-5 caractères/ligne (« Tect / Fem / and »,
  « Bui / Ser / Sys »), instructeurs et topics clippés — échec de
  distribution flex/grid (capture scroll).
- Grands blancs verticaux entre sections (espacements excessifs,
  contribuant aux 25 615 px de hauteur).
- **Clic `ignored`** : bouton « Explore » (558,259) → `ignored` ;
  un second clic sur lien article non testé (même régression attendue).

**Classification** : rendu partiel. **Sévérité** : majeure (cartes
illisibles, nav absente) mais contenu texte accessible.

---

## Constats actionnables

1. **[GFG — bloquant] Révélation `visibility:hidden` jamais exécutée** :
   page entière blanche (extrema RGBA = 255) malgré DOM 155 Ko et layout
   complet — le moteur ne retire pas le `visibility:hidden` inline du
   wrapper racine (révélation dépendant de JS). Site 100 % inutilisable.
   Vérifier si le hook de dévoilement (DOMContentLoaded/raf) est appelé.
2. **[OCW — majeur] Distribution flex/grid des cartes effondrée** :
   cartes « Featured Courses » réduites à ~60 px de large avec texte
   clippé par caractère — vérifier `flex-basis`/`minmax()`/pourcentages.
3. **[OCW — majeur] Header/nav absents** : topbar jamais peinte — à
   corréler avec l'exception CAROUSEL (le bundle `common.b9daa.js` mort
   en entier peut emporter l'initialisation de la nav).
4. **[Coursera — majeur] Squelettes React jamais hydratés** : placeholders
   gris persistants sur « New and popular » ; z-order perdu au scroll
   (squelettes peints au-dessus du header fixe).
5. **[Coursera — majeur] Boutons dimensionnés ~90 px** : texte CTA
   enroulé mot à mot (« Get / started », « Start 7- / day / free /
   trial ») — largeur intrinsèque ou `inline-size`/`fit-content` mal
   résolue.
6. **[W3Schools — moyen] Un token `debugger` tue tout un module** :
   `syntaxe js : debugger is not supported` → 5 × `ReferenceError:
   UserSession is not defined` en cascade ; traiter `debugger` comme
   no-op éviterait la cascade.
7. **[Transverse — régression documentée] Clics `ignored`** : confirmé
   sur GFG (visibility:hidden → hit-test vide), Coursera (×3), OCW —
   mais **NON** sur W3Schools (`navigated` OK). Ne pas re-diagnostiquer ;
   corrélation probable : les cibles dont les liens sont des `<a href>`
   classiques naviguent, les handlers JS/SPA ou contenus masqués
   retournent `ignored`.
8. **[W3Schools — mineur] Micro-texte chevauchant** au-dessus du bouton
   « Try Adventure Mode » — inline-overlap résiduel.
9. **[Coursera — mineur] Barre noire parasite** bord droit du viewport ;
   webfonts non substituées (serif de fallback partout).

### Verdicts rapides

| Cible | Verdict | Sévérité |
|---|---|---|
| W3Schools | Navigable, rendu quasi complet | Faible |
| GeeksforGeeks | Page blanche (visibility:hidden) | Bloquante |
| Khan Academy | Challenge anti-bot non résolu (attendu) | n/a |
| Coursera | Partiel : squelettes + boutons cassés | Majeure |
| MIT OCW | Partiel : cartes effondrées, nav absente | Majeure |

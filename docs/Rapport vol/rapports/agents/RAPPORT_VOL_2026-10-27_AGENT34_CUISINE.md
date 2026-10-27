# Rapport de vol — Agent 34 — Sites cuisine/recettes

- **Date** : 2026-10-27
- **Binaire** : `C:/obe-vol-integ/target/release/explorer-daemon.exe` (intégration V3)
- **Viewport** : 1366×900 — port 9418
- **Protocole** : `docs/vol-agent.md` — frame PNG = verdict terrain
- **Note contexte** : deux daemons consommés — le premier (PID 17164)
  a été tué après blocage dur sur allrecipes.com, le second (PID 14896)
  a servi bbcgoodfood + 750g puis a re-bloqué sur allrecipes (repro ×2).
  Les clics produisent ici de vraies navigations (`{"result":"navigated"}`)
  — progrès net par rapport aux vols où le clic restait `ignored`.

---

## 1. Marmiton — https://www.marmiton.org

- **Titre** : « Marmiton : 78430 recettes de cuisine commentées et
  notées pour toutes les cuisines ! »
- **doc_h** : 11 212,96 px (non clampé)
- **Console** (verbatim, 3 entrées après landing) :

```
exception js : RangeError: heap memory limit exceeded
    at <anonymous> (https://www.flashb.id/assets/js/hbdbrk_97ea3a1e891aae3b8701.js:1:281056)
exception js : RangeError: heap memory limit exceeded — module «https://unpkg.com/mealz-components@0.7/dist/store-locator/store-locator.min.js»
exception js : RangeError: heap memory limit exceeded
    at <anonymous> (https://static.afcdn.com/relmrtn/concat.js?PCM-…:500:2342)
```

Après clic (console de la page album, 3 entrées) :

```
exception js : TypeError: cannot load module `app`: module non chargé : https://statics.marmiton.fr/assets/app-E0uZPb4.js — module «inline:12»
exception js : TypeError: cannot load module `js/front/user.js`: module non chargé : https://statics.marmiton.fr/assets/js/front/user-q3n638M.js — module «https://statics.marmiton.fr/assets/Organism/Header/header-user-FBkN7XD.js»
exception js : ReferenceError: GeolocationPosition is not defined — module «https://unpkg.com/mealz-components@0.7/dist/store-locator/store-locator.min.js»
```

- **Constats mesurés** (`vol34_marmiton.png`, `vol34_marmiton_scroll.png`,
  `vol34_marmiton_click.png`) :
  - Page d'accueil **structurellement correcte** : barre de recherche
    (« Je cherche une recette, un ingrédient, de l'aide… »), bouton bleu
    « Se connecter », nav corail (« Qu'est-ce qu'on mange ce soir ? »,
    « Univers Electroménager », « Actus Food », « Du caddie à
    l'assiette »), section « Consultez nos dernières actus » avec
    titres d'articles corrects.
  - **Toutes les images photographiques absentes** : hero = bloc gris
    pleine largeur (~460 px de haut), vignettes articles = blocs gris
    avec un petit carré blanc en haut à droite (badge/placeholder).
    **Cause mesurée dans le DOM** : 216 occurrences de `data-src` —
    les `<img>` n'ont **pas d'attribut `src`** (`data-src` +
    `data-srcset` + `loading="lazy"`, ex. `166465_w46.webp`) : le
    lazyload JS ne résout jamais `data-src → src` (JS en échec,
    cf. console). Le décodeur n'est donc pas en cause — les URL ne
    sont jamais demandées.
  - Scroll OK (`scroll_y` 0 → 900 → 1 800 → 2 700) ; cartes
    « 30 recettes express et faciles pour la rentrée », « 20 recettes
    simples… », « Rentrée : ces 10 goûters… » rendues (texte correct,
    images grises).
  - **Clic réel fonctionnel** : clic sur « 30 recettes express… »
    (x=120, y=240) → `{"result":"navigated"}` → album
    `recettes/album1358016/recettes-express-et-faciles-pour-la-rentree-0.html#p1`.
    Page album : fil d'Ariane « Accueil / Recettes / 30 recettes… »,
    H1, auteur « par Lucile Vivat », dates — correctement rendue
    (hero toujours gris).
- **Classification** : rendu fonctionnel, JS fortement dégradé
  (3 × `heap memory limit exceeded` au boot + modules ES non chargés
  après nav), images absentes par lazyload non résolu.
- **Sévérité** : moyenne-haute (page lisible et navigable, mais
  visuellement amputée et scripts métier morts).

## 2. Allrecipes — https://www.allrecipes.com

- **Titre** : non obtenu (navigate jamais retourné)
- **Console** : non obtenue (API morte avant toute lecture)
- **Constats mesurés — blocage dur du daemon, reproduit ×2** :
  - **Tentative 1** (daemon PID 17164) : `POST navigate` timeout
    côté client à 60 s ; log daemon : scripts évalués puis
    `apply: début tab=2` à 06:14:33 (cascade 650 ms, boxtree 699 ms,
    layout 807 ms) puis **plus aucune ligne de log**. `GET /api/tabs`
    sans réponse pendant **> 6 min** (plusieurs sondes) ; CPU ~100 %
    d'un cœur (8:39 → 8:59 en 20 s wall), RAM 2,9 Go. Daemon tué.
  - **Tentative 2** (daemon PID 14896, après vols BBC+750g nominaux) :
    `POST navigate` à 06:27:34, timeout client 75 s ; log : evals JS
    jusqu'à 06:27:37, `apply` à 06:28:44-45 (cascade 479 ms,
    boxtree 898 ms, layout 1 402 ms) puis silence ; `GET /api/tabs`
    toujours pendante à 06:30:25 (> 3 min d'absence de réponse),
    CPU 9:00, RAM 2,65 Go. Daemon tué.
  - Aucune frame récupérable sur mes 2 tentatives : la boucle
    événementielle ne sert plus l'HTTP — le blocage est
    **post-layout** (dernier log = fin de `apply: layout`),
    probablement une tâche JS rafale (timers/rAF/promesses) qui
    affame l'exécuteur unique.
  - **La page rend néanmoins avant le gel** (captures du vol
    antérieur interrompu, mêmes fichiers `vol34_allrecipes*.png`,
    26/10) : barre « Find a recipe or ingredient », « Newsletters |
    Sweepstakes », menu « Dinner Tonight » déplié en colonne gauche,
    section « Comfort Food Favorites » avec **photos réelles peintes**
    (meatloaf, plat crémeux), carte « 10 Best Brownie Pans of 2026 »,
    et **bandeau consentement Didomi « We Care About Your Privacy »**
    correctement rendu (Accept All / Reject All / Show Purposes).
    Défauts visibles : titre hero rogné à gauche sous la colonne nav,
    bandeau consentement recouvre le bas. Le gel survient **après**
    le premier rendu utile — fenêtre API exploitable ~10-20 s puis
    starvation totale.
- **Classification** : blocage total — le site rend puis tue le
  daemon (les autres onglets deviennent injoignables : l'API
  elle-même ne répond plus).
- **Sévérité** : critique (worst-case mesuré du vol : pas de
  dégradation gracieuse, starvation de l'API de contrôle).

## 3. BBC Good Food — https://www.bbcgoodfood.com

- **Titre** : « Good Food | Recipes and cooking tips | Good Food »
- **doc_h** : 12 147,84 px (non clampé)
- **Console** (verbatim, 1 entrée dédupliquée ×2) :

```
unhandledrejection: TypeError: `...` outside a call or array literal
    at <anonymous> (inline:54:1:5266)
    at <anonymous> (https://www.bbcgoodfood.com/_next/static/chunks/pages/_app-17549fe9bc647c9c.js:65:215395)
    at <anonymous> (inline:54:1:5142)
    at <anonymous> (inline:54:1:5539)
    at <anonymous> (inline:54:1:6909) ×2
```

- **Constats mesurés** (`vol34_bbcgoodfood.png`,
  `vol34_bbcgoodfood_scroll.png`, `vol34_bbcgoodfood_click.png`) :
  - Rendu **quasi sans CSS** : fond blanc, texte noir brut ; la nav
    mega-menu s'affiche en liste verticale déroulée (« Sign up today »,
    « WhatsApp feed », « What to cook », « Quick and easy »,
    « Everyday recipes », « Family recipes », « One-pots », puis après
    scroll : « Eat for energy », « Best induction pans », « Best stand
    mixers », « Best kettles »…). Le contenu éditorial est noyé sous
    des milliers de px de liens de menu.
  - Une rangée horizontale de catégories subsiste (« Recip… », « New
    and tren… », « Halloween reci… », « Pie recip… », « Sausage
    casse… », « Quick family… », « Newsletter… », « Traybake… »,
    « Cosy one-po… ») — **texte tronqué/clippé** à ~60-100 px par
    colonne (overflow masqué partiellement appliqué).
  - **Cause mesurée dans le DOM** : 4 `<link rel="stylesheet">`
    présents (`/static/styles/css/im-bbcgoodfood-theme/base-…css`,
    `components-…css`, `utilities-…css`, `services-…css`) + `<div
    id="__next">` (Next.js) + `__NEXT_DATA__` — le SSR est livré,
    les feuilles déclarées, mais la cascade ne s'applique visiblement
    pas (échec fetch CSS ou échec parse massif de CSS moderne ;
    à recouper avec la sonde `/net` en file). Le runtime Next ne
    boote pas non plus (cf. erreur de parse `...` dans `_app-….js`).
  - Scroll OK (0 → 2 700). Clic (x=45, y=713 sur « Everyday
    recipes ») → `{"result":"navigated"}` →
    `/recipes/collection/family-meal-recipes` — page collection rendue
    dans le même état dégradé ; console post-nav **vide** (reset par
    navigation confirmé).
- **Classification** : échec de cascade — contenu SSR présent mais
  rendu brut ; boot JS mort sur une erreur de **parseur** jscore.
- **Sévérité** : haute.

## 4. 750g — https://www.750g.com

- **Titre** : « 750g.com : plus de 80 000 recettes de cuisine :
  entrées, plats, desserts, cocktails... »
- **doc_h** : 13 720,52 px (non clampé)
- **Console** : **vide** (« pas de journal ») — landing **et** page
  article : zéro erreur JS, meilleur bilan console du vol.
- **Constats mesurés** (`vol34_750g.png`, `vol34_750g_scroll.png`,
  `vol34_750g_click.png`, `g750_article.png` en tmp) :
  - Header vert « 750g » rendu (logo blanc partiellement rogné en
    haut), zone recherche (boîte avec « C »).
  - **Méga-menu peint ouvert** dès l'atterrissage : colonnes « Par
    thème » / « Par objectif » / « Halloween » / « Actus » étalées
    sur fond blanc — le repli/collapse géré en JS ne s'applique pas.
    **Collisions mesurées** : « Emissions TV et divertissement »
    chevauche « Que faire avec ? » ; « Tests et recommandations »
    empiète sur « Nutrition et santé ». **Textes tronqués** :
    « Cuisine végé », « Sortie et restau », « Technique de cuisson ».
  - Après scroll 2 700 : **le contenu réel apparaît avec des images
    chargées** — photo GiFi (JPEG peint), teasers avec auteurs
    (« Par Béatrice Vigot », « Par Mélanie Duflot », « Par Lucile
    Vivat »), section « Actu », badge bleu « Actu ». Une vignette
    grise de repli à droite (image non résolue). Le bandeau header
    vert reste figé en haut pendant le scroll (sticky partiel ?
    le texte des teasers passe *sous* la barre — collision mesurée
    en haut de frame : « ultra-moelleu…as ? » rogné).
  - **Clic réel** : teaser « chantilly brésilienne… » (x=340, y=105)
    → `{"result":"navigated"}` → article
    `le-secret-d-un-gateau-au-yaourt-ultra-moelleux-…-a44788.htm`.
  - Page article (scroll 2 700) : corps d'article correct (« C'est
    une recette indémodable… », lien « goûter » vert), sidebar avec
    **images piment et sardines réellement peintes**, dates « Le
    22/10/2026 à 17h30 » avec icône horloge.
  - **Contre-preuve images** : DOM 750g = `<img src="https://
    static.750g.com/images/580-340/….jpeg">` (src réel, 0 `data-src`)
    → images peintes. Marmiton = `data-src` sans `src` → gris. Le
    pipeline image fonctionne quand l'URL est dans `src`.
- **Classification** : meilleur rendu du vol — CSS partiel, images
  OK, navigation OK ; défauts = menu peint ouvert + collisions de
  colonnes + sticky header.
- **Sévérité** : faible-moyenne.

---

## Constats actionnables

1. **allrecipes.com fige le daemon entier (critique, repro ×2 +
   témoin antérieur)** : la page rend (menu, articles, photos,
   bandeau consentement — captures `vol34_allrecipes*.png` du vol
   interrompu) puis `apply` termine son layout (807 ms / 1 402 ms)
   et l'exécuteur sature — API REST sans réponse > 3-6 min, ~100 %
   d'un cœur, RAM 2,6-2,9 Go, zéro log après `apply: layout`.
   Suspect : tâche JS en boucle non bornée (timers/rAF/microtasks)
   post-rendu qui affame le runtime ; aucun budget temps
   n'interrompt la rafale. Bloquant pour tout vol multi-onglets —
   instrumenter un watchdog de budget JS global ou isoler la tâche.
   Daemon1 : `_vol34_daemon.log`, daemon2 : `_vol34_daemon2.log`
   (timestamps cités §2).
2. **Lazyload `data-src` jamais résolu → images absentes en masse**
   (Marmiton, 216 `data-src`, 0 `src` sur les vignettes) : le site
   dépend du JS (IntersectionObserver) pour hydrater `src`. Comme le
   JS meurt (`heap memory limit` ×3), les images restent grises.
   Piste : résolution `data-src → src` de repli côté moteur quand
   `loading="lazy"` + pas de src, ou support IntersectionObserver.
   (capture `vol34_marmiton.png`)
3. **`RangeError: heap memory limit exceeded` ×3 au boot Marmiton**
   (flashb.id, unpkg mealz-components, afcdn concat) : le plafond
   heap jscore tue les scripts courants — vérifier le seuil face aux
   bundles pubs/widgets réels (>150 Ko évalués). Console
   `vol34_marmiton_console.json`.
4. **Erreur de parse jscore : `` `...` outside a call or array
   literal``** sur le chunk `_app` Next.js de BBC Good Food (×2) —
   le spread/rest dans une position non supportée casse le boot
   Next entier ; console `vol34_bbcgoodfood_console.json`. Trou de
   grammaire ES à combler (probablement spread dans destructuring/
   template/objet). Sévérité haute : un seul token tue tout le runtime.
5. **Cascade BBC Good Food absente malgré 4 `rel=stylesheet`
   présents** (base/components/utilities/services du thème
   `im-bbcgoodfood`) : rendu quasi-brut, texte clippé, méga-menu
   déroulé sur ~3 000 px. Déterminer si échec fetch (sonde `/net`)
   ou rejet massif de règles CSS modernes. (capture
   `vol34_bbcgoodfood.png`)
6. **Menus overlays peints ouverts + collisions de colonnes** (750g) :
   le menu flyout est rendu déplié (display géré par JS/hover) avec
   chevauchements (« Emissions TV et divertissementQue faire
   avec ? ») et textes tronqués à largeur de colonne. Même symptôme
   BBC (méga-menu en liste). Sévérité moyenne.
   (captures `vol34_750g.png`, `vol34_bbcgoodfood_scroll.png`)
7. **Header sticky qui recouvre le contenu scrollé** (750g : barre
   verte figée, titres teasers rognés en haut de frame au scroll) —
   `position: fixed/sticky` peint sans clip du contenu dessous.
   Sévérité faible-moyenne. (capture `vol34_750g_scroll.png`)
8. **Points positifs mesurés** : clics → vraies navigations ×3
   (album Marmiton, collection BBC, article 750g) ; images `src`
   directes peintes (GiFi, piments, sardines) ; scroll fluide ;
   console 750g vierge ; reset console par navigation confirmé ;
   `doc_h` non clampé jusqu'à 13,7 kpx.

## Arrêt

- Daemon 1 (PID 17164) : tué après blocage allrecipes — onglets
  non supprimables (API morte).
- Daemon 2 (PID 14896) : tué après second blocage allrecipes —
  idem. Onglets 1-2 (BBC, 750g) restaient sains jusqu'au gel.
- Autres daemons utilisateur (PID 18044, 7988, 13992) : intacts.
- Aucun code modifié. Artefacts : captures `vol34_*.png`, logs
  `vol34_*_{dom,layout,console}.json` dans `docs/Rapport vol/`.

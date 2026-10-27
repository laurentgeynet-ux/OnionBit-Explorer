# Rapport de vol — AGENT15 · sites e-commerce internationaux

- **Date** : 2026-10-26
- **Daemon** : `C:/obe-vol-head/target/release/explorer-daemon.exe --port 9399`
  (`EXPLORER_VIEWPORT=1366x900`)
- **Cibles** : ebay.fr, aliexpress.com, amazon.fr, cdiscount.com
- **Captures** : `captures/vol15_*.png` — **Logs** : `logs/vol15_*`

## Synthèse

| Cible | Résultat | Peinture | Console |
|---|---|---|---|
| ebay.fr | HTTP 403 (anti-bot) | page d'erreur moteur (98 % blanc) | `HTTP 403` |
| aliexpress.com | redirection `/punish` (anti-bot AWSC) | page quasi blanche (99,9 % blanc, 42 couleurs) | 2 exceptions JS |
| amazon.fr | chargé (~40 s), titre réel | contenu peint (48 % non blanc, bandeau `#232f3e`) | aucune erreur |
| cdiscount.com | chargé (>3 min de fetch) | contenu dense peint (50 % non blanc, doc 9 772 px) | 4 erreurs JS |

## 1. https://www.ebay.fr (onglet 1)

- **URL finale** : `https://www.ebay.fr/` — titre « Erreur de chargement ».
- **Console verbatim** : `{"console":["HTTP 403"]}`.
- **Constats mesurés** : navigate renvoie `{"error":{"handled":true,"message":"HTTP 403"}}`.
  Frame 1366×900 capturée (1,86 Mo) mais 98,4 % de pixels blancs, 243 couleurs :
  page d'erreur interne uniquement. `/dom` et `/layout` → 404 stable (pas de
  document).
- **Classification** : blocage amont — Akamai/PerimeterX refuse le client
  (User-Agent/empreinte TLS). Pas un trou moteur.
- **Sévérité** : bloquant (hors périmètre moteur ; politique réseau de l'app).

## 2. https://www.aliexpress.com (onglet 2)

- **URL finale** : `https://www.aliexpress.com///_____tmd_____/punish?x5secdata=…`
  — page de punition Alibaba (sufei-punish).
- **Console verbatim** :
  - `exception js : TypeError: cannot read \`toString\` of Undefined` —
    `g.alicdn.com/AWSC/CAPTCHA/0.0.1/awsc.js:1:1777`
  - `exception js : TypeError: \`toDataURL\` is not a function (on
    HTMLCanvasElement)` — `g.alicdn.com/bsop-static/sufei-punish/0.1.129/build/punishpage.min.js:1:100156`
- **Constats mesurés** : DOM 204 Ko présent mais corps vide (0 `<img>`, 0 prix) ;
  frame 1366×900 = 99,9 % blanc, 42 couleurs seulement. Le challenge JS
  (canvas fingerprint) ne peut pas s'exécuter : `HTMLCanvasElement.toDataURL`
  manquant → page blanche définitive.
- **Classification** : anti-bot + trou API canvas (`toDataURL`). Double cause :
  même avec `toDataURL`, la punition resterait probable (empreinte moteur).
- **Sévérité** : bloquant.

## 3. https://www.amazon.fr (onglet 3)

- **URL finale** : `https://www.amazon.fr/` — titre complet Amazon.
  Navigation ≈ 40 s (réponse navigate très lente).
- **Console verbatim** : `pas de journal` — zéro erreur JS.
- **Constats mesurés** :
  - DOM 2,09 Mo ; 62 `<img>` (dont `images-eu.ssl-images-amazon.com`), 256
    `href`, textes nav réels (« Vos listes d'envies », « Votre compte Amazon
    Prime »…) — page servie est la vraie home, **pas** un captcha.
  - 15 symboles `€` dans le DOM mais **0 occurrence `a-price`** : les prix des
    carrousels produits sont construits en JS/lazy — absents du DOM statique.
  - Frame 1366×4050 (doc_h mesuré 2 744 px à la navigation, peint plus grand) :
    48 % non blanc, 6 861 couleurs, le bandeau Amazon `#232f3e`/`#131a22`
    occupe ~16 % du haut — rendu substantiel.
  - **Latence extrême** : chaque `/frame` a exigé 2–5 min (sous-resources
    encore en vol) ; plusieurs requêtes `/api/tabs` ont expiré pendant le
    chargement — famine observable du flux unique pendant les gros fetches.
  - Interactions `scroll` ×2 acceptées, mais la frame post-scroll n'a jamais
    abouti (timeouts à 280 s) — pas de verdict post-scroll.
- **Classification** : succès relatif — parsing/CSS/raster tiennent la home
  Amazon ; JS silencieux (aucune exception) mais prix/carrousels non
  matérialisés ; **perf** = point dur.
- **Sévérité** : majeure (latence), contenu partiel (prix absents).

## 4. https://www.cdiscount.com (onglet 4)

- **URL finale** : `https://www.cdiscount.com/` — titre « Cdiscount ».
  Navigation > 3 min (premier essai expiré à 180 s côté curl ; le fetch a
  continué et abouti).
- **Console verbatim** (post-chargement + scroll) :
  - `exception js : ReferenceError: $ is not defined` — `inline:34:12:124149`
  - `exception js : Error: rootMargin must be specified in pixels or percent` —
    `./getElementToHydrate-K9Znta0z.js:8:71970` (module
    `legacyPcResources-BcpdIAWZ.js`) — **IntersectionObserver** : option
    `rootMargin` invalide/non gérée
  - `syntaxe js 2:157505 : expected \`;\`` — `…lyfnh.io/tag/xcare.js` (script
    obfusqué rejeté par le parseur)
  - `exception js : TypeError: cannot read \`dataset\` of Undefined` — `(2:47)`
- **Constats mesurés** :
  - DOM 1,12 Mo ; 164 `<img>`, 411 `href`, 76 prix `€` — catalogue réel
    présent (« Les incontournables », « Nos meilleures offres », catégories…).
  - Frame 1366×9772 peinte (30,7 Mo PNG) : ~50 % non blanc, liens bleus
    `#3732ff` marqués — grille produits et prix présents à l'écran.
  - `/layout` a demandé 3 tentatives (000, 000, 200) ; `/frame` post-scroll :
    4 timeouts successifs (240–300 s) puis daemon **complètement gelé** :
    `/api/tabs`, `DELETE /api/tabs/4` et `/frame` onglet 3 tous en timeout —
    le JS continu de Cdiscount affame durablement le pipeline (single-writer).
- **Classification** : succès de peinture + trous JS (jQuery/`$` absent,
  `IntersectionObserver.rootMargin`, parseur sur code obfusqué, `dataset`) +
  **gel du daemon sous JS continu** — le constat le plus grave du vol.
- **Sévérité** : critique (gel), majeure (erreurs JS).

## Constats actionnables

1. **CRITIQUE — famine/gel du daemon** : sur pages lourdes à JS continu
   (cdiscount après scroll, et dans une moindre mesure amazon), le flux unique
   ne rend plus la main à l'API : `/api/tabs`, `/frame`, `DELETE` expirent
   >4 min. Borner les budgets JS/fetch ou préempter la boucle pour servir
   l'API. Repro : naviguer cdiscount.com, scroll ×2, puis `GET /frame`.
2. **MAJEUR — `IntersectionObserver.rootMargin`** : lève `Error: rootMargin
   must be specified in pixels or percent` (cdiscount, hydrate React) —
   accepter les unités `px`/`%` et probablement les valeurs multiples.
3. **MAJEUR — `HTMLCanvasElement.toDataURL` absent** (aliexpress punish) —
   bloque tout challenge anti-bot fondé sur canvas fingerprint.
4. **MOYEN — global `$`/jQuery non défini** (cdiscount inline:34) : un bundle
   jQuery n'a pas été chargé ou exécuté avant un script inline — tracer
   l'ordre/exécution des `<script>` externes bloquants.
5. **MOYEN — parseur JS** : `syntaxe js 2:157505 : expected \`;\`` sur
   `xcare.js` obfusqué (cdiscount) — script publicitaire perdu ; vérifier
   s'il s'agit d'une syntaxe non couverte ou d'un slice de chunk.
6. **MOYEN — `element.dataset`** : `cannot read \`dataset\` of Undefined`
   (cdiscount 2:47) — `dataset` non implémenté sur `Element` ou `undefined`
   sur un noeud retourné vide.
7. **MINOR — perf navigation** : amazon ~40 s, cdiscount >3 min avant retour
   de `/navigate` ; envisager de rendre la main dès le document principal.
8. **HORS MOTEUR — anti-bot** : ebay.fr 403, aliexpress punish — documenter
   comme limite de politique réseau (empreinte client), pas comme régression.

## Arrêt

- Onglets : `DELETE` tenté sur l'onglet 4 → timeout (daemon gelé) ; fermeture
  propre impossible en fin de vol.
- Daemon (PID 7396) à tuer via `kill` — voir journal de session.
- Aucun code modifié.

## Annexes

- Captures : `vol15_ebay_home.png` (1366×900), `vol15_aliexpress.png`
  (1366×900), `vol15_amazon.png` (1366×4050, 15 Mo), `vol15_cdiscount.png`
  (1366×9772, 31 Mo).
- Logs : `vol15_*_dom.txt`, `vol15_*_layout.txt`, `vol15_*_console*.txt`
  sous `docs/Rapport vol/logs/` — manquent `vol15_ebay_dom/layout`
  (pas de document, 403) et `vol15_cdiscount_scroll.png` (daemon gelé).

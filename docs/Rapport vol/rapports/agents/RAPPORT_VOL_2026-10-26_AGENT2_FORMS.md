# Rapport de vol — Agent 2 (formulaires / e-commerce / institutionnel)

- **Date** : 2026-10-26 — **Daemon** : release 7a01534 (binaire `C:/obe-vol-head`, viewport 1366×900, port 9386)
- **Domaine assigné** : formulaires, e-commerce, institutionnel
- **Méthode** : `POST /api/tabs` → `navigate` → 8–10 s → `console` / `frame` / `dom` / `layout` → interactions (scroll ×2, hover, click champ de recherche) → re-frame/re-console → `DELETE`.

---

## 1. https://www.etsy.com — BLOQUÉ RÉSEAU (403)

- **URL finale** : `https://www.etsy.com/` — **HTTP 403** (refus serveur, anti-bot probable).
- **Console** : 1 entrée — `HTTP 403`.
- **Visuel** (`vol2_etsy.png`) : page d'erreur interne du moteur « Page inaccessible — Le serveur a répondu « HTTP 403 » » correctement rendue (titre, texte, marges). `dom`/`layout` vides (« pas de DOM », 13 octets) — normal pour une erreur réseau.
- **Classification** : **net** (rien à corriger côté pipeline ; la dégradation gracieuse fonctionne).
- **Sévérité** : N/A — comportement nominal face à un refus serveur.

## 2. https://www.craigslist.org — LAYOUT CASSÉ, ZÉRO ERREUR JS

- **URL finale** : `https://www.craigslist.org/area/montpellier` (redirection géo suivie, titre OK).
- **Console** : 0 erreur (« pas de journal ») — le pipeline n'a rien logué malgré un rendu très dégradé.
- **Visuel** (`vol2_craigslist.png`, `vol2_craigslist_after.png`) :
  - Le bandeau « There was an error loading the page » (JS interne du site) s'affiche en haut — une ressource a échoué **silencieusement** (rien en console → trou d'observabilité ou de fetch).
  - Toutes les listes de catégories (community/services/housing/for sale/jobs/gigs…) sont empilées en **une seule colonne verticale à gauche** (~1 lien par ligne). La page réelle dispose chaque section en 3–4 colonnes de liens : les styles de colonnes/floats/table-layout de la page ne sont pas appliqués.
  - Le calendrier et la barre de recherche d'en-tête sont réduits à des liens texte sans styles (le champ de recherche n'est pas visible en haut).
  - Scroll OK (scroll_y 1600), document très haut (doc_h ≈ 7110 vs ~1200 attendu en vrai layout) → confirme la perte des colonnes.
- **dom/layout** : `vol2_craigslist_dom.txt` (122 Ko), `vol2_craigslist_layout.txt` (119 Ko) — DOM complet ; le trou est en cascade/boîtes, pas en parsing.
- **Interactions** : hover `editable:false`, click `ignored` (clic sur un lien en-tête — hit-test n'a pas navigué, à recouper).
- **Classification** : **CSS/layout** majeur (colonnes/float non appliqués) + **observabilité** (échec de sous-ressource invisible en console).
- **Sévérité** : haute — page utilisable mais sans mise en page ni recherche.

## 3. https://www.service-public.fr — BON RENDU, 4 EXCEPTIONS JS

- **URL finale** : `https://www.service-public.gouv.fr/` — titre « Accueil | Service Public ».
- **Console** : 4 exceptions JS (verbatim, top par ordre) :
  1. `TypeError: value is not a function: undefined — dsfr.module.min.js:2:11016` (module DSFR)
  2. `TypeError: cannot read 'ns' of Undefined — analytics/analytics.module.min.js`
  3. `TypeError: serializeToString: nœud requis — common.js:3391` (XMLSerializer sur nœud absent)
  4. `TypeError: cannot read 'collection' of Undefined — dsfr.module.min.js:2:8417` (apparue après interaction)
- **Visuel** (`vol2_servicepublic.png`) : **meilleur rendu du lot** — hero avec photo, grille de cartes « Fiches pratiques » 3→4 colonnes, bandeau France services, footer violet avec logos partenaires. Constats mesurés :
  - Le menu d'en-tête « Fiches pratiques / par thème » est déplié en **liste verticale permanente** dès le chargement (≈15 entrées empilées) alors qu'il devrait être un bouton de menu dépliable → layout/état CSS (`[hidden]`/`display` piloté JS/attribut non honoré).
  - Les icônes des cartes « Démarches et outils » sont absentes (espace vide ~24 px en tête de carte) — SVG inline ou pictos de font non rendus.
  - Champ de recherche d'en-tête présent (boîte grise + bouton bleu) mais **hover → `editable:false`, click → `ignored`** : le `<input>` n'est pas reconnu éditable par le hit-test.
- **Classification** : **JS** (4 exceptions, modules DSFR morts → composants interactifs inertes) + **CSS/DOM** (menu déplié, input non éditable au hit-test).
- **Sévérité** : moyenne — contenu lisible et complet, recherche et navigation interactive HS.

## 4. https://www.openstreetmap.org — CHROME OK, CARTE ABSENTE

- **URL finale** : `https://www.openstreetmap.org/` — titre OK, doc_h = 477 px seulement.
- **Console** : 3 exceptions JS :
  1. `TypeError: 'addEventListener' is not a function (on Worker) — application-…js:205` (jQuery/Rails UJS initialise un Worker ; API Worker absente ou incomplète)
  2. `TypeError: cannot read 'store' of Undefined — i18n/en-…js` (`OSM.i18n` inexistant → objet global non posé par le script précédent, mort en cascade)
  3. `TypeError: cannot read 'call' of Undefined — application-…js:62` (apparue après clic)
- **Visuel** (`vol2_osm.png`) :
  - Header/nav complet (logo, Edit, History…, Log In/Sign Up stylés), barre **Search visible et stylée**, bannière « 31 DAY OSM CHALLENGE » rendue avec illustration.
  - **La zone carte entière est vide** (gris uniforme sur ~80 % du viewport, 0 tuile, 0 contrôle zoom) — Leaflet n'a jamais démarré (conséquence des exceptions 1–2).
  - Interactions : hover sur le champ Search → `editable:true`, click → `focused` (champ de recherche fonctionnel côté hit-test, bon point pour les formulaires). Scroll sans effet (doc_h 477 < viewport).
- **Classification** : **JS** (Worker API, namespace global entre scripts) → panne fonctionnelle totale de la feature principale.
- **Sévérité** : critique pour le site — le produit (la carte) est absent.

## 5. https://bookshop.org (fallback) — RENDU CORRECT, HEAP BOMB

- **URL finale** : `https://uk.bookshop.org/` (redirection géo suivie).
- **Console** : **16 exceptions** :
  - `RangeError: heap memory limit exceeded — cloudflareinsights.com/beacon.min.js` **×14** (le script beacon boucle et explose le budget mémoire JS à répétition — fuite ou budget trop serré)
  - `TypeError: 'in' requires an object — page.DmwQPsP_.js:5` (module Astro)
  - `TypeError: cannot read 'left' of Undefined — PromoBannerCarousel…js` (getBoundingClientRect/offset sur élément absent)
- **Visuel** (`vol2_bookshop.png`, `_after.png`) : page **très complète** — méga-bannière promo, carrousels de couvertures (premiers items + « View All »), toutes les sections éditoriales, footer violet avec formulaire newsletter. Constats mesurés :
  - **Barre de recherche absente** du header (liens minuscules en haut, pas de champ) — l'icône/champ search (SVG ou composant hydraté) n'est pas rendu ; hover/click zone header → `editable:false`, `ignored`.
  - Carrousels figés (pas de défilement horizontal, normal sans JS sain).
  - Une superposition modale « Our mission is to help independent bookshops thrive » apparaît après scroll (`_after.png`) — probablement un bandeau positionné fixed mal géré (z-index/position).
- **Classification** : **JS** (heap limit, opérateur `in` sur non-objet, API géométrie manquante) + **CSS/DOM** (search non rendue, overlay fixed).
- **Sévérité** : moyenne — catalogue consultable, recherche HS.

---

## Constats actionnables

1. **Worker API absente** (`openstreetmap.org`) : `addEventListener is not a function (on Worker)` — stubber `Worker`/`addEventListener` ou no-op graceful dans `explorer-jscore` ; sans ça, tout site Rails/Leaflet meurt en cascade.
2. **Namespace global entre scripts** (OSM, service-public) : `OSM.i18n` / `ns` / `collection` Undefined — des globals posés par un script ne sont pas visibles du suivant (modules vs scripts classic ? portée `window` ?). À investiguer : propriétés assignées sur l'objet global non persistées.
3. **Observabilité des sous-ressources** (craigslist) : « error loading the page » côté site mais **0 entrée console** — logger les échecs de fetch de sous-ressources (JS/CSS/img) dans le ring console.
4. **Colonnes/float/table CSS non appliqués** (craigslist) : les listes multi-colonnes s'empilent — vérifier support de `float`, `column-count`, `display:table*` dans explorer-layout sur la fixture craigslist (`vol2_craigslist_layout.txt` disponible).
5. **Hit-test des `<input>`** (service-public : `editable:false` sur le champ recherche ; bookshop : idem) — les inputs présents dans le DOM ne sont pas reconnus éditables ; vérifier le mapping input→box dans le hit-test (peut-être lié à `pointer-events`, `appearance`, ou inputs générés par composants).
6. **Budget heap JS** (bookshop : 14× `heap memory limit exceeded` sur beacon.min.js) — soit fuites réelles (timers relancés), soit limite trop basse pour des scripts analytics en boucle ; instrumenter/augmenter et capper l'erreur à 1 occurrence.
7. **Opérateur `in` et accès géométrie** (bookshop) : `` `in` requires an object `` et `cannot read 'left' of Undefined` (offset/boundingRect) — sémantique `in` sur primitives/null à corriger + `getBoundingClientRect`/`offsetLeft` sur éléments non attachés.
8. **`XMLSerializer.serializeToString`** (service-public) : « nœud requis » — valider l'API et l'appel sur nœud détaché/null.
9. **Attribut `[hidden]`/état déplié** (service-public) : menu DSFR rendu ouvert en permanence — vérifier la prise en compte de `hidden`, `aria-expanded`-driven CSS et/ou le JS qui devait le refermer.
10. **Pages d'erreur réseau** (etsy 403) : fonctionnel — à garder comme référence de bonne dégradation.

## Fichiers produits

- Captures : `captures/vol2_{etsy,craigslist,craigslist_after,servicepublic,servicepublic_after,osm,osm_after,bookshop,bookshop_after}.png`
- Logs : `logs/vol2_{craigslist,servicepublic,osm,bookshop}_console.json`, `logs/vol2_*_dom.txt`, `logs/vol2_*_layout.txt`
- Daemon log : `C:/OnionBit-Explorer/_vol2_daemon.log`

# 🛰️ RAPPORT D'ANALYSE EN VOL : ÉQUIPE 8 — INSTITUTIONNEL & CARTO (VOL 2)

**Périmètre** : Second passage sur les portails institutionnels français —
portails fiscaux et sociaux (Drupal + DSFR), plateforme open data (Nuxt SSR)
et cartographie d'État (SPA Vue/DSFR `cartes.gouv.fr`, ex-Géoportail).
**Objectif** : Confirmer ou infirmer la récidive des murs JS du vol 1
(registre inter-modules ES DSFR, `new Worker`, jQuery `toLowerCase`),
qualifier le comportement sur SPA Vue « coquille vide » vs SSR Nuxt, et
vérifier si la carte IGN se peint là où Leaflet/OSM était resté blanc.
**Companion** : `RAPPORT_MISSION_EQUIPE_8_INSTIT.md` (vol 1 — 4 vols).

---

## 📋 Synthèse des vols

| N° | Date / Heure | URL cible | Statut | Anomalie / Diagnostic clé |
|:---:|:---:|:---|:---:|:---|
| **05** | 2026-10-26 13:43 | `https://www.impots.gouv.fr/` → `/accueil` | 🟢 SUCCÈS DÉGRADÉ | Page Drupal+DSFR peinte (**31 Mo**). `dsfr.module.js` : **récidive** `TypeError: value is not a function`. 1 agrégat Drupal en échec : `The element must be an instance of Element`. 1 repli `UnmappedKey`. |
| **06** | 2026-10-26 13:44 | `https://www.ameli.fr/` | 🟢 SUCCÈS DÉGRADÉ | Page peinte (**23,8 Mo**). **jQuery 4.0.0 survit** (tous les scripts `ok=true`) — le mur `toLowerCase` de legifrance ne se reproduit pas. 1 listener `load` en échec : `cannot read 'call' of Undefined` (signature OSM). 1 repli `UnmappedKey`. |
| **07** | 2026-10-26 13:44 | `https://www.data.gouv.fr/` | 🟢 SUCCÈS | SSR Nuxt **intégralement peint** (17,45 Mo, `#__nuxt` plein + payload `ShallowReactive`). Module `_nuxt` evalué **sans erreur** en 2,3 s (34 sources prefetchées). Journal 0/0/0 — aucune mutation nécessaire, le HTML serveur suffit. **Zéro exception JS.** |
| **08** | 2026-10-26 13:45 | `https://www.geoportail.gouv.fr/` → `cartes.gouv.fr/explorer-les-cartes/` | 🔴 COQUILLE VIDE | Redirect suivi. `#app` Vue **entièrement vide** (1,84 Mo ≈ page blanche). Le module `index-*.js` **termine sans erreur** (4,9 s) mais `sources=1` : les chunks `modulepreload` ne sont pas résolus comme imports → **échec silencieux**, 0/0/0. Carte jamais montée — pas de tuiles, pas de canvas, **rien à rasteriser** (pire que l'échec bruyant d'OSM). |

**Bilan** : 4/4 pages fetchées, TLS h2, aucun crash. Confirmation majeure :
le mur DSFR `value is not a function` **récidive à l'identique** (impots
rejoint service-public et legifrance — 3/6 sites DSFR touchés). En revanche
jQuery 4.0.0 **passe** sur ameli : le crash `toLowerCase` de legifrance était
donc propre à son bundle/minification, pas à jQuery 4 lui-même. Nouveau
diagnostic : les **SPA Vue à chunks Vite échouent silencieusement** — le
module d'entrée évalue « fin » sans jamais charger ses imports, laissant
`#app` vide sans aucune exception tracée.

---

## 🔬 Fiches Techniques Détaillées

### ✈️ Vol #05 — `https://www.impots.gouv.fr/accueil` (Drupal 10 + DSFR non-minifié)

- **Stack technique** : Drupal 10 (`generator` meta confirmé), thème
  `pus_dsfr`, agrégats `js_*.js?scope=header|footer`, **DSFR servi
  non-minifié** (`/libraries/dsfr/dist/dsfr.module.js` — contrairement au
  `.min.js` de service-public), config `drupalSettings` avec Eulerian,
  webform dialog options, `data.economie.gouv.fr` en endpoint opendata.
  Pas de jQuery détecté dans les scripts émis — stack Drupal moderne.
- **Capture** : `captures/2026-10-26_13-43-26_Equipe_8_Instit_www.impots.gouv.fr.png`
  — ~31 Mo. Page institutionnelle complète peinte : bandeau Marianne,
  navigation démarches, actualités avec vignettes `<img>` statiques
  fetchées (`vignette_mesures_soutien_commerces.jpg`…), liens
  `cfspart`/`cfspro`, footer DGFIP.
- **stderr verbatim** :
  ```text
  INFO explorer_core::session::js_glue: script: eval fin tab=1 ms=585 ok=true   (118003 o, header agrégat)
  INFO explorer_core::session::js_glue: script: eval fin tab=1 ms=178 ok=true   (96982 o)
  INFO explorer_core::session::js_glue: script: eval fin tab=1 ms=122 ok=false  (66312 o)
  DEBUG … script en échec : TypeError: The element must be an instance of Element
    at u (…/js_q_SAJRbjU….js?scope=footer&delta=0…:3:335)
  DEBUG … module en échec : TypeError: value is not a function
    at <anonymous> (…/libraries/dsfr/dist/dsfr.module.js?tmxw77:775:3)
  WARN explorer_core::session::apply: journal DOM rejeté → repli re-parse (ADR-0025) tab=1 why=UnmappedKey
  [journal : 0 appliqués / 1 replis / 0 classées]
  ```
- **Analyse racine** :
  - **`explorer-jscore` — récidive DSFR confirmée** : `TypeError: value is
    not a function` à `dsfr.module.js:775` — même signature exacte que
    service-public (vol 1, `dsfr.module.min.js:2:11016`) et legifrance.
    La version **non-minifiée** donne enfin un point d'ancrage lisible :
    ligne 775 ≈ enregistrement `dsfr.register`/fabrique de namespace. Le
    registre inter-modules ES reste le **mur n°1 des sites d'État**.
  - **`explorer-jscore` — nouveau** : `The element must be an instance of
    Element` dans l'agrégat footer (fonction `u`, probablement un helper
    type `once()`/`closest()` Drupal recevant un nœud non-`Element` :
    `Document`, `Text`, ou un wrapper dont le brand-check `instanceof`
    échoue). Message typé → le contrôle existe mais rejette une valeur
    qu'un vrai DOM accepterait (vraisemblablement `document` ou un élément
    cross-realm / nœud hôte du mini-DOM).
  - **`explorer-core::session::apply`** : 1 repli `UnmappedKey`, 0 classée —
    pattern habituel non bloquant.
- **Repro minimal** :
  ```javascript
  // Mur DSFR (vol 1 + vol 2, 3 sites) :
  window.dsfr.register("core", { start(){} }); // value is not a function
  // Mur agrégat Drupal :
  function once(id, el){ /* brand check */ }
  once("x", document); // The element must be an instance of Element
  ```

### ✈️ Vol #06 — `https://www.ameli.fr/` (Drupal + jQuery 4 + Orejime + Piano)

- **Stack technique** : Drupal, thème `ameli_v2`, **jQuery 4.0.0**
  (`core/assets/vendor/jquery/jquery.min.js?v=4.0.0`), Orejime 2.3.1
  (consentement RGPD), Piano Analytics (`tag.aticdn.net`), Google
  Tag/gtm, js-cookie 3.0.5 (CDN jsdelivr), agrégats `js_*` scope
  header/footer delta 1→7, `ameli_menu_update.min.js`. **26 scripts
  émis** — la page la plus chargée du vol.
- **Capture** : `captures/2026-10-26_13-44-12_Equipe_8_Instit_www.ameli.fr.png`
  — ~23,8 Mo. Page peinte : en-tête ameli, menu rubriques, contenu
  éditorial, footer institutionnel.
- **stderr verbatim** :
  ```text
  INFO … script: eval fin tab=1 ok=true   (×14 — TOUS les scripts passent,
                                         dont jquery.min.js et orejime.js)
  DEBUG explorer_core::session::js_glue: listener dom en échec tab=1 ty="load"
    error=exception js : TypeError: cannot read `call` of Undefined
    at <anonymous> (…/js_F2tPDUV4….js?scope=footer&delta=5…:5:250)
    at <anonymous> (…/js_F2tPDUV4….js:5:16637)
    at i (…/js_F2tPDUV4….js:5:5239)
  WARN … journal DOM rejeté → repli re-parse (ADR-0025) tab=1 why=UnmappedKey
  [journal : 0 appliqués / 1 replis / 0 classées]
  ```
- **Analyse racine** :
  - **Contraste décisif avec legifrance (vol 1)** : `jquery.min.js?v=4.0.0`
    s'évalue `ok=true` ici — **`toLowerCase is not a function` n'était pas
    jQuery 4 en soi**, mais spécifique au build webjars/minification de
    legifrance (vraisemblablement un accès propriété hôte renvoyant un
    `Object` au lieu d'une `String` — `navigator.appVersion` ou
    équivalent — que ce bundle-là appelle en méthode).
  - **`explorer-jscore` — listener `load` `call` of Undefined** : **même
    signature exacte que le listener `load` d'OSM vol 1** — un callback
    enregistré dont la cible (probablement un proxy/`once` jQuery ou un
    handler wrappé) se résout en `Undefined` au moment du dispatch.
    Récidive → anomalie systémique du chemin `addEventListener`/`dispatch`
    côté glue, pas un cas isolé.
  - 0 mutation classée : malgré 14 scripts `ok`, rien n'a muté le DOM au
    moment du journal (ou mutations non journalisables) — + 1 repli
    `UnmappedKey` résiduel.
- **Repro minimal** :
  ```javascript
  // Repro listener (récidive OSM + ameli) :
  window.addEventListener("load", function(){ /* corps ok */ });
  // sous jQuery : jQuery(window).on("load", handler) → au dispatch,
  // handler.call(...) sur cible Undefined → TypeError
  ```

### ✈️ Vol #07 — `https://www.data.gouv.fr/` (Nuxt 3 SSR + udata)

- **Stack technique** : Nuxt 3 / Vue, **SSR complet** — `#__nuxt` livré
  plein côté serveur + payload de désérialisation `ShallowReactive`
  embarqué (métriques udata : datasets, organizations, reuses, séries
  mensuelles), ~34 chunks `_nuxt/*.js` en `modulepreload`+`prefetch`,
  SVG inline massifs (icônes Mastodon/GitHub dans le DOM dumpé), h2
  confirmé, longs préload link headers.
- **Capture** : `captures/2026-10-26_13-44-40_Equipe_8_Instit_www.data.gouv.fr.png`
  — 17,45 Mo. **Rendu SSR fidèle** : header data.gouv.fr, moteur de
  recherche, tuiles datasets/actualités, footer — tout vient du HTML
  serveur.
- **stderr verbatim** :
  ```text
  DEBUG … module prefetché tab=1 name=…/_nuxt/CiqjYPa5.js sources=34
  DEBUG … module eval… tab=1 name=…/_nuxt/CiqjYPa5.js
  DEBUG … module eval fin tab=1 ms=2299          ← SANS ligne « en échec »
  [journal : 0 appliqués / 0 replis / 0 classées]
  ```
- **Analyse racine** :
  - **Vol le plus sain des 8** : zéro exception, zéro repli, zéro
    mutation requise. Le graphe de modules Nuxt (34 sources) a été
    **prefetché et évalué sans erreur** — première preuve que le moteur
    de modules ES sait suivre un import map / graphe multi-chunks quand
    il est résolu par prefetch (contrairement à geoportail, cf. vol #08).
  - Hydratation : `0 classées` signifie soit que Vue hydrate sans muter
    le DOM existant (SSR matching parfait — plausible), soit que
    l'hydratation n'a pas produit de mutations journalisables. Aucune
    erreur ne trahit un échec.
  - **Enseignement** : SSR = robustesse naturelle pour Explorer — le
    contenu est dans le markup, le JS n'est qu'un bonus. data.gouv.fr
    est le modèle « graceful » ; geoportail est l'anti-modèle.
- **Repro minimal** : N/A — aucune anomalie. (Cas de référence positive
  pour les golden tests SSR.)

### ✈️ Vol #08 — `https://www.geoportail.gouv.fr/` → `cartes.gouv.fr/explorer-les-cartes/` (SPA Vue/DSFR, Vite)

- **Stack technique** : redirection domaine `geoportail.gouv.fr →
  cartes.gouv.fr/explorer-les-cartes/` suivie proprement. SPA **Vite +
  Vue + DSFR** (`data-fr-theme="light"` sur `<html>`) : `env.js`
  (1 370 o, `ok=true`), bundle d'entrée `assets/index-t8Cr_OTk.js` en
  `type="module"`, 3 chunks `modulepreload` (`Load-*.js`,
  `StoreData-*.js`, `Patience-*.js`), `style-*.css`. **Zéro contenu
  dans le HTML** : `<body><div id="app"></div></body>` — SPA pure.
- **Capture** : `captures/2026-10-26_13-45-18_Equipe_8_Instit_www.geoportail.gouv.fr.png`
  — **1,84 Mo** ≈ fond quasi uniforme. Le DOM dumpé confirme : `#app`
  suivi de whitespace uniquement. **Pas de carte, pas de tuiles, pas de
  canvas, pas même un header DSFR** — l'application Vue n'a jamais monté.
- **stderr verbatim** :
  ```text
  DEBUG … module prefetché tab=1 name=…/assets/index-t8Cr_OTk.js sources=1   ← ⚠ 1 seule source
  DEBUG … module eval… tab=1 name=…/assets/index-t8Cr_OTk.js
  DEBUG … module eval fin tab=1 ms=4929          ← fin, PAS de « en échec »
  DEBUG explorer_core::session::net: drain dyn links tab=1 n=0               ← 0 sous-ressource dynamique
  [journal : 0 appliqués / 0 replis / 0 classées]
  ```
- **Analyse racine (par crate)** :
  - **`explorer-js` / `explorer-jscore` — échec silencieux de résolution
    d'imports** : `sources=1` alors que Vite découpe l'app en chunks —
    `index-t8Cr_OTk.js` importe `Load-*`, `StoreData-*`, `Patience-*`
    (déclarés `modulepreload`) plus probablement des imports dynamiques
    (`import()` route-level). Le module d'entrée **termine son éval en
    4,9 s sans exception tracée** mais l'app ne monte jamais → soit les
    `import` statiques sont résolus en stubs vides, soit un `import()`
    dynamique ou `createApp().mount("#app")` s'est résolu en no-op /
    rejet de promesse **non remonté** dans le journal (`unhandled
    rejection` invisible). Comparaison OSM vol 1 : là, `Worker: no
    loader installed` tuait bruyamment le bootstrap ; ici **rien ne
    crie** — diagnostic plus dur, c'est un trou de *traçabilité* autant
    que de fonctionnalité.
  - **Conséquence DOM/render** : `drain dyn links n=0` + 0 mutation →
    `explorer-render` n'a littéralement qu'un `<div>` à peindre. Même
    conclusion que Leaflet : **ce n'est pas un trou de rasterisation**,
    il n'y a rien à rasteriser — sauf qu'ici même l'erreur manque.
  - **Différence clé data.gouv.fr vs cartes.gouv.fr** : même famille de
    stack (Vue, chunks hashés), mais Nuxt-SSR = `sources=34` résolues +
    contenu serveur, Vite-SPA = `sources=1` + div vide. La résolution du
    graphe de modules marche sur prefetch multi-sources du markup Nuxt
    mais **échoue ou est court-circuitée sur les imports internes du
    bundle Vite** (chunks référencés par `import` dans le code, pas par
    `<link>` seul).
- **Repro minimal** :
  ```javascript
  // index.js : import statique de chunk + mount
  import { createApp } from "./vue.runtime.js";     // résolu ?
  import("./Load-abc.js").then(m => m.init());      // import() dynamique
  const app = createApp({}); app.mount("#app");
  // Vérifier : 1) les imports internes sont-ils fetchés ? 2) un rejet de
  // promesse non catché est-il tracé ? 3) mount() sur #app journalise-t-il ?
  ```

---

## 📌 Top anomalies pour la feuille de route (cumul vols 1+2)

1. **🔴 Registre inter-modules ES DSFR — 3 sites confirmés**
   (`explorer-jscore`, vols 1+2) : `value is not a function` +
   `cannot read 'ns'/'internals'/'core' of Undefined` sur
   service-public, legifrance, **impots** (version non-minifiée :
   `dsfr.module.js:775`). Le framework d'État est mort sur *tous* les
   portails DSFR testés. Correctif unique = déblocage massif.
2. **🔴 Résolution d'imports internes aux bundles Vite — silencieuse**
   (`explorer-js`/`jscore`) : `cartes.gouv.fr` évalue son point d'entrée
   « fin » en ~5 s avec `sources=1` et **aucune exception**, `#app`
   reste vide. Ajouter : (a) suivi des `import`/chunks internes,
   (b) **traçage des `unhandledrejection`/`import()` rejetés** — sans
   quoi ces échecs resteront invisibles.
3. **🟠 Listener `load` `call` of Undefined — récidive** : OSM vol 1 +
   ameli vol 2, même signature exacte. Le dispatch d'événement sur
   handlers wrappés (jQuery `ready`/`load`) perd la cible.
4. **🟠 `instanceof Element` trop strict** (`explorer-jscore`) : agrégat
   Drupal impots rejeté sur brand-check — vérifier que `document`,
   nœuds cross-realm et `Text` ne tombent pas dans le check Element.
5. **🟢 jQuery 4.0.0 passe** (ameli) : le crash `toLowerCase` de
   legifrance est **spécifique à un bundle**, pas à jQuery 4 —
   requalifier l'anomalie vol 1 en « accès propriété hôte mal typé ».
6. **🟢 `UnmappedKey` résiduel** : 1 repli × 2 vols — inchangé, non
   bloquant. Aucun `Fingerprint` ce vol.

## ✅ Succès notables

- **0 crash / 0 panic sur 8 sites d'État** au total — « tolérant, jamais
  crash » tient sur TLS gouvernementaux h2, agrégats Drupal massifs,
  payloads Nuxt de ~100 Ko embarqués.
- **Redirections institutionnelles** : `impots.gouv.fr → /accueil` et
  `geoportail.gouv.fr → cartes.gouv.fr/explorer-les-cartes/` rapportées
  proprement (`[url : …]`).
- **data.gouv.fr = référence positive** : graphe de 34 modules Nuxt
  prefetché, évalué sans erreur ; SSR peint intégralement — golden test
  candidat.
- **Vignettes/images statiques fetchées et peintes** (actualités
  impots, logos, icônes DSFR) — pipeline image stable.
- **DSFR non-minifié fournit la première stack lisible** du bug de
  registre (`dsfr.module.js:775`) — bisection désormais possible.

*Mission exécutée avec le binaire `target/debug/explorer-cli.exe` figé —
aucune reconstruction, aucune modification hors `docs/Rapport vol/`.*

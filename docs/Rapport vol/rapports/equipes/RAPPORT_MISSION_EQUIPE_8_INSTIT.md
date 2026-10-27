# 🛰️ RAPPORT D'ANALYSE EN VOL : ÉQUIPE 8 (INSTITUTIONNEL, SERVICES PUBLICS & CARTOGRAPHIE)

**Périmètre** : Portails institutionnels et services publics — conformité d'accessibilité
(RGAA / DSFR, attributs `aria-*`, `role="alert"`, `hidden`), encodages `fr`/`latin`,
frameworks d'État (DSFR 1.14, tarteaucitron RGPD, ReadSpeaker), cartographie tuilée
(Leaflet), TLS gouvernementaux et redirections de domaines publics
(`.fr` → `.gouv.fr`, `europa.eu` → `european-union.europa.eu`).
**Objectif** : Mesurer la tenue du pipeline sur du markup institutionnel massif et
accessibilité-first, vérifier le fetch d'images sous-ressources (tuiles vs. `<img>`
injectés par JS), et qualifier les murs JS propres aux frameworks d'État.

---

## 📋 Synthèse des vols

| N° | Date / Heure | URL cible | Statut | Anomalie / Diagnostic clé |
|:---:|:---:|:---|:---:|:---|
| **01** | 2026-10-26 13:28 | `https://www.service-public.fr/` | 🟢 SUCCÈS | Redirect suivi `→ service-public.gouv.fr`. Page DSFR complète peinte (16,86 Mo). 1 repli ADR-0025 (`UnmappedKey`). Modules DSFR `esm` en échec : `value is not a function`. |
| **02** | 2026-10-26 13:29 | `https://www.openstreetmap.org/` | 🟠 CARTE VIDE | Chrome OSM peint (3,15 Mo) mais **zone carte entièrement vide** : `application.js` crash (`Worker: no loader installed`) → Leaflet jamais démarré → `#map` reste un `<div>` vide, **0 tuile `<img>` injectée**. Journal 0/0/0. |
| **03** | 2026-10-26 13:29 | `https://europa.eu/european-union/index_fr` | 🟢 SUCCÈS DÉGRADÉ | Redirect `→ european-union.europa.eu/index_fr`. PNG **29,56 Mo** (page très haute). **59 mutations classées mais journal rejeté** (`why=Fingerprint`, ADR-0025) → repli re-parse ; menus ECL entièrement dépliés à l'écran. |
| **04** | 2026-10-26 13:30 | `https://www.legifrance.gouv.fr/` | 🟢 SUCCÈS DÉGRADÉ | DOM riche en menus peint (24,11 Mo), **mais cascade jQuery en échec** : `toLowerCase is not a function` sur jquery.min.js → `jQuery is not defined` ×5 → tout l'écosystème (Bootstrap, accordéons ARIA, DSFR modules `internals`/`core` of Undefined) inerte. Journal 0/0/3. |

**Bilan** : 4/4 pages **fetchées, parsées et peintes sans crash** — le pipeline
HTML/CSS/layout/render encaisse du markup d'État massif (jusqu'à ~5 600 lignes de
DOM dumpé). Les échecs sont **exclusivement côté `explorer-jscore`/`explorer-js`** :
modules ES à registre d'interop absent (`ns`, `internals`, `core`, `store` of
`Undefined`), `new Worker` non implémenté, jQuery cassé par un appel de méthode
mal typé, et journal DOM rejeté par empreinte sur la page la plus mutée.

---

## 🔬 Fiches Techniques Détaillées

### ✈️ Vol #01 — `https://www.service-public.fr/` (DSFR 1.14, RGAA)

- **Stack technique** : redirection `www.service-public.fr → www.service-public.gouv.fr`
  (nouveau domaine officiel 2026). DSFR 1.14 (`dsfr.module.min.js` + `analytics.module.min.js`
  en `type="module"`), RequireJS, ReadSpeaker (accessibilité vocale), AB Tasty,
  `h2` (HTTP/2) confirmé en négociation. jQuery absent — stack 100 % DSFR.
- **Capture** : `captures/2026-10-26_13-28-44_Equipe_8_Instit_www.service-public.fr.png`
  — 16 861 984 octets. Code sortie 0. **Rendu quasi intégral** : en-tête Marianne,
  navigation par événements de vie, cartes « Démarches et outils », actualités,
  bandeau France Services, footer partenaires (data.gouv.fr, legifrance…).
- **stderr verbatim** :
  ```text
  DEBUG explorer_core::session::js_glue: module en échec tab=1
    error=exception js : TypeError: value is not a function
    at <anonymous> (…/dsfr-1.14/dist/dsfr.module.min.js:2:11016)
  DEBUG explorer_core::session::js_glue: module en échec tab=1
    error=exception js : TypeError: cannot read `ns` of Undefined
  WARN explorer_core::session::apply: journal DOM rejeté → repli re-parse (ADR-0025) tab=1 why=UnmappedKey
  [journal : 0 appliqués / 1 replis / 1 classées]
  ```
- **URL vivante** (`dom`) : `https://www.service-public.gouv.fr/`.
- **Note RGAA** : le DOM dumpé contient `Une erreur est survenue pendant la
  recherche, merci de réessayer` — **faux positif d'analyse** : c'est le gabarit
  `<p class="fr-error-text" hidden id="search-fs-error" role="alert">` standard
  DSFR (affiché uniquement en cas d'échec de l'autocomplétion), pas un état
  d'erreur réel. Les attributs `role="alert"`, `aria-live="polite"`, `aria-hidden`
  sont parsés et conservés dans l'arène.
- **Analyse racine** :
  - `explorer-jscore` : les modules DSFR sont **préfetchés puis évalués**, mais le
    point d'entrée du module échoue sur `TypeError: value is not a function`
    (enregistrement du namespace `window.dsfr` attendu comme fonction usine —
    vraisemblablement `Object.defineProperty`/`Symbol` ou un getter d'interop
    manquant) ; l'analytics qui consomme `dsfr.ns` tombe ensuite sur
    `cannot read 'ns' of Undefined`. **Le registre de namespace inter-modules
    ES n'est pas propagé à `window`**.
  - `explorer-core::session::apply` : la seule mutation classée référence un
    `NodeId` absent de l'arène → repli re-parse propre (ADR-0025), coût ≈ 3,5 s
    de repaint supplémentaire.
- **Repro minimal** :
  ```javascript
  // Pattern DSFR : le module enregistre une fabrique globale
  window.dsfr = (function(){ const ns = {}; return { ns, register(n,f){ ns[n]=f; } }; })();
  window.dsfr.register("core", { start(){ document.body.dataset.dsfr="ok"; } });
  // Attendu : pas de TypeError; mutation journalisée [1 appliquée]
  ```

### ✈️ Vol #02 — `https://www.openstreetmap.org/` (Rails + Turbo + Leaflet)

- **Stack technique** : frontend Rails/AssetPipeline (`application-*.js`,
  1 529 756 octets évalués en **6 407 ms**), Turbo en `type="module"`, i18n `en.js`,
  Leaflet chargé côté CSS (`leaflet-all-*.css`) **mais intégralement piloté par JS** —
  la page ne contient **aucune tuile `<img>` dans le HTML initial**.
- **Capture** : `captures/2026-10-26_13-29-19_Equipe_8_Instit_www.openstreetmap.org.png`
  — 3 147 679 octets. Le **chrome OSM est peint** (logo, Edit/History/Export…,
  barre de recherche, Log In/Sign Up, bannière « 31 DAY OSM CHALLENGE »),
  mais **la zone carte à droite est un rectangle gris vide** — vérification
  visuelle directe du PNG.
- **stderr verbatim** :
  ```text
  INFO explorer_core::session::js_glue: script: eval… tab=1 bytes=1529756
  INFO explorer_core::session::js_glue: script: eval fin tab=1 ms=6407 ok=false
  DEBUG … script en échec : TypeError: Worker: no loader installed
    at <anonymous> (…/application-3901b15b…js:222:20668)
  DEBUG … script en échec : TypeError: cannot read `store` of Undefined
  DEBUG … listener dom en échec ty="load" : TypeError: cannot read `call` of Undefined
  [journal : 0 appliqués / 0 replis / 0 classées]
  ```
- **DOM vivant** : `<div id="map" class="bg-body-secondary flex-grow-1 z-0">` —
  **contenu vide**. Aucune sous-class `.leaflet-pane`, aucun `<img>` de tuile
  `tile.openstreetmap.org`, aucun `<canvas>`.
- **Analyse racine (par crate)** :
  - **`explorer-jscore`** : `new Worker(url)` lève `TypeError: Worker: no loader
    installed` — le stub Worker existe (le message est typé, pas un `is not a
    constructor`) mais aucun chargeur de script worker n'est branché → le
    bootstrap `application.js` meurt *avant* d'atteindre `L.map("map")`. Erreurs
    en cascade : `store` of Undefined (store Turbo/sessionStorage wrapper) puis
    le listener `load` échoue sur `call` of Undefined.
  - **Conséquence DOM** : Leaflet n'ayant jamais tourné, **aucune mutation n'est
    journalisée** (0 classée) — le `#map` statique reste vide. Ce n'est donc
    *pas* un trou d'`explorer-render` : il n'y a littéralement **rien à
    rasteriser**. Le pipeline image (`<img>` fetch + décodage, éprouvé sur les
    autres vols) n'est jamais sollicité car les tuiles Leaflet sont injectées
    par `L.TileLayer` en JS — pas par le markup.
  - **Distinction clé** : carte OSM = tuiles `<img>` **créées dynamiquement**
    (pas de canvas sur Leaflet 1.x pour les tuiles raster de base). Correctif
    = faire démarrer `application.js`, pas toucher `explorer-render`.
- **Repro minimal** :
  ```javascript
  // 1) Le mur primaire
  const w = new Worker("/assets/map-worker.js"); // attendu : stub tolérant ou impl
  // 2) Le résultat attendu après démarrage Leaflet
  const map = document.getElementById("map");
  const img = document.createElement("img");
  img.src = "https://tile.openstreetmap.org/5/16/11.png";
  img.className = "leaflet-tile";
  map.appendChild(img); // attendu : mutation journalisée + tuile peinte
  ```
- **Perf** : eval du bundle 1,5 Mo en 6,4 s — budget OK mais à surveiller.

### ✈️ Vol #03 — `https://europa.eu/european-union/index_fr` (Drupal EWCMS / ECL)

- **Stack technique** : redirection `europa.eu/european-union/index_fr →
  european-union.europa.eu/index_fr`. Drupal EWCMS : jQuery 4.0.0, Duet Design
  System (`duet.esm.js` module), agrégats JS versionnés (`js_*.js?scope=footer
  &delta=N&language=fr`), webtools Europa (`load.js?theme=eu`), survey
  `wr_survey.js`, JSON-LD `GovernmentOrganization` embarqué.
- **Capture** : `captures/2026-10-26_13-29-47_Equipe_8_Instit_europa.eu.png` —
  **29 560 643 octets** (page extrêmement haute : menu « réponse à la carte »
  entièrement déplié + actualités photo). Code sortie 0. **Aucune exception JS**
  sur tout le vol — tous les scripts `ok=true` (dont un inline de 415 702 octets
  en 2 416 ms et le module `duet.esm.js` 75 ms).
- **stderr verbatim** :
  ```text
  INFO explorer_core::session::apply: apply: début tab=1 journal=true
  WARN explorer_core::session::apply: journal DOM rejeté → repli re-parse (ADR-0025) tab=1 why=Fingerprint
  INFO explorer_core::session::apply: apply: début tab=1 journal=false
  … apply: fin tab=1 ms=2276
  [journal : 0 appliqués / 1 replis / 59 classées]
  ```
- **Analyse racine** :
  - **`explorer-dom` / `explorer-core` (ADR-0025)** : cas **différent** des
    `UnmappedKey` habituels — ici **59 mutations ont été produites et classées**
    (les scripts tournent correctement : ECL markup injecté, webtools), mais la
    **vérification d'empreinte** entre l'arène attendue par le journal et
    l'arène courante diverge (`why=Fingerprint`) → rejet en bloc + re-parse
    complet (~2,3 s). Hypothèse : les scripts ont muté l'arbre **entre** la
    capture d'empreinte et l'application (mutations de type texte/attribut sur
    un nœud dont la génération a déjà tourné), ou le hash d'empreinte ne couvre
    pas un type de mutation produit par les agrégats Drupal.
  - **`explorer-render`** : PNG 29,5 Mo — le menu ECL déplié étire la page ;
    cohérent avec le bornage (pas un débordement type YouTube 48 Mo : le contenu
    est réellement long). Menus `ecl-menu` rendus **toujours ouverts** faute de
    JS de repli — dégradation acceptable RGAA mais très consommatrice de hauteur.
- **Repro minimal** :
  ```javascript
  // Provoquer ~50 mutations classées puis une mutation tardive avant apply
  for (let i=0;i<50;i++){ const d=document.createElement("div");
    d.textContent="m"+i; document.body.appendChild(d); }
  setTimeout(()=>{ document.title="late"; }, 0);
  // Vérifier : journal rejeté why=Fingerprint ? Quelle mutation décale l'empreinte ?
  ```

### ✈️ Vol #04 — `https://www.legifrance.gouv.fr/` (jQuery + Bootstrap + DSFR)

- **Stack technique** : portail DILA — jQuery webjars, Bootstrap, js-cookie,
  `jquery-accessible-accordion-aria`, `jquery-accessible-tabs`, datepicker FR,
  tarteaucitron (consentement RGPD), `email-decode` Cloudflare, **15 modules
  DSFR** (`core`, `scheme`, `card`, `accordion`, `navigation`, `breadcrumb`,
  `sidemenu`, `modal`, `header`, `display`, `tag`, `tile`, `toggle`, `checkbox`,
  `tooltip` — tous `type="module"`).
- **Capture** : `captures/2026-10-26_13-30-51_Equipe_8_Instit_www.legifrance.gouv.fr.png`
  — 24 112 396 octets. **DOM riche en menus rendu** : arborescence complète
  (Codes, Jurisprudence, JO, Kali…), pavé JORF avec dates, « Votre réseau social »,
  actualités, footer République française.
- **stderr verbatim** (cascade JS la plus dense des 4 vols) :
  ```text
  script en échec : TypeError: `toLowerCase` is not a function (on Object)   ← jquery.min.js
  script en échec : TypeError: Bootstrap's JavaScript requires jQuery…
  script en échec : ReferenceError: jQuery is not defined        (×4)
  script en échec : ReferenceError: $ is not defined             (×2, dont DOMContentLoaded)
  script en échec : Error: Datepicker's JavaScript requires jQuery
  module en échec : TypeError: value is not a function                        ← dsfr core.module
  module en échec : TypeError: cannot read `internals` of Undefined  (×10)   ← modules DSFR
  module en échec : TypeError: cannot read `core` of Undefined       (×3)
  [journal : 0 appliqués / 0 replis / 3 classées]
  ```
- **Analyse racine** :
  - **`explorer-jscore` — faute première** : `jquery.min.js` meurt sur
    ``TypeError: `toLowerCase` is not a function (on Object)``. Probable : un
    appel `x.toLowerCase()` où `x` est un objet non-chaîne (détection
    `document.documentMode`/`navigator.userAgent` renvoyant `Object`, ou appel
    non lié `jQuery.fn.toLowerCase`) → **le global `jQuery`/`$` n'est jamais
    installé** → toute la pile applicative (10+ scripts) échoue en
    `ReferenceError`. Une seule erreur primaire masque tout le reste.
  - **`explorer-js` — modules DSFR** : même signature que Vol #01
    (`internals`/`core` of Undefined) — le contrat de registre DSFR
    (`dsfr.core` → `internals.register` partagé entre modules ES) n'est pas
    honoré : **les namespaces `export`/`window.dsfr` ne se propagent pas
    entre modules `type="module"`**.
  - 3 mutations classées (probablement tarteaucitron ou email-decode Cloudflare)
    **non appliquées** — journal vide côté apply (`journal=false` direct).
- **Repro minimal** :
  ```javascript
  // Mur primaire à bissecter dans jquery.min.js :
  var s = {}; s.toLowerCase(); // → doit lever TypeError lisible mais jQuery doit survivre ?
  // En réalité : un accès propriété→méthode sur hôte (navigator/appVersion)
  // renvoyant Object au lieu de String. Tester :
  "X".toLowerCase.call({});          // TypeError attendu
  typeof navigator.appVersion;       // doit être "string"
  ```

---

## 📌 Top anomalies pour la feuille de route

1. **🔴 Registre inter-modules ES absent** (`explorer-jscore`, **2/4 vols**) :
   `cannot read 'ns'/'internals'/'core' of Undefined` sur **tous** les modules
   DSFR (`type="module"`) — le pattern `window.dsfr.register` entre modules ESM
   ne fonctionne pas. C'est *le* framework d'État français : corriger cette
   seule couture débloque service-public **et** legifrance.
2. **🔴 `new Worker` = mur dur** (`explorer-jscore`) : `TypeError: Worker: no
   loader installed` tue `application.js` OSM (1,5 Mo, 6,4 s d'eval gâchée) en
   plein bootstrap → carte Leaflet jamais créée. Soit implémenter un Worker
   minimal no-op tolerant, soit un stub qui ne throw pas.
3. **🟠 jQuery cassé par `toLowerCase` sur objet hôte** (`explorer-jscore`) :
   un seul appel mal typé dans `jquery.min.js` empêche l'installation du global
   `$` → cascade de 8+ `ReferenceError`. Impact large : jQuery reste le script
   le plus déployé du web (aussi présent sur europa.eu).
4. **🟠 `why=Fingerprint` sur gros journal** (ADR-0025, `explorer-dom`/`core`) :
   europa.eu produit **59 mutations classées** — la plus grosse rafale observée
   — et se fait rejeter sur empreinte, pas sur clé. Bissecter quelle mutation
   invalide l'empreinte ; le coût du re-parse (~2,3 s ×2) est réel.
5. **🟢 `UnmappedKey` résiduel** : 1 repli sur service-public — même cause
   qu'équipes 1/7 (mutation vers nœud hors arène), non bloquant.

## ✅ Succès notables

- **0 crash / 0 panic sur 4 sites d'État** — TLS gouvernementaux (h2, ALPN),
  `data:` URLs, JSON-LD massif : « tolérant, jamais crash » tient.
- **Redirections institutionnelles suivies** : `service-public.fr → .gouv.fr`
  et `europa.eu → european-union.europa.eu` rapportées proprement au `dom`.
- **Pipeline image fonctionnel** : les `<img>` **statiques** (photos presse
  europa.eu, icônes DSFR, logo OSM, bannière challenge) sont fetchées, décodées
  et peintes — seul le cas « `<img>` créés par JS » (tuiles Leaflet) est orphelin.
- **RGAA/ARIA préservé** : `role="alert"`, `aria-live`, `aria-hidden`, `hidden`
  parsés fidèlement (le `fr-error-text` de service-public est bien `hidden` dans
  le dump — pas de fuite d'accessibilité en rendu).
- **Modules ES prefetchés et tentés** : les 15 modules DSFR legifrance +
  `duet.esm.js` + `turbo` passent tous par `module eval…` — la couture module
  existe, il ne manque que le registre de namespace.

*Mission exécutée avec le binaire `target/debug/explorer-cli.exe` figé — aucune
reconstruction, aucune modification hors `docs/Rapport vol/`.*

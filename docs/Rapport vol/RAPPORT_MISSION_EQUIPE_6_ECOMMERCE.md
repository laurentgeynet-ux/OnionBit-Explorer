# 🛰️ RAPPORT D'ANALYSE EN VOL : ÉQUIPE 6 (E-COMMERCE & SITES TRANSACTIONNELS)

**Périmètre** : Places de marché, retail transactionnel, petites annonces —
pages massives à forte télémétrie, machinerie anti-bot de premier plan.
**Objectif** : Mesurer la tenue du pipeline `fetch → parse → style → layout →
render` face aux DOM les plus lourds du web réel, évaluer `explorer-jscore`
sur les frameworks marchands (AUI/ue_csm, Remix/Fika) et qualifier les murs
anti-automatisation externes.

---

## 📋 Synthèse des vols

| N° | Date / Heure | URL cible | Statut | Anomalie / Diagnostic clé |
|:---:|:---:|:---|:---:|:---|
| **01** | 2026-10-26 12:28 | `https://www.amazon.fr/` | 🟠 SUCCÈS DÉGRADÉ | Shot 10,45 Mo produit, 194 scripts inline évalués sans erreur, **mais 2 replis ADR-0025 `UnmappedKey`** et **commentaires DOM peints comme texte** (`<#comment>` visible à l'écran). |
| **02** | 2026-10-26 12:30 | `https://www.ebay.fr/` | 🔴 BLOQUÉ EXTERNE | **HTTP 403** sur shot ET dom — refus anti-bot (CDN/Akamai). Aucun PNG. Pipeline interne non coupable : la réponse a quand même été parsée/layoutée. |
| **03** | 2026-10-26 12:30 | `https://www.leboncoin.fr/` | 🔴 BLOQUÉ EXTERNE | **HTTP 403** shot et dom — protection **Datadome** attendue. Réponse interstitielle reçue END_STREAM en ~10 ms. |
| **04** | 2026-10-26 12:30 | `https://www.ikea.com/fr/fr/` | 🟠 SUCCÈS DÉGRADÉ | Shot 3,19 Mo, SSR Remix exploitable, modules ES évalués (customElements, `import()` dynamique OK) — **2 erreurs jscore** (`proxy ownKeys`, `` `...` `` hors littéral) ; hydratation Fika amorcée mais incomplète. |

---

## 🔬 Fiches Techniques Détaillées

### ✈️ Vol #01 — `https://www.amazon.fr/` (AUI / ue_csm)

- **Stack technique** : page d'accueil Amazon FR, ~200 requêtes H2 multiplexées
  (stream_id 1→179+), grappe de scripts inline `ue_csm`/`ueLogError`/`AUI
  PageJS`, stubs service worker (`P.execute("RetailPageServiceWorker")`),
  CSS sprites `m.media-amazon.com`, télémétrie `fls-eu.amazon.fr`.
- **Rendu visuel** (`captures/…_www.amazon.fr.png`, 10 456 251 octets) :
  barre de navigation sombre complète (livraison « Mettre à jour
  l'emplacement », compte, panier), bandeau promo NBA visible, une carte
  produit vert clair « Haut de gamme à prix do… » peinte à droite, pied de
  page intégral (4 colonnes + liens annexes + sélecteur langue/pays).
  **La zone centrale de la page (carrousels produits) est quasi vide** :
  contenu peuplé par `data-*` / chargement différé côté AUI, jamais
  matérialisé. **Anomalie de rendu marquante** : des littéraux
  `<#comment>` sont peints comme texte visible en haut de page, entre les
  sections et en pied — les nœuds commentaires du DOM fuient dans la
  display list / la couche texte (probable artefact de sérialisation du
  shot, à confirmer côté `explorer-render`/`explorer-core`).
- **Journal DOM** : `[journal : 0 appliqués / 2 replis / 20 classées]` —
  **0 mutation journalisée acceptée**, 2 rejets `UnmappedKey` forçant le
  re-parse complet ADR-0025, 20 mutations classées non appliquées. Le
  journal DOM est de fait inopérant sur cette page (repli systématique,
  coûteux : re-cascade ~1 800 ms, layout ~2 500 ms, repaint ~4 000 ms par
  cycle — `apply: fin ms=6066` au plus lourd).
- **Moteur JS** : **194 scripts évalués, tous `ok=true`, zéro exception
  jscore.** Toute la machinerie inline ue_csm (stubs `ue.count`,
  `ueLogError`, `csa.plugin`, `P.declare/register`) s'exécute sans faute.
  Pas de `<script type="module">` ni de `customElements` sur ce site :
  charge classique uniquement. Les `P.execute("RetailPageServiceWorker")`
  figurent dans le DOM mais `navigator.serviceWorker` reste un stub —
  l'enregistrement SW est appelé, dégradé silencieusement côté site.

#### 💥 Anomalie détectée (Core / journal ADR-0025) :
```text
WARN explorer_core::session::apply: journal DOM rejeté → repli re-parse
     (ADR-0025) tab=1 why=UnmappedKey
```
Deux occurrences par phase (shot puis dom). Un `NodeId` de la mutation
journalisée ne correspond à aucune clé de la table de mapping — signature
typique d'un nœud créé par le script (insertion AUI dans `<head>`,
placeholder `data-a-*`) absent de l'instantané de mapping.

#### 🔍 Analyse racine par crate :
- **`explorer-core` (`session::apply`)** : le repli ADR-0025 fonctionne
  comme prévu (tolérance, pas de crash), mais le taux de rejet 2/2 rend le
  journal inutilisable sur les pages à mutations dès le parse — la clé
  manquante mérite un test sur `document.createElement` + insertion avant
  premier apply.
- **`explorer-jscore`** : impeccable côté exécution — la grappe ue_csm,
  volontairement défensive (`try/catch`, stubs, `ue.exec`), est digérée
  sans erreur. Limite constatée : aucun `serviceWorker` — Amazon détecte
  l'absence et journalise `sw:sw_message_no_ctrl` côté page (inoffensif).
- **`explorer-net`** : nominal, ~200 streams H2 concurrents
  (`max_concurrent_streams: 128` annoncé par Amazon), GoAway propre en fin
  de vol.
- **`explorer-render`** : à investiguer — commentaires peints en texte.

---

### ✈️ Vol #02 — `https://www.ebay.fr/` (anti-bot CDN)

- **Stack technique** : Akamai/eBay edge, handshake H2 complet puis refus.
- **Rendu visuel** : **aucun** — pas de PNG produit.

#### 💥 Mur détecté (externe — `explorer-net` recevant, non fautif) :
```text
DEBUG explorer_core::session::paint: pipeline en échec tab=1 error=HTTP 403
erreur : HTTP 403
```
Identique sur shot et dom. Le serveur répond cependant **un corps 403
complet** : le pipeline a cascade=2 ms / layout=41 ms / repaint=70 ms
sur la page d'erreur (Akamai renvoie un HTML d'interdiction, que
`explorer-html` a loyalement parsé — comportement tolérant correct,
dommage collatéral : l'erreur est signalée seulement après repaint).

#### 🔍 Analyse racine :
- **Mur externe anti-bot** : eBay filtre en amont. Indices moteur qui
  facilitent la détection : `User-Agent: Explorer/0.1`
  (`crates/explorer-net/src/h1.rs:204`, propagé en H2 via
  `h2.rs:140`) — signature immédiatement identifiable comme non-navigateur
  ; empreinte H2 minimaliste (SETTINGS par défaut, pas de pseudo-en-têtes
  navigateur type `sec-fetch-*`, `sec-ch-ua`, `Accept` text/html complet) ;
  fingerprint TLS rustls ≠ Chrome/Firefox.
- **Pas de bug moteur** : le fetch, le parse et l'erreur typée remontent
  proprement. Point d'amélioration : signaler le statut HTTP **avant**
  d'investir layout+repaint sur le corps d'erreur (court-circuit
  `explorer-core` si `status >= 400` et `content-type` non document ?).

---

### ✈️ Vol #03 — `https://www.leboncoin.fr/` (Datadome)

- **Stack technique** : front leboncoin derrière **Datadome**, fenêtre H2
  généreuse (`initial_window_size: 65536`, `max_frame_size: 16777215`,
  `WindowUpdate 2147418112` — signature Datadome/CDN).
- **Rendu visuel** : **aucun** — pas de PNG.

#### 💥 Mur détecté (externe) :
```text
DEBUG explorer_core::session::paint: pipeline en échec tab=1 error=HTTP 403
erreur : HTTP 403
```
Réponse Data+END_STREAM en **~10 ms** après les headers : page
d'interdiction Datadome compacte (challenge JS/cookie attendu). Là encore
le pipeline exécute cascade/layout/repaint sur le corps du challenge.

#### 🔍 Analyse racine :
- Datadome évalue cookie `datadome`, fingerprint TLS/H2, UA et
  comportement : `Explorer/0.1` + absence de challenge-solving = 403
  garanti. **Aucune action moteur réaliste** à ce stade (résoudre les
  challenges Datadome est hors périmètre ; la politique anti-bot est
  déléguée à l'application hôte — cf. AGENTS §9).
- Même remarque core : le 403 pourrait court-circuiter le pipeline en
  amont du layout.

---

### ✈️ Vol #04 — `https://www.ikea.com/fr/fr/` (Remix / Fika SSR+hydratation)

- **Stack technique** : SSR Remix complet — `<script type="text/hydrate">`
  embarquant `loaderData` JSON massifs par fragment
  (`data-fika-hydration-id`), modules ES `dwf/*` (nav, lower-funnel, hri),
  web component `in-clude`, bundle UGC `rr-pip`, OneTrust cookie consent
  livré en *dyn link*.
- **Rendu visuel** (`captures/…_www.ikea.com.png`, 3 190 522 octets) :
  excellent pour un moteur jeune — bandeau noir langue/code
  postal/magasin, **logo IKEA bleu-jaune rasterisé**, nav Produits/
  Pièces/Bonnes affaires/Conception/Services, champ « Que cherchez-vous ? »
  avec icônes, carrousel catégories (Bonnes affaires → Arts de la table),
  **hero promotionnel « 10 % offerts* sur votre nouvelle cuisine METOD »**
  avec photo de cuisine nette. Mêmes fuites `<#comment>` qu'Amazon en
  marge supérieure. Le SSR seul porte 90 % de ce rendu.
- **Hydratation** : **amorcée, non aboutie.** Le DOM vivant contient à la
  fois `data-fika-hydrating="true"` (2 occurrences, état « en cours ») et
  `data-fika-hydrated="true"` (3 fragments terminés) — l'orchestrateur
  Fika a partiellement hydraté avant de caler.
- **Activité JS** : 72 evals ; modules `rr-leave-review`, `in-clude.min.js`
  (→ `customElements.define name=in-clude` ✅), `lower-funnel-fragments/
  root`, `nav/root` (29 732 ms !) et `hri/root` évalués ; **`import()`
  dynamique** résolu (`./global-store-picker.route-…`,
  `./header.route-…`, `./page.route-…`) — le chargeur de modules de
  `explorer-jscore` fonctionne sur un vrai site.

#### 💥 Murs détectés (`explorer-jscore`) :
```text
module en échec tab=1 error=exception js :
  TypeError: proxy ownKeys: must list all keys of non-extensible target
    at rr (…/rr-pip.17257f2c.bundle.js:2:75041) …
```
```text
module en échec tab=1 error=exception js :
  TypeError: `...` outside a call or array literal
    at d (./sentry-ipcEF7xJ.js:1:530)
```
(chaque erreur réapparaît en phase dom — lignes 3191/3217 — reproductible).

#### 🔍 Analyse racine :
1. **`proxy ownKeys`** : la trappe `ownKeys` du Proxy interne de jscore
   rejette une cible non-extensible dont la liste de clés diffère —
   vraisemblablement le proxy exposé au code page (namespace d'hydratation
   ou `Reflect.ownKeys` sur un objet gelé par le bundle). À chercher dans
   l'implémentation `Proxy`/`ownKeys` de `explorer-jscore`.
2. **`` `...` `` hors littéral** : le parser jscore classe le `...` comme
   spread-strict ; le bundle `sentry-*.js` utilise vraisemblablement le
   **rest/spread dans un destructuring d'objet ou un pattern** (`{...x}`,
   `f(...args)`, `[...it]` dans une position non prévue, voire spread dans
   un template/argument de `import()`). Le parseur doit accepter `...` en
   position rest-element/destructuring (ES2018+).
3. **Conséquence** : le module `hri/root` (haut de page : hero,
   carrousels) plante sur le *second* — l'hydratation s'arrête en route,
   cohérent avec les `data-fika-hydrating` résiduels et le carrousel
   statique du PNG.

#### 🧪 Repro minimal test (spécialiste JS) :
```javascript
// 1) ownKeys sur proxy de cible gelée
const t = Object.freeze({a:1});
const p = new Proxy(t, { ownKeys: () => ["a","b"] });
try { Reflect.ownKeys(p); } catch(e){ /* TypeError attendu — mais jscore le lève sur un cas légitime */ }

// 2) spread/rest en positions non-littéral
const {a, ...rest} = {a:1, b:2, c:3};           // rest destructuring objet (ES2018)
function f(...args){ return args.length; }      // rest parameter
const copy = {...rest};                          // spread objet
```

---

## 🧭 Croisement des anomalies

| Anomalie | Crate | Vol(s) | Gravité |
|:---|:---|:---:|:---:|
| `journal DOM rejeté → UnmappedKey` (repli ADR-0025 systématique) | `explorer-core` | Amazon ×2 | 🟠 coût perf (re-cascade ~2 s) |
| `<#comment>` peints comme texte visible | `explorer-render`/`explorer-dom` (sérialisation shot ?) | Amazon, IKEA | 🟠 visuel, bloquant UX |
| `TypeError: proxy ownKeys` | `explorer-jscore` | IKEA | 🟠 bloque module UGC |
| `` TypeError: `...` outside a call or array literal `` | `explorer-jscore` (parser) | IKEA | 🔴 bloque `hri/root` → hydratation incomplète |
| HTTP 403 eBay / leboncoin | **externe** (anti-bot ; aggravé par `UA: Explorer/0.1`) | ×2 | ⚪ hors moteur — à documenter |
| Layout+repaint exécutés sur corps d'erreur 403 | `explorer-core` | eBay, leboncoin | ⚪ optimisation (court-circuit statut) |

**Verdict d'équipe** : deux murs purement externes (Datadome, edge eBay)
confirmés non-moteur ; sur les deux sites accessibles, le pipeline tient la
charge du web transactionnel réel (200+ requêtes H2, DOM massif, modules ES)
et `explorer-jscore` atteint l'hydratation partielle d'un Remix de
production. Priorités : **parser `...` (ES2018 rest/spread)** — c'est le
verrou qui empêche l'hydratation complète IKEA — puis la fuite de
`<#comment>` dans la couche texte et le diagnostic `UnmappedKey` du
journal.

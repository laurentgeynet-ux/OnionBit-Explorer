# 🛰️ RAPPORT D'ANALYSE EN VOL : ÉQUIPE 5 (MÉDIAS, MURS DE CONSENTEMENT & PAYWALLS)

**Périmètre** : Grands sites d'information — portails avec CMP (Consent Management Platform), GDPR/TCF, tag-soup industriel et hydratation JS lourde.  
**Objectif** : Éprouver le moteur sur des pages de presse réelles : tenue de la cascade sur des DOM de plusieurs dizaines de milliers de nœuds, comportement des stubs `__tcfapi`/`Didomi`, résistance des parseurs face aux idiomes minifiés des bundles média, et impact visuel des murs de consentement non exécutés.

---

## 📋 Synthèse des vols

| N° | Date / Heure | URL cible | Statut | Anomalie / Diagnostic clé |
|:---:|:---:|:---|:---:|:---|
| **01** | 2026-10-26 13:28 | `https://www.lemonde.fr/` | 🟠 ANOMALIE | **CMP Didomi présente dans le DOM mais non affichée** ; **12 exceptions JS** (`TypeError: cannot read 'hosts' of Undefined`, `ReferenceError: lmd is not defined`) + **1 erreur de syntaxe** sur le bundle DataDome 254 Ko (`expected ')')`) ; rendu émaillé de grands blancs ; 1 repli ADR-0025 `UnmappedKey`. |
| **02** | 2026-10-26 13:30 | `https://www.bbc.com/news` | 🟢 SUCCÈS | **Rendu quasi-parfait** — le meilleur de l'équipe. 64 scripts evalués `ok=true`, 0 exception, journal 0/0/0. PNG 19,9 Mo (page très longue, pas une anomalie). Stub CMP `window.dotcom.cmd` évalué sans erreur. |
| **03** | 2026-10-26 13:30 | `https://www.franceinfo.fr/` | 🟠 ANOMALIE | **Texte parasite `</#comment>` peint à l'écran** (fuite tree-builder `explorer-html`) ; menu compte déplié peint en plein écran (CSS `display:none`/popover non honoré) ; 1 repli ADR-0025 + **10 mutations classées** ; 0 exception JS. |
| **04** | 2026-10-26 13:31 | `https://text.npr.org/` | 🟢 SUCCÈS PUR | **Témoin de référence** : rendu texte instantané (commit ms=1507, puis re-shot ms=90), journal 0/0/0, zéro anomalie. |

---

## 🔬 Fiches Techniques Détaillées

### ✈️ Vol #01 — `https://www.lemonde.fr/` (Paywall Piano + CMP Didomi)
- **Stack technique** : design-system maison (`ds-*`), CMP `cmp.lemonde.fr` (Didomi/TCF stub `__tcfapi`), paywall Piano (`tinypass`), DataDome (`/_fs-ch-*/init.js`), scripts différés via `data-gdpr-src` gating par finalité (`ads`, `analytics`, `personalization`).
- **Rendu visuel** (`captures/2026-10-26_13-28-44_Equipe_5_Medias_www.lemonde.fr.png`, 2,57 Mo) : header « Le Monde » et barre Menu/Live peints correctement, mais **de vastes zones blanches** au-dessus du premier article ; le bloc « Live » Macron est **poussé hors de la colonne de droite** (débordement horizontal de grille). Aucun bandeau cookies visible à l'écran bien que le DOM vivant contienne `gdpr-cs-parameters-link` (« Gestion des cookies »), le stub TCF inline et ~10 scripts `data-gdpr-src` jamais activés — sans consentement, la CMP ne charge pas et les scripts différés restent muets : comportement somme toute « privacité-first » cohérent, mais la grille d'articles JS-dépendante reste vide.

#### 💥 Mur Détecté (JavaScript Engine `explorer-jscore`) :
```text
script async en échec tab=1 error=exception js : TypeError: cannot read `hosts` of Undefined
    at <anonymous> (inline:10:1:67404)
script async en échec tab=1 error=exception js : TypeError: cannot read `context` of Undefined
    at <anonymous> (inline:8:2:23348)  at i (inline:8:2:130991)
script async en échec tab=1 error=exception js : ReferenceError: lmd is not defined
    at <anonymous> (inline:11:1:2)
script async en échec tab=1 error=exception js : TypeError: Invalid URL
    at <anonymous> (inline:5:2:20826)
script en échec tab=1 error=syntaxe js 1:226510 : expected `)`
    (eval… tab=1 bytes=254226  →  eval fin ms=162 ok=false)
```

#### 🔍 Analyse Racine :
1. **`var lmd = {...}` non lié dans la portée globale** : le site injecte `var lmd = {"consent":{"version":1},"context":{...},"hosts":{"abo":"abo.lemonde.fr",...}}` (dump ligne 2439). Les scripts externes lisent ensuite `lmd.hosts` / `lmd.context` → `cannot read 'hosts' of Undefined` ×4, `cannot read 'context' of Undefined` ×4, `ReferenceError: lmd is not defined` ×2. Le `lmd` inline semble scopé dans un autre realm/script : probablement le `var` top-level n'est pas propagé comme propriété de `window` (global object binding) dans `explorer-jscore`. **Crate fautif : `explorer-jscore`** (liaison var↔global object).
2. **`TypeError: Invalid URL`** dans `cmp.lemonde.fr/js/lemonde.min.js` : récurrence du bug `new URL` déjà vu Équipe 1 (FastAPI) — constructeur URL fragile sur base non résolue.
3. **`expected ')'` à 1:226510 sur 254 226 octets** : le bundle DataDome `init.js` échoue au parse à ~89 % du fichier — idiome syntaxique non couvert (regexp, `?.`, ou commentaire en fin de fichier tronqué). **Crate : `explorer-jscore` (lexer/parser)**.
4. **Repli ADR-0025** (`journal DOM rejeté → repli re-parse why=UnmappedKey`) : journal incrémental rejeté lors de l'apply post-script ; coût réel mesuré : cascade 5405 ms + boxtree 5820 ms + layout 6410 ms + repaint 9937 ms. **Crate : `explorer-dom`/`explorer-core`** — l'attribut injecté par script (clé non mappée) n'est pas journalisable.
5. **Blancs de grille** : layout `data-zone="top-articles"` dépendant du JS (slideshow `ds-slideshow-web`) → `explorer-layout` rend un conteneur dimensionné vide. Dégradation acceptable.

#### 🧪 Repro Minimal Test :
```javascript
// Global var binding : les scripts <script> var doivent peupler window
<script>var lmd = {hosts: {abo: "x"}, context: {}};</script>
// script externe équivalent :
assert(window.lmd !== undefined);
assert(lmd.hosts.abo === "x");

// Constructeur URL fragile
new URL("/path", document.baseURI);

// Fin de bundle minifié (parser) — vérifier regexp/class fields à >200 Ko
```

---

### ✈️ Vol #02 — `https://www.bbc.com/news` (Next.js + NGAS ads + Zephr)
- **Stack technique** : Next.js (`_next/static/chunks/*`, `__NEXT_DATA__`/`pageProps` dans le DOM vivant), RequireJS `static.bbci.co.uk`, Optimizely, Tinypass (Piano paywall), module ES `dotcom-bootstrap.js` (NGAS) prefetché (`async_mods`/`module prefetché sources=1`), stub consentement `window.dotcom = window.dotcom || { cmd: [], consent: {} }`.
- **Rendu visuel** (`captures/2026-10-26_13-30-00_Equipe_5_Medias_www.bbc.com.png`, 19,9 Mo) : **remarquablement complet** — masthead BBC, grille News à trois colonnes, sections « More to explore », « Also in news », « Most read » numérotées 1-10, section Sport, newsletter et footer tous peints ; seules les images (lazy-load `srcset` non résolu) restent grises. Pas de bandeau de consentement visible. Le poids du PNG s'explique par la hauteur de page (≈10 écrans), pas par un bug raster.

#### ✅ Points de conformité remarquables :
```text
[journal : 0 appliqués / 0 replis / 0 classées]
script émis src=...  (64 scripts, tous sous-ressource sec_ok=true sri_ok=true)
module prefetché tab=1 name=.../dotcom-bootstrap.js sources=1
script: eval fin tab=1 ms=18 ok=true   (× ~60, aucun ok=false, aucune exception)
```
- Le code source de la page contient `console.error("NGAS load timeout")` — présent comme **chaîne littérale** dans le bundle, jamais exécuté (aucune trace console dans le log) : le loader NGAS s'est évalué proprement.
- **0 exception JS sur un bundle Next.js production** : fait majeur pour `explorer-jscore` — le plus gros site du panier passe sans repli ni crash.
- Hydratation Next limitée (pas de navigation client), mais le SSR suffit à un rendu complet : **référence positive de l'équipe**.

---

### ✈️ Vol #03 — `https://www.franceinfo.fr/` (France TV — Next.js + Didomi)
- **Stack technique** : Next.js (`pageProps`, timeline live JSON embarquée avec publications datées `2026-10-26T12:23` — données fraîches), CMP Didomi (`window.didomiConfig`, `type="didomi/javascript"` scripts gated, `__cmpLocator` stub, `gtag('consent','default')`, `javascript:Didomi.preferences.show()`), images `data-src` lazy.
- **Rendu visuel** (`captures/2026-10-26_13-30-49_Equipe_5_Medias_www.franceinfo.fr.png`, 4,37 Mo) : page d'accueil reconnaissable (badges « direct », titre article Macron-Marseille, photo peinte), MAIS :
  1. **Menu compte rendu déplié en pleine largeur** : « se connecter / mon espace / À lire plus tard / Mes commentaires / Mes newsletters / Mes informations / Se déconnecter » occupent tout le haut de page — le mécanisme de repli (CSS `hidden`/`display:none`/attribut `popover` ou classe toggled par JS) n'est pas appliqué. **Suspects : `explorer-css` (sélecteur d'attribut/`hidden` non honoré) ou `explorer-jscore` (classe jamais retirée car le script d'init n'a pas tourné)** — or 0 exception JS n'est loguée : la piste CSS est favorite.
  2. **Texte parasite `</#comment>` peint en clair** sous la barre de nav : fuite de balisage.

#### 💥 Mur Détecté (HTML tree-builder `explorer-html`) :
```text
(dump DOM vivant)
"président de la République<#comment>"
    <!-- #comment -->
```
Le fragment `</#comment>` apparaît **comme nœud texte** dans le DOM vivant et est peint. Le construct `<#comment>` (élément custom/non-standard rencontré dans la page, probablement issu d'un template ou d'une fermeture fantôme `<!-- -->`) est matérialisé en texte au lieu d'être ignoré/fermé correctement.

#### 📊 Journal DOM — 10 mutations classées :
```text
WARN explorer_core::session::apply: journal DOM rejeté → repli re-parse (ADR-0025) tab=1 why=UnmappedKey
[journal : 0 appliqués / 1 replis / 10 classées]
```
10 mutations ont été « classées » (mises de côté) avant le repli — le journal incrémental ADR-0025 couvre mal ce site (même `UnmappedKey` que lemonde). Coût du repli : cascade 2640-2691 ms, boxtree 6319-6378 ms, layout 7067 ms, repaint 10372 ms → **~11 s d'apply post-script**, le plus lourd du panier.

#### 🔍 Analyse Racine :
- `</#comment>` en texte → **`explorer-html`** : le tree-builder émet un nœud texte pour une fin de balise non reconnue au lieu de la traiter comme commentaire/bogus tag. Repro : `<div>foo<#comment>bar</div>` ou `</#comment>` en flux.
- Menu déplié → **`explorer-css`** (règle de masquage non appliquée) — à confirmer en comparant `display` calculé du conteneur.
- Didomi : `window.didomiConfig` posé par un script `ok=true`, mais les scripts `type="didomi/javascript"` ne doivent jamais s'exécuter (type non-js) — vérifier qu'ils sont bien ignorés (aucun eval échoué correspondant, conforme).

#### 🧪 Repro Minimal Test :
```html
<div>avant</div>
</#comment>
<div>après</div>
<!-- attendu : aucun texte "</#comment>" peint -->
```

---

### ✈️ Vol #04 — `https://text.npr.org/` (Témoin texte pur)
- **Stack technique** : HTML statique minimal, aucun framework, aucune CMP.
- **Rendu visuel** (`captures/2026-10-26_13-31-54_Equipe_5_Medias_text.npr.org.png`, 3,12 Mo) : parfait — liste d'articles en liens bleus, séparateurs tricolores, date « Monday, October 26, 2026 », footer © NPR. 
- **Perf** : premier shot ms=1503 (fetch+parse inclus), re-shot ms=90 ; cascade 11 ms, boxtree 14 ms, layout 286-300 ms, repaint 352-365 ms.
```text
[journal : 0 appliqués / 0 replis / 0 classées]
0 exception, 0 warning
```
**Site témoin validant la chaîne complète** : quand ni CMP ni JS hostile n'interviennent, le pipeline `fetch → parse → style → layout → paint` est sain de bout en bout.

---

## 🧭 Enseignements transverses

1. **Le mur de consentement est un « non-événement » gracieux** : sur lemonde/franceinfo, les CMP (Didomi/TCF) injectent un stub (`__tcfapi`, `__cmpBuffer`) évalué `ok=true`, puis les scripts réels sont gatés par `data-gdpr-src`/`type="didomi/javascript"` et ne se chargent jamais sans consentement → pas de bandeau peint, pas de boucle d'erreur. C'est la dégradation *attendue* — le navigateur « refuse » le consentement par défaut. Documenté comme comportement cible, pas comme bug.
2. **Anomalie récurrente `UnmappedKey` (ADR-0025)** : 2 sites/4 déclenchent le repli re-parse ; facturé ~10 s d'apply sur franceinfo. À investiguer côté `explorer-dom` : quelle clé d'attribut/namespace injectée par script n'est pas mappable au journal.
3. **Top suspects `explorer-jscore`** : (a) propagation `var` top-level → `window` (cassure `lmd` lemonde, probablement générique), (b) `new URL` (récidive Équipe 1+5), (c) fin de gros bundles minifiés (`expected ')'` à 226510/254226).
4. **`explorer-html`** : fuite `</#comment>` en texte peint — bug concret, reproductible, nouveau.
5. **`explorer-css`/`explorer-layout`** : overlays/menus repliables peints ouverts (franceinfo) ; grilles JS-dépendantes vides sans invalidation (lemonde).
6. **Hiérarchie de maturité mesurée** : text.npr (parfait) > bbc.com (quasi-parfait, 64 scripts OK) ≫ franceinfo / lemonde (rendu partiel + erreurs JS). Le web « SSR-first » passe ; le web « JS-hydrated + CMP » expose les trois derniers gros chantiers.

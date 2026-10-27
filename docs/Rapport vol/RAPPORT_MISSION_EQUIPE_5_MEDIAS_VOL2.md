# 🛰️ RAPPORT D'ANALYSE EN VOL : ÉQUIPE 5 (MÉDIAS) — VOL 2

**Périmètre** : Deuxième vague de sites d'information — presse anglophone (The Guardian, Deutsche Welle) et presse française à fort trafic (Le Figaro, Ouest-France), toutes sous CMP/paywall et hydration JS lourde.
**Objectif** : Confirmer/infirmer les anomalies du vol 1 (`UnmappedKey` ADR-0025, `</#comment>` peint, `new URL`, bindings globaux) sur des stacks différentes : Emotion/DCR (Guardian), design system maison + Web Components (Figaro), SIPA UI / `data-oflazyload` (Ouest-France), SPA Vue/Nuxt SSR (DW).
**Méthode** : `shot` puis `dom` vivant sous `RUST_LOG=debug` — logs `logs/eq5_*.log`, captures `captures/2026-10-26_13-4*_Equipe_5_Medias_*.png`.

---

## 📋 Synthèse des vols

| N° | Date / Heure (PNG) | URL cible | Statut | Anomalie / Diagnostic clé |
|:---:|:---:|:---|:---:|:---|
| **01** | 2026-10-26 13:43:25 | `https://www.theguardian.com/international` | 🟠 ANOMALIE | **PNG géant 54 Mo** (page ~30+ écrans, pas un bug raster) ; **erreur de syntaxe `invalid regex`** à `104:1098` (classe de caractères surrogate + `?`) dans un script inline — motif répété à chaque nav ; **1 repli ADR-0025 `UnmappedKey`** par navigation ; repaint post-script **ms=16059** (record du panier) ; `<#comment>` ×119 dans le DOM vivant. |
| **02** | 2026-10-26 13:44:49 | `https://www.lefigaro.fr/` | 🟠 ANOMALIE | **Premier cas de journal `0/0/16` : 16 mutations CLASSÉES** (record) ; module `globals.*.js` en exception `cannot read 'call' of Undefined` ; script inline `syntaxe 1:6272 : expected ','` ; listener `DOMContentLoaded` → `TypeError: Invalid scheme` ; menu latéral peint **déplié** (Actu locale/Économie/Culture) ; `<#comment>` ×36. |
| **03** | 2026-10-26 13:46:01 | `https://www.ouest-france.fr/` | 🟠 ANOMALIE | Journal `0/0/3` ; 4 exceptions async : **`TypeError: value is not a function`** + **`cannot read 'push' of Undefined`** ×3 ; rendu : logo OK, nav tronquée, **overlay « À lire / connectez-vous » peint ouvert**, lecteur média plein cadre peint, titre DIRECT géant débordant ; `<#comment>` ×36. |
| **04** | 2026-10-26 13:50:13 | `https://www.dw.com/fr/` → `/fr/actualités/s-10261` | 🟠 ANOMALIE | Premier essai sur mauvais chemin → **404**, refly `/fr/` : **redirect suivie** vers `/fr/actualités/s-10261` ✓ ; journal `0/0/0` ; 2 exceptions : **`cannot read 'document' of Null`** (script inline) + module `index-*.js` → **`Illegal component name, ""`** ; rendu **catastrophique** : fond bleu nuit, pas de grille, liens en colonne, 2 rects noirs (vidéos) ; PNG 27,3 Mo. |

---

## 🔬 Fiches Techniques Détaillées

### ✈️ Vol #01 — `https://www.theguardian.com/international` (DCR/React + Emotion + Sourcepoint)
- **Stack technique** : Dotcom Rendering (React SSR, classes `dcr-*` hashées Emotion — attribut `data-emotion` de plusieurs kilooctets sur le `<html>`), polyfill.io `assets.guim.co.uk` (callback `guardianPolyfilled`, features es6…IntersectionObserver), CMP Sourcepoint (`sourcepoint.theguardian.com`), télémétrie Ophan, modules ES prefetchés (`frameworks.client.web.*.js`, `index.client.web.*.js`).
- **Rendu visuel** (`captures/2026-10-26_13-43-25_…theguardian.com.png`, **54 034 199 octets**) : page **effectivement peinte de bout en bout** — masthead bleu « The Guardian », editions picker, colonnes « News / Opinion / Sport / Culture / Lifestyle », **photos réellement décodées et peintes** (contrairement aux lazy-loaders du vol 1), sections « Most viewed », « Headlines », footer complet. Le poids s'explique par la **hauteur du document** (≈ 30 écrans) et la densité d'images raster — même phénomène que BBC 19,9 Mo au vol 1, en ×2,7 : **pas un bug de raster**, mais un signal que le framebuffer alloue la page entière — à surveiller côté `explorer-render` (tiling/compression) si les pages longues deviennent la norme.
- **Perf** : fetch+parse ms=5083 (puis 2725 au refly), commit ms=8972 ; apply post-script : cascade 3130 / boxtree 3402 / layout 7049 / **repaint ms=16059** — le repaint le plus lourd observé, proportionnel à la hauteur peinte.

#### 💥 Mur Détecté (JS `explorer-jscore` + DOM `explorer-core`) :
```text
script: eval fin tab=1 ms=128 ok=false
script en échec tab=1 error=syntaxe js 104:1098 : invalid regex
    /[؀-퟿][?-�]?|[^؀-�]|./:
    Parsing error at position 8: Target of repeat operator is invalid
WARN explorer_core::session::apply: journal DOM rejeté → repli re-parse (ADR-0025) tab=1 why=UnmappedKey
[journal : 0 appliqués / 1 replis / 0 classées]
```

#### 🔍 Analyse Racine :
1. **`invalid regex` — lexer `explorer-jscore`** : l'idiome fautif est la regexp de splitting UTF-16 classique `/[\uD800-\uDBFF][\uDC00-\uDFFF]?|[^\uD800-\uDFFF]|./g`. Le lexer rapporte « Target of repeat operator is invalid » en position 8 : il refuse `?` appliqué à une **classe de caractères** dans une alternative — soit la classe n'est pas reconnue comme atome quantifiable, soit l'échappement `\uXXXX` n'est pas résolu avant l'analyse du quantifier. Repro trivial et **hautement générique** : cette regexp exacte est dans des milliers de bundles.
2. **`UnmappedKey` ×1/navigation** — récidive du motif lemonde/franceinfo : une clé d'attribut injectée par script (vraisemblablement `data-emotion` ou attribut namespacé du runtime Emotion/React) n'est pas journalisable → repli re-parse complet, facturé ~16 s de repaint.
3. **`<#comment>` ×119** dans le DOM vivant (record du panier — le Guardian génère des balises de placeholder `<#comment>` dans ses templates DCR) : le tree-builder émet **un nœud texte `"<#comment>"` *et* un nœud commentaire `<!-- #comment -->`** pour le même construct — duplication fautive dans `explorer-html` (voir § transverse).
4. Aucune exception runtime : les modules client.web s'évaluent `ok=true` (ms=106-173) — l'échec regex est **au parse**, pas à l'exécution.

#### 🧪 Repro Minimal Test :
```javascript
// Lexer : quantifier sur classe de caractères dans une alternative
const re = /[؀-퟿][?-�]?|[^؀-�]|./g;
assert("abc".split(re).length === 4);
```

---

### ✈️ Vol #02 — `https://www.lefigaro.fr/` (Design system fig-* + Web Components)
- **Stack technique** : design tokens `fig-var:root` (z-index, couleurs `fig-color-*`, motion), CSS reset embarqué, build moderne `/build/modern/js/globals/*` en **modules ES**, `customElements.define name=extends-br` (Web Components), GTM inline `dataLayer`, paywall, modules `pageAutoRefresh`.
- **Rendu visuel** (`captures/2026-10-26_13-44-49_…lefigaro.fr.png`, 2 369 147 octets) : structure éditoriale reconnaissable — article « EN DIRECT - Colère sociale : … Emmanuel Macron » parfaitement typographié (serif, date de publication), colonne « Le Flash » avec heures 13:44/13:42/13:34 — **données fraîches rendues**. MAIS : **le menu de navigation est peint entièrement déplié** en colonne gauche (« Actu locale », « La semaine du Figaro Bordeaux/Lyon/Marseille/Nantes/Nice », « Économie », « Patrimoine », « Culture et Loisirs », « Les lettres des journalistes ») — le mécanisme de repli n'est pas honoré (même classe d'anomalie que le menu franceinfo du vol 1) ; une **bande noire pleine largeur** occupe le haut de page (conteneur image/média non rempli) ; « Se connecter » est cassé en « Se / connecter » en haut à droite.
- **Perf** : fetch+parse ms=9242, commit ms=11322 — page lourde mais sans repli.

#### 💥 Mur Détecté (`explorer-jscore` + journal ADR-0025) :
```text
module eval… name=…/build/modern/js/globals/globals.593d6cd385119e1a.js
dom_host: customElements.define name=extends-br
module en échec tab=1 error=exception js : TypeError: cannot read `call` of Undefined
script: eval… tab=1 bytes=13469 → eval fin ms=15 ok=false
script en échec tab=1 error=syntaxe js 1:6272 : expected `,`
listener dom en échec tab=1 ty="DOMContentLoaded" error=exception js : TypeError: Invalid scheme
[journal : 0 appliqués / 0 replis / 16 classées]
```

#### 🔍 Analyse Racine :
1. **`cannot read 'call' of Undefined` dans `globals.*.js`** : le module fait typiquement `something.call(...)` (probablement `Function.prototype.call` déstructuré, `Reflect`, ou appel sur un import non résolu comme `webpack_require__.call`). Le `customElements.define name=extends-br` juste avant suggère que l'enregistrement du composant a partiellement réussi puis qu'un accès à un objet hôte non implémenté (registry `customElements` incomplet ? `document.currentScript` ?) renvoie `Undefined`. **Crate : `explorer-jscore`** (API DOM hôte ou binding de module).
2. **`syntaxe 1:6272 : expected ','` sur un script de 13 469 octets** : parse échoue à ~47 % — idiome non couvert (probablement paramètre de fonction avec default destructuré, `...` en position inattendue, ou `import()` dynamique). À extraire et réduire. **Crate : `explorer-jscore` (parser)**.
3. **`TypeError: Invalid scheme` dans un listener `DOMContentLoaded`** : récurrence de la famille « URL » des vols précédents — un handler lit `new URL(...)` ou `fetch()` sur une URL relative/schéma non résolu **au moment de l'événement** (pas à l'eval). **Crate : `explorer-jscore`** (résolution d'URL dans le contexte listener).
4. **`16 mutations classées` — record** : les scripts (GTM `dataLayer`, modules figaro) produisent 16 mutations DOM que le journal ADR-0025 **classe** (ni appliquées ni repli déclenché). Premier cas où `classées > replis` : le journal tolère mais **ignore** ces mutations — risque d'incohérence DOM rendu/DOM vivant. **Crate : `explorer-dom`/`explorer-core`** — instrumenter *quoi* est classé (attribut ? insertion ?).
5. **Menu déplié** : même signature que franceinfo → piste `explorer-css` (masquage `display:none`/classe toggled par JS non retirée).

#### 🧪 Repro Minimal Test :
```javascript
// 1. Module + customElements + .call
customElements.define("x-a", class extends HTMLElement {});
const f = function(){}; f.call(null);            // OK attendu
const g = window.queueMicrotask; g.call(null, ()=>{});

// 2. Listener + URL relative
document.addEventListener("DOMContentLoaded", () => {
  new URL("./x", location.href);
});
```

---

### ✈️ Vol #03 — `https://www.ouest-france.fr/` (SIPA UI + lazy `data-oflazyload`)
- **Stack technique** : SIPA Press (`window.sipaui_debug`, `window.firstparty`/`thirdparty`/`cmsfetch` tableaux de gating par consentement, `window.refonteOf=1`), images via `data-oflazyload="{"offset":200,"srcset":"…"}"` (JSON dans attribut — jamais résolu sans JS), régions via `bp_algolia_organizations` (JSON embarqué massif : annuaire des titres SIPA), `fetchpriority="low"` sur les images.
- **Rendu visuel** (`captures/2026-10-26_13-46-01_…ouest-france.fr.png`, 2 541 621 octets) : **page reconnaissable mais overlays partout** — logo Ouest-France rouge peint correctement, date « Lundi 26 octobre 2026 », titre direct « DIRECT. Manifestations : samedi prochain sera « à risque », prévient Emm… » en bleu géant **débordant du viewport** ; MAIS : (a) nav horizontale tronquée (« Communes… », « che… », « ferm… » coupés), (b) **panneau « À lire — Vous devez vous connecter pour afficher vos articles sauvegardés / fermer » peint ouvert** en haut à droite (overlay qui devrait être masqué), (c) **lecteur média plein cadre** peint en bas (« Reculer / Avancer lecture / 00:00 / fermer »), (d) grand vide central — la grille d'articles dépend du JS/JSON qui n'a pas hydraté, les images `data-oflazyload` restent vides.
- **Perf** : fetch+parse ms=1691, commit ms=4538 — le plus rapide du vol.

#### 💥 Mur Détecté (JS `explorer-jscore` — scripts async) :
```text
script async en échec tab=1 error=exception js : TypeError: value is not a function
script async en échec tab=1 error=exception js : TypeError: cannot read `push` of Undefined
script async en échec tab=1 error=exception js : TypeError: cannot read `push` of Undefined
script async en échec tab=1 error=exception js : TypeError: cannot read `push` of Undefined
[journal : 0 appliqués / 0 replis / 3 classées]
```

#### 🔍 Analyse Racine :
1. **`cannot read 'push' of Undefined` ×3** : les scripts SIPA font `window.firstparty.push({...})`, `window.thirdparty.push(...)`, `window.cmsfetch.push(...)` — l'inline qui **initialise ces tableaux** (`window.firstparty=[],window.thirdparty=[],…` visible dans le dump ligne 1191) s'est bien évalué… ou **pas dans le bon ordre / pas propagé à `window`** : c'est potentiellement la **même famille que le bug `lmd` du Monde** (vol 1) — assignation `window.x=[…]` exécutée mais `x.push` lu comme global non résolu, ou le script inline a tourné dans un autre realm. **Crate : `explorer-jscore`** (liaison `window.*` ↔ scope global des scripts async — hypothèse forte à confirmer en rejouant l'ordre d'éval).
2. **`value is not a function`** : un appel sur une valeur non fonction (probablement `Object.observe`-like, ou `callback()` sur un stub CMP absent).
3. **3 mutations classées** : même constat qu'au Figaro en plus petit — le journal ADR-0025 tolère sans appliquer.
4. **Overlays peints** : « À lire », lecteur média, et nav tronquée → `explorer-css`/`explorer-layout` : règles de masquage (attribut `hidden`, classes `is-*`, z-index/`position:fixed` hors-écran) non appliquées, et **inline size non contraint** (titre DIRECT qui déborde = `overflow`/`text-overflow` ou largeur flex non bornée).
5. **`<#comment>` ×36** dans le dump — anomalie `explorer-html` confirmée sur une 3e stack.

#### 🧪 Repro Minimal Test :
```html
<script>window.firstparty=[],window.thirdparty=[];</script>
<script async>window.firstparty.push({a:1}); thirdparty.push({b:2});</script>
<!-- attendu : aucun `cannot read 'push' of Undefined` -->
```

---

### ✈️ Vol #04 — `https://www.dw.com/fr/` → `/fr/actualités/s-10261` (SPA SSR + `__APP_STATE__`)
- **Stack technique** : application isomorphe type Nuxt/Vue SSR — `window.__APP_STATE__` JSON massif (toutes les traductions i18n fr embarquées : `home_top_story`, `most_clicked`, `gdpr.consent_layer`…), bundle `/static/index-zqmBTyBB.js` en module prefetché, CMP maison (`gdpr.consent_layer` dans l'état), `fetchpriority="high"` sur l'image une.
- **Incident de vol** : premier essai sur un chemin invalide → **404** (mauvaise URL saisie) ; refly sur `https://www.dw.com/fr/` : **redirect HTTP suivie correctement** vers `/fr/actualités/s-10261` (dump `[url : https://www.dw.com/fr/actualit%C3%A9s/s-10261]`) — la chaîne `explorer-net` gère proprement redirect + URL UTF-8 percent-encodée. Point positif.
- **Rendu visuel** (`captures/2026-10-26_13-50-13_…dw.com.png`, 27 308 007 octets) : **le pire rendu du vol** — fond bleu nuit dominant (feuille de style partiellement appliquée : la couleur de fond passe mais pas la grille), header cyan peint, contenu **entièrement en colonne de liens bleus** empilés (titres d'articles, tags, timestamps « 26.10.2026 »), **deux grands rectangles noirs** (players vidéo dimensionnés sans contenu), footer « Service / À propos de DW » peint correctement. Le SSR livre le HTML complet mais l'hydratation/la grille CSS (probablement grid/flex moderne ou classes toggled par JS) ne s'applique pas → tout s'empile en flux block.
- **Perf** : fetch+parse ms=356 puis 254 (page SSR compacte), commit ms=2446 — le plus rapide ; journal `0/0/0`.

#### 💥 Mur Détecté (JS `explorer-jscore`) :
```text
script: eval fin tab=1 ms=3 ok=false
script en échec tab=1 error=exception js : TypeError: cannot read `document` of Null
module en échec tab=1 error=exception js : Error: Illegal component name, "";
    must be a non-empty string.
[journal : 0 appliqués / 0 replis / 0 classées]
```

#### 🔍 Analyse Racine :
1. **`cannot read 'document' of Null`** — erreur **nouvelle et inédite** : un script lit `X.document` où `X` est `null` (et non `undefined`). Candidats : `window.frames[i].document`, `iframe.contentWindow.document`, ou `event.target.ownerDocument` sur une cible nulle. Le `Null` vs `Undefined` dans le message montre que `explorer-jscore` distingue les deux — l'objet hôte renvoyé est **explicitement null** : vérifier `contentWindow`/`contentDocument`/`ownerDocument` des stubs DOM hôte. **Crate : `explorer-jscore`** (dom_host).
2. **`Illegal component name, ""`** dans le module `index-*.js` : le runtime de composants (type Vue `app.component(name, …)` ou `customElements.define("", …)`) reçoit un nom vide — probablement un composant enregistré via une prop calculée que le moteur évalue à `""`, ou `customElements.define` appelé avec une chaîne vide alors que notre hôte valide strictement (le vrai navigateur lèverait aussi, mais le site fonctionne → le nom vide vient d'une expression que `explorer-jscore` évalue mal, ex. `tag.name`/`constructor.name` sur classe minifiée). **Crate : `explorer-jscore`** (réflexion `Function.name`/binding module).
3. **Rendu colonne cassé** : `explorer-css` — la grille `display:grid`/variables du design-system DW ne s'applique pas ; le fond passe mais le layout non → vérifier si une feuille `@media`/`@supports` a été droppée au parse.
4. Seuls **8 evals** au total : le site est quasi entièrement servi par un module unique prefetché — quand il échoue, plus rien n'hydrate. Fragilité « tout dans un bundle ».

#### 🧪 Repro Minimal Test :
```javascript
// 1. Accès document sur hôte null
const f = document.createElement("iframe");
f.contentWindow.document; // → ne doit pas lever "cannot read `document` of Null"

// 2. Nom de composant dérivé
class MyComp {}
customElements.define(MyComp.name.toLowerCase(), MyComp);
// ou app.component(Foo.name, Foo) — vérifier .name sur classes minifiées
```

---

## 🧭 Enseignements transverses (vol 2 vs vol 1)

1. **`<#comment>` : anomalie confirmée et généralisée** — présente dans les 4 dumps du vol 2 (guardian ×119, figaro ×36, ouest-france ×36, dw ×4) après franceinfo au vol 1. Le dump montre la pathologie exacte : le tree-builder émet **deux nœuds pour un seul construct** — un nœud texte `"<#comment>"` **puis** un nœud commentaire `<!-- #comment -->` (figaro lignes 1181-1182). Hypothèse : ces sites servent des placeholders de template littéraux `<#comment>` ; `explorer-html` tokenise `<#comment>` à la fois comme *bogus comment* (→ nœud commentaire) **et** comme texte (→ nœud texte peint). **Crate fautif : `explorer-html`** — bug concret, reproductible, maintenant **prioritaire** car il touche 5 sites/8 mesurés.

2. **Nouvelle classe de bug `explorer-jscore` : scope global des scripts** — ouest-france `cannot read 'push' of Undefined` ×3 (tableaux `window.firstparty`/`thirdparty`/`cmsfetch` initialisés par un inline puis lus par des async) fait fortement penser au bug `lmd` du Monde (vol 1) : **la propagation `var`/`window.x =` top-level vers la portée globale partagée entre scripts est suspecte**. À traiter comme un seul chantier « global object binding ».

3. **Lexer regexp : trou béant** — `[\uD800-\uDBFF][\uDC00-\uDFFF]?` est un idiome standard de splitting UTF-16 présent dans d'innombrables bundles ; l'échec « Target of repeat operator is invalid » sur quantifier de classe est un repro d'une ligne.

4. **Overlays/popovers peints ouverts : 3e et 4e occurrences** — après le menu franceinfo (vol 1), même signature sur figaro (nav dépliée) et ouest-france (panneau « À lire » + lecteur plein cadre). **Crate suspect : `explorer-css`** — masquage par attribut `hidden`, `popover`, ou classes d'état jamais retirées par le JS d'init.

5. **Journal ADR-0025 : nouvelle dynamique** — premier cas `classées > 0` sans repli : figaro **0/0/16**, ouest-france 0/0/3. Les mutations « classées » sont tolérées-silencieusement : risque de divergence DOM peint / DOM vivant. Guardian conserve le repli `UnmappedKey` ×1/nav. Instrumenter la *nature* des mutations classées devient nécessaire.

6. **Poids des captures = hauteur × images** — guardian 54 Mo n'est pas un bug raster : c'est une page ~30 écrans avec photos décodées. Mais cela expose un **risque capacitaire** : framebuffer pleine page non tuilé. À corréler avec `explorer-render` si des pages >50 MPix deviennent courantes ; repaint ms=16059 est le coût réel.

7. **Hiérarchie de maturité du vol 2** : aucun site « propre » cette fois — le moins mauvais est figaro (contenu éditorial lisible malgré menu et journal 16) ; dw est le plus dégradé (hydratation unique-bundle tombée → page en colonne). Contraste avec bbc.com (vol 1) : **le SSR-first robuste passe, le SSR-qui-exige-l'hydratation cascade en rendu dégradé**.

8. **Redirect + URL non-ASCII** : dw `/fr/` → `/fr/actualités/s-10261` suivie proprement (percent-encoding `actualit%C3%A9s` dans le dump) — `explorer-net` solide sur ce point.

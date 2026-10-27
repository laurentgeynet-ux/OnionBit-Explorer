# 🛰️ RAPPORT D'ANALYSE EN VOL : ÉQUIPE 7 — RÉSEAUX SOCIAUX

**Périmètre** : Premier panier de l'équipe 7 — réseaux sociaux à fort mur applicatif : Reddit ancien et nouveau (`old.` = login-wall dur, `www.` = défi anti-bot Shreddit), YouTube mobile (shell `watch-skeleton` Polymer/Web Components), Mastodon (React SPA à modules JS).
**Objectif** : Éprouver le moteur face aux trois obstacles dominants du social web : redirections d'authentification, challenges anti-bot servis côté serveur, et hydratation intégrale par Web Components/modules ES.
**Suite** : `RAPPORT_MISSION_EQUIPE_7_SOCIAL_VOL2.md` (Twitch, Pinterest, Quora, Discord) — où apparaît le motif `NoBaseline` absent de ce premier vol (ici : `UnmappedKey` uniquement).

---

## 📋 Synthèse des vols

| N° | Date / Heure (captures PNG) | URL cible | URL finale | Statut | Anomalie / Diagnostic clé |
|:---:|:---:|:---|:---|:---:|:---|
| **01** | 2026-10-26 13:27 → 13:28 | `https://old.reddit.com/` | `https://old.reddit.com/login/?reason=lor2&dest=…` | 🟠 MUR EXTERNE | **Login-wall serveur** : redirection 302 vers `/login/` — la page de connexion (stack **Shreddit** servie aussi sur `old.`) est peinte (3,66 Mo). Repli ADR-0025 `why=UnmappedKey`, journal `0/1/3`. **3 `import()` dynamiques `data:text/javascript` refusés** (`résolution/CSP refusé`) par navigation — polyfills-loader + Sentry loader tués avant évaluation. |
| **02** | 2026-10-26 13:28 → 13:29 | `https://www.reddit.com/` | `https://www.reddit.com/?solution=…&js_challenge=1&jsc_token=…&jsc_orig_r=` | 🟠 ANOMALIE | **Défi anti-bot** : la 1ʳᵉ réponse est un challenge JS (`js_challenge=1`), la 2ᵉ navigation aboutit sur l'URL signée. Shell **Shreddit** peint (3,55 Mo) mais **jamais hydraté** : `customElements.define` exécutés (`suspense-placeholder`, `suspense-replace`, `suspense-execute-deferred-scripts`) puis le `import()` dynamique `data:` qui arme `window.navigation`/Sentry est **refusé** (`résolution/CSP refusé`, `ok=false deps=0`) → pas de boot applicatif. 50 evals `ok=true`, **0 `en échec`**, repli `UnmappedKey`, journal `0/1/3`. |
| **03** | 2026-10-26 13:29 → 13:31 | `https://m.youtube.com/` | `https://www.youtube.com/?app=desktop` | 🔴 ANOMALIE | **Squelette `watch-skeleton` peint, hydratation morte + exceptions avalées** : 86 evals, **6 `en échec`** (×3 par navigation : `__shady_native_addEventListener is not a function`, `initCustomEvent is not a function`, **`syntaxe js : invalid regex`** pos. 12071:1763) — toutes tracées en **DEBUG uniquement**, journal `0/0/0` : aucune erreur ne remonte au résumé de session (anomalie d'observabilité). **PNG 48,7 Mo** — le plus gros artefact du panier, bornage/hauteur de page anormal. |
| **04** | 2026-10-26 13:30 → 13:33 | `https://mastodon.social/` | `https://mastodon.social/` | 🟠 ANOMALIE | **`<div id="mastodon">` vide** : les 3 packs ES modules (`polyfills`, `common`, `application` — 64 sources prefetchées) s'évaluent, puis `application-*.js` échoue sur **`TypeError: cannot load module './axios-Blsu9zSB.js': module non chargé`** (import de chunk non résolu — le prefetch module ne couvre pas les imports dynamiques/relatifs du graphe) → React jamais booté, journal `0/0/0`, PNG 1,87 Mo. |

---

## 🔬 Fiches Techniques Détaillées

### ✈️ Vol #01 — `https://old.reddit.com/` → `/login/?reason=lor2` (login-wall + dyn imports `data:`)

- **Stack technique** : le domaine `old.` sert désormais la page de connexion sur la stack **Shreddit** : scripts `www.redditstatic.com/shreddit/polyfills/*.js` (es-module-shims, regenerator-runtime, fetch, intersection-observer, webcomponents, lit-polyfill-support, core-js), `customElements.define name=suspense-execute-deferred-scripts`, loader Sentry lazy et `browsers_list_default` (filtrage anti-bot par UA) embarqués dans un `import()` `data:text/javascript`.
- **Rendu visuel** (`captures/2026-10-26_13-27-15_Equipe_7_Social_old.reddit.com_.png` puis `…_13-28-44_…old.reddit.com.png`, 3 660 743 octets) : **page de login Reddit peinte** — formulaire servi en HTML, peinture complète. Le contenu demandé (listing) est hors d'atteinte : mur d'authentification externe, conforme.
- **stderr** (`captures/e7_oldreddit_stderr.log`) :

```text
WARN explorer_core::session::apply: journal DOM rejeté → repli re-parse (ADR-0025)
     tab=1 why=UnmappedKey
[url : https://old.reddit.com/login/?reason=lor2&dest=https%3A%2F%2Fold.reddit.com%2F]
```

#### 💥 Signaux dans le log (`logs/eq7_old.reddit.com.log`) :

```text
nav: fetch+parse ok → peinture tab=1 ms=303
nav: sous-ressources ok → commit tab=1 ms=668
script: eval fin tab=1 ms=0-42 ok=true            (× 17 / navigation, 34 au total)
DEBUG explorer_jscore::module: import() dynamique spec="data:text/javascript,…"
DEBUG explorer_core::session::net: dyn import : résolution/CSP refusé — rejet tab=1
     url=data:text/javascript,…                   (× 3 par navigation, seq=0..2)
deliver dyn import tab=1 seq=N ok=false deps=0
WARN  apply: journal DOM rejeté → repli re-parse (ADR-0025) why=UnmappedKey
[journal : 0 appliqués / 1 replis / 3 classées]
apply: cascade ms=316 / boxtree ms=350 / layout ms=361 / repaint ms=459 / fin ms=533
```

#### 🔍 Analyse Racine :

1. **Mur externe nominal** : la redirection `/ → /login/?reason=lor2` est suivie sainement (`explorer-net`), la page peinte est celle servie — pas un bug moteur.
2. **`import()` sur `data:` URL refusé ×3** : le polyfills-loader (`e.push(e('…polyfills/es-module-shims…'))`, polyfill `fetch`, `IntersectionObserver`, `attachShadow`/lit) et le bootstrap Sentry/`window.navigation` sont injectés par `import()` dynamique sur `data:text/javascript`. Notre pipeline `spawn dyn import → résolution/CSP refusé → deliver ok=false` les **rejette tous**. Effet : le code qui aurait armé les globals (`window.navigation` polyfill, fetch instrumenté, sendBeacon) ne tourne jamais. Crate : `explorer-core` (politique de résolution des `import()` — les `data:` URL modules sont légaux et massifs en pratique).
3. **`UnmappedKey` + `3 classées`** : les mutations produites par les scripts inline de la page login (probablement l'injection du contexte/`customElements`) ne ré-apparient pas la table miroir → re-parse de repli conforme ADR-0025, coût ~530 ms par apply.
4. **Aucune exception JS** : toutes les evals synchrones passent — l'échec est dans la *livraison* des modules dynamiques, pas dans le langage.

#### 🧪 Repro Minimal Test :

```javascript
// import() dynamique sur data: URL — actuellement refusé par
// explorer_core::session::net (résolution/CSP)
const m = await import("data:text/javascript,export default 42");
assert(m.default === 42);
// attendu moteur : résoudre et évaluer le module inline ;
// observé : "résolution/CSP refusé — rejet", ok=false
```

---

### ✈️ Vol #02 — `https://www.reddit.com/` (défi anti-bot + shell Shreddit non hydraté)

- **Stack technique** : réponse #1 = **challenge anti-bot** (`?js_challenge=1&jsc_token=…`) — deux `nav: fetch+parse` consécutives par passage ; réponse #2 = shell applicatif Shreddit : `shreddit-app`, `suspense-*` custom elements, polyfills-loader identique au vol #01, Sentry lazy loader, `browsers_list_default` regex de filtrage UA, modules `redditstatic.com/shreddit/*.js`.
- **Rendu visuel** (`captures/2026-10-26_13-28-52_Equipe_7_Social_www.reddit.com.png` / `…_13-29-00_…_.png`, 3 545 716 octets) : **shell Shreddit peint** — chrome de page (masthead, cadre `shreddit-app`, `shreddit-gallery-listener` visibles dans le dump DOM) mais aucun contenu : l'hydratation Web Components n'a jamais démarré.
- **stderr** (`captures/e7_www.reddit.com_stderr.log`) : `why=UnmappedKey`, `[journal : 0 appliqués / 1 replis / 3 classées]`.

#### 💥 Signaux dans le log (`logs/eq7_www.reddit.com.log`) :

```text
nav: fetch+parse… tab=1  (×2 — challenge puis URL signée)
nav: sous-ressources ok → commit tab=1 ms=1855
customElements.define name=suspense-placeholder
customElements.define name=suspense-replace
customElements.define name=suspense-execute-deferred-scripts
import() dynamique spec="data:text/javascript,let navPromise=null,
     hasNativeNavigation=typeof window<'u'&&!!window.navigation;
     async function getNavigation(){…await import(`…/apply-polyfill-V7rfsCUB.js`)…}"
dyn import : résolution/CSP refusé — rejet tab=1   (× 3 par navigation)
WARN apply: journal DOM rejeté → repli re-parse (ADR-0025) why=UnmappedKey
[journal : 0 appliqués / 1 replis / 3 classées]
apply: cascade ms=1650 / boxtree ms=1741 / layout ms=1893 / repaint ms=3893 / fin ms=4123
[url : https://www.reddit.com/?solution=b7195d8f…&js_challenge=1&jsc_token=2824be10…&jsc_orig_r=]
```

#### 🔍 Analyse Racine :

1. **Le défi est *passé* côté réseau** : l'URL finale porte `solution=…&jsc_token=…` — le serveur a accepté le second fetch et servi le shell. `explorer-net` n'est pas en cause ; le challenge côté client (la partie JS qui aurait calculé `solution`) est contourné ou servi résolu.
2. **Hydratation bloquée par le même mur `data:` que le vol #01** : le module `getNavigation()`/`sentryLazyLoader` est le *premier maillon* du boot Shreddit (`window.navigation` polyfill → `import('apply-polyfill…')` → Sentry → `requestIdleCallback` → boot). Refusé à la résolution → `deliver ok=false deps=0` → **toute la chaîne d'hydratation est morte en amont**, sans exception visible (le rejet n'est pas une erreur JS, c'est une livraison `ok=false` en DEBUG). Crate : `explorer-core` (résolution `import()` `data:`) — **cause racine commune aux deux Reddit**.
3. **50 evals `ok=true`, 0 `en échec`** : tout le JS *synchrone* (dont l'enregistrement des custom elements `suspense-*`) s'évalue parfaitement — le langage n'est pas le goulot ; c'est le *chargement de modules* (`explorer-jscore::module` + politique `explorer-core`).
4. **Repaint ms=3893** : le re-parse de repli ADR-0025 sur le shell Shreddit (DOM large, CSS embarqué lourd) se paie ~4 s par apply — coût de fallback à surveiller.

#### 🧪 Repro Minimal Test :

```html
<my-el></my-el>
<script type="module">
  customElements.define("my-el", class extends HTMLElement {
    connectedCallback() { this.textContent = "hydrated"; }
  });
  // chaîne équivalente au boot Shreddit :
  import("data:text/javascript,globalThis.__booted=true")
    .then(() => document.body.dataset.boot = "1");
</script>
<!-- attendu : data-boot="1" ; observé : import refusé, rien -->
```

---

### ✈️ Vol #03 — `https://m.youtube.com/` → `www.youtube.com/?app=desktop` (exceptions avalées + PNG 48,7 Mo)

- **Stack technique** : redirection `m. → www. ?app=desktop` ; page desktop « Kevlar » : scripts `jsbin/` (`webcomponents-all-noPatch`, `fetch-polyfill`, `intersection-observer`, `scheduler`, `spf`, `network`, `web-animations-next-lite`), bundle `ytmainappweb.kevlar_base.*.es5`, `ytcfg.set({…})` avec ~700 flags d'expérience, squelette SSR `ytd-masthead.shell` + `home-page-skeleton` / `rich-grid-media-skeleton` (placeholders `skeleton-bg-color`).
- **Rendu visuel** (`captures/2026-10-26_13-29-19_Equipe_7_Social_m.youtube.com.png` / `…_13-31-00_…_.png`, **48 726 367 octets — anormal**) : le squelette home est peint (masthead + grille de vignettes fantômes), mais la taille du PNG suggère un document **extrêmement haut** — le bornage viewport/hauteur de page n'a pas joué, ou la grille skeleton s'est déroulée sans limite (les placeholders `rich-grid-media-skeleton` se répètent en boucle dans le dump DOM).

#### 💥 Signaux dans le log (`logs/eq7_m.youtube.com.log`) :

```text
nav: sous-ressources ok → commit tab=1 ms=8843      (puis ms=5791 au 2ᵉ passage)
DEBUG js_glue: script en échec tab=1 error=exception js :
  TypeError: `__shady_native_addEventListener` is not a function (on Object)
DEBUG js_glue: script en échec tab=1 error=exception js :
  TypeError: `initCustomEvent` is not a function (on Event)
DEBUG js_glue: script en échec tab=1 error=syntaxe js 12071:1763 :
  invalid regex /[!-#%-\*,-/:;\?@\[-\]_\{\}\xA1\xA7…\u05C3\u05C6\u05F…]/
   (la même série ×3 est rejouée à la seconde navigation — 6 échecs au total)
script: eval fin tab=1 … ok=true                  (× 86)
[journal : 0 appliqués / 0 replis / 0 classées]
apply: repaint ms=4027
captures/…_m.youtube.com.png — 48726367 octets
[url : https://www.youtube.com/?app=desktop]
```

#### 🔍 Analyse Racine :

1. **`__shady_native_addEventListener is not a function`** : `webcomponents-all-noPatch.js` (ShadyDOM) tente de patcher `EventTarget.prototype.addEventListener` vers `__shady_native_*` — la glue DOM `explorer-jscore`/`dom_host` n'expose pas le hook natif attendu par le polyfill → le polyfill crash **avant** d'installer les Web Components → Polymer ne démarre jamais. Crate : `explorer-jscore` (glue DOM — symboles natifs attendus par les polyfills webcomponents.js).
2. **`initCustomEvent is not a function (on Event)`** : API DOM Legacy absente (`document.createEvent`/`initCustomEvent`) — requise par `spf.js`/l'event bus YouTube. Crate : `explorer-jscore` (glue DOM, API legacy `Document.createEvent`).
3. **Premier `invalid regex` du vol** (`syntaxe js 12071:1763`) : classe de caractères Unicode générée (plages `\u037E…\u05C3` — ponctuation/symboles, probablement regexpu ou le tokenizer de `www-i18n-constants`) rejetée par le lexer regex. **Même famille que le vol 2 (Pinterest `pos. 1884`)** → confirme un trou systématique du lexer sur les échappements `\xXX`/`\uXXXX` étendus en classe. Crate : `explorer-jscore` (lexer regex).
4. **Anomalie d'observabilité (transverse)** : les 6 échecs ne sortent qu'en `DEBUG js_glue` — le journal de session reste `0 appliqués / 0 replis / 0 classées` et **aucun WARN ne comptabilise les exceptions JS**. Un lecteur de niveau INFO/WARN conclurait « tout est vert ». Crate : `explorer-js`/`explorer-core` — les `script en échec` devraient être WARN comptabilisés.
5. **PNG 48,7 Mo** : soit le framebuffer n'est pas borné à un plafond de hauteur, soit la grille skeleton non stylée (CSS de `www-main-desktop-home-page-skeleton.css` partiellement appliquée) s'étale sur des dizaines de milliers de pixels. Crate : `explorer-render`/`explorer-layout` — à reproduire en mesurant la hauteur du boxtree.

#### 🧪 Repro Minimal Test :

```javascript
// 1) Hook natif attendu par webcomponents.js/ShadyDOM :
assert(typeof EventTarget.prototype.addEventListener === "function");
// le polyfill fait : const native = EventTarget.prototype.addEventListener;
//                    globalThis.__shady_native_addEventListener = native;
// vérifier que la glue expose une sémantique compatible.

// 2) API legacy :
const e = document.createEvent("CustomEvent");
e.initCustomEvent("x", true, true, null);

// 3) Classe regex unicode (famille du motif fautif) :
const r = /[!-#%-\*,-/:;\?@\[-\]_\{\}\xA1\xA7\xAB\u037E\u0387\u055A-\u055F]/;
assert(r.test("¡") === true);
```

---

### ✈️ Vol #04 — `https://mastodon.social/` (module graph — `axios` non chargé)

- **Stack technique** : Rails + React SPA, packs ES modules (`/packs/polyfills-*.js`, `common-*.js`, `application-*.js` — `application` prefetché avec **64 sources**), `skeleton-*.js`, JSON initial `meta`/`compose`/`languages` embarqué (~30 Ko), point de montage `<div id="mastodon">`, message `<noscript>` « please enable JavaScript ».
- **Rendu visuel** (`captures/2026-10-26_13-30-01_Equipe_7_Social_mastodon.social.png` / `…_13-33-00_…_.png`, 1 869 079 octets) : page Mastodon **quasi vide** — le `<div id="mastodon">` reste vide dans le dump DOM final : React n'a jamais hydraté.

#### 💥 Signaux dans le log (`logs/eq7_mastodon.social.log`) :

```text
sous-ressource script tab=1 src="…/packs/application-CBLJxgs9.js"
     module=true sec_ok=true sri_ok=true
module prefetché tab=1 name=…/application-CBLJxgs9.js sources=64
module eval fin tab=1 ms=173    (polyfills)
module eval fin tab=1 ms=59     (common)
module en échec tab=1 error=exception js :
  TypeError: cannot load module `./axios-Blsu9zSB.js`:
  module non chargé : https://mastodon.social/packs/axios-Blsu9zSB.js
   (×2 — rejoué à la seconde navigation, ms=…)
nav: sous-ressources ok → commit tab=1 ms=4614
apply: cascade ms=393 / boxtree ms=429 / layout ms=473 / repaint ms=498
[journal : 0 appliqués / 0 replis / 0 classées]
```

#### 🔍 Analyse Racine :

1. **Chunk dynamique hors du graphe prefetché** : `application-*.js` est prefetché avec 64 sources, mais `axios-Blsu9zSB.js` — importé en `import()` dynamique *ou* en import relatif résolu tardivement — n'en fait pas partie : `module non chargé` signifie que le chargeur de modules `explorer-jscore` exige que la cible soit déjà dans la table (`prefetché`) et ne sait pas la charger à la demande. **Crate : `explorer-jscore` (résolution de modules — `import()` dynamique sans fetch à la demande) + `explorer-core` (politique de prefetch qui ne suit pas les imports dynamiques)**.
2. **Erreur unique, effet total** : un seul `TypeError` en tête du graphe tue `application` entier → `ReactDOM.createRoot` jamais appelé → `#mastodon` vide. Dégradation propre (pas de crash, PNG servi) mais rendu nul pour l'utilisateur.
3. **Journal `0/0/0`** : aucune mutation n'a même été tentée — cohérent avec un crash *avant* tout code applicatif.
4. **Contraste vol #02** : ici l'import fautif est une **URL relative de chunk** (légitime, même origine, même répertoire `/packs/`) — plus simple à corriger que le cas `data:` : il manque un fallback « fetch + instantiate à la demande » dans le chargeur.

#### 🧪 Repro Minimal Test :

```javascript
// module principal prefetché, chunk dynamique absent du graphe :
// main.js :  export const go = () => import("./chunk.js");
// chunk.js : export default 1;
// attendu : fetch + éval de chunk.js à la demande
// observé : TypeError "cannot load module './chunk.js': module non chargé"
```

---

## 🧭 Enseignements transverses (vol 1)

1. **`import()` dynamique = le mur n°1 du panier social.** Trois sites sur quatre bloqués par le chargement de modules à la demande : `data:` URL refusés (reddit ×2, `résolution/CSP refusé`), chunk relatif non préchargé (mastodon, `module non chargé`). Deux trous distincts dans le même tuyau `explorer-core::session::net` + `explorer-jscore::module` : (a) accepter les `data:`/`blob:` URL comme cibles de module, (b) fetcher un chunk non prefetché à la demande. Fix unique à fort rendement : c'est *le* point d'entrée du boot de Reddit et Mastodon.
2. **Observabilité : les exceptions JS n'atteignent jamais le niveau WARN.** YouTube : 6 `script en échec` (2 TypeError DOM + 1 syntaxe regex, rejouées ×2) tracées en DEBUG, journal `0/0/0` — un rapport fondé sur le journal conclurait à tort « aucune erreur ». Action : compter `script en échec`/`module en échec` dans le résumé `[journal]` ou un WARN dédié (`crates/explorer-core/src/session/js_glue.rs` — les lignes `en échec` sont déjà `DEBUG`, à promouvoir).
3. **`UnmappedKey` (ADR-0025) sur les deux Reddit** : le motif du vol 1 est « clé précise non appariée » — les mutations du shell Shreddit (custom elements, attributs `nonce`, injection contexte) ne retrouvent pas leurs nœuds dans la keymap. Le vol 2 montrera `NoBaseline` (pire) ; ici le repli reste *ciblé* mais systématique sur cette stack.
4. **Premier `invalid regex` de la série** : la classe Unicode multi-échappements de YouTube (`pos. 12071:1763`) et celle de Pinterest au vol 2 (`pos. 1884`) dessinent un trou récurrent du lexer regex `explorer-jscore` sur les sorties regexpu/transpilées — à traiter comme famille de bugs, pas cas isolés.
5. **Budgets/bornage** : le PNG YouTube de 48,7 Mo (vs 1,8–3,7 Mo ailleurs) suggère un document sans plafond de hauteur rasterisée — vérifier le bornage viewport `explorer-render` et la hauteur produite par la grille skeleton non hydratée.
6. **Murs externes identifiés, hors périmètre moteur** : login-wall `old.reddit` (302 → `/login/`), défi anti-bot `www.reddit` (résolu côté serveur à la 2ᵉ passe — bon signe pour `explorer-net`).

---

## 🎯 Top anomalies — ordre d'action suggéré

| Rang | Anomalie | Crate | Sites touchés |
|:---:|:---|:---:|:---|
| 1 | `import()` dynamique refusé — `data:` URL (`résolution/CSP refusé`) et chunks relatifs non prefetchés (`module non chargé`) | `explorer-core` + `explorer-jscore::module` | reddit ×2, mastodon — **bloque tout boot SPA** |
| 2 | Exceptions JS avalées en DEBUG — journal `0/0/0` malgré 6 échecs | `explorer-js`/`explorer-core` (observabilité) | youtube (et tout le panier, sous-diagnostic) |
| 3 | PNG 48,7 Mo — hauteur framebuffer/boxtree non bornée sur skeleton | `explorer-render`/`explorer-layout` | youtube |
| 4 | Glue DOM manquante : `__shady_native_addEventListener`, `initCustomEvent`/`createEvent` | `explorer-jscore` (glue DOM) | youtube (polyfills webcomponents.js/spf) |
| 5 | `invalid regex` classes `\xXX`/`\uXXXX` étendues | `explorer-jscore` (lexer regex) | youtube (famille confirmée au vol 2 : pinterest) |
| 6 | `UnmappedKey` systématique sur stack Shreddit | `explorer-core` (keymap ADR-0025) | old. + www.reddit.com |

# 🛰️ RAPPORT D'ANALYSE EN VOL : ÉQUIPE 7 — RÉSEAUX SOCIAUX & APPS (VOL 2)

**Périmètre** : Second panier de l'équipe 7 — SPA de streaming (Twitch), réseaux à login-wall doux (Pinterest, Quora) et site marketing applicatif (Discord/Webflow).
**Objectif** : Éprouver le moteur sur les architectures les plus hostiles du panier : React SPA sans SSR utile (Twitch/Twilight), shells PPR « shellReady » (Pinterest), redirections de locale (`.fr → fr.*`), et bundles Webflow/Lottie à propriétés figées (Discord).
**Continuité vol 1** : ce rapport complète `RAPPORT_MISSION_EQUIPE_7_SOCIAL.md` (reddit ×2, m.youtube.com, mastodon.social). Il y apparaît un **motif de repli ADR-0025 inédit** : `why=NoBaseline`.

---

## 📋 Synthèse des vols

| N° | Date / Heure | URL cible | URL finale | Statut | Anomalie / Diagnostic clé |
|:---:|:---:|:---|:---|:---:|:---|
| **01** | 2026-10-26 13:43 | `https://www.twitch.tv/` | `https://www.twitch.tv/` | 🟠 ANOMALIE | **Hydratation silencieuse** : 88 scripts évalués `ok=true`, **0 exception, journal 0/0/0** — le runtime Twilight tourne sans erreur mais ne commit aucune mutation DOM visible ; PNG 1,85 Mo = shell SPA non hydratée (mur React, rien à journaliser). |
| **02** | 2026-10-26 13:43 | `https://www.pinterest.fr/` | `https://fr.pinterest.com/` | 🟠 ANOMALIE | **Erreur de syntaxe regex `/u`** : `invalid regex … Parsing error at position 1884: Invalid codepoint for hex or unicode escape` sur le bundle `s.pinimg.com` — échappement astral `\uD800[…]` non couvert par le lexer regex `explorer-jscore`. 20 evals ok, journal 0/0/**8 classées**, repli absent. |
| **03** | 2026-10-26 13:44 | `https://www.quora.com/` | `https://fr.quora.com/` | 🟠 ANOMALIE | **`why=NoBaseline` — motif jamais observé au vol 1** (×2, une fois par navigation) + **`moteur figé — éval dépassée`** : un JS livré par le réseau a dépassé le budget d'éval. 32 evals ok, 0 exception syntaxe, journal 0/1/2. |
| **04** | 2026-10-26 13:45 | `https://discord.com/` | `https://discord.com/` | 🔴 ANOMALIE | **4 exceptions JS** (rejouées ×2 navigations) : `cannot define property 'QuickEffectIds'` (Webflow schunk), `cannot read 'open' of Undefined` (eval **20 578 ms** — le plus long du panier), `cannot read 'lottie' of Undefined` (global `bodymovin` absent), listener `DOMContentLoaded` → `cannot read 'appendChild' of Undefined`. `NoBaseline` ×2, journal 0/1/3, repaint jusqu'à **7 937 ms**, PNG 27,9 Mo. |

---

## 🔬 Fiches Techniques Détaillées

### ✈️ Vol #01 — `https://www.twitch.tv/` (Twilight SPA — React sans SSR)

- **Stack technique** : app « Twilight » (React, chunks webpack `assets.twitch.tv/assets/*.js`, loader runtime inline avec table de chunks `sites.sunlight.*`/`features.*`/`webviews.*`), télémétrie `gql.twitch.tv` via `XMLHttpRequest`/`Blob`, Sentry debug-ids, sonde de capacités `window.__twilightSupportsJavaScriptSyntaxLevel` (le site **teste activement le niveau de syntaxe ES** du moteur avant de s'engager), détection arabe `isAr()` → `dir="rtl"`.
- **Rendu visuel** (`captures/2026-10-26_13-43-25_Equipe_7_Social_www.twitch.tv.png`, 1,85 Mo) : **shell SPA non hydratée** — le PNG montre l'enveloppe servie en SSR minimal (fond sombre, structure de page, textes du template) mais **aucun contenu de flux** (pas de carousel de chaînes, pas de navigation enrichie). À rapprocher des shells « mur » du vol 1 : le document peint est celui du HTML livré, React n'a jamais pris la main.

#### ✅ Ce qui est remarquable — et trompeur :

```text
script: eval fin tab=1 ms=… ok=true        (× 88, aucun ok=false)
[journal : 0 appliqués / 0 replis / 0 classées]
nav: sous-ressources ok → commit tab=1 ms=5416
apply: cascade ms=73 / boxtree ms=944 / layout ms=951 / repaint ms=979
0 exception, 0 WARN hors GoAway h2 (fermeture de connexions, non-bug)
```

#### 🔍 Analyse Racine :

1. **Journal 0/0/0 avec 88 scripts ok = hydratation morte-née silencieuse.** Le runtime webpack s'évalue intégralement (dont un eval de **4 983 ms** sur le runtime principal ~180 Ko minifié), aucune exception n'est levée, et pourtant **aucune `DomMutation` n'est produite** : React n'atteint jamais son premier commit. Hypothèses, par ordre de probabilité :
   - **la sonde `__twilightSupportsJavaScriptSyntaxLevel` échoue en douceur** : le site exécute une évaluation de syntaxe ES2020+ et décide de **ne pas booter** l'app si le niveau est insuffisant — comportement `try{}catch{}` donc **aucune exception ne remonte au journal** (le `catch` avale l'erreur). Crate : `explorer-jscore` (feature syntaxique manquante non détectée) ;
   - **attente sur une promesse/timer jamais résolu** : l'hydratation React est schedulée (microtask/`setTimeout(0)`/`requestIdleCallback`) et la pompe d'événements s'arrête avant le commit — `drain dyn links n=0` et `pump.js_run_sync` tournent en rond sur des micro-tâches sans mutation. Crate : `explorer-core` (pompe d'événements / budget) ;
   - **`fetch`/GraphQL bootstrap en attente** : la donnée du home feed vient de `gql.twitch.tv` ; si la promesse ne résout pas dans la fenêtre d'apply, React ne rend rien. La sonde `fetchlike()` retombe sur `XMLHttpRequest` — vérifier que `fetch` est présent et que les callbacks de résolution tournent.
2. **Comparaison vol 1** : youtube.com (48,7 Mo) et mastodon montraient des erreurs explicites ; twitch est le premier cas de **défaillance totalement silencieuse** — le pire cas diagnostique : tout est vert, rien n'est peint.

#### 🧪 Repro Minimal Test :

```html
<div id="root"></div>
<script>
  // Sonde de capacités : le boot ne doit pas dépendre d'une
  // syntaxe que le moteur ne couvre pas — et l'échec doit
  // être OBSERVABLE (console.warn au minimum).
  try {
    eval("(()=>{class C{ #x=1 } return new C()})()");
    document.getElementById("root").textContent = "hydrated";
  } catch (e) { /* silencieux côté site : log moteur attendu */ }
  // attendu : <div id="root">hydrated</div> OU trace exploitable
</script>
```

---

### ✈️ Vol #02 — `https://www.pinterest.fr/` → `https://fr.pinterest.com/` (PPR shell + regex `/u`)

- **Stack technique** : shell PPR (« `renderMode":"shellReady"` », `initialHandlerId":"www/index.js"`), bundles `s.pinimg.com/webapp/*-<hash>.js`, contexte JSON embarqué massif (~70 Ko : `experiments.triggerable` ≈ 150 flags, `routeTree` de ~200 routes lazy), consent gating `dweb_unauth_cookie_consent_modal`, UA remontée `"Explorer/0.1"` classée `browser_name:"Other"` + `is_perf_metrics_bot:true` (Pinterest nous détecte comme bot de perf et sert la version mesurée).
- **Rendu visuel** (`captures/2026-10-26_13-43-53_Equipe_7_Social_www.pinterest.fr.png`, 1,85 Mo) : **shell Pinterest servi mais non rempli** — structure de la home non connectée (header, cadre de login/signup) sans le mur d'épingles, qui attend l'hydratation du handler `www/index.js` et les requêtes GraphQL.

#### 💥 Mur Détecté (moteur JS `explorer-jscore` — lexer regex) :

```text
script async en échec tab=1 error=syntaxe js 1:35998 :
invalid regex /[\$\+<->\^`\|~\xA2-\xA6…\uD800[\uDD37-\uDD3F…]
 \uD802[\uDC77\uDC78\uDEC8]|\uD805\uDF3F|\uD807[\uDFD5-\uDFF1]…]/:
Parsing error at position 1884: Invalid codepoint for hex or unicode escape
   (×2 — rejoué à la seconde navigation)
script: eval fin tab=1 ms=0-2 ok=true   (× 20 — tout le reste passe)
[journal : 0 appliqués / 0 replis / 8 classées]
nav: sous-ressources ok → commit tab=1 ms=11809
apply: cascade ms=277 / boxtree ms=278 / layout ms=283 / repaint ms=315 / fin ms=528
```

#### 🔍 Analyse Racine :

1. **Regex `/u` à échappements astraux** : la regex fautive est la classe Unicode « symboles » générée par **regexpu** (le même motif que Babel/Webpack injecte pour transpiler `/\p{Symbol}/u`). En mode `u`, les séquences `\uD800[…]` dénotent des *surrogate pairs* valides — le lexer regex `explorer-jscore` rejette `\uD800` hors paire complète à la position 1884 du motif. **Crate fautive : `explorer-jscore` (lexer de classes de caractères regex, échappements `\uXXXX` astraux + plages de paires)**. C'est le **premier échec de *parse* regex** du panier — distinct des échecs `expected ')'` de fin de bundle (vols précédents).
2. **`8 classées` sans repli** : 8 mutations ont été mises de côté et le journal s'est terminé en `journal=false` (re-serialisation) **sans WARN ADR-0025** — chemin nominal du fallback, mais à auditer : 8 ops classées suggèrent des mutations produites *pendant* la fenêtre de commit (probablement l'injection du contexte JSON/`__PWS_DATA__`).
3. **La redirection `.fr → fr.` fonctionne** : deux `nav: fetch+parse…` consécutives, le dump final porte `[url : https://fr.pinterest.com/]` — le suivi de redirection 301/302 est sain côté `explorer-net`/`explorer-core`.
4. **Impact rendu** : le script en échec est `async` — le shell PPR s'affiche quand même, mais tout le code d'hydratation qui dépendait de ce chunk est mort → mur d'épingles vide. Dégradation gracieuse conforme (pas de crash), page incomplète.

#### 🧪 Repro Minimal Test :

```javascript
// Regexpu / \p{Symbol} transpilé — échappements astraux en classe :
const r = /[\uD800-\uDBFF][\uDC00-\uDFFF]|\uD834[\uDC00-\uDCF5]/u;
assert(r.test("𝄞") === true);
// Variante exacte du bundle :
const s = /[\$\+<->\^`\|~\xA2-\xA6]/u;   // classes + \xXX
```

---

### ✈️ Vol #03 — `https://www.quora.com/` → `https://fr.quora.com/` (motif `NoBaseline` inédit)

- **Stack technique** : React maison « ans » (`window.ansFrontendGlobals.settings` ~100 Ko : `tchannelData` websocket, `experiments`, `gates`, Cloudflare Turnstile `turnstilePreClearance`, `broadcastId`), bundles `qs*.fs.quoracdn.net`, redirection locale `quora.com → fr.quora.com`.
- **Rendu visuel** (`captures/2026-10-26_13-44-21_Equipe_7_Social_www.quora.com.png`, 1,86 Mo) : shell Quora partiel — chrome de page servi, feed de questions absent (contenu derrière JS + login-wall doux).

#### 💥 Mur Détecté (journal ADR-0025 — **motif inédit**) :

```text
WARN explorer_core::session::apply: journal DOM rejeté → repli re-parse (ADR-0025)
     tab=1 why=NoBaseline          ← ×2 (à chaque navigation)
[journal : 0 appliqués / 1 replis / 2 classées]
DEBUG explorer_core::session::net: livraison réseau js tab=1
     error=exception js : moteur figé — éval dépassée
nav: sous-ressources ok → commit tab=1 ms=4148 / ms=3177
apply: cascade ms=2006-2490 / layout ms=2424-2626 / repaint ms=5090-7937
```

#### 🔍 Analyse Racine — `NoBaseline` vs `UnmappedKey` vs `Fingerprint` :

D'après `crates/explorer-core/src/session/apply.rs` (`enum JournalReject`, lignes 716-733 et `key_node`/`apply_journal` lignes 961-1012) :

| Motif | Signification | Gravité diagnostique |
|:---|:---|:---|
| **`NoBaseline`** | `p.keymap.is_empty()` : **la table d'identité `miroir → NodeId` n'a jamais été semée** — le journal arrive alors qu'aucune baseline cléée n'existe (moteur JS sans ids, ou première mutation avant tout commit cléé). | ⚠️ **Structurel** : ce n'est pas une divergence, c'est l'absence de point de départ. |
| `UnmappedKey` | Une clé miroir précise est absente de la table (nœud jamais vu ou désaccord d'appariement). | Ciblé : un nœud fautif. |
| `Fingerprint` | Empreinte post-rejeu ≠ miroir : divergence structurelle **après** application. | Le plus grave : le rejeu a menti. |

Sur Quora et Discord, `NoBaseline` signifie que **le miroir JS n'a pas de keymap au moment de la première livraison de journal** : les scripts mutent le DOM (ops présentes — d'où les `classées`) mais la baseline cléée attendue par `apply_journal` n'a jamais été posée. Hypothèses : (a) la synchronisation miroir cléée (`DomSyncKeyed`) n'est pas armée sur ces pages — condition de seed liée à un événement qui ne se produit pas ; (b) la baseline est seedée *après* le premier journal et l'ordre d'application est inversé ; (c) le `moteur figé` (voir ci-dessous) a empêché la remontée du miroir. **Crate : `explorer-core` (chemin journal/baseline) — à confirmer contre `explorer-js`.**

Le second signal, **`livraison réseau js … moteur figé — éval dépassée`** (12:45:05, ~30 s après le dernier `drain dyn links`), indique qu'**un script livré par le réseau a été évalué au-delà du budget** et que la livraison a été abandonnée — le garde-fou budget a fonctionné (pas de crash), mais le script concerné est perdu, et ses mutations n'arriveront jamais.

32 evals `ok=true`, **0 erreur de syntaxe** : le parseur JS passe entièrement le stack Quora — l'échec est dans le *runtime/budget* et dans le *journal*, pas dans le langage.

#### 🧪 Repro Minimal Test :

```rust
// explorer-core : seed de baseline avant premier journal
// 1. commit DOM initial sans keymap JS
// 2. un script produit une DomMutation dès le premier apply
// attendu : NoBaseline tracé + re-parse (actuel) ;
// à décider : armer DomSyncKeyed dès le commit initial pour
// que le premier journal soit rejouable.
```

---

### ✈️ Vol #04 — `https://discord.com/` (Webflow marketing — le plus lourd du panier)

- **Stack technique** : site marketing **Webflow** (`cdn.prod.website-files.com/6257adef…/`, `discord-2022.schunk.*.js`, `webflow-scripts/homepageRefreshLatest.js`, `index-consolidated.js`), animations **Lottie** (`data-animation-type="lottie"`, `*.lottie`), iframe HubSpot injectée par script `nonce`, images `srcset` multi-densités — page immense (PNG **27,9 Mo**, le plus gros artefact du vol).
- **Rendu visuel** (`captures/2026-10-26_13-45-35_Equipe_7_Social_discord.com.png`, 27,9 Mo) : page marketing très longue **majoritairement peinte** (hero, sections produit, illustrations — le poids vient de la hauteur + illustrations rasterisées), mais les animations Lottie restent figées et les zones pilotées par `index-consolidated.js` sont dégradées.

#### 💥 Mur Détecté (moteur JS `explorer-jscore`) :

```text
script en échec tab=1 error=exception js :
  TypeError: cannot define property `QuickEffectIds`
    at <anonymous> (…/js/discord-2022.schunk.5ba7acebf7921e71.js:1:113763)
script en échec tab=1 error=exception js :
  TypeError: cannot read `open` of Undefined
    at <anonymous> (…/index-consolidated.js:1147:398)   [eval ms=20578 → ok=false]
script en échec tab=1 error=exception js :
  TypeError: cannot read `lottie` of Undefined
    at <anonymous> (…/webflow-scripts/homepageRefreshLatest.js:1:7662)
listener dom en échec tab=1 ty="DOMContentLoaded" error=exception js :
  TypeError: cannot read `appendChild` of Undefined     at c (1:13)
   (la même série ×4 est rejouée à la seconde navigation, ms=20936)
[journal : 0 appliqués / 1 replis / 3 classées]   why=NoBaseline ×2
apply: repaint ms=5090 / ms=7937   ← pic de repaint du panier
```

#### 🔍 Analyse Racine :

1. **`cannot define property 'QuickEffectIds'`** : le schunk Webflow fait un `Object.defineProperty` (ou équivalent) sur un objet **figé/non-extensible** — soit l'objet cible a été gelé trop tôt dans le moteur, soit `defineProperty` sur une propriété existante non-configurable échoue là où les moteurs tolèrent la redéfinition identique. **Crate : `explorer-jscore` (sémantique `[[DefineOwnProperty]]`, same-value redefinition sur descripteur non-configurable).**
2. **`cannot read 'open' of Undefined` dans `index-consolidated.js`** — eval de **20,5 s** avant exception : un module attend une API globale absente (probablement `document.open`/contexte iframe ou un namespace Webflow jamais initialisé car le schunk précédent a crashé — **effet cascade** : le crash #1 prive #2 de sa dépendance).
3. **`cannot read 'lottie' of Undefined`** : `homepageRefreshLatest.js` lit `… .lottie` — le global `bodymovin`/`lottie` n'est jamais chargé (la lib `lottie.min.js` n'a pas été évaluée ou s'est évaluée dans un autre realm). Conséquence directe : tous les `data-animation-type="lottie"` restent des conteneurs vides.
4. **Listener `DOMContentLoaded` → `appendChild` of Undefined** (`at c (1:13)`) : le snippet d'injection iframe (`contentDocument`/`contentWindow.document`) — `contentDocument` de l'iframe retourne `undefined` dans le moteur → `.appendChild` sur `Undefined`. **Crate : `explorer-jscore`/glue DOM (propriété `iframe.contentDocument` non implémentée).**
5. **`NoBaseline` ×2** : même motif inédit que Quora — ici **avec** des scripts qui mutent (3 classées), ce qui renforce l'hypothèse « baseline jamais seedée », pas « jamais de mutation ».
6. **Perf** : `repaint ms=7937` + eval 20,5 s → le vol le plus coûteux des 4 ; le re-parse de repli sur un DOM aussi large se paie plein tarif à chaque apply.

#### 🧪 Repro Minimal Test :

```javascript
// 1) Redéfinition identique sur propriété non-configurable
const o = Object.freeze({ a: 1 });
try { Object.defineProperty(o, "a", { value: 1 }); }  // ne doit pas throw si identique? — vérifier spec
catch (e) { /* cannot define property */ }

// 2) iframe.contentDocument
const f = document.createElement("iframe");
document.body.appendChild(f);
assert(f.contentDocument !== undefined);   // undefined aujourd'hui

// 3) Dépendance de chunk : crash d'un schunk → cascade d'Undefined en aval
```

---

## 🧭 Enseignements transverses (vol 2 vs vol 1)

1. **`NoBaseline` : nouveau motif ADR-0025, 2 sites/4 ce vol, absent du vol 1.** Contrairement à `UnmappedKey` (clé précise manquante) et `Fingerprint` (divergence post-rejeu), `NoBaseline` dit « **la table d'identité n'a jamais existé** » — le miroir JS livre des mutations dans le vide. Priorité d'audit : quand `DomSyncKeyed` est-il seedé par rapport au premier apply scripté ? Fichier : `crates/explorer-core/src/session/apply.rs:1011` + armement côté `explorer-js`. Ce motif est probablement la **cause racine commune** des replis observés ici, pas un symptôme.
2. **Twitch établit le pire cas diagnostique : l'échec parfaitement silencieux.** 88 evals verts, 0 exception, 0 mutation — il manque un signal d'observabilité (« scripts évalués sans jamais produire de mutation DOM » devrait être un WARN). À instrumenter côté `explorer-core` : compteur `evals_sans_mutation` → WARN au commit.
3. **Premier échec de *parse* regex** (`explorer-jscore`) : les échappements astraux `\uXXXX` en classe de caractères `/u` (sortie regexpu `\p{Symbol}`/`\\uD800[…]`) — récurrent attendu sur tout bundle Babel-transpilé (Pinterest aujourd'hui, la moitié du web demain). Fix prioritaire lexer regex.
4. **`moteur figé — éval dépassée`** (Quora) : le budget d'éval protège, mais le signal arrive via `livraison réseau js` en DEBUG — remonter en WARN comptabilisé : un script perdu = des mutations perdues = un `NoBaseline`/`classée` en cascade probable.
5. **Discord montre la cascade de dépendances** : un crash `defineProperty` sur schunk Webflow prive `index-consolidated.js` de ses globals → `open`/`lottie` Undefined en aval. Les messages « cannot read X of Undefined » en rafale sont souvent **un seul bug amont** — lire les erreurs dans l'ordre temporel.
6. **API manquantes confirmées** : `iframe.contentDocument` (discord), possiblement `fetch`/`requestIdleCallback`/`localStorage` partiels (twitch). Recenser les globals lus en `Undefined` sur tout le panier pour construire la matrice d'API DOM priorisée.
7. **Perf** : les evals >20 s (discord ×2) et repaints ~8 s (discord, quora) dépassent le confort d'un navigateur réel — à corréler avec le budget « moteur figé » pour vérifier que la limite s'applique aux evals synchrones aussi (l'eval de 20 578 ms s'est terminée `ok=false` *après* 20 s, pas interrompue à la limite).

---

## 🎯 Top anomalies — ordre d'action suggéré

| Rang | Anomalie | Crate | Sites touchés |
|:---:|:---|:---:|:---|
| 1 | `NoBaseline` — baseline keymap jamais seedée (repli ADR-0025 inédit) | `explorer-core` / `explorer-js` | quora ×2, discord ×2 |
| 2 | Hydratation silencieuse — 88 evals ok / 0 mutation / 0 erreur | `explorer-jscore` + `explorer-core` (observabilité) | twitch |
| 3 | Regex `/u` échappements astraux — `Invalid codepoint` pos. 1884 | `explorer-jscore` (lexer regex) | pinterest (et tout bundle regexpu) |
| 4 | `Object.defineProperty` sur objet figé — `cannot define property` | `explorer-jscore` | discord → cascade `open`/`lottie` |
| 5 | `iframe.contentDocument` === undefined | `explorer-jscore` / glue DOM | discord (listener DOMContentLoaded) |
| 6 | `moteur figé — éval dépassée` + evals 20 s non interrompues | `explorer-jscore` (budget) / `explorer-core` | quora, discord |

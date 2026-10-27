# Rapport de vol — Agent 23 — Sites référence/divertissement

- **Date** : 2026-10-26
- **Binaire** : `C:/obe-vol-integ/target/release/explorer-daemon.exe` (build intégration, ~20 fixes)
- **Viewport** : 1366×900 (`EXPLORER_VIEWPORT`), port daemon 9407
- **Protocole** : `docs/vol-agent.md` — navigate → grâce 8–14 s → console verbatim + frame + dom + layout, interactions scroll/clic/clavier.

## Synthèse

| Cible | Verdict | Console | Rendu |
|---|---|---|---|
| duckduckgo.com (home) | ✅ Rendue | vide | frame 3,3 Mo, layout complet |
| duckduckgo.com SERP (`q=openai`) | ⚠️ Partiel | 1 exception JS | shell serveur, liste résultats **vide** |
| imdb.com | ❌ Bloqué | 2 exceptions JS | page AWS WAF, body vide |
| weather.com | ✅ Rendue | vide | SSR Next.js complet, scroll OK |
| tiktok.com/explore | ❌ Bloqué | 2 exceptions JS | `#app` vide, frame blanche |
| letterboxd.com | ✅ Rendue | 4–5 exceptions modules | contenu SSR complet, clic → nav fiche film |

Score : **3 cibles pleinement utilisables / 6 navigations** (DDG home, weather, letterboxd + fiche film).

---

## 1. duckduckgo.com — home

- URL : `https://duckduckgo.com/` — titre « DuckDuckGo - Protection. Privacy. Peace of mind. », `doc_h=972`.
- Console : **vide** (« pas de journal »).
- Layout complet : textarea de recherche `[395 468.5 554x20]`, bouton submit `[949 462.5 32x32]`, chips « Search » / « Duck.ai », tagline « Protection. Privacy. Peace of mind. », footer avec liens.
- Capture : `captures/vol23_ddg_home.png` (3,36 Mo — contenu riche).
- **Fix confirmé** : le checkbox `#sidemenu-toggle` (input caché) apparaît en `2x19.2` px — normal (toggle masqué).

### Recherche « openai » → SERP

- Séquence : `click` 600,478 → `{"result":"focused"}` ; 6 × `key` (o,p,e,n,a,i) → `{"result":"ignored"}` (retour nominal des key events) ; `key Enter` → **navigation réelle vers la SERP** (`rq="openai"`, `display_query:"openai"`, titre « openai at DuckDuckGo »).
- La saisie clavier + soumission de formulaire **fonctionne** : la requête arrive au serveur.
- **Mais** : `#react-layout` reste vide — la liste des résultats (`results--main`/`mainline`) ne se monte pas côté JS. Console verbatim :

```
exception js : TypeError: cannot read `get` of Undefined
    at <anonymous> (https://duckduckgo.com/dist/wpmv.89e9d9d1b57ba39cf3ff.js:2:454272)
    at l (https://duckduckgo.com/dist/wpm.main.39e091dbe214d4fd1007.js:1:1194997)
    ... — module «https://duckduckgo.com/dist/wpm.main.39e091dbe214d4fd1007.js»
```

- `<noscript>` présent propose la redirection `/html?q=openai` (non suivie automatiquement — `meta refresh` dans noscript non honoré).
- Frame SERP : `captures/vol23_ddg_serp.png` — md5 identique à une page blanche (constat mesuré : le shell n'affiche rien de visible en dessous du fold initial ; DOM 303 lignes présent mais résultats absents).

## 2. imdb.com — bloqué AWS WAF

- URL : `https://www.imdb.com/` → réponse = **page challenge AWS WAF** (`awsWafCookieDomainList`, `gokuProps`, `#challenge-container`), `doc_h=8`, body vide.
- Console verbatim :

```
exception js : TypeError: messagefunction _0x42e97f() { [code] } is not a constructor or null
    at <anonymous> (…token.awswaf.com/…/challenge.js:1:519951)
    at <anonymous> (…challenge.js:1:526596)
    at <anonymous> (…challenge.js:1:307393)
    at <anonymous> (…challenge.js:1:9409)
exception js : ReferenceError: AwsWafIntegration is not defined
    — src «AwsWafIntegration.saveReferrer(); AwsWafIntegration.checkForceRefresh()…»
```

- Le challenge cryptographique WAF ne s'exécute pas (fonction obfusquée non constructible) → `AwsWafIntegration` jamais défini → pas de reload → page blanche définitive.
- Capture : `captures/vol23_imdb.png` (blanche).

## 3. weather.com — succès

- URL : `https://weather.com` → redirection locale `…/fr-FR/fr/paris/locality/2nd-arrondissement/today?Goto=Redirected` **suivie correctement**.
- Titre : « Prévisions et conditions météo pour 2e arrondissement, Paris - The Weather Channel », `doc_h=3337.56`.
- Console : **vide** — zéro exception sur une SPA Next.js complète.
- DOM 347 Ko : widgets `HourlyChart`, données `__next_f`, schema.org WebPage complet.
- Interactions : `scroll` ×2 → `scroll_y` 900 → 1800, reframe OK.
- Captures : `vol23_weather.png` (3,5 Mo), `vol23_weather_scroll.png` (1,95 Mo) — frames distinctes, défilement réel.

## 4. tiktok.com/explore — bloqué

- URL : `https://www.tiktok.com/explore` — titre OK « TikTok - Make Your Day », `doc_h=900` mais `doc_h` = hauteur viewport (contenu = un div plein écran vide).
- DOM 374 Ko présent (shell SSR + `__UNIVERSAL_DATA_FOR_REHYDRATION__` complet, `botType:"others"` dans les données serveur) mais layout réduit à `block<html>` > `block<body>` > `svg 0x0` + `div 1366x900` vide → **`#app` non monté**.
- Console verbatim :

```
exception js : TypeError: Cannot call a class as a function
    at n (inline:28:7:3450)
    at e (inline:21:1:16268)
    at <anonymous> (inline:15:1:16915) …
exception js : TypeError: cannot read `bind` of Undefined
    at <anonymous> (…secsdk_runtime_bundler_1104_ebc73ad6.js:5:86)
    at <anonymous> (…secsdk_runtime_bundler_1104_ebc73ad6.js:5:2)
    — src «/*! * @byted/secsdk-strategy v1.0.42 * (c) 2026 */ …»
```

- Le bundle principal lève « Cannot call a class as a function » (vraisemblablement `classCallCheck` Babel sur une builtin/ES class native mal gérée) → hydration React jamais lancée.
- Capture : `vol23_tiktok.png` — md5 = page blanche.

## 5. letterboxd.com — succès avec réserve JS

- URL : `https://letterboxd.com/` — titre « Letterboxd • Social film discovery. »
- Console verbatim (×4–5, répété) :

```
exception js : TypeError: cannot load module `./xPr8QFGw.js`: module non chargé : https://s.ltrbxd.com/static/js/es/chunks/xPr8QFGw.js — module «https://s.ltrbxd.com/static/js/es/main-CxejZr2R.js»
exception js : TypeError: cannot load module `./C6ZBybhW.js`: module non chargé : https://s.ltrbxd.com/static/js/es/chunks/C6ZBybhW.js — module «https://s.ltrbxd.com/static/js/es/homepage-CHiow-7a.js»
```

- **Constat** : `import()` dynamique (code-splitting) échoue → enhancements JS absents, mais le site est SSR riche : DOM 396 Ko, layout 100 Ko, nav, posters, « Create account », liens `/films/`.
- Interactions : scroll 800 OK ; `click` 300,400 → **`{"result":"navigated"}`** → fiche film (frame 5 Mo distincte) — navigation par lien fonctionnelle.
- Captures : `vol23_lbxd.png`, `vol23_lbxd_scroll.png`, `vol23_lbxd_film.png`.

---

## Constats actionnables

1. **[Sévérité haute] Dynamic `import()` non résolu** — letterboxd : `TypeError: cannot load module './xPr8QFGw.js': module non chargé`. Les imports relatifs de chunks depuis un module distant échouent. Couperait les enhancements de nombreux sites code-splittés (Vite/webpack). *(explorer-jscore / loader modules)*
2. **[Sévérité haute] `TypeError: Cannot call a class as a function` sur bundle webpack/Babel** — tiktok : tue l'hydration entière de la SPA (`#app` vide). Probable helper `_classCallCheck` appliqué à une classe native ou classe ES non transpilée. *(explorer-jscore, sémantique `class`/new.target)*
3. **[Sévérité haute] `cannot read 'get' of Undefined` dans le bootstrap SERP DDG** — duckduckgo : les résultats React ne se montent pas (conteneur `#react-layout` vide). Probable `Map`/`URLSearchParams`/`dataset` ou paramètre non initialisé lu via `.get`. *(explorer-jscore, builtins)*
4. **[Sévérité haute] `cannot read 'bind' of Undefined` (secsdk TikTok)** — accès `.bind` sur une méthode/API absente ; casse la chaîne de sécurité Bytedance. *(explorer-jscore, surface API manquante)*
5. **[Sévérité moyenne] AWS WAF challenge irrésolvable** — imdb : `messagefunction … is not a constructor or null` puis `AwsWafIntegration is not defined`. Le challenge obfusqué échoue → blocage complet du site. Attendu pour un moteur jeune, mais tout site Amazon/WAF est hors de portée. *(explorer-jscore)*
6. **[Sévérité basse] `<meta http-equiv="refresh">` dans `<noscript>` non honoré** — DDG SERP propose `/html?q=…` en fallback noscript ; jamais suivi. Un fallback graceful améliorerait les SERP quand le JS casse. *(explorer-html/core — politique noscript)*
7. **[Info] `doc_h` rapporté au navigate peut être transitoire** — letterboxd a rapporté `doc_h=0` à la réponse `navigate` alors que le layout final fait ~5000 px de haut ; race entre réponse navigate et layout final (déjà documenté comme 404/état transitoire).

## Fixes intégration confirmés (non reproduits)

- **Saisie clavier → champ texte → soumission formulaire → navigation** : fonctionne de bout en bout (DDG home → SERP `q=openai`).
- **Scroll** : événements `scrolled` avec `scroll_y` exact (900/1800) sur weather.com et letterboxd.
- **Clic → navigation par `<a>`** : letterboxd home → fiche film (`{"result":"navigated"}`), frame régénérée.
- **Console** : captures propres, pas de troncature observée (fix « console 512 »).
- **Aucun crash daemon** sur 5 sites hostiles (WAF, obfusqué, SPA lourde) — tolérance aux erreurs respectée.

## Artéfacts

- Captures : `docs/Rapport vol/captures/vol23_{ddg_home,ddg_serp,imdb,weather,weather_scroll,tiktok,lbxd,lbxd_scroll,lbxd_film}.png`
- Logs : `docs/Rapport vol/logs/vol23_*_{console,dom,layout}.*`
- Daemon : PID 17144, arrêté proprement ; onglets 1–5 fermés. Aucun code modifié.

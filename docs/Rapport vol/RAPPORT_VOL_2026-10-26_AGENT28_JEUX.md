# Rapport de vol — Agent 28 — Jeux web (canvas, animations, input continu)

- **Date** : 2026-10-26
- **Binaire** : `C:/obe-vol-integ/target/release/explorer-daemon.exe` (intégration V2)
- **Viewport** : 1366×900, daemon sur port 9412
- **Méthode** : protocole `docs/vol-agent.md` — navigate → grâce 8–12 s → console verbatim → frame PNG → dom/layout → input (click/key/scroll) → frame/console.

## Synthèse

| Cible | Page chargée | Jeu démarré | Erreurs console | Sévérité |
|---|---|---|---|---|
| lichess.org | ✅ (doc_h 5761) | partiel — SPA montée, plateau non vérifiable | aucune | moyen |
| chess.com | ✅ (doc_h 4596) | ❌ boot JS cassé | chunk module + TypeError en boucle | élevé |
| play2048.co | ⚠️ (doc_h 51) | ❌ `#app` restée vide | aucune (échec silencieux) | élevé |
| wordle (NYT) | ✅ | ❌ jeu dans iframe 0×0 non rendu | Fides overlay ×3 | élevé |
| agar.io (fallback) | ❌ HTTP 403 | — | `HTTP 403` | n/a (blocage serveur) |

Aucun jeu n'est réellement jouable : les deux jeux « DOM pur » attendus
légers sont en fait des SPA modernes (React/PixiJS pour play2048,
custom elements + iframe pour Wordle) dont le boot JS ne complète pas.

## Cible 1 — lichess.org

- URL : `https://lichess.org/` — titre « lichess.org • Free Online Chess », doc_h 5761,27.
- Console : **« pas de journal »** — zéro erreur JS. Fix confirmé si un ancien
  constat signalait des erreurs de boot lichess.
- Frame : `captures/vol28_lichess.png` (1,88 Mo, 200).
- DOM 55,7 Ko / layout 43,3 Ko — la page marketing est montée
  (layout conséquent). Le plateau d'échecs réel nécessite `/` → partie
  (websocket + canvas) non testée en profondeur.
- Input : click (683,450) → `{"result":"ignored"}` ; scroll dy 800 →
  `{"result":"scrolled","scroll_y":800.0}` — scroll fonctionne.
- **Classification** : rendu pipeline OK, interactivité jeu non mesurable.

## Cible 2 — chess.com

- URL : `https://www.chess.com/` — titre correct, doc_h 4596.
- Console verbatim (extraits ; le `TypeError` se répète ≥ 60 fois) :

```
unhandledrejection: TypeError: cannot load module `./shared.0f3f0d72.B4owK9-jLp.chunk.js`: module non chargé : https://www.chess.com/bundles/app/vite/shared.0f3f0d72.B4owK9-jLp.chunk.js
unhandledrejection: ReferenceError: e is not defined
unhandledrejection: TypeError: `n` is not a function (on Object)
    at <anonymous> (https://www.chess.com/bundles/app/vite/global.DqQfOMUhl3.js:2:521)   ×~60
```

- Frame : `captures/vol28_chesscom.png` (5,9 Mo — page dense peinte).
- DOM 162 Ko ; layout seulement 17,6 Ko (beaucoup de contenu hors
  layout / display:none probable).
- Click → `ignored`. La boucle `n is not a function` (≥ 60 occurrences,
  même stack) suggère un `setInterval`/retry qui retombe sur la même
  fonction manquante — probablement un résidu d'API non implémentée
  ou un chunk dont l'échec de chargement casse une factory partagée.
- **Constat actionnable 1** : l'import dynamique de chunk Vite
  (`import()` de `./shared.*.chunk.js`) échoue alors que le module
  principal a chargé — vérifier la résolution d'URL relative dans les
  dynamic imports de `explorer-jscore`.
- **Constat actionnable 2** : `ReferenceError: e is not defined` —
  binding manquant dans un module minifié (possible variable globale
  attendue non définie, ou catch binding).
- **Constat actionnable 3** : boucle infinie de `TypeError: n is not
  a function` dans `global.*.js` — une API appelée en timer n'existe
  pas ; le site ne s'en remet pas. Envisager un budget/throttle pour
  les unhandledrejection répétées (remplit le ring console de 512).

## Cible 3 — play2048.co

- URL : `https://play2048.co/` — titre correct mais **doc_h = 51 px**.
- Console : **« pas de journal »** — aucune erreur malgré l'échec.
- DOM : `#app` présent mais **vide** ; le body ne contient que le
  `<noscript>` + le lien « Privacy Policy » (layout : 3 boîtes,
  hauteur 51 px). La page est un Vite SPA **React + PixiJS**
  (`/assets/index-*.js` type=module, `pixi.js-*.js` en modulepreload) —
  le « JS léger » supposé est faux : c'est du canvas WebGL.
- Frame : `captures/vol28_2048.png` — fond seul.
- Key `ArrowUp` → `ignored` (pas de handler monté).
- **Constat actionnable 4** : le module `index-*.js` ne produit ni
  erreur ni rendu — soit il n'est jamais exécuté (module loader),
  soit il échoue silencieusement avant le mount React. Ajouter un
  trace de fin d'exécution de module côté jscore pour distinguer
  « pas exécuté » vs « exécuté puis exception avalée ».

## Cible 4 — NYT Wordle

- URL : `https://www.nytimes.com/games/wordle/index.html` — titre
  « Wordle — The New York Times », doc_h = 900 (= viewport, page à
  hauteur fixe).
- Console verbatim :

```
unhandledrejection: Error: There was a problem rendering the Fides overlay.
    at <anonymous> (https://static01.nyt.com/vi-assets/static-assets/fides-ffeeec6bc4efda2c53511fd1e2e8ff9a.js:1:99586)
    at <anonymous> (https://static01.nyt.com/vi-assets/static-assets/fides-ffeeec6bc4efda2c53511fd1e2e8ff9a.js:1:99499)   ×3
```

- DOM 201 Ko : le document contient bien les custom elements Wordle
  (16 correspondances `game-board`/`tile`), mais le **layout fait 190
  octets** : `html`/`body` à 900 px, tous les div à hauteur 0, et un
  `inline<iframe> [0 0 0x0]` — le jeu vit dans une iframe 0×0 non
  affichée (et/ou en custom elements non enregistrés).
- Interactions : click (683,600) → `ignored` ; key `a` →
  `{"result":"unfocused"}` — pas de cible de focus, le clavier ne
  peut pas atteindre le jeu.
- **Constat actionnable 5** : le contenu de l'iframe Wordle est à
  0×0 et ne peint pas — si les sous-contextes iframe sont censés
  peindre (vol-agent §4), vérifier pourquoi celui-ci reste à 0.
- **Constat actionnable 6** : `key` retourne `unfocused` quand rien
  n'a le focus — pour les jeux clavier (wordle/2048), le document
  devrait avoir un focus implicite sur `body` ; à investiguer côté
  core/input.
- **Constat actionnable 7** : l'erreur « Fides overlay » (gestion
  consentement) se répète 3× — wrapper DOM (probablement attachShadow
  ou createElement sur custom element) qui échoue ; mêmes classes de
  cause que les custom elements non enregistrés.

## Cible 5 — agar.io (fallback)

- Réponse : `{"error":{"handled":true,"message":"HTTP 403"}}`, console
  `HTTP 403`. Blocage côté serveur (anti-bot) — dégradation gérée
  proprement, pas un défaut moteur. Capture posée à titre de trace.

## Constats actionables (index)

1. **Dynamic import de chunks relatifs cassé** (chess.com) — `import()`
   d'un chunk sibling échoue ; bloque le boot de toute app Vite code-splittée.
2. **`ReferenceError: e is not defined`** (chess.com) — binding global/builtin manquant.
3. **API appelée en timer inexistante** → boucle de `TypeError` ×60+ (chess.com) ; envisager un plafond de logs identiques.
4. **SPA qui échoue silencieusement** (play2048) — module app jamais monté, zéro erreur console ; manque de traçabilité de l'exécution des modules.
5. **iframe de jeu à 0×0 non peinte** (wordle) — sous-contexte présent dans le layout mais sans surface.
6. **Pas de focus clavier par défaut** (`unfocused` sur `key`) — bloque tout jeu clavier.
7. **Custom elements / overlay Fides** échouent au rendu (wordle) — API DOM manquante probable (`attachShadow`/`customElements.define`).

## Verdict global

Les jeux web 2026 sont **hors de portée actuelle** : pas à cause du
rendu (les pages statiques peignent, scroll OK) mais du **boot JS** —
dynamic imports, APIs DOM manquantes, custom elements, focus clavier.
La piste JS-Cxx devrait prioriser : dynamic `import()` relatif (1),
`customElements`/shadow DOM (5,7), focus implicite document (6).

## Artefacts

- Captures : `captures/vol28_{lichess,lichess_click,chesscom,2048,wordle,wordle_click,wordle_key,agario}.png`
- Logs : `logs/vol28_{lichess,chesscom,2048,wordle}_{dom,layout}.json`
- Daemon log : `_vol28_daemon.log`

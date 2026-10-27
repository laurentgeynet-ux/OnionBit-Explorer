<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 22 — RE-VOL DE CONFIRMATION (2026-10-26)

Binaire : `C:/obe-vol-integ/target/release/explorer-daemon.exe` (branche
`nav/vol-integration` = HEAD + ~20 fixes mergés : regex Unicode astraux /
`\xNN` / `[-]`, `<#comment>`, `XHR.prototype`, `XPathEvaluator`,
`Intl.supportedValuesOf`, `Worker.addEventListener`, `super()` ctor,
`in_gen` générateurs, primitives boxées, table anonyme, inset shorthand,
hit-test z-order, popover UA, noscript `!important`, échecs
sous-ressources→console, lazy images, scroll projeté, frame/viewport,
console 512, onglet parasite `.js`, meta refresh noscript).

Daemon : port 9406, viewport 1366×900, release.

## Verdict global

**Toutes les erreurs verbatim d'avant ont disparu.** Aucune régression
d'exception constatée sur 5 cibles ; l'erreur fatale a reculé partout
d'un cran dans la chaîne de boot. Un site passe de « 100 % blanche »
à « application montée » (Excalidraw). Aucun artefact `<#comment>`
dans les dumps DOM (0 occurrence sur les 5 cibles).

## Tableau avant / après

| Cible | Erreur verbatim AVANT | État AVANT | Erreur verbatim APRÈS | État APRÈS |
|---|---|---|---|---|
| bsky.app | `invalid regex` (Unicode astral) | `#root` vide | `TypeError: atob: invalid byte sequence` (bundle expo index, ligne 2047) | regex corrigée, boot JS progresse jusqu'à `atob` ; `#root` toujours vide, frame blanche, doc_h=900 (body 0 px) |
| pinterest.com | `invalid regex`, doc_h=38 | squelette mort | **aucune erreur console** (« pas de journal ») | SSR HTML complet (1,5 Mo, `__PWS_ROOT__` présent) mais React ne monte **silencieusement** — doc_h=0, frame blanche, scroll inerte (`scroll_y` reste 0) |
| gitlab.com/explore | `invalid regex` (emoji) | page quasi blanche | `TypeError: cannot read appendChild of Null` (super_sidebar) + `TypeError: constructor is not a function: function () { [code] }` (framework) + `SyntaxError: unexpected token in JSON` (pages.explore GraphQL) | squelette réel peint : topbar, sidebar 240 px, `<h1>Explore projects` en layout ; JS webpack s'exécute profondément avant de mourir |
| excalidraw.com | `'in' requires an object` | 100 % blanche | **aucune erreur console** | **app montée** : écran d'accueil Excalidraw rendu (logo SVG, « Your drawings are saved… », menu Open / Help / Live collaboration / Sign up, contrôles zoom ±100 %) — chevauchements de texte mineurs |
| youtube.com | regex kevlar + ShadyDOM morts | squelette gris figé | `TypeError: cannot read throwIfAborted of Undefined` (kevlar_base) | squelette gris visuellement similaire MAIS structure saine : `ytd-app`/`ytd-masthead` en layout, **doc_h=12234** (vs viewport figé avant), **scroll fonctionnel** (`scroll_y` 800→1600), masthead + icônes top-right peints ; vignettes encore placeholder (lazy images ?) |

## Nouveaux blocages (première fatale restante, par cible)

1. **bsky.app** — `atob: invalid byte sequence` : le décodeur base64
   rejette un input légal (probablement whitespace ou alphabet URL-safe
   `-_` / padding absent, courant dans les bundles Sentry/expo).
2. **pinterest.com** — mort **silencieuse** : aucune exception loguée,
   `__PWS_ROOT__` reste vide. Suspects : script `async` jamais
   exécuté, promise non résolue sans rejet tracé, ou API manquante
   avalée. À diagnostiquer par instrumentation (pas de verbatim).
3. **gitlab.com** — `constructor is not a function: function () { [code] }` :
   `new` sur une fonction non-constructible (arrow/bundled) — lié au
   fix `super()` ctor ? Plus `JSON.parse` qui jette `unexpected token`
   sur le payload GraphQL embarqué.
4. **youtube.com** — `throwIfAborted of Undefined` : appel
   `signal.throwIfAborted()` (ou équivalent) où `AbortSignal` vaut
   `undefined` — binding `AbortSignal`/`AbortController` incomplet.
5. **youtube.com** — iframe `accounts.google.com/ServiceLogin` en
   **HTTP 401** : attendu (signin passif bloqué), non bloquant,
   mais le nouveau report « sous-ressource en échec → console »
   fonctionne (le message apparaît bien — validation du fix).

## Constats actionnables

1. `atob` : accepter l'alphabet base64url (`-`,`_`), ignorer les
   espaces blancs dans l'input et tolérer l'absence de padding —
   débloque le bundle bsky (Sentry debug ids) [NAV/jscore].
2. `AbortSignal.throwIfAborted` + `signal` par défaut : implémenter
   le binding complet (signal undefined → opération non-abortable)
   débloque kevlar YouTube [NAV/jscore].
3. `new` sur fonction non-constructible : le message
   `constructor is not a function: function () { [code] }` suggère
   qu'une arrow function ou une méthode shorthand est prise pour un
   constructeur après transpile — vérifier `IsConstructor` sur les
   fonctions bundlées (rspack/webpack runtime) [NAV/jscore].
4. Pinterest : ajouter un tracing des scripts `async` non exécutés et
   des rejections de promises non catchées — la console vide masque
   la cause de mort ; sans verbatim, impossible de prioriser
   [NAV/core].
5. `JSON.parse` sur les blobs `<script type="application/json">` /
   payloads GraphQL GitLab : `unexpected token` à investiguer —
   possible troncature ou séquence d'échappement (u2028/u2029 ?)
   mal gérée [NAV/jscore].
6. YouTube : malgré la fatale kevlar, le scroll projeté et
   doc_h=12234 sont validés ; les vignettes restent grises —
   vérifier que les `<img>` lazy (classe `yt-img-shadow`) déclenchent
   bien le fetch au scroll [NAV/core].
7. GitLab : `appendChild of Null` en super_sidebar = cible DOM
   `getElementById` retournant null — vérifier l'ordre d'exécution
   des scripts différés vs insertion du conteneur [NAV/core].
8. Rendu texte Excalidraw : chevauchements de runs inline (logo +
   paragraphes superposés) — probablement `letter-spacing`/flex gap
   ou police excalifont non chargée ; non bloquant [PIPELINE/layout].

## Fichiers produits

- Captures : `captures/vol22_{bsky,pinterest,gitlab,gitlab_after,excalidraw,youtube,youtube_scrolled}.png`
- Logs : `logs/vol22_{bsky,pinterest,gitlab,excalidraw,youtube}_{console.json,dom.txt,layout.txt}`
- Log daemon : `_vol22_daemon.log` (racine dépôt)

## Arrêt propre

Onglets fermés, daemon PID tué. Aucun code modifié.

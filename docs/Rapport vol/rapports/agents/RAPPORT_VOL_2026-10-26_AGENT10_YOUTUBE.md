<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# RAPPORT VOL — AGENT 10 — YOUTUBE

Date : 2026-10-26 — Daemon : `C:/obe-vol-head/target/release/explorer-daemon.exe`
(release, HEAD propre), port 9394, PID 11624, viewport 1366×900.

## Résumé exécutif

YouTube boote **jusqu'au squelette SSR puis s'arrête net** : deux erreurs
JS indépendantes et chacune fatale tuent le boot Polymer —
1. un `TypeError` d'exécution dans le polyfill `webcomponents-all-noPatch.js`,
2. une **erreur de syntaxe** (regex) dans le bundle `kevlar_base` qui
   empêche l'évaluation complète du bundle applicatif.

Résultat mesuré : `customElements` jamais installé, aucun élément
`ytd-*` upgradé (seuls `<ytd-app>` et `<ytd-masthead>` existent, inertes),
rendu = page fantôme grise (« ghost/shimmer skeleton », 192 occurrences
de classes skeleton dans le DOM home). Aucun contenu réel n'apparaît
sur les 4 cibles. Bonus watch : `fetch` lu sur `Undefined` dans
`player_ias/base.js`.

## Cible 1 — https://www.youtube.com (home)

- URL finale : `https://www.youtube.com/` — title `YouTube`, `doc_h` 12234.
- Console : **2 erreurs** (verbatim ci-dessous).
- Métriques : `dom_nodes` 473, `layout_boxes` 234, `js_heap` 4,4 Mo.
- Frame : `captures/vol10_youtube_home.png` (doc complet, 48 Mo) +
  `vol10_youtube_home_view.png` (viewport) : squelette gris — masthead
  fantôme + rangées de placeholders, **zéro texte, zéro miniature**.
- DOM : `<ytd-app>` et `<ytd-masthead>` présents mais **non upgradés** ;
  aucun `ytd-video-renderer`, aucun `tp-yt-*` ; 18 occurrences `ytcfg`
  (données embarquées intactes), 1 `customElements` (script tag).
- Interactions : scroll ×2 OK (`scroll_y` 800→1600), mousemove champ
  recherche → hit-test `editable:false` (le champ est un div fantôme),
  clic sur une « miniature » → `{"result":"ignored"}` (rien de cliquable).

## Cible 2 — https://m.youtube.com (mobile)

- **Redirection serveur** vers `https://www.youtube.com/?app=desktop`
  (le moteur reçoit la version desktop — UA non reconnu comme mobile).
- Strictement identique à la cible 1 : mêmes 2 erreurs console, mêmes
  `dom_nodes` 473 / `layout_boxes` 234, même squelette.
- Aucune version allégée n'a pu être testée : la redirection rend la
  comparaison caduque.

## Cible 3 — https://www.youtube.com/results?search_query=rust+programming

- URL finale inchangée, `title` null, `doc_h` **0,0** au premier relevé.
- Console : **2 erreurs** (les mêmes signatures).
- Frame (`vol10_results_view.png`) : **page quasi blanche** — masthead
  fantôme seul (burger + 3 icônes), aucune ligne de résultat.
- Layout : `<html>/<body>` à hauteur 0 ; `ytd-app` 1366×900 avec
  masthead 56px puis **rien d'autre**.
- DOM (1,8 Mo) : les données de recherche **sont là** (37 occurrences
  `search_query`, JSON embarqué) mais jamais rendues — hydration morte.
- Constat pire que la home : sur `results` le SSR ne livre même pas de
  grille fantôme de résultats — tout dépend du JS tué au boot.

## Cible 4 — https://www.youtube.com/watch?v=dQw4w9WgXcQ (watch)

- Title correct : `Rick Astley - Never Gonna Give You Up … - YouTube`,
  `doc_h` 405 (page tronquée : squelette partiel).
- Console : **3 erreurs** — les 2 du boot + :
  `exception js : TypeError: cannot read 'fetch' of Undefined
   at <anonymous> (…/player_ias.vflset/fr_FR/base.js:2950:5)`
  (le player accède `fetch` sur un objet Undefined — probablement un
  namespace `yt.*`/global manquant à cause du boot mort).
- Frame (`vol10_watch_view.png`) : squelette watch (barres grises titre/
  channel/actions), **pas de `<video>`** (0 occurrence dans le DOM).
- Métriques : `dom_nodes` 403, `layout_boxes` 46, `js_heap` **14,8 Mo**
  (le plus gros heap des 4 — le player pousse du JS avant de mourir).
- Scroll → `scroll_y` 0,0 (page 405px < viewport : rien à scroller).

## Erreurs console verbatim (top)

1. `exception js : TypeError: '__shady_native_addEventListener' is not a
   function (on Object)` — `webcomponents-all-noPatch.js:135:1` (fn `ke`)
   puis rethrow top-level `:1:2`. **Première erreur fatale du boot** :
   le polyfill ShadyDOM appelle une fonction jamais installée
   (`__shady_native_*` = hooks natifs que le polyfill attend de la
   plateforme ou d'un patch préalable) → `customElements`/Shadow DOM
   jamais branchés → aucun `ytd-*` ne s'upgrade.
2. `syntaxe js 12071:1763 : invalid regex /[!-#%-\*,-/:;…\uFF65]|\uD800[
   \uDD00-\uDD02…]/ : Parsing error at position 1674: Invalid codepoint
   for hex or unicode escape` — bundle `kevlar_base.en_US…es5.O`
   (`m=kevlar_base_module,kevlar_main_module,kevlar_base_sync_mod_chunk`).
   **Erreur de parse = bundle entier jamais évalué** → toute l'app
   Kevlar morte même si le point 1 était corrigé. Indépendante du point 1.
3. (watch seulement) `TypeError: cannot read 'fetch' of Undefined` —
   `player_ias/base.js:2950:5`. Conséquence du boot mort (global manquant)
   ou `fetch` non exposé sur l'objet attendu.

## Constats actionnables

1. **[JS — bloquant, sévérité critique]** Parser de regex :
   `Invalid codepoint for hex or unicode escape` à la position 1674 d'un
   regex de classes Unicode (surrogate pairs `\uD800[\uDD00-…]` +
   échappements `\xA1` dans une classe massive). Le bundle `kevlar_base`
   entier est rejeté au parse → application jamais évaluée. Repro isolé :
   extraire le literal regex ligne 12071 et le soumettre au lexer.
   Priorité : **une seule erreur de parse tue 100 % de YouTube**.
2. **[JS/DOM — bloquant, sévérité critique]** `__shady_native_addEventListener`
   absent : le polyfill webcomponents (webcomponentsjs) attend des hooks
   natifs (`__shady_native_*`) qui doivent être installés par l'environnement
   ou un fichier patch amont. Sans eux, `customElements` ne fonctionne pas
   → SPA Polymer condamnée. Investiguer quel global le polyfill
   `webcomponents-all-noPatch` suppose (`ke` à :135:1).
3. **[API JS — majeur]** `fetch` : `cannot read 'fetch' of Undefined`
   (player base.js:2950) — vérifier l'exposition de `fetch` sur
   `window`/globalThis et les namespaces `yt.*` attendus.
4. **[NET/UA — moyen]** `m.youtube.com` redirige vers la version desktop :
   le serveur YouTube ne classe pas le moteur en mobile. Tester avec un
   UA mobile explicite si une surface allégée est souhaitée pour la CI.
5. **[Rendu — informationnel]** Le squelette SSR se peint correctement
   (masthead, ghost rows, doc 12234px, scroll fonctionnel) : le pipeline
   HTML/CSS n'est **pas** en cause — tout l'échec est JS. Bon cas de
   non-régression visuelle.

## Classification

- État : **boot SPA mort au niveau polyfill** — pages fantômes.
- Sévérité : bloquante (YouTube inutilisable), deux causes racines
  indépendantes (regex lexer + hooks ShadyDOM).
- Artifacts : `captures/vol10_*.png`, `logs/vol10_*_{console,dom,layout}`.

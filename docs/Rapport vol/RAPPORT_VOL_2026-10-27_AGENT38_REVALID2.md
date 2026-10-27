<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 38 — Re-validation morts silencieuses §B-4/§B-5

- **Date** : 2026-10-27
- **Binaire** : `C:/obe-vol-target/release/explorer-daemon.exe` — tag
  **`[V4-main]`** (inclut WIP équipe fix — J3 `data:` modules en cours,
  build 08:54)
- **Viewport** : 1366×900 (`EXPLORER_VIEWPORT`) — port 9423 — daemon
  PID 21952 — journal `_vol38_daemon.log` → `logs/daemons/`
- **Protocole** : `docs/vol-agent.md` §8 — chaque constat porte `crate`
  + ID §B ; statuts canoniques §G.
- **Mission** : re-valider les morts silencieuses §B-4 et le bras
  MapLoader §B-5/§B-6 sur quatre cibles : reddit r/rust (§B-6 + §B-15),
  mastodon.social (§B-5 module ESM), pinterest.com (§B-4), play2048.co
  (§B-4).

## Verdicts en une table

| Cible | ID §B | Verdict `[V4-main]` |
|---|---|---|
| reddit.com/r/rust | §B-6 `import()` `data:` refusé | **TOUJOURS PRÉSENT** — refus identique, **mais désormais tracé** (WARN daemon, fin du silence) |
| reddit.com/r/rust | §B-15 partials faceplate/shreddit | **DÉGRADÉ EN MIEUX** — partials peuplés et peints (13 `shreddit-post` avec contenu réel), upgrade custom elements tenté mais échoue (`invalid custom element name "undefined"` — tracé console) |
| reddit.com/r/rust | §B-4 mort silencieuse | **non reproduit ici** — la page rend, 1 entrée console + WARNs daemon |
| mastodon.social | §B-5 `module non chargé` | **RÉSOLU** — signature d'erreur disparue ; graphe de modules instancié (heap JS 31 Mio) |
| mastodon.social | §B-4 mort silencieuse | **TOUJOURS PRÉSENT** — page blanche, « pas de journal », zéro trace sur la mort de l'app |
| pinterest.com | §B-4 mort silencieuse | **TOUJOURS PRÉSENT** — page blanche, **zéro trace console** (question posée par la mission : non, toujours aucune) |
| play2048.co | §B-4 mort silencieuse | **TOUJOURS PRÉSENT** — page blanche, « pas de journal », heap JS 25 Mio |

Constats secondaires : le « silence » recule en traçabilité (tous les
refus `data:` de reddit sont désormais des WARN nommés, le rejet
`unhandledrejection` de reddit est en ring console) mais **pas en
issue** — les quatre morts restent fonctionnellement identiques :
l'échec survient à une étape post-chargement (exécution des modules
defer/module, montage applicatif) que rien n'instrumente encore.
Note perf : pompe `sync_profile` ~4 Hz/onglet toujours permanente
(constat vol 36 confirmé).

---

## 1. Reddit — https://www.reddit.com/r/rust — §B-6 **TOUJOURS PRÉSENT (tracé)** · §B-15 **DÉGRADÉ EN MIEUX**

- **Titre** : « Reddit - The heart of the internet » — doc_h 1 225 px,
  DOM 767 Ko / 3 280 nœuds, 34 évals `ok=true`, scroll fonctionnel
  (`scroll_y` 325 = max, doc_h 1 225 < 1 700 de contenu scrollé : la
  page est plus courte que le feed réel mais le scroll répond).
- **Fait marquant** : la vraie page r/rust **rend** — le challenge JS
  serveur est franchi (URL finale porte `?solution=804406bd…
  &js_challenge=1&jsc_token=…` ; `can_back=true` = une redirection
  interne résolue). Frame `captures/vol38_reddit_rust.png` : entête
  r/rust avec avatar, onglets tri, **posts peints avec titres et
  contenus** (« systemd adds support for programs written in Rust »
  avec lien GitHub, « PSA: If your program does HTTP requests… »,
  « Wild linker version 0.11.0 released with Wasm support » avec
  corps de texte complet), sidebar droite « Community Bookmarks » +
  « r/rust Rules » + image. Au scroll (`vol38_reddit_rust_scroll.png`)
  corps de post entier avec liens et flairs « news »/« project ».
  Il y a ~3 ans de chemin parcouru vs le « 28 partials vides » du
  login reddit.
- **§B-6 — `data:` toujours refusé, désormais tracé** : le journal
  daemon porte les deux bras historiques, en WARN nommés (avant :
  silence total) :
  - `WARN explorer_core::session::scripts: script refusé en plan —
    résolution/mixed-content/CSP src=data:text/javascript,…` ×3 —
    le polyfills-loader Shreddit et le bootstrap `data:` restent
    rejetés à la planification ;
  - `WARN explorer_core::session::net: dyn import : résolution/CSP
    refusé — rejet tab=1 url=data:text/javascript,…` — le
    `import()` dynamique sur `data:` (boot Shreddit) est toujours
    rejeté.
  Verdict `explorer-core` → **§B-6 TOUJOURS PRÉSENT** : le
  comportement (refus) est inchangé sur le WIP J3 présent dans ce
  binaire ; le gain est l'**observabilité** — chaque rejet est une
  ligne WARN avec URL, plus une mort silencieuse.
- **§B-15 — partials faceplate/shreddit** : 13 `shreddit-post` avec
  attributs `post-title` réels et contenu peint (titres, liens,
  vignette, flair), 16 `shreddit-app`, 3 `shreddit-feed`,
  faceplate-* nombreux (`faceplate-img`, `faceplate-dropdown-menu`,
  `faceplate-hovercard`…). L'upgrade custom elements est **tenté**
  et échoue proprement, **tracé en console** :
  `unhandledrejection: SyntaxError: invalid custom element name
  "undefined" (×3)` — un module lit un nom de tag `undefined` au
  `customElements.define`. Le contenu SSR remplit les partials
  (peint), l'hydratation propre reste en échec partiel →
  `explorer-jscore` → **§B-15 DÉGRADÉ EN MIEUX**.
- **§B-4 sur cette cible** : non reproduit — erreur en console +
  WARNs daemon. Ce n'est plus une mort silencieuse.
- Artefacts résiduels mineurs visibles : superpositions de texte dans
  la barre de tri (« Community highlights » chevauchant
  « Sort by »), barre latérale login qui recouvre des liens
  (`explorer-layout` → parenté §B-18).

## 2. Mastodon — https://mastodon.social — §B-5 **RÉSOLU** · §B-4 **TOUJOURS PRÉSENT**

- **Titre** : « Mastodon.social » — doc_h 0,0 px, DOM 59 Ko /
  278 nœuds, layout 8 boîtes toutes 0-hauteur, `journal_fallbacks=1`
  (`journal DOM rejeté → repli re-parse (ADR-0025) why=NoBaseline`).
- **Frame** `captures/vol38_mastodon.png` (+ `_scroll.png` identique)
  : **blanc total**. Le `app-holder`/`#mastodon` ne contient que les
  deux SVG du logo (79×79 + 261×66) — React n'a jamais monté.
- **§B-5 — la signature a disparu** : les vols précédents montraient
  `module non chargé : <url>` sur le graphe `application-*.js`
  (mastodon = cible de la racine fermée `2b61012`, cap 64→256).
  Ici : **console « pas de journal »** — l'erreur n'existe plus ;
  **heap JS = 31 Mio** → le graphe de modules (134 `<script>`,
  essentiellement `type="module"`) est chargé et instancié. Le fix
  J3 livré tient → `explorer-js`/`core` → **§B-5 RÉSOLU sur
  mastodon** (la ligne §B-5 note chess/letterboxd/marmiton comme
  restes à re-voler ; marmiton restait « toujours présent » au
  vol 36 — à distinguer par cible).
- **§B-4 — la mort se déplace en aval, toujours silencieuse** : les
  modules tournent (31 Mio de heap), l'app meurt au montage sans
  aucune entrée console ni WARN daemon — **zéro ligne hors
  `sync_profile` pour tab=2** (seul le `NoBaseline` re-parse).
  `explorer-core`/`jscore` → **§B-4 TOUJOURS PRÉSENT** : le bras
  « modules drainés, exception aval non tracée » décrit dans la
  ligne §B-4 est confirmé en V4.

## 3. Pinterest — https://www.pinterest.com — §B-4 **TOUJOURS PRÉSENT (zéro trace)**

- **Titre** : « Pinterest » — doc_h 0,0 px, DOM 1,32 Mo / 272 nœuds
  (quasi tout en `<head>` : 81 `<script>`), layout 4 boîtes 0-hauteur.
- **Frame** `captures/vol38_pinterest.png` (+ `_scroll.png`
  identique) : **blanc total**. Le `#shell-loader` (spinner SSR
  censé tourner avant boot) est présent dans le DOM mais s'effondre
  à 1366×0 — la CSS Gestalt n'est jamais appliquée (styles chargés
  par le JS mort).
- **Question de la mission — une trace console ?** **Non.**
  `GET /console` → « pas de journal », inchangé par rapport au
  vol 22 post-fix. C'est la cible la plus silencieuse du vol.
- **Traçage fin dans le journal daemon** : **10 évals seulement**,
  tous `ok=true`, tous minuscules (16–6 723 octets — scripts
  inline). Les ~70 scripts externes Gestalt ne produisent **ni
  `eval fin`, ni `ok=false`, ni WARN fetch** — jamais planifiés
  (defer ?) ou drainés sans trace ; heap JS ~5 Mio. Aucune ligne
  hors `sync_profile` pour tab=3. `explorer-core`/`jscore` →
  **§B-4 TOUJOURS PRÉSENT**, bras « scripts externes jamais
  exécutés sans diagnostic ».
- **Rapprochement §B-19** : le profil (scripts `defer` non drainés,
  cf. GFG vol 36) suggère la famille ordonnancement post-commit —
  même signature : scripts inline passent, scripts différés
  disparaissent.

## 4. Play2048 — https://play2048.co — §B-4 **TOUJOURS PRÉSENT**

- **Titre** : « 2048 • Play the Free Online Game » — doc_h 51 px,
  DOM 6,7 Ko / 65 nœuds (shell SPA : `<div id="app">` **vide**),
  layout 7 boîtes.
- **Frame** `captures/vol38_play2048.png` (+ `_scroll.png`
  identique) : **blanc total**. Le jeu (PixiJS + Web Worker,
  `index-BTR0qhsQ.js` en `type="module"` + 7 `modulepreload`)
  n'a jamais monté son canvas dans `#app`.
- **Console « pas de journal »** — 3 évals `ok=true` (710 o, 129 o
  en 329 ms, 84 o — les inline gtag/RumDash). **heap JS = 25 Mio** :
  le graphe de modules PixiJS est chargé/compilé (comme mastodon,
  le chemin module ne logue pas `eval fin` — les 3 `eval fin` ne
  couvrent que les inline). Mort au montage, sans exception ni
  rejet tracé → `explorer-core`/`jscore` → **§B-4 TOUJOURS
  PRÉSENT**.
- Scroll ×2 : `scroll_y` reste 0 (doc_h 51 px < viewport — rien à
  scroller, cohérent).

---

## Synthèse §8

- **`[V4-main]`** : **1 RÉSOLU** — §B-5 sur mastodon (la signature
  `module non chargé` est éteinte, modules instanciés) ; **1
  DÉGRADÉ EN MIEUX** — §B-15 reddit (partials peuplés + upgrade
  custom elements tracé en échec) ; **TOUJOURS PRÉSENT** — §B-6
  (refus `data:` inchangé **mais désormais WARN-tracé**) et §B-4
  sur mastodon/pinterest/play2048.
- **Lecture transverse de §B-4** : la frontière du silence a
  *reculé d'une étape*. Il n'y a plus de mort « au chargement »
  non tracée — fetch/script/import refusés sont tous WARNés
  (preuve : reddit, où chaque refus est nommé). Ce qui reste
  silencieux est strictement **post-exécution** : (a) scripts
  `defer`/modules jamais drainés (pinterest : 0 `eval fin` externe,
  même pattern que les `defer` Next de GFG au vol 36 → recoupe
  §B-19) ; (b) exceptions aval après instanciation de modules
  (mastodon/play2048 : heap 25–31 Mio, app jamais montée, zéro
  rejet tracé — le `unhandledrejection` JS-C154 n'attrape pas ces
  bras, probablement parce que le rejet n'est pas produit ou est
  intercepté avant).
- **Points d'attention pour SYNTHESE** : §B-5 — cocher mastodon
  comme validé post-fix (restes : chess/letterboxd/marmiton) ;
  §B-6 — noter « refus toujours actif, tracé WARN (WIP J3) » ;
  §B-15 — noter progrès reddit (partials peuplés, define échoue
  sur nom `undefined` ×3 — nouvelle entrée candidate de la
  matrice §B-14 ?) ; §B-4 — reformuler « restent : drains de
  modules/defer non exécutés + exceptions aval post-instanciation
  non tracées » ; §B-18 — ajouter chevauchements barre de tri +
  sidebar login reddit ; §B-1 — pompe `sync_profile` confirmée
  ~4 Hz/onglet en continu.

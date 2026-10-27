<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 12 — Streaming / médias lourds

Date : 2026-10-26
Binaire : `C:/obe-vol-head/target/release/explorer-daemon.exe` (worktree `obe-vol-head`, release)
Viewport : `EXPLORER_VIEWPORT=1366x900` — daemon dédié `:9396`
HEAD du dépôt principal au moment du vol : `7a01534` (`feat(js): JS-C131/132/133 …`)
Procédure : `docs/vol-agent.md`

Cibles : Twitch, Spotify (web player), Vimeo, SoundCloud, + fallback Dailymotion.
Artefacts : `captures/vol12_*.png`, `logs/vol12_*` dans `docs/Rapport vol/`.

---

## Synthèse

| Cible | URL finale | doc_h | Verdict | Sévérité |
|---|---|---|---|---|
| Twitch | `https://www.twitch.tv/` | 900 (coquille) | Page blanche hors shell statique ; hydratation jamais partie, **zéro erreur journalisée** | Critique |
| Spotify | `https://open.spotify.com/` | 1786 | Contenu SSR/landing visible ; chunk vendor rspack mort au **parse** | Haute |
| Vimeo | `https://vimeo.com/` | 13 957 | SSR marketing rendu ; **moteur JS figé dès le 1er script** (« éval dépassée »), toute la suite inerte | Critique |
| SoundCloud | `https://soundcloud.com/` | 105 → erreur | SPA bootée puis tombée dans **son propre error boundary** (« Sorry! Something went wrong ») | Haute |
| Dailymotion | `https://www.dailymotion.com/` | 900 | Page vide ; **commentaires HTML rendus comme texte visible** `<#comment>` | Critique (rendu) + Haute (JS) |

Aucun site n'est fonctionnel ; le meilleur vol (Vimeo/Spotify) tient au contenu **SSR**, jamais au JS client.

---

## 1. Twitch — `https://www.twitch.tv`

**URL finale** : `https://www.twitch.tv/` — titre « Twitch ».

**Console** : `404 pas de journal` — **aucune erreur enregistrée**, avant comme après interactions.

**Constats mesurés** :
- DOM complet livré (118 `<script>`, loader webpack inline ~183 Ko dans le dump DOM) ; `#root` ne contient **que le shell statique** (`.shell-nav`, `.shell-loader`) — l'app React ne s'est jamais montée.
- Layout : `body 1366×122` seulement — barre logo + zone loader ; `doc_h=900` artificiel.
- Le script inline de boot (`document.querySelector("body").classList.add(...)`, DOM dump l. 289) n'a **pas laissé de trace** : `<body>` sans attribut `class` dans le dump → le JS n'a vraisemblablement **jamais exécuté** (ou a échoué sans remonter d'erreur).
- Frame : page blanche, logo Twitch + 3 barres skeleton + glyphe loader. Scroll ×2 + clic : frame **octet-identique** (1 848 391 o avant/après).
- **Première erreur fatale** : non observable — trou d'observabilité, voir constat 1.

**Classification** : échec JS **silencieux total** — le pire cas pour la bissection. Sévérité **critique**.

## 2. Spotify — `https://open.spotify.com`

**URL finale** : `https://open.spotify.com/` — titre « Spotify - Web Player: Listen to Music, Podcasts, and Audiobooks Online ».

**Console (verbatim)** — une seule erreur, rejouée après le clic :
```
syntaxe js 1:119880 : expected `;` —
  https://open.spotifycdn.com/cdn/build/mobile-web-player/vendor~mobile-web-player.344bcab5.js —
  src «(("u">typeof self?self:global).rspackChunk=("u">typeof self?self:global).rspackChunk||[]).push([…»
```
**Première erreur fatale (bissection)** : erreur de **parse** à `1:119880` sur l'expression `("u">typeof self?self:global)` — pattern rspack/webpack standard et **valide** en JS (`"u" > typeof self` → `false` → `global`). Notre parseur rejette `typeof` après l'opérateur `>` (expression unaire en opérande droit de relationnel) → **tout le vendor chunk mort** → toute l'app morte.

**Constats mesurés** :
- Malgré ça, le document sert du contenu SSR : « Trending songs », « Popular artists » (Damso, Booba, Lady Gaga, The Weeknd, Gazo…), pochettes d'albums partiellement chargées, `doc_h=1786`.
- Layout dégradé : grille non appliquée (items en file, textes tronqués « S D F Ri L Ti Co Ed Sho »), bandeau nav flottant en surimpression (position:fixed non géré ?).
- Scroll + clic : même erreur de parse rejouée (re-éval), aucune navigation SPA.

**Classification** : trou **parseur JS** (unaire après `>`) + trous CSS grid/flex/fixed. Sévérité **haute** — la correction du parse débloque potentiellement la cascade entière.

## 3. Vimeo — `https://vimeo.com`

**URL finale** : `https://vimeo.com/` — titre « Vimeo - All-in-One Video Platform ».

**Console (verbatim, top)** — 69 entrées au repos, 74 après interactions :
```
exception js : moteur figé — éval dépassée
moteur js inerte — src «(self.__next_f=self.__next_f||[]).push([0])»
moteur js inerte — src «self.__next_f.push([1,"1:\"$Sreact.fragment\"\n2:I[85341,[],\"\"]\n3:I[90025,[],\"\"]\n"])»
moteur js inerte — src «self.__next_f.push([1,"4:I[30909,[\"9166\",\"static/chunks/620c25d9-919e177b5531293d.js\",\"5246…»
… (× ~70, tous « moteur js inerte »)
```
**Première erreur fatale** : `exception js : moteur figé — éval dépassée` — le **premier** script évalué a dépassé le budget d'évaluation (timeout/step budget) ; le moteur passe ensuite en mode « inerte » et **refuse tout le reste** — mécanisme de coupure sûr, mais il transforme un seul dépassement en mort totale du JS.

**Constats mesurés** :
- Le SSR Next.js est exploitable : nav « Vimeo / Features / Use cases / Enterprise / Learn / Pricing », bouton « Join » cyan, hero « Built for how you work », `doc_h=13 957` (frame pleine page 1366×13958, 63 Mo de PNG).
- Layout dégradé : menus dropdown rendus **ouverts et superposés** (visibilité/hover non honorée), blocs mal alignés.
- Scroll ×2 : fonctionne (page haute). Clic : +5 entrées « moteur js inerte » — le moteur reste figé.
- Bouton play : pas de player sur la home ; clic plausible tenté en nav, aucun effet.

**Classification** : **budget d'évaluation JS trop court / moteur non résilient** — un seul script lent fige tout. Sévérité **critique** (design : la coupure « inerte » devrait être par script, pas globale).

## 4. SoundCloud — `https://soundcloud.com`

**URL finale** : `https://soundcloud.com/` — titre « Stream and listen to music online for free with SoundCloud ».

**Console (verbatim)** :
```
exception js : TypeError: cannot read `setRequestHeader` of Undefined
    at <anonymous> (inline:1:2:40804)
    at <anonymous> (inline:1:2:48576)
    at <anonymous> (inline:1:2:20)
exception js : TypeError: `toLowerCase` is not a function (on Object)
    at <anonymous> (https://a-v2.sndcdn.com/assets/1-06633502.js:78:60345)
    at <anonymous> (https://a-v2.sndcdn.com/assets/1-06633502.js:67:2755)
    at <anonymous> (https://a-v2.sndcdn.com/assets/1-06633502.js:67:47)
    at <anonymous> (https://a-v2.sndcdn.com/assets/1-06633502.js:54:9042)
    at c (https://a-v2.sndcdn.com/assets/58-6ba357f4.js:1:475)
    … — webpackJsonp push
```
**Première erreur fatale** : `cannot read setRequestHeader of Undefined` (script inline, `inline:1:2:40804`) → **`XMLHttpRequest` absent** du moteur JS. La seconde (`toLowerCase is not a function on Object`, chunk webpack `1-06633502.js:78:60345`) suggère un mismatch de type dans notre impl — probablement un receiver non-string.

**Constats mesurés** :
- Point notable : le moteur a exécuté **assez** de webpack pour que l'app monte son **error boundary** — rendu « Sorry! Something went wrong / Is your network connection unstable or browser outdated? » avec logo et illustration — c'est le comportement dégradé « correct » de l'app elle-même.
- `doc_h=105` puis page d'erreur ; clic : console inchangée (2 entrées), aucune navigation.

**Classification** : API Web manquante (**XMLHttpRequest**) + bug typage `toLowerCase`. Sévérité **haute** — XHR est la porte d'entrée de quasiment toutes les SPA.

## 5. Dailymotion — `https://www.dailymotion.com` (fallback)

**URL finale** : `https://www.dailymotion.com/` — titre « Dailymotion ».

**Console (verbatim)** :
```
exception js : TypeError: cannot read `config` of Undefined
    at <anonymous> (inline:10:1:127367)
    at o (inline:10:1:409)
    at <anonymous> (inline:10:1:136)
```

**Constats mesurés** :
- **Bug HTML/rendu isolé** : les commentaires du document sont rendus comme **texte visible** `<#comment>` en haut de page (9 occurrences en ligne 1 du viewport, layout `text "<#comment>"`). Le dump DOM montre le même artefact → probablement le serializer de dump qui fuit dans le rendu, ou un nœud commentaire promu en texte par le tree-builder.
- `body 1366×36.8` — sous le bandeau de commentaires, tout est à hauteur 0 : l'app (JS-mort : `cannot read config of Undefined`, bootstrap webpack inline) n'a rien construit.
- Frame : quasi blanche, `doc_h=900` artificiel.

**Classification** : trou **tree-builder/rendu** (commentaire → texte) + JS mort. Sévérité **critique** pour le bug de commentaire (défaut visible même sans JS, reproductible sur tout site à `<!-- -->` en body).

---

## Constats actionnables

1. **Parseur JS — unaire après opérateur relationnel** : `("u">typeof self?self:global)` rejeté (`expected ;` à 1:119880) — pattern émis par rspack/webpack sur quasi tous les bundles modernes. Corriger la grammaire des expressions unaires en opérande de `<`/`>`. *Une seule correction → débloque Spotify entièrement, probablement d'autres sites.* (Spotify, sémantique exacte à reproduire sur `vendor~mobile-web-player.344bcab5.js`.)
2. **Moteur JS — budget d'évaluation = coupure globale** : « moteur figé — éval dépassée » au 1er script de Vimeo puis « moteur js inerte » ×70+. Rendre la coupure **par script** (ou relever le budget / le rendre proportionnel à la taille) — un seul chunk lent ne doit pas tuer la page entière.
3. **XMLHttpRequest absent** : `setRequestHeader of Undefined` tue SoundCloud en bootstrap. Implémenter un XHR minimal (`open`/`setRequestHeader`/`send`/`onload`) suffirait à faire passer l'app au-delà de son error boundary.
4. **`toLowerCase is not a function (on Object)`** — `1-06633502.js:78:60345` (SoundCloud) : coercition/méthode prototype manquante sur Object ; à reproduire en unitaire.
5. **Trou d'observabilité — Twitch** : 118 scripts, hydratation morte, **zéro entrée console** — ni erreur de fetch, ni exception, ni « inerte ». Le script inline `classList.add` n'a pas muté `<body>` → JS probablement jamais exécuté et l'échec n'est remonté nulle part. Journaliser au minimum « script exécuté/échoué/filtré » pour que `/console` ne réponde plus `pas de journal` quand tout est mort en silence.
6. **Commentaires HTML rendus en texte `<#comment>`** (Dailymotion, visible aussi dans le dump DOM) : nœuds commentaires promus en texte rendu — bug tree-builder ou serializer de frame. Repro minimale : `<!-- x -->` en body.
7. **`cannot read config of Undefined`** (Dailymotion, bootstrap inline) — même famille que #3 : un global Web attendu manquant (`XMLHttpRequest`/`fetch`-adjacent ou `navigator`-adjacent à confirmer en repro inline).
8. **CSS — menus hover/dropdown rendus ouverts** (Vimeo) et **grid/fixed dégradés** (Spotify : items en file, nav flottante) : `[hidden]`/`visibility`/`:hover`-gated content affiché ; grille CSS non appliquée. Sévérité moindre que le JS mais visible sur tout SSR.
9. **`doc_h=900` artificiel** (Twitch, Dailymotion) : la hauteur de document retournée colle au viewport quand le body fait ~37-122 px — métrique trompeuse pour la mesure de « page vide » ; vérifier le clamp de `doc_h`.

## Ordre d'attaque suggéré

1. Constat 1 (parse `>`+unaire) — trivial à tester, déblocage maximal.
2. Constat 2 (budget/inerte par script) — transforme les échecs « tout ou rien » en dégradés mesurables.
3. Constat 3+7 (XHR + globals manquants) — la porte d'entrée des SPA.
4. Constat 5 (console silencieuse) — sans lui, les bissections Twitch-like restent aveugles.
5. Constats 6, 8, 9 — trous rendu/métrique.

---

## Annexes techniques

- Onglets utilisés : 1 (Twitch), 2 (Spotify), 3 (Vimeo), 4 (SoundCloud), 5 (Dailymotion) — tous fermés en fin de vol.
- Captures : `vol12_twitch.png` / `_after` (identiques), `vol12_spotify.png` / `_after`, `vol12_vimeo.png` (pleine page 1366×13958) + `vol12_vimeo_top.png` + `_scrolled`, `vol12_soundcloud.png`, `vol12_dailymotion.png`.
- Logs : `vol12_*_console.json`, `vol12_*_dom.txt`, `vol12_*_layout.txt`.

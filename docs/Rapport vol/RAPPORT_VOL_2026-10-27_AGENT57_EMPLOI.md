<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 57 — Sites d'emploi (2026-10-27)

- **Binaire** : `[V4-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (WIP `depth_limit` embarqué), `--port 9439`,
  `EXPLORER_VIEWPORT=1366x900`, journal `_vol57_daemon.log`
  (racine, copié `logs/daemons/_vol57_daemon.log`, 330 963 lignes)
- **Cibles** : www.welcometothejungle.com · fr.indeed.com ·
  www.pole-emploi.fr (→ francetravail.fr) — famille « emploi »,
  dont **re-vol indeed** pour le bandeau `fixed` (FIX-2 « `fixed`
  échappe au clip `overflow` ancêtre (bandeau indeed) », §B-18)
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de
  `SYNTHESE.md`
- **RAM daemon (PID 17648)** : 375 Mo au boot → 672 Mo (wttj +
  retour) → 1 385 Mo (indeed /companies) → **1 500 Mo pic**
  (francetravail) → DELETE ×3 → **477 Mo, stable — libération
  complète**. **§B-30 non reproduit**, **§B-26 non reproduit**
  (API restée responsive, 0 wedge).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| welcometothejungle.com | **Partiel** — header+hero peints mais colonne texte effondrée (mot/ligne), smear SVG en-tête | vide puis `HTTP 403` | **§B-33 proposé** — colonne `block` à largeur 0 · **§B-34 proposé** — SVG hors de sa boîte · clic→nav réelle → 403 externe |
| fr.indeed.com | **~85 % fidèle** — page complète, bandeau consent fixe correct | vide | **Re-vol bandeau `fixed` : CONFIRMÉ RÉPARÉ** (§B-18 bras) · **§B-34** — logo SVG peint géant ~700 px (boîte 300×81,6) pixelisé · clic→nav /companies ~90 % |
| www.pole-emploi.fr → francetravail.fr/accueil | **Contenu SSR complet voilé** — backdrop `pe-cookies` plein écran | vide | **§B-17 confirmé** (backdrop francetravail, déjà catalogué) + **§B-15** (DSD aplatie, modal 0×0) · clic sous overlay `ignored` (variante §B-13) · **2 évals `ok=false` sans entrée console** (trou §B-4, identique vol 55) |

Acquis : zéro panic, zéro stack overflow (WIP `depth_limit` non
sollicité en échec), zéro wedge, RAM intégralement libérée,
redirection cross-domaine pole-emploi→francetravail transparente,
2 navigations réelles par clic sur 3 tentées (la 3ᵉ sous overlay).
Journal : 3 WARN seulement (`journal DOM rejeté — Fingerprint`,
×2 tab=2, ×1 tab=3 — recouverts, `journal_fallbacks` 1 et 3).
**98 évals de scripts** (8/86/4 par onglet), 47 `ok=true`,
2 `ok=false` — les deux sur tab=3, **sans trace console** (§B-4).

---

## 1. welcometothejungle.com — colonne héroïque à largeur 0, smear SVG d'en-tête

- `POST navigate` → titre « Welcome to the Jungle — Work with the
  company you belong to », `doc_h` **10 987**, URL finale
  `https://www.welcometothejungle.com/en` (redir `/` → `/en` OK).
- `/console` : « pas de journal » (ring vide).
- `/mem` : 1 100 nœuds DOM, 378 boîtes, 488 display items,
  `js_heap` 16,3 Mo, `images_bytes` **129,9 Mo**,
  `journal_applies:0`, `journal_fallbacks:0`.
- `/dom` 200 (3 398 lignes), `/layout` 200 (379 lignes).
- 8 évals script dans le journal daemon, aucune `ok=false`.

### Rendu — `captures/vol57_wttj.png` / `_scroll.png`

Header rendu (logo WTTJ, nav Find a job / Explore companies /
Resources, « US »), fond beige correct, carte « Senior Product
Manager » (badges, boutons Save/Skip) correctement peinte au
scroll (`scroll_y` 900 → 1 800, `doc_h` réel exploité).
**Deux défauts majeurs** :

### §B-33 (proposé) — colonne de contenu effondrée à largeur 0 → texte mot-par-ligne

- Layout : `block<div> [40 215 0x876.0801]` contient
  `block<h2> [40 254.84 0x518.4]` — le `h2` « Stop looking for any
  job. Find where you belong. » a une **boîte de largeur 0** : chaque
  mot part sur sa propre ligne (frame : « Stop / looking / for /
  any / job. / Find / where / you / belong. »). Idem le paragraphe
  `block<p> [40 829.08 0x194]` (« Match / with / your … ») et, au
  scroll, la colonne gauche (~60 px de visible, texte tronqué
  « and / Get / ely »).
- Le parent immédiat `block<div> [40 215 0x876]` est lui-même à
  largeur 0 alors que son parent `block<section> [40 155
  1286x1008]` est plein — effondrement de largeur sur un item
  (grid/flex probable, la page est en CSS moderne). Le frère
  `block [0 -5 1366x0]` est effondré en **hauteur** — signature
  mixte.
- Famille proche de §B-24 (hauteur 0 du stretched-link) et §B-28
  (`height:100%` effondrée) mais ici c'est la **largeur d'une
  colonne de flux** qui tombe à 0 → propose une entrée distincte.
  Crates `explorer-layout`/`css`. **NOUVEAU — `[V4-main]`**.

### §B-34 (proposé) — SVG peint/positionné hors de sa boîte de layout

- En-tête droit : `block<button> [1231.55 55 87.4x34]` contient
  **deux** `block<svg>` dimensionnés `300x16` — le premier placé à
  `x=941.55` (soit **290 px à gauche de son parent**, chevauchant
  les liens « I'm a recruiter »/« Sign in » → le gribouillis noir
  visible sur la frame, runs de texte écrasés sous le SVG).
- Sur indeed (§2), même famille inversée : boîte `block<svg>
  [533 146 300x81.58]` conforme (fix `viewBox` sans dims → 300 px
  appliqué) **mais le paint déborde** : le wordmark est rasterisé
  ~700 px de large, pixelisé, recouvrant le `h2` — vérifié par
  zoom région `/frame?x=400&y=120&w=700&h=280`
  (`captures/vol57_indeed_logo_zoom.png`).
- Lecture : le pipeline de paint SVG n'honore pas systématiquement
  le mapping boîte-de-layout → viewport utilisateur (position du
  fragment dans le parent sur wttj ; échelle sur indeed). Recoupe
  le symptôme §B-32 (rasters pixelisés — ici la pixellisation est
  la conséquence du sur-étirement). Crates `explorer-layout` +
  `explorer-render`. **NOUVEAU — `[V4-main]`**.

### Clic → navigation réelle puis 403 externe

- Clic `{"x":306,"y":72}` (centre du lien « Find a job »
  `[265.7,50,81.5x44]`, `scroll_y=0`) → **`{"result":"navigated"}`**
  → `https://www.welcometothejungle.com/en/get-started/job-title`,
  titre « Erreur de chargement », console `HTTP 403`
  (`captures/vol57_wttj_403.png`). Le hit-test et le dispatch
  fonctionnent (§B-13 reste sain) ; le 403 vient du serveur sur
  cette route interne — **famille ⚪ externe** (anti-bot/
  session), pas un bug moteur. `POST /back` → retour `/en`
  correct (`can_forward:true`).

## 2. fr.indeed.com — bandeau `fixed` réparé, logo SVG géant

- `POST navigate` → « Emplois | Indeed », `doc_h` **918,4**
  (page courte, conforme), URL stable.
- `/console` : « pas de journal » ; 86 évals script, 0 `ok=false`.
- `/mem` : 1 210 nœuds DOM, 296 boîtes, 282 display items,
  `js_heap` 18,2 Mo, `journal_fallbacks:1` (WARN `Fingerprint`
  recouvert).
- `/dom` 200 (3 886 lignes), `/layout` 200 (297 lignes).

### Re-vol §B-18 — bandeau consent `fixed` : **RÉPARÉ**

- Le bandeau cookies (`block<div> [0 786 1366x114]`, ancêtre
  `block<header> [0 0 1366x28]`) est **peint et reste fixé en bas
  de viewport** sur frame initiale ET après `scroll_y=18,4`
  (max de la page) — `captures/vol57_indeed.png`,
  `vol57_indeed_scroll.png`, `vol57_indeed_companies.png`.
- Le fix « `fixed` échappe au clip `overflow` ancêtre » tient en
  conditions réelles. **Verdict re-vol : OK** pour ce bras de
  §B-18.
- Reste : la colonne boutons du bandeau déborde à droite —
  `block<div> [1133.78 843 601.04x154.4]` sort du viewport
  (1133+601 > 1366), bouton « Autoriser tous les cookies » peint
  tronqué (« Autoriser tous les co… »). Overflow horizontal
  résiduel → rattacher à §B-18 (layout résiduel), gravit y mineure.

### §B-34 — wordmark SVG géant pixelisé (preuve ci-dessus, §1)

`block<svg> [533 146 300x81.58]` peint ~700 px → occulte « Trouvez
votre prochain emploi ici » ; icône flèche du bouton « Commencer »
idé (`block<svg> [682 285 94.5x94.5]` dans un bouton
`106x96`) ; grand check « Tendances » `139.8x139.8`. Même anomalie
que le smear wttj, pôle « échelle au paint ».

### Navigation par clic + /companies — sain, rendu ~90 %

- Clic `(275,14)` sur « Avis sur les entreprises »
  `[202.6,3.5,144.5x21]` → `navigated` → `/companies` (« Meilleures
  entreprises »). `captures/vol57_indeed_companies.png` : grille
  3×3 d'entreprises avec **vrais logos décodés** (Vitalliance,
  L'Oréal, Enedis, Verisure, GEODIS, Castorama, EDF, La Poste,
  Intersport), titres, compteurs d'avis, barre de recherche —
  ~90 % fidèle. Console vide.
- Cosmétique (sans ID — famille « anomalies cosmétiques » §B) :
  liens inline collés sans espace — « Page d'accueil**A**vis sur
  les entreprises**E**stimation de salaire », « Salaires**Q**uestions**E**mplois
  ouverts » ; header tronqué à droite (« …offre d'emplo »).

## 3. pole-emploi.fr → francetravail.fr — SSR complet sous voile `pe-cookies`

- `POST navigate https://www.pole-emploi.fr` → **redirection
  cross-domaine transparente** → `https://www.francetravail.fr/
  accueil/`, titre « Accueil | France Travail », `doc_h` 4 606.
- `/console` : « pas de journal » — **malgré 2 évals `ok=false`**
  (voir §B-4 ci-dessous).
- `/mem` : 640 nœuds DOM, 818 boîtes, 1 056 display items,
  `js_heap` 16,7 Mo, `images_bytes` 32,4 Mo, `journal_fallbacks:3`.
- `/dom` 200 (1 443 lignes), `/layout` 200 (818 lignes).

### §B-17 confirmé — backdrop `pe-cookies` plein écran `[V4-main]`

- Frame (`captures/vol57_francetravail.png` + `_scroll.png`) :
  contenu SSR **complet et correct** — « Recherche d'offres /
  747540 offres disponibles », formulaire, cartes allocations/
  recherche/formation, articles avec **photos réelles décodées**,
  footer — mais tout est **voilé par un assombrissement uniforme**.
- Cause précise dans le layout : `block<pe-cookies> [0 5268.7
  1366x0]` contient `block<div> [0 0 1366x900]` — le div du
  **shadow DOM déclaratif** (`<template shadowrootmode="open">`)
  peint en `position:fixed` à l'origine document → backdrop plein
  viewport. Le contenu modal (main/footer du consentement) reste
  `0x0` inline — **seul le voile est peint, pas la boîte de
  dialogue** : la page est inutilisable (aucun moyen de consentir).
- C'est le « backdrop francetravail » déjà listé sous §B-17 —
  **confirmé toujours présent `[V4-main]`**, et cette fois avec le
  mécanisme nommé : **DSD aplatie en light DOM** → recoupe §B-15
  (custom elements : `pe-cookies`, `ft-header`, `ft-footer`
  non upgradés — le `<ft-header>` a d'ailleurs une nav quasi vide
  `[1181 0 185x100]` sans intitulés).
- Clic `(400,500)` sur le lien « Rechercher des offres par région
  … » (visible sous le voile, `scroll_y=0`) → **`ignored`** —
  variante attendue « élément sous overlay » de §B-13
  (comportement correct du hit-test, l'overlay intercepte).

### §B-4 confirmé — évals `ok=false` muettes `[V4-main]`

Journal daemon tab=3 :

```
script: eval… tab=3 bytes=384019 → ms=78  ok=false
script: eval… tab=3 bytes=3972   → ms=8   ok=false
```

(2 autres évals inline ok). **Zéro entrée `/console`** pour ces
deux échecs — reproduction exacte du trou signalé au vol 55
(diagrams.net) : une éval en échec non remontée au ring = bras
aveugle de §B-4. Probablement le bundle applicatif (384 Ko) et
`pe-cookies`/tarteaucitron — cohérent avec le voile jamais levé.
À noter : les erreurs ne sont **pas** des `unhandledrejection` —
l'échec est à l'éval synchrone, dont le chemin de reporting vers
le ring reste manquant.

## Statuts proposés (§8)

| §B | Bras | Verdict ce vol |
|---|---|---|
| §B-18 (bras `fixed`) | bandeau indeed échappe au clip ancêtre | **RÉSOLU sur cette cible `[V4-main]`** — bandeau consent peint, fixe au scroll, boutons lisibles (overflow-x résiduel mineur à garder sous §B-18) |
| §B-33 (proposé) | **Colonne de contenu effondrée à largeur 0** → texte mot-par-ligne (h2/p/grid item) | **NOUVEAU `[V4-main]`** — wttj, `logs/vol57_wttj_layout.txt` (`[40 215 0x876]`), crate `explorer-layout`/`css` |
| §B-34 (proposé) | **SVG peint hors de sa boîte** — position du fragment hors parent (wttj) / échelle non respectée (indeed), pixellisation associée | **NOUVEAU `[V4-main]`** — crates `explorer-layout`/`render`, recoupe le symptôme §B-32 |
| §B-17 | overlays fermés par JS — backdrop francetravail | **confirmé `[V4-main]`** — mécanisme nommé : DSD `pe-cookies` aplatie, backdrop `position:fixed` 1366×900 à y=0, modal 0×0 |
| §B-15 | shadow DOM / custom elements | **+1 bras `[V4-main]`** — `pe-cookies`/`ft-header`/`ft-footer` non upgradés, template DSD rendu en light DOM |
| §B-4 | morts/erreurs JS silencieuses | **confirmé `[V4-main]`** — 2 évals `ok=false` (384 Ko + 4 Ko) sans aucune entrée `/console`, francetravail — même trou qu'au vol 55 |
| §B-13 | hit-test | **sain** — 2/2 clics hors overlay → navigations réelles ; clic sous overlay `ignored` (variante attendue) |
| §B-30 | fuite post-DELETE | **NON REPRODUIT** — 1 500 → 477 Mo après DELETE ×3, stable |
| §B-26 | wedge DOM/apply | **NON REPRODUIT** — `/dom`, `/frame`, `/mem` 200 immédiats sur les 3 cibles |
| §B-3 | stack overflow | **NON REPRODUIT** — aucune cible à pattern récursif observé |

## Hygiène de vol

- Onglets 1 (wttj), 2 (indeed), 3 (francetravail) fermés par
  `DELETE /api/tabs/{id}` → `{"tabs":[]}`.
- Daemon tué par son **PID 17648** (`taskkill //PID`) ; daemons
  voisins (PID 10764, 2056 — autres vols) non touchés. Deux
  anciens PID (3444, 18264) ont disparu d'eux-mêmes en cours de
  vol — non liés à ce vol.
- Artefacts :
  - `captures/vol57_{wttj,wttj_scroll,wttj_403,indeed,
    indeed_logo_zoom,indeed_scroll,indeed_companies,
    francetravail,francetravail_scroll}.png`
  - `logs/vol57_{wttj,indeed,francetravail}_{console.json,
    dom.txt,layout.txt,mem.txt}` + `vol57_indeed_companies_
    console.json`
  - `logs/daemons/_vol57_daemon.log` (330 963 lignes)
- NB : scroll wttj vérifié `scroll_y` 0→900→1 800→0 ; indeed
  plafonné à 18,4 (= `doc_h`−viewport, conforme) ; francetravail
  0→900→1 800→0.

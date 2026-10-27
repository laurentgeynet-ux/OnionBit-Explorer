<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 32 — Outils de productivité/notes

- **Date** : 2026-10-26
- **Binaire** : `C:/obe-vol-integ/target/release/explorer-daemon.exe` (release)
- **Viewport** : `EXPLORER_VIEWPORT=1366x900`, port 9416
- **Cibles** : notion.so, dropbox.com, evernote.com, todoist.com/fr
  (fallback figma non joué — les 4 cibles ont répondu)
- **Méthode** : `docs/vol-agent.md` — POST /api/tabs, navigate,
  grâce ~10-12 s, `/console` verbatim, `/frame` PNG, `/dom` + `/layout`
  vers `logs/vol32_*`, scroll ×2 puis clic plausible.

## Cible 1 — notion.so

- **URL finale** : `https://www.notion.com/` (redirect suivi)
- **Titre** : « The AI workspace that works for you. | Notion »
- **Console** : `pas de journal` (aucune erreur)
- **Mesures** : doc_h = 3060.6 px (non clampé), 1024 nœuds DOM,
  862 boîtes, 578 display items, JS heap 32,6 Mio, total 71,3 Mio.
- **Constats mesurés** (`vol32_notion.png`, `vol32_notion_scroll.png`) :
  - Héro écrasé en une **colonne d'environ 110 px de large** centrée
    (~x 645–755 sur 1366) : « Where teams and agents together. »
    rend un mot par ligne ; boutons « Get Notion free » / « Request
    a demo » tronqués à la même largeur. Layout flex/grid du héro
    non résolu.
  - **Sous le fold** (scroll_y=1600) : sections cartes rendues
    correctement en grille 2+5 colonnes (« Bring everything into one
    system of record », « Triage product feedback »…) — le problème
    est localisé au héro. Texte déborde légèrement du bas des cartes
    (mots coupés à la bordure).
  - Scroll ×2 OK (`scroll_y` 800 → 1600). Clic (683,450) :
    `{"result":"ignored"}` — zone héro non hydratée (comportement
    documenté, pas re-diagnostiqué).
- **Classification** : défaut layout (dimensionnement du conteneur
  héro, probablement `grid`/`minmax` ou `clamp()` non supporté).
- **Sévérité** : majeure (above-the-fold illisible).

## Cible 2 — dropbox.com

- **URL finale** : `https://www.dropbox.com/`
- **Titre** : « Dropbox: Secure cloud storage, file sharing, and more »
- **Console** : `pas de journal`
- **Mesures** : doc_h = 7836.8 px, 1969 nœuds DOM, 1827 boîtes,
  1339 display items, JS heap 6,1 Mio, total 49 Mio.
- **Constats mesurés** (`vol32_dropbox.png`) :
  - **Page rendue quasi sans CSS** : liens bleu souligné par défaut,
    puces `•` de liste visibles, zéro image de contenu dans le
    viewport (`img_bodies_bytes` = 0 !), menus « Products » /
    « Solutions » affichés dépliés (tous les sous-menus visibles).
  - 10+ `<link rel="stylesheet">` présents dans le DOM
    (`logs/vol32_dropbox_dom.txt` l. 365-457) pointant vers
    `cfl.dropboxstatic.com`, tous avec
    `data-loader="dbx_edison_page[preload_css]"` et `crossorigin` —
    CSS chargée par un loader JS (probablement `rel=preload` → swap
    en stylesheet par script) : le moteur voit les `<link>` mais le
    style n'est jamais appliqué.
  - Scroll ×2 OK (1600). Clic (300,400) : `ignored`.
- **Classification** : CSS non appliquée — piste probable : pattern
  preload→stylesheet piloté par JS non rejoué par le moteur, ou
  fetch CSS échoué silencieusement (aucune erreur console/log).
- **Sévérité** : critique (page utilisable mais brut HTML 1995).

## Cible 3 — evernote.com

- **URL finale** : `https://evernote.com/`
- **Titre** : « Best Note Taking App - Organize Your Notes with Evernote »
- **Console** : `pas de journal`
- **Mesures** : doc_h = 7426.6 px, 1362 nœuds DOM, 1063 boîtes,
  735 display items, **images 101,7 Mio décodées** (images_bytes),
  JS heap 4,3 Mio, total 148,4 Mio.
- **Constats mesurés** (`vol32_evernote.png`, `vol32_evernote_scroll.png`) :
  - **Meilleur rendu du vol** : logo vert + wordmark, nav horizontale,
    héro « Your second brain » centré, fond crème — fidèle.
  - Bouton « Get Evernote free » rendu comme texte simple (style
    bouton/CTA non appliqué), idem « Discover more ».
  - Scroll : sticky header correct ; section onglets « Note-Taking…
    & Spaces » présente des **libellés d'onglets qui se
    superposent** (~y 105-160 : « Evernote & Spaces » chevauchant
    « Personal/Group »…) — carousel non hydraté, tous les panneaux
    empilés au même endroit.
  - Section sombre « Your productivity, supercharged » correcte.
  - Clic (600,500) : `ignored`.
- **Classification** : rendu majoritairement bon ; défauts = CTA non
  stylés + panneaux d'onglets non masqués (JS d'initialisation non
  abouti, sans erreur console).
- **Sévérité** : mineure à modérée.

## Cible 4 — todoist.com/fr/

- **URL finale** : `https://www.todoist.com/fr`
- **Titre** : « Todoist | Une to do list pour organiser vie et travail »
- **Console** (verbatim, 20 entrées) :
  `unhandledrejection: fetch échoué` ×20 — le fix
  `unhandledrejection` remonte bien en console (confirmé).
- **Mesures** : doc_h = 9932.2 px, 6175 nœuds DOM, 2274 boîtes,
  2131 display items, JS heap 51,1 Mio, total 117,7 Mio.
- **Constats mesurés** (`vol32_todoist.png`, `vol32_todoist_scroll.png`) :
  - Barre nav : libellés de menus **superposés** (« Projects /
    Personal use / Businesses… » empilés sur la même ligne, illisible).
  - Colonne de texte héro gauche **réduite à ~45 px de large**
    (« Enfin plus de clarté. » un mot par ligne, texte clippé) ;
    l'illustration (capture de l'app, téléphone incliné) occupe
    ~75 % du viewport à droite — les images se chargent (12 Mio).
  - **Sous le fold** : rendu très correct — texte français complet
    (« Centralisez les tâches de votre équipe »…), badge AICPA SOC2,
    captures produit nettes. Le défaut est donc au niveau du
    layout héro (grid/flex + positionnement absolu).
  - 20 `fetch` rejetés pendant le boot → probablement cause du nav
    non hydraté et des menus superposés.
  - Scroll ×2 OK (1600). Clic (683,450) : `ignored`.
- **Classification** : défaut layout héro + hydratation JS incomplète
  (fetch échoués).
- **Sévérité** : majeure au-dessus du fold, modérée globalement.

## Constats actionnables

1. **Dropbox — aucune CSS appliquée malgré 10+ `<link rel=stylesheet>`
   présents** : investiguer le pattern `data-loader="…[preload_css]"`
   (link swapé de `preload` à `stylesheet` par JS). Vérifier si les
   fetchs CSS vers `cfl.dropboxstatic.com` partent et si un changement
   d'attribut `rel` post-parser re-déclenche l'application. Aucune
   erreur journalisée — ajouter de la télémétrie sur les feuilles
   de style chargées/appliquées.
2. **Notion + Todoist — conteneur héro effondré à ~45–110 px** :
   même signature (colonne de texte héro réduite à sa largeur min,
   un mot par ligne, le reste du layout OK sous le fold). Bissection
   commune probable : `grid-template-columns` avec `minmax`/`fr`,
   `clamp()`, ou flex-basis — repro minimal à extraire des deux DOM
   (`logs/vol32_notion_layout.txt`, `logs/vol32_todoist_layout.txt`).
3. **Todoist — 20 × `unhandledrejection: fetch échoué`** : les fetch
   du boot échouent tous (analytics ? API ?). Identifier la cause
   (CORS ? endpoint ?) — le mécanisme de remontée `unhandledrejection`
   fonctionne, c'est la couche fetch qui est à creuser.
4. **Evernote/Todoist — panneaux d'onglets non masqués** : contenus
   `hidden`/`aria-hidden` ou classes togglées par JS restent visibles
   et se superposent (Evernote onglets ~y 105-160 ; Todoist nav).
   À corréler avec l'hydratation incomplète, possiblement même
   cause racine que (3).
5. **Clics `ignored` systématiques** sur zones non hydratées
   (4/4 cibles) — comportement connu, documenté ici pour la trace,
   pas de re-diagnostic.

## Hygiène de vol

- Aucun daemon résiduel sur 9416 au départ (vérif netstat).
- Onglets supprimés via `DELETE /api/tabs/{id}`, daemon tué par PID.
- Aucun code modifié.

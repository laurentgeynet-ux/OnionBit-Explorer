<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 39 — Sites voyage (2026-10-27)

- **Binaire** : `[V4-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
- **Daemon** : `--port 9424`, `EXPLORER_VIEWPORT=1366x900`, journal `_vol39_daemon.log`
- **Cibles** : sncf-connect.com · blablacar.fr · airbnb.fr (SSR+hydratation §B-4) · vinted.fr
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de `SYNTHESE.md`

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| sncf-connect.com | **403 anti-bot** — page d'erreur moteur propre | `HTTP 403` | ⚪ externe (signature 403 catalogue) |
| blablacar.fr | **SSR fonctionnel, layout dégradé** | vide | §B-18 (chevauchements + repli serif) ; nav interne → 403 externe |
| airbnb.fr | **Shell SSR + squelettes, hydratation morte** | `unhandledrejection` ×6 | §B-4 recoupe (tracée, non silencieuse) + suspect §B-10/§B-14 |
| vinted.fr | **Rendu quasi-parfait — cards incliquables** | vide | **NOUVEAU §B-23** : overlay stretched-link à hauteur 0 |

Acquis : 3/4 pages servies avec contenu réel ; scroll sain partout où le
document dépasse ; 2 navigations réelles par clic ; `unhandledrejection`
tracé ; zéro panic, zéro freeze, API restée responsive en continu.

---

## 1. sncf-connect.com — 403 externe

- `POST navigate` → `{"error":{"handled":true,"message":"HTTP 403"}}`
- `/console` verbatim : `{"console":["HTTP 403"]}`
- `/dom` → `pas de DOM` ; `/layout` → `pas de layout`
- Frame : page d'erreur moteur « Page inaccessible — Le serveur a
  répondu « HTTP 403 ». Réessayez plus tard… » — peinture propre
  (`captures/vol39_sncf-connect.png`).
- Input : `scroll` → `{"result":"scrolled","scroll_y":0.0}` ; `click`
  → `ignored` (attendu : pas de document).

**Constat** : refus côté serveur, hors moteur — `explorer-net` externe.
Signature cumulée identique au catalogue ⚪ (403 sec : ebay, etsy,
stackoverflow…) : ajouter sncf-connect.com à la liste des 403.
Le repli « Page inaccessible » fait correctement son office.

## 2. blablacar.fr — SSR réel, layout dégradé `[V4-main]`

- Navigate : `doc_h=15161`, titre réel « Covoiturage, Bus ou Train… »
- `/console` : **vide** — aucune erreur JS (55 `<script>`, 6 feuilles).
- DOM 389 Ko, layout 131 Ko — contenu SSR complet : nav, formulaire
  De/Vers/Départ/Retour/Passagers, itinéraires, listes de villes.

### Rendu (`captures/vol39_blablacar.png` + `_scroll.png`)

- **Police repli serif** partout (Times-like) alors que le site utilise
  un sans-serif propriétaire — `@font-face` non appliqué ou webfont
  jamais résolue → `explorer-css`/`explorer-net`, joint à §B-18 (à
  qualifier : absence de warn fetch dans la console).
- **Chevauchements de runs inline** : le carrousel d'itinéraires peint
  « Lille 5,49 € Amsterdam 8,99 € Strasbourg 8,99 € Metz 8,49 € »
  superposé sur l'image hero droite ; la nav du header recouvre le
  contenu au scroll (header `fixed` correct mais texte du carrousel
  déborde sa piste) → `explorer-layout` → **§B-18** (famille
  « chevauchements de runs inline » + débordement carrousel).
- Champs de recherche peints en texte plat sans boîtier visible ;
  les images réelles (ville, portraits) chargent correctement.
- **WARN daemon** : `journal DOM rejeté → repli re-parse (ADR-0025)
  why=Fingerprint` (tab=2) — repli fonctionnel, rendu produit.

### Input

- `scroll` ×2 → `scroll_y=1600` réel, contenu repeint correctement.
- Clic « Dunkerque » (625,272 — `<a href="/ride-sharing/dunkerque/
  lille">`) → **`{"result":"navigated"}`** : navigation réelle
  (`/ride-sharing/dunkerque/lille`) → **§B-13 : confirmation de
  résolution sur ancres standard `[V4-main]`**.
  - La page cible renvoie ensuite **HTTP 403** (anti-bot sur requête
    interne — Datadome probable) → ⚪ externe.
- Clic header « Connexion » (1170,20 viewport, header visible fixe
  au scroll) → `ignored` alors que le texte est à doc [1141,12] :
  élément `fixed` peint hors de sa position document → le hit-test ne
  retrouve pas l'ancre. Sous-cas résiduel §B-13 (« hit-test sur
  `position:fixed` en scroll ») — `explorer-layout`/`core`, mineur.

## 3. airbnb.fr — hydratation morte `[V4-main]` → §B-4

- Navigate : `doc_h=900.0` (= viewport — signature « `doc_h` trompeur »,
  §B-perf), titre SSR correct.
- `/console` verbatim :

```
unhandledrejection: TypeError: `preloadRestQuery` is not a function (on Object)
    at <anonymous> (…/web/common/ef24.4098de3376.js:58:1090)
    at <anonymous> (…/web/common/ef24.4098de3376.js:58:702)
    at _ (…/web/fr/e6f0.71a78c9b83.js:62:209)
    at s (…/web/fr/e6f0.71a78c9b83.js:62:709) (×2)
[même entrée] (×4)
```

- Frame (`captures/vol39_airbnb.png`) : logo, onglets
  Logements/Hôtels/Expériences/Services, bouton recherche — puis
  **uniquement les placeholders squelettes** (barres grises). DOM
  645 Ko complet ; layout 40 Ko (hauteur bornée au shell).
- `scroll` ×2 → `scroll_y` reste `0.0` (document = viewport).
- Clic onglet « Logements » → **`{"result":"navigated"}`** vers
  `/homes` — frame identique squelettes (`vol39_airbnb_homes.png`),
  mêmes rejets dans la console.

**Diagnostic** : la chaîne de préchargement Relay/boot meurt sur
`preloadRestQuery` absent de l'objet — typique d'un **registre global
inter-scripts non propagé** → recoupe **§B-10** (scope `var`/top-level
→ `window`) ou §B-14 (API absente). Erreur **tracée** via
`unhandledrejection` (JS-C154) — ce n'est **pas** une mort silencieuse,
mais le résultat est le bras §B-4 : « boot JS échoue, page figée ».
Hydratation Next.js jamais amorcée.

## 4. vinted.fr — rendu excellent, cards incliquables `[V4-main]`

- Navigate : `doc_h=3873.77`, titre réel. `/console` : **vide**.
- Frame (`captures/vol39_vinted.png`) : header complet (logo, onglets
  Articles, champ recherche, S'inscrire), barre catégories, hero image
  — fidèle. Scroll (`vol39_vinted_scroll.png`) : **grille produit
  réelle** — images WebP, prix « € incl. », compteurs favoris, marques.
  Meilleur rendu de la session (~95 % fidèle, webfont appliquée).
- Cosmétique : une card (coin sup. droit, « mra666 ») a son image
  peinte tronquée en hauteur (~30 px au lieu de ~350) — résidu
  mineur → §B-18 ; header « S'inscrire » très légèrement clippé.
- **WARN daemon** : `journal DOM rejeté → repli re-parse why=UnmappedKey`
  (tab=4) — repli fonctionnel.

### NOUVEAU §B-23 — overlay « stretched-link » à hauteur 0 → cards incliquables

- Clic card produit (viewport 180,250 → doc ~1850) → `{"result":"ignored"}`.
- Layout : `block<a> [63 1666.875 235.2x0]` — **hauteur 0** pour toutes
  les ancres produit (×15 dans le dump).
- DOM : `<a class="NoC3Da_new-item-box__overlay
  NoC3Da_new-item-box__overlay--clickable" href="/items/…">` —
  le pattern classique **stretched-link** : ancre `position:absolute`
  étirée `inset:0` sur toute la card. Son contenu est un `<div>` vide.
- La largeur se résout (235.2 = largeur card) mais **la hauteur reste
  0** → le hit-test n'a aucune surface à tester → `ignored`.
  Pendant ce temps l'image/texte (siblings hors ancre) se peignent —
  d'où une page **visuellement parfaite mais entièrement incliquable**.
- Crate : `explorer-layout` (résolution `inset`/`top`+`bottom` sur
  abs-pos dont le bloc conteneur a une hauteur non définie) ou
  `explorer-css` — à bissecter ; le fix `inset` shorthand (FIX-2)
  couvrait les ancres guardian, pas ce stretch.
- Comportement conforme en revanche : onglets catégories
  `<li role="tab">` sans ancre → `ignored` légitime (éléments JS).

## 5. Télémétrie / observabilité

- `_vol39_daemon.log` : 140 k lignes `INFO sync_profile`, 2 `WARN`
  (rejets journal ADR-0025 ci-dessus), **0 error/panic**.
- `/dom`, `/layout`, `/frame`, `/console` : toujours 200, aucune
  alternance 404 transitoire observée.
- `doc_h` trompeur confirmé sur Airbnb (900 = viewport sur page morte)
  → §B-perf documenté.

## 6. Actions proposées

| # | Proposition | Cible |
|---|---|---|
| §B-23 | Overlay stretched-link `position:absolute`+stretch → hauteur 0, hit-test `ignored` sur toutes les cards (vinted) | `explorer-layout`/`css` — **bloquant UX majeur** : page parfaite incliquable |
| §B-4 | Airbnb : `preloadRestQuery` absent → recoupe §B-10/§B-14 ; boot tracé mais mort | `explorer-jscore` |
| §B-13 | Variante résiduelle : hit-test sur `position:fixed` en scroll (header blablacar) | `explorer-layout`/`core` — mineur |
| §B-18 | + police serif fallback sans warn fetch stylesheet/font (blablacar) | `explorer-css`/`net` |
| ⚪ | + sncf-connect.com et blablacar.fr interne à la liste 403 | externe |

**Statuts** : §B-13 `RÉSOLU partiel confirmé [V4-main]` (ancres
standard naviguent) · §B-4 `OUVERT — recoupe V4` · §B-18 `PARTIEL —
recoupe V4` · §B-23 `OUVERT — NOUVEAU`.

### Fichiers produits

- `captures/vol39_sncf-connect.png` · `vol39_blablacar.png` ·
  `vol39_blablacar_scroll.png` · `vol39_airbnb.png` ·
  `vol39_airbnb_homes.png` · `vol39_vinted.png` · `vol39_vinted_scroll.png`
- `logs/vol39_{sncf-connect,blablacar,airbnb,vinted}_{dom,layout}.txt`
- `_vol39_daemon.log` (racine)

<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 69 : loisirs & quotidien `[V6-main]`

- **Binaire** : `[V6-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (contient le fix §B-26 `1e24942`, borne `emit_scoped_at` —
  signature `cycle de report cassé` observée en vol).
- **Mission** : vol libre sur trois cibles fraîches grand public —
  allocine.fr (cinéma), auchan.fr (e-commerce), seloger.com
  (immobilier). Cibles prudence §B-26/§B-30/§B-41 évitées.
- **Daemon** : PID 2248, port 9451, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol69_daemon.log`. RAM max ~454 Mo — très loin du
  seuil de prudence 1,5 Go.
- **Fenêtre** : 14:05 → 14:14 UTC (~9 min).
- **Verdict global** : **2/3 cibles servies et navigables** —
  allocine excellent (clic → vraie navigation, bfcache actif),
  auchan bien rendu mais **clics avalés par un layer off-canvas
  invisible** (`mini-cart`), seloger bloqué par anti-bot HTTP 403
  (page d'erreur moteur propre). La borne §B-26 a tiré ×2
  (`[0,1,1]` / `[0,1,1,1]`) sans wedge — fix confirmé efficace
  sur cibles nouvelles.

---

## 1. allocine.fr — rendu ~90 %, navigation clic validée

`POST /navigate` → **200 en 4,6 s** (title « AlloCiné : Cinéma,
Séries TV… », `doc_h` 10 572, non clampé). Console **vide**
(« pas de journal » — aucune erreur JS enregistrée).

**Rendu** (`captures/vol69_allocine.png`) : header jaune + logo,
bandeau promo noir « Le Mystère Lucy Lost… », carrousel hero avec
flèches ‹ › et pastilles de pagination, section « À la Une » —
très proche du réel.

**Anomalies mineures** :

- **Rangée nav non peinte** : `block<nav> [53 76 1260x34]` avec
  ancres « News / Cinéma / Séries / Streaming » présentes dans le
  layout (`logs/vol69_allocine_layout.txt` l.20-33) mais invisibles
  à l'écran — probablement texte blanc sur fond jaune du header
  (la nav réelle est sur fond blanc sous le header) → suspect
  `background`/`z-index` non résolu, `explorer-css`/`render`,
  cosmétique (famille §B-18).
- **Affiches en placeholder gris** : les vignettes films
  (sections « Séances », « Dernières news ») restent grises avec
  icône après scroll — lazy `data-src` non résolu → **recoupe
  §B-20** (`explorer-core`, images lazy).

**Interactions** : scroll ×2 → `scroll_y` 800 puis 1 600, frame
repeinte correctement (`vol69_allocine_scroll.png`). **Clic sur
« Tous les films encore à l'affiche » (520,385) → `navigated`
vers `/film/aucinema/`** — vraie navigation, page « Films à
l'affiche » rendue avec breadcrumb, bandeau « navigateur non
supporté » (détection UA — conforme §B-25 feature-detect) et
**affiche Ducobu réellement peinte** (`vol69_allocine_click.png`).
`bfcache_pages=1` — le retour arrière est armé.

Daemon : `cycle de report cassé (§B-26) dropped=1 path=[0,1,1]`
×1 — borne efficace, apply complet jusqu'à `repaint`.

## 2. auchan.fr — rendu ~85 % mais **hit-test avalé par overlay invisible**

`POST /navigate` → **200 en 11,5 s** (title « Auchan. Promos… »,
`doc_h` 2 261 — court vs réel : contenu lazy absent, boot JS
partiel → famille §B-4/§B-14).

**Rendu** (`captures/vol69_auchan.png`) : header complet
(Rayons/Promos/recherche/panier), « Bienvenue :) », carrousel
« 25 Jours Auchan » avec visuel Barilla net, tuiles Catalogues /
Maison & Loisirs, footer parfait (icônes paiement, mentions).
Réserves : texte blanc invisible sur tuile « On a tous du bio »
(image non chargée dessous), bloc SEO replié en rectangle gris
vide après relayout post-clic.

**Console** (`logs/vol69_auchan_console.json`) — 2 signatures
→ §B-14 (matrice APIs DOM) :

```
unhandledrejection: ReferenceError: O is not defined
  (halc.iadvize.com/iadvize.js) ×2
exception js : TypeError: cannot read `removeChild` of Null
  (a_crestfragment2/index.min.js) ×2
```

**Anomalie principale — PROPOSITION §B-43** :
`<mini-cart class="layer layer--sidebar rtl mini-cart">` est un
**tiroir off-canvas fermé** : boîte `[0 0 1366x900]` dans le
layout (panneau interne 600×900 « Chargement du panier… »),
**invisible à l'écran mais prioritaire au hit-test**. Les deux
clics tentés — « Promos » (295,40) et « Politique de
confidentialité » (760,712) — retournent `edited` avec le log
`click livré au DOM — aucune action par défaut` au lieu de
naviguer : le clic atterrit dans l'overlay. Aucune ancre réelle
de la page n'est atteignable par clic. `explorer-core`/hit-test —
variante §B-42 (z-order hit-test vs `position:fixed`) croisée
§B-17 (off-canvas JS-closable : ici le layer n'est même pas
peint, son transform/état fermé n'est pas pris en compte par le
hit-test). Preuve : `logs/vol69_auchan_layout.txt` l.3-16,
`logs/vol69_auchan_dom.txt` l.1013+.

Daemon : `journal DOM rejeté → re-parse (ADR-0025) tab=2
why=Fingerprint` (journal_fallbacks=1 — repli propre) +
`cycle de report cassé (§B-26) dropped=1 path=[0,1,1,1]` ×1.
Un bandeau sticky « Grand Jeu Crazy Chariot » est apparu
post-clic (JS résiduel vivant — timers actifs).

## 3. seloger.com — HTTP 403 anti-bot, dégradation propre

`POST /navigate` → **réponse erreur gérée en 1,4 s**
(`{"error":{"handled":true,"message":"HTTP 403"}}` — protection
DataDome vraisemblable). La **page d'erreur moteur** est rendue
correctement : « Page inaccessible — Le serveur a répondu
« HTTP 403 » » (`captures/vol69_seloger.png`). `/dom` = 0 nœud,
18 boîtes layout, console = `HTTP 403`. **Pas un bug moteur** —
comportement nominal face à un blocage bot. Cible à écarter des
vols futurs ou à doter d'une cible de secours immobilier
(leboncoin déjà vu, pap.fr ?).

## 4. Tableau de synthèse

| Cible | Navigate | Rendu | Scroll | Clic | Anomalies |
|---|---|---|---|---|---|
| allocine.fr | 200 / 4,6 s | ~90 % | OK ×2 | **navigué** → `/film/aucinema/` | nav non peinte (cosm.), vignettes lazy → §B-20 |
| auchan.fr | 200 / 11,5 s | ~85 % | OK ×2 | **intercepté ×2** (`edited`) | **§B-43 proposé** + §B-14 ×2, §B-4 lazy |
| seloger.com | HTTP 403 géré | page erreur propre | — | — | anti-bot, hors moteur |

## 5. Observations annexes

- La borne §B-26 (`emit_scoped_at`) a tiré **×2 sur des cibles
  nouvelles** (`[0,1,1]` allocine, `[0,1,1,1]` auchan) sans
  conséquence : cycles apply complets jusqu'à `repaint`, RAM
  stable — le fix tient en vol libre, pas seulement sur les
  porteurs historiques.
- `journal_fallbacks` : 1 (auchan, `Fingerprint` → re-parse
  ADR-0025 propre), 1 (seloger). Aucun panic — §B-37 non
  reproduit.
- RAM daemon : 454 Mo max, 448 Mo post-DELETE — retour baseline
  quasi complet, pas de fuite §B-30 sur ces cibles.
- seloger rejoint la liste des cibles anti-bot (avec leclerc &
  co) — suggéré : maintenir un sous-ensemble « e-commerce passant
  anti-bot » dans le protocole.

## 6. Arrêt

DELETE tabs 1/2/3 → `{"tabs":[]}`. Daemon terminé par **PID
(2248)** — port 9451 libéré, vérifié `netstat` (le daemon
parallèle PID 18704 :9450, autre vol, non touché).

**Fichiers** : `captures/vol69_{allocine,allocine_scroll,allocine_click,auchan,auchan_scroll,auchan_click,seloger}.png`,
`logs/vol69_{allocine,auchan}_{dom.txt,layout.txt,mem.json,console.json}`,
`logs/vol69_seloger_{dom.txt,console.json}` (403 : layout/mem
vides non archivés), journal `_vol69_daemon.log`.

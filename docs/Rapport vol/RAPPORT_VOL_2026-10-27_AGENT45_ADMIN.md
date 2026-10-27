<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 45 — Sites administratifs (2026-10-27)

- **Binaire** : `[V4-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (WIP `depth_limit` embarqué), `--port 9427`, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol45_daemon.log` (racine)
- **Cibles** : impots.gouv.fr (DSFR — test JS-C150 `super()`) · ratp.fr · urssaf.fr
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de `SYNTHESE.md`

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| impots.gouv.fr | **Rendu quasi-parfait + DSFR JS initialisé** | **vide** | §B-10 bras DSFR → **RÉSOLU ici** ; cosmétiques §B-18 |
| ratp.fr | **Rendu très fidèle, nav réelle** | 1 erreur `contains` | §B-14 (`Node.contains` sur document) + §B-17 (off-canvas fermé déployé, intercepte le hit-test) |
| urssaf.fr | **RST connexion ×2 — externe** | erreur fetch tracée | ⚪ externe (WAF — reproduit par curl natif) |

Acquis : 2/3 sites servis avec un contenu réel quasi complet ; **zéro panic,
zéro freeze, API responsive en continu, RAM daemon stable (~1,0 Go, trois
onglets dont une page de 11 210 px)** ; 2 vraies navigations par clic + 1
soumission de formulaire réelle + 2 retours bfcache fonctionnels.

**Fait marquant** : sur impots.gouv.fr le moteur **DSFR s'initialise** —
37 attributs `data-fr-js-*` injectés dans le DOM sérialisé
(`data-fr-js-modal`, `-modal-button`, `-navigation-item`, `-ratio`,
`-display`, `-header-links`) et `data-fr-opened="false"` posé par le JS —
avec une **console entièrement vide**. C'est la preuve terrain que
JS-C150 (`super()` en ctor dérivé, `[[HomeObject]]`) débloque le bras
DSFR de §B-10 : l'erreur historique `value is not a function` DSFR a
disparu.

---

## 1. impots.gouv.fr — DSFR fonctionnel `[V4-main]` → §B-10 (bras DSFR)

- `POST navigate` → `doc_h=11210.87`, titre réel
  « Accueil impots.gouv.fr », URL finale `/accueil`.
- `/console` verbatim : `pas de journal` — **0 erreur** sur 8 `<script>`.
- `/dom` 509 Ko (3353 nœuds), `/layout` 109 Ko (1488 boîtes).
- Frame (`captures/vol45_impots.png`) : marianne RF, wordmark,
  liens connexion particulier/pro, barre de recherche, nav principale
  (Accueil → International/International (EN)), hero « aide de 100 € »,
  « Votre avis sur le site » — ~95 % fidèle, webfonts appliquées.

### DSFR JS effectivement exécuté (preuve §B-10)

DOM sérialisé (télémétrie post-boot) :

```
data-fr-js-display        ×1     data-fr-js-modal-body      ×1
data-fr-js-header-links   ×1     data-fr-js-modal-button    ×6
data-fr-js-header-modal   ×2     data-fr-js-navigation-item ×9
data-fr-js-modal          ×3     data-fr-js-ratio           ×14
data-fr-opened="false"    ×3
```

Ces attributs sont **injectés par le JS DSFR à l'initialisation** des
composants — leur présence dans le dump prouve que le bundle DSFR a
tourné sans exception (avant JS-C150 il mourait au `super()` du
composant — §B-10, FIX-1). Les composants sont instrumentés **et**
refermés (`data-fr-opened="false"` ×3, modales non peintes — conforme,
rien de coincé ouvert → **pas de repro §B-17 sur cette cible**).

### Input

- `scroll` ×2 → `scroll_y=1600` réel, contenu repeint
  (`vol45_impots_scroll.png`) — cards actualités complètes.
- Clic nav « Particulier » (270,155) → **`navigated`** : les entrées
  de la nav sont de vraies ancres (`fr-nav__link`, pas de
  `fr-nav__btn` — ce site n'a **pas** de méga-menu dépliable dans la
  barre principale ; le test « dépliage » ne s'applique donc pas ici).
  Page Particulier peinte avec fil d'Ariane (`vol45_impots_particulier.png`).
- `POST /back` → retour `/accueil` instantané (bfcache : 1 page,
  2096 nœuds retenus dans `/mem`).
- `mousemove` sur « Particulier » → `{"result":"hover",
  "url":"…/particulier"}` — survol résolu, pas de dépliage (attendu :
  hover n'ouvre rien sur ce site).
- Clic bouton recherche (1349,92 — `block<button> [1330 72 38x40]`)
  → **`navigated` vers `/recherche?…=*`** : soumission de formulaire
  réelle ; page « Résultats de recherche — 167 résultats » peinte avec
  liste d'articles, badges, pagination
  (`vol45_impots_modal.png`, 15,5 Mo — document complet).

### Cosmétique résiduel → §B-18

- Texte du bouton recherche `fr-btn--search` peint **clippé**
  (« eche » visible — texte 88 px dans une boîte 38 px : l'icône
  seule devrait être visible, le texte est masqué par overflow plutôt
  que non peint — rendu mineur).
- Au scroll, le badge « Bulletin "DGFIP" / Communication générale »
  **chevauche le titre** des cards actualités → famille §B-18
  (chevauchements de runs inline).

### WARN daemon

`journal DOM rejeté → repli re-parse (ADR-0025) why=Fingerprint` ×2,
`UnmappedKey` ×1 (tab=1) — repli fonctionnel, rendu produit.

## 2. ratp.fr — fidèle, 1 erreur JS + overlay off-canvas `[V4-main]`

- `POST navigate` → `doc_h=6012.31`, titre réel (nav >10 s — site
  lent, pas de timeout).
- `/console` verbatim :

```
exception js : TypeError: `contains` is not a function (on HTMLDocument)
    at <anonymous> (https://www.ratp.fr/cdn-cgi/scripts/7d0fa10a/cloudflare-static/rocket-loader.min.js:1:11572)
    at <anonymous> (…/rocket-loader.min.js:1:10939)
    at <anonymous> (…/rocket-loader.min.js:1:10872)
    at <anonymous> (…/rocket-loader.min.js:1:2894)
    at <anonymous> (…/rocket-loader.min.js:1:8070)
```

→ `Node.prototype.contains` absent sur `HTMLDocument` (Cloudflare
rocket-loader y appelle `document.contains`) — **§B-14**, à ajouter à
la matrice APIs DOM (`explorer-jscore`). Une seule entrée console :
le reste du boot n'émet aucune erreur.

### Rendu (`captures/vol45_ratp.png` + `_scroll.png`)

- Header complet : logo RATP, « Rejoignez-nous », recherche,
  burger ☰ ; nav « Itinéraires Horaires Plans Trafic Tarifs » ;
  hero photo METRO ; formulaire « Où allons-nous ? » (De/À,
  Préférences, date/heure) ; « Information trafic à 09:37 » avec
  pastilles lignes RER/M — **rendu très fidèle**.
- Défauts : items de nav à moitié clippés par le hero (bordure
  header/hero, `overflow` ou z-index) ; un rectangle vide peint
  ~[1046..1366, 97..137] (= le sélecteur « Français » du menu
  off-canvas — voir §B-17 ci-dessous) ; carrousel au scroll =
  grandes cases grises quasi vides (images absentes — lazy/JS).
- `scroll` ×2 → `scroll_y=1600`, repeint sain.

### §B-17 confirmé — menu off-canvas fermé déployé dans le layout

Layout : `block<nav> [0 0 1366x0]` (le `<nav>` latéral a une hauteur
propre 0 — fermé) mais **ses enfants sont déployés**
`[1046..1366 × 0..1457]` : bouton `[1055 4 63x58]`, contenu
`[1046 0 320x1457]`, sélecteur langue `[1062 97 310x40]`, liste nav
`[1046 162 320x1173]`. Le panneau devrait être repoussé hors écran
par `transform: translateX(100%)` (ou équivalent JS) ; il est peint
(fond blanc — invisible sur fond blanc, mais le sélecteur « Français »
et sa bordure percent au-dessus du hero → le « rectangle vide » de la
frame).

Conséquence mesurée : **le clic sur le burger** (1298,131, centre
exact de `block<button> [1271 103 55x57]`) → `ignored` — le hit-test
conscient de l'ordre d'empilement teste d'abord la couche off-canvas
superposée et n'atteint pas le bouton. Sous-cas concret de §B-17 +
variante §B-13 (« hit-test sous overlay invisible »).

Contre-preuve : clic « Plans » (755,123 — `inline<a>
[734.7 107.5 40.7x31.7]`) → **`navigated`**, page Plans fidèle
(`vol45_ratp_plans.png` : onglets modes de transport, puces lignes
1/2/3bis/4/5, lien PDF, fil d'Ariane). Hit-test sain hors zone
recouverte → §B-13 reste « probablement résolu » sur ancres
standard.

## 3. urssaf.fr — RST connexion ⚪ externe

- `POST navigate` ×2 → `{"error":{"handled":true,"message":"fetch :
  I/O : Une connexion existante a dû être fermée par l'hôte distant.
  (os error 10054)"}}`.
- `/console` : l'erreur fetch est **tracée** dans le ring (même
  message). `/dom` → `pas de DOM` ; `/layout` → `pas de layout`.
- Frame (`vol45_urssaf.png`) : page d'erreur moteur « Page
  inaccessible — Impossible de joindre le serveur… » — propre.
- **Contre-preuve externe** : `curl https://www.urssaf.fr` (schannel
  natif Windows) échoue pareil — TLS OK puis `Recv failure:
  Connection was reset` au premier `GET`. Le serveur (WAF) coupe la
  connexion hors navigateur réel — signature **anti-bot externe** à
  ajouter au catalogue ⚪ (aux côtés des 403 sec) : urssaf.fr.

## 4. Télémétrie / observabilité

- `/mem` : tab1 impots — `dom_nodes=3353`, `frame_bytes=61,3 Mo`
  (doc complet 11 210 px → raster pleine page, §B-perf « framebuffer
  pleine page » illustré), `js_heap=10,2 Mo`, `images=13,6 Mo`,
  bfcache 1 page. Tab2 ratp — `dom_nodes=1913`, `frame=14,1 Mo`,
  `js_heap=2,0 Mo`. Tab3 — vide (0 nœud).
- RAM daemon (PID 16604, `tasklist`) : 713 Mo après impots → 1 032 Mo
  après ratp → **997 Mo en fin de vol** — stable, aucune dérive
  post-crash observée.
- `_vol45_daemon.log` : 3 WARN (rejets journal ADR-0025 tab=1,
  repli fonctionnel), **0 ERROR, 0 panic**.
- `/dom`, `/layout`, `/frame`, `/console`, `/mem` : 200 constants,
  aucune alternance 404 transitoire.
- Frame « pleine page » : `/frame` de la page résultats impots =
  PNG 15,5 Mo (document entier, cf. §B-perf P4 tuilage à planifier).

## 5. Actions proposées

| # | Proposition | Cible |
|---|---|---|
| §B-10 | **Bras DSFR RÉSOLU `[V4-main]`** — DSFR initialise (37 `data-fr-js-*` injectés, modales instrumentées et refermées, console vide) sur impots.gouv.fr ; re-vols de clôture suggérés : legifrance, service-public | `explorer-jscore` — JS-C150 validé terrain |
| §B-14 | + `Node.prototype.contains` sur `HTMLDocument` (ratp — rocket-loader Cloudflare : `document.contains`) | `explorer-jscore` — matrice J5 |
| §B-17 | **Confirmé V4** : off-canvas ratp fermé (`nav` hauteur 0, `translate`) avec enfants déployés `[1046..1366 × 0..1457]` — peint (sélecteur langue visible) **et intercepte le hit-test** du burger → `ignored` | `explorer-layout`/`css` (transform non appliqué à l'off-canvas) + `core` (hit-test) |
| §B-18 | + badge card chevauchant le titre (impots scroll) ; bouton `fr-btn--search` texte clippé « eche » ; nav ratp clippée sous le hero | `explorer-layout`/`css` |
| ⚪ | + urssaf.fr à la liste anti-bot (RST connexion, reproduit curl natif) | externe |
| §B-13 | Variante résiduelle documentée : hit-test sous overlay off-canvas (burger ratp) — ancres standard OK (« Plans » → nav réelle) | `explorer-core`/`layout` |

**Statuts proposés** : §B-10 bras DSFR `RÉSOLU [V4-main]` (sur
impots.gouv.fr — à confirmer sur legifrance/service-public) ·
§B-17 `OUVERT — confirmé V4, recoupe hit-test` · §B-14 `PARTIEL —
+1 entrée matrice` · §B-18 `PARTIEL — recoupe V4` · ⚪ catalogue
enrichi.

### Fichiers produits

- `captures/vol45_impots.png` · `vol45_impots_scroll.png` ·
  `vol45_impots_particulier.png` · `vol45_impots_hover.png` ·
  `vol45_impots_modal.png` · `vol45_ratp.png` ·
  `vol45_ratp_scroll.png` · `vol45_ratp_menu.png` ·
  `vol45_ratp_plans.png` · `vol45_urssaf.png`
- `logs/vol45_impots_{dom,layout}.txt` ·
  `logs/vol45_ratp_{dom,layout}.txt` ·
  `logs/vol45_urssaf_{dom,layout}.txt`
- `_vol45_daemon.log` (racine)

# Rapport de vol — Agent 20 : sites finance / data

Date : 2026-10-26 — Daemon `explorer-daemon` port 9404, viewport 1366×900.
Protocole : `docs/vol-agent.md`. Aucun code modifié.

Périmètre : pages financières denses — tables de cotations, sparklines,
charts canvas, applications JS lourdes (Next.js, webpack), consent/CMP,
challenge anti-bot (AWS WAF).

## Synthèse

| Cible | Verdict | doc_h | DOM | JS fatal |
|---|---|---|---|---|
| coinmarketcap.com | Rendu exploitable | 5 497 px | 1,29 Mo | oui (non bloquant pour le SSR) |
| tradingview.com/symbols/BTCUSD | Mort (app JS) | 900 px (= viewport) | 859 ko | oui — `createContextualFragment` |
| boursorama.com | Inutilisable | 52 438 px (clampé) | 1,64 Mo | oui ×2 |
| finance.yahoo.com | Partiel — contenu réel, chrome cassé | 21 301 px (clampé) | 1,80 Mo | oui (CMP) |
| investing.com | Rendu exploitable | 15 419 px | 1,81 Mo | oui (non bloquant) |

Toutes les cibles ont livré un **squelette DOM conséquent** (≥ 850 ko) —
le pipeline fetch→parse→layout tient sur les 5 sites. Les trous sont en
amont JS (API DOM manquantes) et en aval rendu (commentaires peints,
menus accordéon non repliés, canvas absent).

---

## 1. coinmarketcap.com — rendu exploitable

URL : `https://coinmarketcap.com/` → titre OK, doc 5 497 px.

Capture : `captures/vol20_coinmarketcap.png` (+ `_scroll`, `_click`).

**Constats mesurés**

- La table des cotations est **réellement peinte** : colonnes #, Name,
  Price, 1 h %, 24 h %, 7 d %, Market Cap, Volume(24 h), Circulating
  Supply lisibles avec valeurs réelles (BTC $68,557.76, ETH $2,321.79…).
- Dégradation nette passé ~15 lignes : les rangées 16+ s'effondrent en
  lignes fines « nom + prix » sans colonnes — la structure tabulaire du
  bas de table n'est pas reproduite (virtualisation Next.js absente ou
  cellules non instanciées).
- Sparklines (colonne « Last 7 Days ») **absentes** — cellules vides
  (canvas non rendu).
- Barre supérieure « Top Trending Watchlist … New » **chevauchante** :
  items superposés illisibles.
- Panneaux « Trending » (GM, FXS, M, wCIFM…) : boîtes présentes mais
  texte superposé à l'intérieur.
- Clic à (90, 25) sur la table défilée → `{"result":"ignored"}`
  (hit-test n'a pas trouvé de cible activable là où un lien est peint).
- Scroll : `scroll_y` 0 → 800 → 1 600, frames distinctes, OK.

**Erreurs verbatim (console)**

```
exception js : TypeError: cannot read `open` of Undefined
    at <anonymous> (7:14) — src «(function() { if (false) return; window.__renderTime__ = {"pageRender":1793039726232}; window.__…»
exception js : TypeError: messagefunction _0x42e97f() { [code] } is not a constructor or null
    at <anonymous> (https://3f0fb9bcf568.edge.sdk.awswaf.com/.../challenge.js:1:519927)
    … (AWS WAF challenge)
syntaxe js 2:33333 : expected `)` — https://s2.coinmarketcap.com/v1/cmc/_next/static/chunks/22tt88klqjx-p.js
```

**Classification** : succès partiel — SSR exploitable, hydratation
cassée (WAF challenge + chunk Next.js en erreur de syntaxe + globales
manquantes). **Sévérité : moyenne** (page lisible, pas interactive).

## 2. tradingview.com/symbols/BTCUSD — cible morte

URL : `https://www.tradingview.com/symbols/BTCUSD/` → titre OK.

Capture : `captures/vol20_tradingview.png`.

**Constats mesurés**

- doc_h = **900 px = hauteur viewport** : la page fait la taille de
  l'écran, scroll impossible (`scroll_y` reste 0.0 après input dy:800).
- Le chart (produit principal) est **totalement absent** ; l'app React/
  webpack meurt au bootstrap.
- Littéraux **`</comment>` et `<#comment>` peints comme texte visible**
  en tête et dans tout le flux (≈20 occurrences à l'écran).
- Chrome réduite à une liste de liens non stylés (Products, Community,
  Markets…) ; logo BTC agrandi en bitmap géant superposé ; onglets
  « Overview News Community Technicals … » sur une seule ligne
  collée. Seul le bandeau de performance (24 hours 5.07 %, 1 week
  −2.07 %, … All time 736.64 K%) s'affiche en barre tabulée correcte.
- DOM squelette présent : 859 ko.

**Erreur fatale verbatim (première)**

```
exception js : TypeError: `createContextualFragment` is not a function (on Range)
    at s (https://static.tradingview.com/static/bundles/36115.d4b63a110c8186327c2f.js:5:296)
    at i (…/36115.….js:5:518)
    … cascade webpack — tue toute la chaîne de bundles
```

**Classification** : échec — fatal JS à l'amorçage + bug de
tokenisation (commentaires rendus). **Sévérité : critique**.

## 3. boursorama.com — inutilisable

URL : `https://www.boursorama.com/` → titre OK, doc_h **52 438 px
(clampée, `doc_h_clamped: true`)**.

Captures : `vol20_boursorama.png`, `vol20_boursorama_scroll.png`
(scroll_y = 6 000).

**Constats mesurés**

- Le viewport peint ne contient que : « Rechercher », deux fois
  « Sur quel site souhaitez-vous vous connecter ? » et des **triangles
  SVG gris géants** (logo déployé hors de sa boîte, ~600 px).
- À scroll_y 6 000 : un **menu accordéon latéral étalé à plat** —
  entrées dupliquées (« OPCVM » ×2, « Palmarès » ×2, « Actualités -
  Pédagogie » ×2, « Partenaires »…) chacune sur ~250 px de haut. Le
  menu de navigation complet semble occuper l'essentiel des 52 438 px.
- Contenu éditorial (actus, cours CAC 40) **jamais atteint** en 6 000 px
  de défilement.
- 3 premiers scrolls → `{"result":"ignored"}` (drain en cours), le 4ᵉ
  → `scrolled 6000.0` — le backpressure existe, observable côté API.

**Erreurs verbatim**

```
exception js : TypeError: cannot read `triggerListElements` of Undefined
    at A (https://www.boursorama.com/build/critical-module7172a15b6e6d86d91b3a.js:1:21334)
    at O (…:1:19918) at y (…:1:19887) at <anonymous> (…:1:7127)
exception js : TypeError: cannot read `USER_HASH` of Undefined
    at <anonymous> (https://www.boursorama.com/build/footac912f2e7f85bdff6503.js:259:2657)
    at <anonymous> (…/469214455cf0018a1959c11d.js:10:39099)
```

**Classification** : échec — CSS de masquage des menus non appliqué
(ou `hidden`/media-query mal résolu) → page noyée sous son propre
menu ; JS critique mort en plus. **Sévérité : critique**.

## 4. finance.yahoo.com — partiel

URL : `https://finance.yahoo.com/` → titre OK, doc_h **21 301 px
(clampée)**. Premier `GET /frame` a rendu **0 octet** (transitoire,
retry OK — même phénomène que noté dans vol-agent.md).

Captures : `vol20_yahoo.png`, `vol20_yahoo_scroll.png` (scroll_y 4 500).

**Constats mesurés**

- Haut de page **submergé de littéraux `</comment>`/`<#comment>`**
  (≈200 occurrences, 8 lignes pleines) — même bug que TradingView.
- En-tête/nav : méli-mélo de liens superposés illisibles (« portfolio
  finance football basketball … », barre recherche doublée, « Sign
  in » géant).
- **Données de marché réelles peintes** : « S&P 500 7,678.58
  +54.90 +0.72 % », « Dow 30 52,349.26 +134.10 +0.26 % », sparkline
  SVG visible sous chaque indice, « U.S. markets close in 1h 18m ».
- À scroll_y 4 500 : flux d'actualités **lisible et correctement
  empilé** (photo pleine largeur, « Foldable fears come true for some
  iPhone Duo buyers — Bloomberg · 1h ago », badge LIVE, tickers avec
  variations). Colonnes/grille absentes mais contenu consommable.
- DOM squelette : 1,80 Mo.

**Erreur verbatim**

```
exception js : TypeError: cannot define property `TcfCaV1DisclosedVendorsSegment`
    at <anonymous> (inline:7:2:233482) … (bundle CMP/consent inline)
```

**Classification** : partiel — chrome cassée + bug commentaires, mais
le contenu data est là. **Sévérité : haute** (utilisable en défilant,
premier écran catastrophique).

## 5. investing.com — rendu exploitable (fallback)

URL : `https://www.investing.com/` → titre OK, doc 15 419 px non
clampée. Une seule erreur JS :

```
exception js : TypeError: cannot read `appendChild` of Undefined
    at c (1:13)
```

Capture : `vol20_investing.png` (page entière). Tables d'indices avec
valeurs réelles, images de charts, « World Indices », « Commodities »,
« Trending Stocks », news et footer s'empilent lisiblement ; les
sections du haut sont compressées/chevauchées mais l'ensemble tient.
**Sévérité : moyenne**.

---

## Constats actionnables

1. **Nœuds commentaires peints comme texte** — `</comment>` /
   `<#comment>` s'affichent en flux sur TradingView et Yahoo (sites à
   templates `<!-- -->` / gabarits commentés). Probable fuite du
   tree-builder HTML ou sérialisation de nœuds comment en texte.
   Impact massif et très visible. *Sévérité critique, effort faible —
   à bissecter dans `explorer-html`.*
2. **`Range.createContextualFragment` absent** — fatal bootstrap sur
   TradingView (toute la chaîne webpack meurt). API DOM simple à
   implémenter dans `explorer-jscore`/binding DOM. *Critique.*
3. **Charts canvas absents** — sparklines CMC (cellules vides) et
   chart TradingView inexistants : l'écosystème finance repose sur
   `<canvas>` 2D. Sans lui, la valeur métier de ces pages s'effondre.
   *Critique pour le segment, chantier large.*
4. **`syntaxe js : expected ')'`** sur un chunk Next.js de CMC
   (22tt88klqjx-p.js:2:33333) — le parseur `explorer-jscore` bute sur
   une syntaxe moderne à identifier (bissecter l'octet fautif).
   *Haute.*
5. **Cascade `cannot read X of Undefined`** — `open` (probablement
   `window.open` ou `document.open` sur global manquant),
   `triggerListElements`, `USER_HASH`, `appendChild` of Undefined :
   objets globaux hôtes incomplets (window/document propriétés,
   timing DOMContentLoaded). *Haute — pattern transverse à 4 sites.*
6. **Menus accordéon/off-canvas étalés** — Boursorama : nav latérale
   non masquée → doc 52 438 px et contenu hors d'atteinte. Suspects :
   `hidden`, `display:none` via classe, media queries ou `visibility`
   non résolus. Reproduit partiellement le haut de page Yahoo.
   *Haute.*
7. **`doc_h_clamped` fréquent** (Yahoo 21 301, Boursorama 52 438) —
   le plafond de hauteur document est atteint sur pages réelles ;
   vérifier que le clamp ne tronque pas la peinture du contenu utile
   (Yahoo : le bas de page était-il accessible ?). *Moyenne.*
8. **Hit-test lacunaire** — clic sur un lien peint de la table CMC →
   `{"result":"ignored"}`. *Moyenne.*
9. **`GET /frame` à 0 octet transitoire** confirmé (Yahoo, 1ʳᵉ
   tentative ; retry OK après ~4 s). Non bloquant mais à tracer.
   *Faible.*
10. **Colonnes de table perdues sous ~15 rangées** (CMC) — la
    virtualisation/listes `position:sticky` ou lignes injectées JS ne
    survivent pas ; la table réelle du bas est remplacée par des
    lignes nom+prix. *Moyenne.*

## Annexes

- Captures : `docs/Rapport vol/captures/vol20_*.png` (8 fichiers).
- Logs : `docs/Rapport vol/logs/vol20_*_{console.json,dom.txt,layout.txt}`.
- Daemon : PID 9236, démarré 19:37, arrêté proprement 19:47, onglets
  fermés, `_vol20_daemon.log` resté vide (stdout bufferisé).

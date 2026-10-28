<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 93 : REVALID3 `[V9-main]`

- **Binaire** : `[V9-main]` —
  `C:/OnionBit-Explorer/target/release/explorer-daemon.exe`
  (horodaté 28/10 19:40, HEAD `8260b6fa` — post-Étape 148,
  post R-127).
- **Mission** : REVALID3 — re-vol de validation de correctifs
  posés mais jamais confirmés terrain : §B-3 (stack overflow
  alpinejs), §B-4(d) (data.gouv prefetch modulepreload),
  §B-26 (bfmtv dent-de-scie), §B-30/§B-41 (flixbus
  commit.drains + fuite DELETE), §B-33 (francebleu nav_paint
  borné), §B-37 (caniuse panic event.dispatch).
- **Daemon** : PID **18204**, port 9512,
  `EXPLORER_VIEWPORT=1366x900`, journal `_vol93_daemon.log`
  (~76 000 lignes) → `logs/daemons/`.
- **RAM** : base ~356 Mo → pics 721 (alpinejs) / 610
  (data.gouv) / 864→1206 (bfmtv, palier unique au dump
  layout) / **1335→1856 Mo en dérive continue** (flixbus,
  ~0,6 Mo/s pendant le wedge, jamais stabilisé).
- **Verdict global** : **3 correctifs confirmés RÉSOLUS
  `[V9-main]`** (§B-3, §B-4(d), §B-26) — alpinejs navigable
  de bout en bout, data.gouv entièrement montée, bfmtv
  stable avec `apply: fin` et disjoncteur B-41 armé. **Vol
  interrompu par un wedge nouveau sur flixbus** : `navigate`
  jamais retourné (>13 min), boucle événementielle en
  phases `event.*` non bornées (poll_net ≤62 s,
  dispatch ~5,5 s, mirror_sync ~11 s) → verrou session
  global → **API entière morte >6 min** ; §B-33 et §B-37
  non re-testables.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| alpinejs.dev | **§B-3 NON REPRODUIT → RÉSOLU** — rendu complet, 36/36 évals `ok=true`, clic → vraie nav | 2 `uncaught ReferenceError` **nommées** (`currentStep`, `number`) — dégradation typée | icônes tofu mineures ; re-parse `Fingerprint` ×2 absorbé |
| www.data.gouv.fr | **§B-4(d) RÉSOLU** — homepage SSR+Nuxt montée et stylée, module `DL4muOy-.js` fetché+évalué (exception nommée) | `TypeError: cannot read get of Undefined — module «_nuxt/DL4muOy-.js»` | page interne `/datasets` naviguée mais **non stylée** (colonne brute) |
| www.bfmtv.com | **§B-26 RÉSOLU** — `apply: fin ms=1440`, RAM stable ~864 Mo (vs 6,88 Go), disjoncteur B-41 armé | 2 `TypeError` nommées (`toLowerCase` — sliders) + 4 évals `ok=false` | `data:image/svg+xml` brut peint en texte ; mégamenu « Toutes les rubriques » peint brut au scroll alors que layout x=-344 |
| www.flixbus.fr | **TOUJOURS PRÉSENT (variante) → proposition §B-55** — `navigate` jamais retourné >13 min, API morte | inaccessible (API morte) — 21 évals, 2 `ok=false` (bundle 209 Ko) | `commit.drains` 53 s ×1 (famille §B-41 vivante) + `pump.offlock` 10,9 s + cycles `event.*` 30-80 s non bornés |
| www.francebleu.fr | **NON COUVERTE** — daemon livelocké par flixbus | — | §B-33 non re-testé |
| caniuse.com | **NON COUVERTE** — idem | — | §B-37 non re-testé |

Points marquants :

- **§B-3 fermé** : le tueur `loose_eq`×Symbol ne tue plus —
  le process survit à Alpine.js *entièrement fonctionnel* :
  36 évals, hydratation complète (directives x-data/x-for
  évaluées), clic « Get started » → navigation réelle vers
  la doc. Les seules erreurs sont des `ReferenceError`
  nommées dans `/console` (contextes Alpine résolus à
  `undefined` — dégradation attendue, jamais un crash).
- **Étape 148 validée terrain** : data.gouv.fr, « meilleur
  vol perdu » de `[V7]` vol 83 (0 fetch/0 éval/0 trace),
  s'affiche désormais **complète** — header République
  Française, nav, hero, cartes jeux de données avec logos
  ministériels, stats réelles (76K/382K/7K). Le chunk Nuxt
  `DL4muOy-.js` est fetché, évalué, et meurt sur une
  `TypeError` **nommée à `/console`** — exactement le
  critère demandé.
- **§B-26 fermé** : bfmtv, ex-tueur d'API (6,88 Go en
  5 min), tient ~4 min de vie JS continue avec RAM plate
  ~864 Mo, `apply: fin tab=4 ms=1440` journalisé, repaints
  ~700 ms, et le **disjoncteur B-41 a tiré**
  (`tempête de pompes — disjoncteur armé (B-41) streak=64`)
  — c'est le fix qui travaille, visible dans le log.
- **flixbus = le wedge a migré** : `commit.drains` est
  presque éteint (µs partout sur bfmtv, **1×53 s** sur
  flixbus) mais la boucle événementielle prend le relais
  non borné : chaque événement = `poll_net` (15-62 s) +
  `dispatch` (~5,5 s constants) + `mirror_sync` (~11 s) →
  ~30-80 s par cycle, ~18 cycles en 13 min → `navigate`
  n'atteint jamais l'idle → verrou global → tout l'API
  morte. Signature distincte → **proposition §B-55**.

---

## 1. alpinejs.dev — §B-3 non reproduit, site entièrement fonctionnel

`POST /navigate` → **1,4 s**, `doc_h=4 677` non clampé,
titre « Alpine.js ». **Le process survit** — c'était le
critère : §B-3 était le seul crash process connu
(stack overflow `loose_eq`×Symbol, fix JS-C167 `adf9273`).

**`/console` verbatim** — 2 `uncaught` **nommées**
(dégradation typée, pas de crash) :
`ReferenceError: currentStep is not defined` et
`ReferenceError: number is not defined`, stacks pointant
dans `cdn.jsdelivr.net/npm/alpinejs@3.x.x/dist/cdn.min.js`
(expressions `x-for`/`x-show` évaluées hors contexte).

**Journal daemon** : 36 `script: eval fin tab=2`
**tous `ok=true`** (ms=0-255), **zéro** occurrence
`overflow|panic|STATUS_`. Re-parse ADR-0025 `Fingerprint`
×2 absorbés proprement.

**Rendu** (`captures/vol93_alpinejs.png`) : page d'accueil
complète — logo Alpine.js, tagline, « Get started → »,
bloc code `<script src="//unpkg.com/alpinejs">`, barre de
recherche, nav Docs/Components + icône GitHub.

**Scroll → `scroll_y=800`** (`vol93_alpinejs_scroll.png`) :
« Simple. Lightweight. Powerful as hell. », sections
x-data/directives — tout peint.

**Clic** « Get started » à `scroll_y=0` (692,423) →
**`navigated`** → page « Start Here » de la doc
(`vol93_alpinejs_click.png`) : sidebar complète des
directives (x-data…x-id, Magics, $el), TOC « On this
page », « Building a counter », blocs code colorés,
encart sponsor — **Alpine.js lui-même tourne dans
Explorer**.

**Télémétrie** (`logs/vol93_alpinejs_mem2.json`) :
`dom_nodes=2 517`, `layout_boxes=1 975`,
`display_items=2 439`, `js_heap=31,0 Mo`,
`journal_fallbacks=2`, bfcache 1 page. Pic 721 Mo →
**purge 402 Mo** au DELETE, `{"tabs":[]}`.

## 2. www.data.gouv.fr — Étape 148 validée : module préfetché, page montée

`POST /navigate` → **8,0 s**, `doc_h=5 675`, titre réel
« data.gouv.fr : Plateforme ouverte des données publiques
françaises ». Contre vol 83 `[V7]` : **0 fetch/0 éval/0
trace** — ici la page s'affiche.

**`/console` verbatim** — exception **nommée** :
`TypeError: cannot read \`get\` of Undefined — module
«https://www.data.gouv.fr/_nuxt/DL4muOy-.js»`. La
nomination du module `DL4muOy-.js` prouve le fetch +
l'évaluation du chunk Nuxt (frontier `modulepreload`
ensemencée). Côté `script: eval` classique : 4 `eval fin`
`ok=true` (2 inline 202 o + 6 367 o, rejoués à la
seconde page). La pompe tourne en continu (phases
`commit.*` µs — hydratation partielle vivante).

**Rendu** (`captures/vol93_datagouv.png`) : homepage
**entièrement stylée** — bandeau République Française,
logo datagouv, recherche + bouton bleu, nav
Données/API/Réutilisations/Organisations/Nouveautés/Nous
écrire, « Se connecter / S'enregistrer », badges « En ce
moment » / « Actualités », hero « La plateforme des
données publiques françaises » + 2 CTA.

**Scroll → `scroll_y=800`** (`vol93_datagouv_scroll.png`)
: « Le catalogue des données françaises » — stats réelles
**76K jeux de données / 382K fichiers / 7K
organisations**, logos Santé Publique France/CNIL/Insee/
ministères, cartes datasets (Base Sirene, Répertoire
national des élus, COVID-19, Prix des carburants, Agence
Bio, Parcoursup) — données dynamiques servies.

**Clics** : « Découvrez les jeux de données » (396,671) →
`edited` (bouton JS — hydratation incomplète du composant,
cohérent avec la mort du module). Nav « Données »
(107,171) → **`navigated`** → page de recherche datasets
(`vol93_datagouv_click.png`) : contenu réel (listes de
jeux de données, descriptions complètes) **mais non
stylé** — colonne brute à puces, famille « CSS de la
route interne non appliquée » (anomalie, voir §8).

**Télémétrie** (`vol93_datagouv_mem{,2}.json`) :
`dom_nodes=1 156` (accueil) → `2 495` (/datasets),
`js_heap=4,0 Mo`, `journal_fallbacks=2`, bfcache 1 page.
Pic 610 Mo → **purge 427 Mo** au DELETE.

## 3. www.bfmtv.com — §B-26 fermé : RAM plate, disjoncteur armé

`POST /navigate` → **9,6 s**, `doc_h=17 715` **clampé**,
titre « Actualités en continu et info en direct et
replay - BFM ». Avant : 6,88 Go en 5 min + API morte
(`push_z`/emit_scoped non bornés — fix R-98).

**Critères tous remplis** :
- `apply: début tab=4 journal=false` → **`apply: fin
  tab=4 ms=1440`** journalisé (ligne 58 265 du log) ;
  repaints continus `ms=685-925`, `display_list`
  ~7 200-9 000 items — chaque cycle **se termine**.
- **RAM stable** : ~864 Mo constant sur 60 s de pompe JS
  continue (mesures ×6), puis palier unique à 1 206 Mo au
  scroll + dump layout — **plat ×4 mesures ensuite**
  (1 206,92 Mo identique), jamais de dent-de-scie.
- **API vivante** : toutes les requêtes répondent pendant
  et après le vol du site.
- Bonus fix : `WARN js_glue: tempête de pompes —
  disjoncteur armé (B-41) tab=4 streak=64` — la borne a
  **tiré** sur un vrai site hostile.
- `commit.drains` tab=4 : `us=126-339` partout (vs
  60-185 s avant) — drains éteints ici.

**`/console` verbatim** (première lecture) : 2 `TypeError:
cannot read \`toLowerCase\` of Undefined` nommées —
`sliderSelectionRedac.b43ef88e….js` et `sliderVertical.
73f6302e….js` (via `window.nextScriptsToExecute`). 104
évals, **4 `ok=false`** (bytes=105 836 + 69, ×2 — les
deux sliders, rejoués une fois). NB : le fichier
`vol93_bfmtv_console.json` capturé ~30 s plus tard dit
« pas de journal » — le ring a été **purgé/rotaté** sur
page à JS continu (cohérent avec les écritures
incessantes constatées).

**Rendu** (`vol93_bfmtv.png`) : header BFM (logo,
« Collision de trains en Espagne » / « Colère sociale »
en flashes, nav complète International…Shopping),
bandeau « Présidentielle 2027 », hero « Une année
meurtrière… accidents ferroviaires en Espagne », « En ce
moment : 100% Mabrouk », **« Fil actu 19h44 » vivant**,
badges « Le direct »/« Live en cours ». **Anomalies** :
un `data:image/svg+xml` brut peint en texte dans le
header (SVG data-uri non décodé → fallback texte) ;
placeholder « Un train de la compagnie ferroviaire
espagnole Renfe ».

**Scroll → `scroll_y=800`** (`vol93_bfmtv_scroll.png`) :
le mégamenu « Toutes les rubriques » (BFM Business, BFM
Lyon… rubriques Présidentielle/Politique/Élections)
apparaît **peint brut non stylé** en plein flux — alors
que le layout le place à **x=-344** (`vol93_bfmtv_layout.
txt` l.225 : `text "Toutes les rubriques" [-344 675 136×16]`)
: boîte déclarée hors écran mais raster visible → écart
peinture/layout (anomalie, §8). `journal_fallbacks=3`
(`UnmappedKey` — re-parse ADR-0025 absorbé).

**Télémétrie** (`vol93_bfmtv_mem.json`) : `dom_nodes=4 506`,
`layout_boxes=4 059`, `display_items=8 106`, bfcache 1
page (`js_heap` re-caché 42,9 Mo). **Purge 1 206→462 Mo**
au DELETE, `{"tabs":[]}` — aucune fuite §B-30.

## 4. www.flixbus.fr — wedge migré : event.* non bornés → API morte (§B-55)

`POST /navigate` → **jamais retourné** : curl timeout à
150 s, requête serveur toujours pendante à +13 min quand
le vol s'arrête. Avant : `commit.drains` 60-185 s +
`pump.js_run_sync` persistant + fuite ~28 Mo/s
post-DELETE.

**Ce qui a marché côté fixes** :
- 21 `eval fin`, tous rapides (ms=0-4), **2 `ok=false`**
  dont le bundle applicatif **209 246 o** (+ 415 o).
- `commit.drains` tab=5 : une seule occurrence mesurée —
  **`us=53 081 868` = 53 s** (famille §B-41 en nette
  régression vs 60-185 s mais **toujours présente**).
- Un re-parse `UnmappedKey` absorbé à 18:54:10.

**Le wedge, jalons du log** (`_vol93_daemon.log`,
75 800+) — ~18 cycles `event.*` en ~13 min, chaque cycle
= un événement complet :

| phase | min | max | signature |
|---|---|---|---|
| `event.poll_net` | 17 µs | **61 839 794 µs = 61,8 s** | attente réseau long-poll 15-62 s |
| `event.dispatch` | 149 µs | 6 417 584 µs | **~5,5 s constants** (4,67-6,42) |
| `event.mirror_sync` | 134 µs | **13 282 023 µs = 13,3 s** | ~10-13 s par miroir DOM |
| `event.apply_queued` | 153 µs | 57,7 ms | négligeable |
| `pump.offlock` | — | **10 909 462 µs = 10,9 s** | un épisode |

Chronologie : 18:51:25 évals ok → 18:51:37 premier
`poll_net` 11,5 s → 18:52:23 `poll_net` **46 s** →
18:53:39 `poll_net` **61,8 s** → cycles
`dispatch ~5,5 s + mirror_sync ~10-13 s` en boucle
jusqu'à la fin du vol (19:06+, toujours actif).

**Conséquence** : `navigate` n'atteint jamais son
critère de retour (l'onglet n'est jamais idle — le JS
flite dispatch des événements en continu, chacun à
~30-80 s) → **verrou session global** → API **entière**
morte : `/api/tabs`, `DELETE /api/tabs/5`, `/frame`,
`/console` tous timeout (15 s, 20 s, 30 s, 60 s, 90 s
sondés — aucune réponse, >6 min). Le daemon n'est **pas**
mort : le log grandit en continu — c'est un *livelock*,
pas un deadlock.

**RAM pendant le wedge** : 1 335 → 1 856 Mo en ~15 min
(**~0,6 Mo/s**, dérive lente continue — pas la fuite
28 Mo/s d'avant, mais non bornée). Fuite post-DELETE
**non mesurable** : DELETE impossible (API morte).

**Verdict** : `commit.drains` en nette régression (53 s
unique vs 60-185 s en continu, µs sur bfmtv) mais le
site wedge **toujours**, par un bras nouveau : phases
`event.*` sans borne. Le disjoncteur B-41 n'a **pas**
tiré ici (pompes rapides, ce sont les phases qui sont
longues — pas la fréquence).

## 5-6. francebleu.fr · caniuse.com — NON COUVERTES

Daemon livelocké par flixbus : tout l'API indisponible
>6 min au moment de la clôture. §B-33 (nav_paint borné
30 s / `CoreError::Timeout` `20f2336`) et §B-37 (panic
`index out of bounds` `event.dispatch` NodeId obsolète,
fix `101a427`) **non re-testés ce vol** — à reprendre au
prochain créneau, idéalement **avant** toute cible de la
famille wedge.

## Statuts proposés (§8)

| §B | Bras | Verdict ce vol |
|---|---|---|
| §B-3 | stack overflow `loose_eq`×Symbol | **RÉSOLU `[V9-main]` vol 93** — process survit à alpinejs complet (36/36 évals ok, nav réelle, 0 overflow/panic log) ; seuls `ReferenceError` nommées à `/console` |
| §B-4(d) | data.gouv module muet | **RÉSOLU `[V9-main]` vol 93** — chunk `DL4muOy-.js` fetché+évalué (Étape 148), exception `TypeError` nommée à `/console`, homepage montée entière |
| §B-26 | borne cycle emit | **RÉSOLU `[V9-main]` vol 93** — `apply: fin ms=1440`, RAM ~864 Mo plate (vs 6,88 Go), API vivante ; `tempête de pompes — disjoncteur armé (B-41) streak=64` tiré |
| §B-41 | commit.drains / pompe | **EN DÉCROISSANCE** — drains µs sur bfmtv (126-339 µs) ; reste **1×53 s** sur flixbus + relais non borné par `event.*` → §B-55 |
| §B-30 | fuite post-DELETE | **signal favorable ×3** — purges complètes 721→402 / 610→427 / 1 206→462 Mo ; non mesurable sur flixbus (DELETE impossible) |
| §B-50 | re-parse ADR-0025 | **signal favorable** — `Fingerprint`×2 + `UnmappedKey`×4 absorbés sur 3 cibles, pages vivantes |
| §B-33 | nav_paint borné | **NON RE-TESTÉ** (daemon wedged) |
| §B-37 | panic event.dispatch | **NON RE-TESTÉ** (daemon wedged) — note : `event.dispatch` 4,7-6,4 s ×18 sur flixbus **sans panic** dans le log, le correctif `.get()` tient au moins là |
| **§B-55 (nouveau)** | **wedge navigate : phases `event.*` non bornées** | **proposition d'entrée** — flixbus : `poll_net` ≤61,8 s + `dispatch` ~5,5 s + `mirror_sync` ~11 s par événement, ~18 cycles/13 min → `navigate` jamais retourné → verrou session global → API entière morte >6 min ; disjoncteur B-41 inopérant (phases longues, pas fréquence) — `explorer-core` session/pump — preuve `logs/daemons/_vol93_daemon.log` |

**Anomalies secondaires constatées** (à rattacher ou
nouvelles entrées au jugement du pilote) :
- bfmtv : mégamenu « Toutes les rubriques » **peint**
  alors que boîte à x=-344 (`vol93_bfmtv_scroll.png` vs
  `vol93_bfmtv_layout.txt` l.225) — écart raster/layout
  sur contenu hors-écran.
- bfmtv : `data:image/svg+xml` brut peint en texte dans
  le header (`vol93_bfmtv.png`) — data-uri SVG non
  décodée → fallback texte visible.
- data.gouv : page interne `/datasets` rendue **non
  stylée** (`vol93_datagouv_click.png`) — CSS de la route
  interne non appliquée (SSR fetchée mais styles absents).

## Hygiène de vol

- Onglets 2 (alpinejs), 3 (data.gouv), 4 (bfmtv) :
  `DELETE` propres, purge complète à chaque fois,
  `{"tabs":[]}` confirmé entre cibles.
- Onglet 5 (flixbus) : `DELETE` **impossible** — API
  wedged (timeouts 20 s/60 s). Onglet laissé en place,
  daemon **vivant** (PID 18204, livelocké mais log
  actif) — le pilote l'arrête.
- Daemon voisin PID 12052 (port 9510) jamais touché ;
  aucun `taskkill`.
- Artefacts : `captures/vol93_{alpinejs,alpinejs_scroll,
  alpinejs_click,datagouv,datagouv_scroll,datagouv_click,
  bfmtv,bfmtv_scroll}.png` ;
  `logs/vol93_*_{console,mem,layout}.json/txt` ;
  `logs/daemons/_vol93_daemon.log` (fichier local, motif
  `_vol*` gitignoré — 76 135 lignes).

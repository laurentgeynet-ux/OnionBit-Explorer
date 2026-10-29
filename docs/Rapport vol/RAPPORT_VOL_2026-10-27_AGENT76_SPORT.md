<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 76 : sport & outdoor `[V6-main]`

- **Binaire** : `[V6-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (contient le fix §B-26 `1e24942`, borne `emit_scoped_at` — tirée
  sur ce vol : `apply: fin` journalisé partout).
- **Mission** : 3 cibles sport — decathlon.fr, intersport.fr,
  skiplan.com. Cibles prudence §B-26/§B-30/§B-41/§B-43/§B-46/§B-48/
  §B-50 évitées.
- **Daemon** : PID 2248, port 9458, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol76_daemon.log`. **RAM max ~1 280 Mo** — sous le seuil
  de prudence 1,5 Go (pic lié aux ~205 Mo d'images décodées decathlon
  + framebuffer 56 Mo lumiplan). Daemon parallèle PID 8896 (autre
  vol) non touché.
- **Fenêtre** : 14:53 → 15:05 UTC (~12 min).
- **Verdict global** : **1 rendu réel dégradé mais navigable
  (decathlon — SSR Next.js, 25 évals ok, 0 erreur console), 1 refus
  edge externe (intersport 403 sec), 1 reproduction §B-50 en variante
  inédite** (skiplan→lumiplan : boucle `UnmappedKey` infinie — 164
  rejets en ~7 min — avec **réattachement** entre cycles, le DOM
  n'est pas perdu en permanence comme bordeaux.fr).

---

## 1. decathlon.fr — SSR Next.js rendu, hydratation absente, clic ancre réel

`POST /navigate` → **14,6 s** — titre complet « Decathlon | Chaussures
de sport, équipements et matériel sportif », `doc_h=23 113`
(`doc_h_clamped:true`), URL stable, 0 nav auto (§B-2 tient).

**`/console` verbatim : `pas de journal`** — zéro erreur, zéro warn.
Journal daemon : **25 évals, 25× `ok=true`** — exclusivement des
petits scripts inline (payloads `self.__next_f.push` RSC) ;
`js_heap=2,27 Mio` seulement → les bundles d'hydratation/module
Next.js ne semblent jamais évalués (signature §B-4 bras (a) —
drains modules/defer inertes, mutisme total).

**Télémétrie** (`logs/vol76_decathlon_mem.json`) : `dom_nodes=1 950`,
`layout_boxes=1 670`, `display_items=1 246`, `frame_bytes=4 917 600`,
**`images_bytes=204 763 044`** (~205 Mo de rasters décodés — record
du vol), `js_heap=2 273 561`, `journal_fallbacks=1`
(1 rejet `UnmappedKey` transitoire à 14:54:06 pendant le chargement,
récupéré — tab=1 n'a jamais bouclé).

**Rendu** (`captures/vol76_decathlon.png`) : page réelle et lisible —
bandeau newsletter, champ « Rechercher », méga-menu peint
(Tous les sports / Femme / Homme / Enfant & bébé / Équipements &
loisirs / Électronique / Nutrition & Santé / Seconde vie / Bons plans /
Services et voyages / Carte cadeau), grille produits avec photos
(rollers, skateboards Looney Tunes). **Dégradations visibles** :
hero cassé — vignettes empilées, boutons « Découvrir » tronqués dans
des boîtes trop petites, titre « Nouvelle collection » chevauchant les
cartes, « Recevoir l'alerte sms » comprimé ; icônes du header
correctes (tofu PUA absent — fix tient). → famille **§B-18**
(layout résiduel : runs/cartes chevauchées, carrousel JS-mort éclaté).

**Scroll ×2** : `scrolled scroll_y=800` puis `1600` — frame
`vol76_decathlon_scroll.png` : carte vélo « randonnée EXPL 50 »
pleine largeur peinte proprement.

**Clics** :
- carte vélo (600,400) → `{"result":"edited"}` (livré au DOM, pas
  d'action par défaut — carte JS) ;
- bouton « Femme » du méga-menu (205,212) → `edited` — les items nav
  sont des `<button>` React (méga-menu hydraté jamais monté → §B-4/
  §B-17) ;
- ancre « En savoir plus » (400,30) → **`{"result":"navigated"}`**
  vers `https://www.decathlon.fr/#newsletter`, `can_back:true` —
  navigation ancre réelle validée (§B-13 confirme résolu sur `<a>`).

**Constats** : §B-18 (hero chevauché), §B-4 probable (bundles
hydratation jamais évalués, 0 erreur — mutisme cohérent avec le bras
« drains modules inertes »).

## 2. intersport.fr — HTTP 403 externe

`POST /navigate` → `{"error":{"handled":true,"message":"HTTP 403"}}`
en **273 ms**. `/console` verbatim : `{"console":["HTTP 403"]}`.
`/dom` → 404 « pas de DOM », `/layout` → 404 « pas de layout » ;
`/mem` : `layout_boxes=18`, `display_items=29`, `dom_nodes=0`
(`logs/vol76_intersport_mem.json`).

**Rendu** (`captures/vol76_intersport.png`) : page d'erreur interne
fidèle — « Page inaccessible — La page https://www.intersport.fr/
n'a pas pu être chargée. Le serveur a répondu « HTTP 403 ». »

**Sondage** : scroll ×2 → `scrolled scroll_y=0` (butée, nominal) ;
clic (683,450) → `ignored` (aucune cible, nominal).

**Classification** : **externe** — 403 sec ~0,3 s, edge sans
challenge → **entrée externe à ajouter** (« 403 sec —
intersport.fr »), même signature cumulée que le catalogue
(UA `Explorer/0.1`, rustls, pas de `sec-ch-ua`).

## 3. skiplan.com → lumiplan.com — **§B-50 variante : boucle `UnmappedKey` infinie avec réattachement**

`POST /navigate` → **7,2 s** — skiplan.com redirige vers
`https://www.lumiplan.com/smart-tourism-ski`, titre « Smart Tourism -
Domaines de montagne - Lumiplan », `doc_h=10 421` (non clampé).

**`/console` : `pas de journal`** malgré **60 évals dont 2 `ok=false`**
(le même script 3 457 o échoue à 14:57:29 puis à 14:59:08 après
re-parse) — **éval ko sans entrée console : trou d'observabilité**
(le traçage « scripts en échec » livré ne couvre pas ce cas).

**Anomalie majeure — reproduction §B-50 en variante** :
- Dès 14:57:35, le journal émet en continu :
  `WARN apply: journal DOM rejeté → repli re-parse (ADR-0025)
  tab=3 why=UnmappedKey` — **164 occurrences** de 14:57:35 à
  15:04:18 (~0,4/s soutenu, sans interruption) ;
- `journal_fallbacks` : 7 (premier /mem) → 64 → 129+ — chaque cycle
  JS produit un journal rejeté → re-parse → apply complet
  (cascade ~390 ms / boxtree ~420 ms / layout ~440 ms /
  repaint ~600 ms, items=1 037, regions=46-47 — la borne §B-26
  `emit_scoped_at` **tient**, `apply: fin` toujours atteinte) →
  rejet suivant ;
- Conséquence : `/dom` 404 / `/frame` 204 / `dom_nodes=0` **par
  fenêtres** — mais contrairement à bordeaux.fr (`>7 min` mort), le
  DOM **se réattache entre deux rejets** : la même sonde retourne
  `dom_nodes=1 876` puis 404 à la seconde suivante — oscillation
  permanente attach/detach, CPU ~1 apply/seconde à vie ;
- `tempête de pompes — disjoncteur armé (B-41) tab=3 streak=64`
  à 14:59:18 — le disjoncteur pompe s'arme mais la boucle reprend ;
- `js_heap` 47→53 Mio pendant la tempête, retombé à 0 au dernier
  /mem (moteur recréé au re-parse ?) ; RAM ~1,28 Go **stable, pas de
  fuite** (§B-26 contenu, pas d'escalade §B-1).

**Rendu** (fenêtres 200 — `captures/vol76_skiplan.png` accueil,
`vol76_skiplan_click.png` page « DS - Plan des pistes ») : contenu
réel peint — photo hero montagne (skieur Grand-Bornand, famille
neige), fil d'Ariane, texte propres, CTAs bleus. **Défauts layout** :
hero image **clipée à ~58 % de la largeur viewport** (zone blanche
droite x>790 — bandeau qui devrait être 100 vw) ; header éclaté :
nav « Nos filiales / Le Groupe… » à gauche sur la photo, « Nos
médias / Smart City / Smart Mobility » repoussés dans la zone
blanche ; onglet « CONTACT » fixe pivoté 90° peint ouvert, logo en
arcs SVG géant bas-droite. → recoupe **§B-18/§B-17** (layout résiduel
+ overlay résiduel JS-closable) — le découpage 58 %/42 % évoque
aussi une grille dont la 2ᵉ colonne absorbe les liens nav.

**Scroll ×2** : `scrolled` 800→1600 — `vol76_skiplan_scroll.png` :
sections texte propres (« Écrans numériques », CTAs) mais grands
blancs là où des images/parallaxe attendues.

**Clic** (220,168) « Découvrez nos plans des pistes » →
**`{"result":"navigated"}`** vers
`https://www.lumiplan.com/smart-tourism-ski/ds-plan-des-pistes`,
titre « DS - Plan des pistes - Lumiplan » — **vraie navigation par
clic malgré la boucle de re-parse** (hit-test servi dans une fenêtre
attachée).

## 4. Tableau de synthèse

| Cible | Navigate | Rendu | Scroll | Clic | Anomalies |
|---|---|---|---|---|---|
| decathlon.fr | ok 14,6 s — titre/doc_h corrects | SSR lisible, hero chevauché, grille ok | `scrolled` 1600 | ancre → `navigated` #newsletter ; boutons React → `edited` | §B-18 hero ; §B-4 hydratation inerte (25 évals ok, 0 err, heap 2,3 Mo) |
| intersport.fr | **HTTP 403** / 273 ms | page d'erreur propre | butée nominal | `ignored` nominal | externe 403 sec — à cataloguer |
| skiplan.com → lumiplan.com | ok 7,2 s — redir absorbée | contenu peint par fenêtres, hero clipé 58 % | `scrolled` 1600 | CTA → `navigated` /ds-plan-des-pistes | **§B-50 variante : 164 `UnmappedKey`, attach/detach oscillant ~7 min** ; 2 évals `ok=false` hors `/console` ; disjoncteur B-41 armé ; layout §B-18/17 |

## 5. Observations annexes

- **§B-50 variante à documenter** (proposition de précision de
  l'entrée existante, pas de nouvelle entrée) : sur lumiplan le
  repli re-parse **réussit** — le DOM se réattache et la page peint
  par intermittence — mais chaque mutation JS rejoue un journal non
  mappable → re-parse indéfini. Signature différentielle vs
  bordeaux.fr : `dom_nodes` alterne 0↔1 876 au lieu de rester 0 ;
  `/frame` alterne 204↔200 ; CPU soutenu (~1 apply/s) sans fuite RAM
  — la borne `emit_scoped_at` (§B-26) reste efficace. Le mécanisme
  amont est commun : **chaque journal est rejeté `UnmappedKey`** —
  sur bordeaux.fr le repli échouait en plus, ici il boucle.
- **Trou d'observabilité** : 2 `script: eval fin … ok=false`
  (tab=3) sans aucune entrée `/console` — le ring reste
  « pas de journal ». À recouper avec le traçage scripts-en-échec
  (note FIX-3) : soit le rejet est post-commit et le reset console
  l'efface, soit ce type d'échec n'est pas branché sur le ring.
- **§B-4 bras (a) probable sur decathlon** : 25 évals toutes
  `ok=true`, aucune erreur, heap 2,3 Mio, méga-menu React inerte —
  les chunks JS Next.js (modules ?) jamais évalués ni tracés ;
  mutisme complet cohérent avec « drains modules/defer inertes ».
- **§B-13 / §B-31 / §B-42 confirmés sains sur `<a>`** : 2/2 clics
  sur ancres réelles → `navigated` (ancre #newsletter, CTA page
  interne) ; les `edited` tombent sur `<button>` React ou carte
  JS-only — comportement correct du hit-test, absence d'action
  par défaut attendue.
- **`images_bytes` ~205 Mo sur decathlon** : premier dépassement
  net du poste « rasters décodés » — à rapprocher de la note perf
  (heap JS ~50 Mio/page ; ici ce sont les images qui dominent) —
  RAM daemon 1,28 Go au pic, libérée au DELETE.
- **`/frame` 204 intermittent = état transitoire, pas wedge** :
  l'API reste intégralement responsive pendant toute la tempête
  (latence requêtes <1 s) — à distinguer des wedges §B-33/§B-46.

## 6. Arrêt

DELETE tabs 1/2/3 → `{"tabs":[]}` puis **PID 2248** tué par
`taskkill /PID` (jamais `/IM`) — daemon parallèle PID 8896 non
touché. RAM max observée ~1,28 Go < seuil 1,5 Go (règle prudence
non déclenchée ; surveillance continue pendant la tempête §B-50).

**Fichiers** : `captures/vol76_{decathlon,decathlon_scroll,
intersport,skiplan,skiplan_scroll,skiplan_click}.png`,
`logs/vol76_{decathlon,intersport,skiplan}_{console.json,dom.txt,
layout.txt,mem.json}`, `logs/vol76_skiplan_click_{dom.txt,layout.txt}`,
journal `_vol76_daemon.log` (164 WARN `UnmappedKey` tab=3 conservées).

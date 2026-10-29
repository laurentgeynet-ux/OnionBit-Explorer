<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 84 : conforama · bouygues-immobilier · nexity `[V7-main]`

- **Binaire** : `[V7-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (build 27/10 18:30).
- **Mission** : 3 cibles « immobilier / habitat / e-commerce » —
  conforama.fr, bouygues-immobilier.com, nexity.fr. Cibles prudence
  §B-26/30/33/41/43/46/48/50 évitées.
- **Daemon** : PID **16516**, port **9467**, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol84_daemon.log` → `logs/daemons/_vol84_daemon.log`
  (~47 214 lignes).
- **RAM** : base ~389 Mo → pic ~1 043 Mo (bouygues, re-parse continu) →
  **415 Mo stable post-DELETE**. Seuil prudence 1,5 Go jamais franchi.
- **Fenêtre** : ~17:46 → 17:54 UTC (~8 min).
- **Verdict global** : **2/3 pages réelles servies** (conforama,
  bouygues — doc_h 9 295 / 10 124), **1 navigation réelle** (clic
  conforama → page catégorie « Canapé », bfcache armé), nexity = **403
  sec externe**. Signature dominante : **méga-menus peints ouverts**
  sur les deux sites réels (→ §B-17, variante « tous évals ok » sur
  conforama), **1 exception §B-14** (`nodeName` of Null, Drupal
  bouygues), **tempête de re-parses `UnmappedKey` ×56** sur bouygues
  (contenue, API vivante — proposition §B-52), chevauchements de runs
  footer conforama (→ §B-18). 0 wedge, 0 fuite, purge propre.

---

## 1. conforama.fr — page réelle complète + méga-menu peint ouvert (§B-17)

`POST /navigate` → **8,2 s** — `doc_h=9 295`, titre UTF-8 correct
« Conforama : meuble, cuisine, électroménager, décoration » (**pas de
mojibake §B-47** — titre accentué propre en JSON).

**Rendu** (`captures/vol84_conforama.png`) : la vraie page est peinte —
bandeau promo « Exclu Conforama.fr : Payez en 4X SANS FRAIS… », hero
Black November « -50 % » (jaune/noir fidèle), champ recherche avec
placeholder complet, barre catégories (« Nos Promotions, Black
November, Matelas, Canapé, Lit adulte, Lave-linge, Télévisions,
Réfrigérateur »), carte produit visible (lave-linge Samsung +
badges « Le Bon Plan ! » / « Nouveau »). **Mais** le panneau
méga-menu « Par Taille / Canapé 2 places / … / Banquette clic clac »
est **peint ouvert en surimpression du hero** — layout :
`text "Par Taille" [554 270 …]`, items jusqu'à y≈750
(`logs/vol84_conforama_layout.txt` l.158-231). Signature §B-17
(menus clos par JS restent ouverts) — **variante notable** :
ici **toutes les évals sont `ok=true`** (381 évals, 0 erreur,
console vide) — le menu n'est pas maintenu ouvert par un boot mort ;
l'état « fermé » attendu (classe/hover/desktop) n'est pas atteint.

**`/console` verbatim** (`logs/vol84_conforama_console.json`) :
« pas de journal » — zéro erreur.

**Télémétrie** (`logs/vol84_conforama_mem.json`) : `dom_nodes=6 661`,
`layout_boxes=5 239`, `display_items=4 417`, `images_bytes=12 465 872`
(images réellement chargées), `js_heap=2 724 958`,
`journal_fallbacks=0`.

**Interactions** : scroll ×2 → `scrolled` 800 → 1 600 ; **clic
(300,400) → `navigated`** — vraie navigation vers
`/canape-salon-sejour/canape/tous-les-canapes/c/020117`, titre
« Canapé (canapés d'angle, convertible, en cuir) | Conforama »,
`bfcache_pages=1` armé.

**Page catégorie** (`captures/vol84_conforama_canape.png` +
`_scroll.png` + `_bas.png`, `logs/vol84_conforama_canape_dom.txt`) :
le haut de page réaffiche les titres du méga-menu sur fond blanc
(« Par Taille » y≈280, « Fauteuil » y≈510, « Banquette » y≈720 —
même overlay ouvert). Le catalogue produit existe dans le DOM
(258 occurrences « € », 122 « EUR », 307 `<script>`). À
scroll_y=12 000 : **pied de page complet et riche peint** (bloc
newsletter + mentions RGPD en entier, colonnes « Nos services /
Besoin d'aide / Conforama et moi / À propos », icônes sociales,
« Paiement sécurisé » + logos CB) — **mais avec chevauchements de
runs inline** : les libellés de facettes (« 100% Coton (24) »,
« Bois (201) », « Massif acacia (1472) », « destockage (2362) »,
« Le Bon Plan ! (290) »…) se superposent aux colonnes
bricolage/outillage (visible verbatim sur la capture) → **§B-18**
(runs inline superposés, famille lemonde/ouest-france).

**Constats** : §B-17 reproduit (variante évals saines) ; §B-18
reproduit (footer) ; navigation/clic/scroll/bfcache **nominaux**.

## 2. bouygues-immobilier.com — nav dépliée + §B-14 + tempête `UnmappedKey` ×56

`POST /navigate` → **8,6 s** — `doc_h=10 124`, titre correct
« Bouygues Immobilier | Programmes d'immobilier neuf et
d'investissement locatif ».

**Rendu** (`captures/vol84_bouygues.png` + `_scroll.png`) : site
**Drupal** — la page se peint comme une **liste de liens entièrement
dépliée** : barre haute correcte (« Nous appeler », « Poser une
question », « Se connecter » — ce dernier peint ×2), puis le
méga-menu complet en flux (« Votre projet immobilier neuf »,
« Acheter pour habiter », « Les aides à l'achat », « PTZ : … »,
« Calculez votre budget »… ~4 000 px de liens), puis sections
grisées (« Votre satisfaction : notre priorité », « Nos
collections », « autopromo »…). Aucune image peinte en viewport alors
que `images_bytes=32 556 216` sont chargées — les `<img>` du sous-menu
(`<picture>`+`srcset`, `alt="autopromo"`) apparaissent en layout comme
`text<img> "autopromo"` (alt text, image non peinte — un
`block<img> [43 2883 1280x500]` existe plus bas). → **§B-17**
(même signature : structure de navigation jamais refermée).

**`/console` verbatim** (`logs/vol84_bouygues_console.json`) — **1
exception tracée, §B-14** :
`exception js : TypeError: cannot read nodeName of Null` dans
`js_aP7sdGtC6XHOl6UnFN13gMgPQmJc2sa0UbCMTSmq71A.js?scope=footer…`
(agrégat Drupal `bi_stable`, ligne 15). Famille `X of Null`
cataloguée (marmiton `GeolocationPosition`, ratp `contains`, dw
`document` of Null…) — ici `nodeName` sur nœud null, probablement
le script menu/search Drupal mourant → cohérent avec le nav ouvert.

**Télémétrie** (`logs/vol84_bouygues_mem.json`) : `dom_nodes=4 640`,
`layout_boxes=2 422`, `display_items=1 770`, `js_heap=66 486 770`
(au-dessus du ~50 Mio/page habituel), `journal_fallbacks=8` à
l'instant T — **le compteur montait encore**.

**Constat observabilité/perf — tempête `UnmappedKey`** : le journal
daemon montre **56 `journal DOM rejeté → repli re-parse (ADR-0025)
why=UnmappedKey` sur tab=2** entre 17:50:32 et 17:52:49 — cadence
régulière ~1 re-parse / 5 s (le site mute le DOM en continu :
carrousel/timer). Chaque re-parse = pipeline complet
`parse ~29 + cascade ~230 + boxtree ~300 + layout ~350 + repaint
~420 ms ≈ 1,3 s`. **Contenu** : API toujours responsive, timings
stables (pas d'escalade §B-44 — repaint constant ~420 ms), purge
propre au DELETE. Mais ~20 % de CPU moteur part en re-parses
perpétuels. **Proposition d'entrée §B-52** : « rafale `UnmappedKey`
continue — re-parse complet par mutation DOM sur site à mutations
périodiques » (famille §B-41 sans le wedge ; signature : fallback
systématique, jamais de journal appliqué). Note annexe :
`journal_fallbacks` remonté `=56` sur `/mem` de tab 3 (nexity, page
d'erreur) → le compteur est **global daemon**, pas par onglet
(libellé trompeur).

**Interactions** : scroll ×2 → `scrolled` 800 → 1 600 ; clic
(600,450) → `edited` (livré au DOM, pas de cible — point dans la
liste de liens, aucun `<a>` résolu sous le point).

## 3. nexity.fr — HTTP 403 sec externe

`POST /navigate` → **187 ms** —
`{"error":{"handled":true,"message":"HTTP 403"}}`. `/console` :
`{"console":["HTTP 403"]}`. `/dom` → « pas de DOM », `/layout` →
« pas de layout » ; `/mem` : `layout_boxes=18`, `display_items=29`,
`dom_nodes=0`, `js_heap=0` (`logs/vol84_nexity_mem.json`).

**Rendu** (`captures/vol84_nexity.png`) : page d'erreur interne
fidèle — « Page inaccessible — La page https://www.nexity.fr/ n'a
pas pu être chargée. Le serveur a répondu « HTTP 403 ». »

Scroll → `scrolled scroll_y=0` (butée nominale) ; clic (683,450) →
`ignored` (aucune cible, nominal). Titre onglet « Erreur de
chargement » propre.

**Classification** : **externe** — refus sec <200 ms sans challenge.
nexity.fr **absent** de la liste 403 du catalogue → **à ajouter**
(signature cumulée UA `Explorer/0.1` + rustls + absence
`sec-ch-ua`/`sec-fetch-*` déjà documentée).

---

## Synthèse des constats

| Cible | Navigate | Rendu | Scroll | Clic | IDs §B | Crate |
|---|---|---|---|---|---|---|
| conforama.fr | 8,2 s, doc_h 9 295 | Page complète + **méga-menu peint ouvert** sur hero ; footer riche avec **runs superposés** | `scrolled` ×2 → 1 600 ; scroll_y 12 000 OK | **`navigated`** → catégorie Canapé (bfcache armé) | §B-17 (variante évals ok), §B-18 | `layout`/`jscore`-état |
| bouygues-immobilier.com | 8,6 s, doc_h 10 124 | **Nav dépliée intégrale**, images `text<img>` alt, pas d'imagerie en viewport | `scrolled` ×2 → 1 600 | `edited` (pas de cible) | §B-17 ; §B-14 (`nodeName` of Null) ; **proposition §B-52** (56 `UnmappedKey`) | `jscore`/`core` |
| nexity.fr | **HTTP 403** / 187 ms | Page erreur moteur propre | `scroll_y=0` (butée) | `ignored` (nominal) | Externe — +1 entrée liste 403 | — (externe) |

### Mises à jour de statut suggérées (SYNTHESE.md)

- **§B-17** : +2 confirmations `[V7-main]` vol 84 — conforama
  (méga-menu « Par Taille/Canapé/Fauteuil/Banquette » peint ouvert
  sur hero, **toutes évals ok=true** → variante « état menu jamais
  refermé hors boot mort ») et bouygues-immobilier (nav Drupal
  dépliée intégrale).
- **§B-14** : +1 entrée — `TypeError: cannot read nodeName of Null`
  (agrégat footer Drupal `bi_stable`, bouygues-immobilier, vol 84).
- **§B-18** : +1 — facettes filtre chevauchant les colonnes footer
  (conforama page catégorie, scroll_y 12 000).
- **§B-52 (proposition)** : « rafale `UnmappedKey` ADR-0025 —
  re-parse complet par mutation DOM » — 56 rejets ~1/5 s sur
  bouygues (~1,3 s pipeline chacun, API vivante, pas d'escalade) ;
  famille §B-41 sans wedge ; + note `journal_fallbacks` compteur
  global affiché par onglet.
- **Externe** : ajouter **nexity.fr** à la liste 403 sec.
- **§B-47** : non reproduit — titres accentués (« Conforama :
  meuble, cuisine, électroménager, décoration ») propres en JSON.
- **§B-30** : signal favorable — DELETE pendant vie des deux pages
  réelles → purge 1 043→~410 Mo, `{"tabs":[]}`, API responsive.

## Artifacts

- Captures : `captures/vol84_conforama.png` (accueil, méga-menu
  ouvert), `vol84_conforama_canape.png`, `vol84_conforama_canape_
  scroll.png`, `vol84_conforama_bas.png` (footer + chevauchements),
  `vol84_bouygues.png`, `vol84_bouygues_scroll.png`,
  `vol84_nexity.png` (page erreur 403).
- Logs : `logs/vol84_conforama_{console.json,dom.txt,layout.txt,
  mem.json}`, `logs/vol84_conforama_canape_dom.txt`,
  `logs/vol84_bouygues_{console.json,dom.txt,layout.txt,mem.json}`,
  `logs/vol84_nexity_{console.json,dom.txt,layout.txt,mem.json}`
  (dom/layout = « pas de DOM »/« pas de layout » — archivés pour
  traçabilité) ; journal `logs/daemons/_vol84_daemon.log`.

## Arrêt

DELETE onglets 1-3 (200 ×3, `{"tabs":[]}`) — RAM 1 043 → **415 Mo
stable**, purge complète sans in-flight résiduel (signal §B-30
favorable). Daemon terminé par **PID 16516** (`Stop-Process -Id`,
jamais `/IM`) — daemons parallèles (PID 7296/13000/19652, autres
vols) non touchés.

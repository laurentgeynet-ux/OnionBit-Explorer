<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 82 : rtl · europe1 · funradio `[V7-main]`

- **Binaire** : `[V7-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`.
- **Mission** : 3 cibles radio **hors groupe Radio France** —
  www.rtl.fr · www.europe1.fr · www.funradio.fr (rtl/funradio =
  plateforme commune groupe RTL/M6 — même gabarit, mêmes défauts).
- **Daemon** : PID **19652**, port 9465, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol82_daemon.log` (~108 700 lignes) → `logs/daemons/`.
- **RAM** : base ~371 Mo → pics 889 (rtl) / 1 067 (europe1) /
  1 203 Mo (funradio) — **seuil 1,5 Go jamais franchi**, purge
  complète ~430-472 Mo après chaque DELETE (aucune fuite §B-30).
- **Verdict global** : **3/3 sites volés sans wedge** — rendu SSR
  ~80-85 %, clics = vraies navigations (3/3). Deux reproductions
  notables : **§B-51** (bundle applicatif 4,1 Mo `eval ok=false`
  silencieux sur rtl+funradio → JS mort au boot, `/console` vierge)
  et **§B-8 variante partielle** (textes non repeints au scroll>0
  sur les deux cibles RTL, images peintes ; europe1 indemne).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| www.rtl.fr | Rendu ~80 % — header/ticker/cartes peints ; clic « Actu » → nav réelle | `pas de journal` — mensonger : bundle 4 147 298 o `ok=false` ×2 | **§B-51 confirmé** + **§B-8 scroll** + textes cartes rognés (§B-18) |
| www.europe1.fr | Rendu ~85 % — le plus sain ; 26/26 évals ok ; clic ancre à scroll 1600 → **bon article** | homepage vierge ; **page article : 5 entrées `unhandledrejection` Marfeel SDK tracées** | re-parse `Fingerprint` ×3 + disjoncteur **B-41 armé streak=64** (contenu) |
| www.funradio.fr | Rendu ~80 % — clone rtl.fr ; clic « Radios » → nav | `pas de journal` — mensonger : bundle 4 147 308 o `ok=false` ×2 | **§B-51 confirmé** + **§B-8 scroll** (retour 0 = repeint) |

Points marquants :

- **Le bundle applicatif meurt en silence sur la plateforme RTL** :
  script **4 147 298 o** → `eval fin ms=1775 ok=false` (rtl), puis
  ré-évalué `ok=false ms=1753` après re-parse `Fingerprint` — et
  `/console` = `pas de journal` à chaque instant. Funradio :
  **même bundle** 4 147 308 o → `ok=false ms=1719`, re-éval
  `ms=1703`. La cause exacte de l'échec n'est **jamais tracée**
  (ni l'erreur, ni même le fait qu'un échec a eu lieu — §B-51 au
  mot près : « évals `ok=false` silencieuses »). Conséquence :
  `js_heap` reste à 0 sur funradio — le site tourne **sans aucun
  JS** (menus, carrousels, lecteur inertes → §B-4/§B-17 latents).
- **§B-8 reproduit `[V7-main]` ×2, en variante partielle** : à
  `scroll_y=1600`, les `<img>` et filets de séparation peignent,
  mais **aucun run de texte** des articles layoutés (titres h3,
  tags, résumés présents dans `/layout` à doc_y 1 935-2 300) —
  sauf un titre dans une colonne `fixed` (« Les plus consultés »).
  Retour à `scroll_y=0` → texte repeint (diff pixel 1,69 % vs
  frame initiale, contenu dynamique seul). Europe1 au même
  scroll : **tout peint** → défaut site-dépendant, suspect
  `build_display_list_incremental` comme §B-8.
- **Contre-exemple §B-40** : clic sur un `<a>` réel à
  `scroll_y=1600` → `navigated` vers **la bonne URL** (article
  Smic, europe1). Le défaut trainline n'est pas universel.
- **Garde-fous visibles et efficaces** : `cycle de report cassé
  (§B-26) dropped=3 path=Some([0, 1])` ×4 (rtl ×2, funradio ×2 —
  borné, aucun wedge, RAM contenue) ; `journal DOM rejeté → repli
  re-parse (ADR-0025) why=Fingerprint` ×3 (europe1, 2 fallbacks
  consommés) ; `tempête de pompes — disjoncteur armé (B-41)
  streak=64` (europe1 — contenu, page restée vivante).

---

## 1. www.rtl.fr — plateforme RTL, JS mort au boot (§B-51)

`POST /navigate` → `doc_h=19 584` **clamped**, titre
« Toute l'actualité en direct et en vidéo sur RTL.fr… ».

**`/console` verbatim** : `pas de journal` — **alors que le
journal daemon montre** `script: eval… tab=1 bytes=4147298` →
`eval fin ms=1775 ok=false`, puis après re-parse `Fingerprint`
re-éval `ms=1753 ok=false`. Les 5 petits scripts (134-3 452 o)
passent `ok=true ms=0`. **§B-51 confirmé `[V7-main]`** : l'échec
d'éval d'un bundle de 4,1 Mo n'émet aucune entrée console —
« pas de journal » mensonger.

**Rendu** (`captures/vol82_rtl.png`) : header rouge + logo RTL +
nav (« Actu Radios Grille Podcasts »), bandeau ticker noir
(items tronqués à droite — clip marquee attendu), puits
« Publicité » vide, cartes podcasts peintes **mais texte rogné
aux bords de chaque carte** (« casts », « s pour braquer »,
« uage stupéfiant pa » — débordement/clipping horizontal du run,
à rattacher §B-18), grande colonne grise vide (slot pub/carrousel
non hydraté — JS mort), bouton « Écouter le direct » avec un
bloc blanc (icône absente — PUA muet connu).

**Télémétrie** (`logs/vol82_rtl_mem.json`) : `dom_nodes=6 543`,
`layout_boxes=3 466`, `display_items=1 149`,
`images_bytes=12,3 Mo`, `js_heap=30,9 Mo`, `journal_fallbacks=0`.
Daemon : `cycle de report cassé (§B-26) dropped=3 path=[0,1]` ×2.

**Scroll ×2 → `scroll_y=1 600`** (`captures/vol82_rtl_scroll.png`)
: header `fixed` repeint, 2 vignettes `<img>` peintes
(`figure [83 1919 145×96]`), **mais les runs texte des articles
layoutés à doc_y 2 025-2 300 absents** — zone blanche. `/layout`
prouve les runs présents (« Violences du 17 octobre »,
h3 « "Arbitraire policier", responsabilité co… »). → **§B-8**,
variante partielle (textes seuls perdus).

**Clic** « Actu » (184,12) → `navigated` → page section peinte
(`captures/vol82_rtl_actu.png` : hero Mbappé, puits pubs).
Console post-nav : toujours vierge.

## 2. www.europe1.fr — JS vivant, disjoncteur B-41 sollicité

`POST /navigate` → `doc_h=21 919` clamped, titre « Actualités du
jour | Europe 1 ». **26/26 évals `ok=true`** — le seul site du
trio dont le JS tourne.

**`/console` homepage** : `pas de journal`. Après clic → article :
**5 entrées `unhandledrejection` tracées** (verbatim
`logs/vol82_europe1_article_console.json`) — SDK Marfeel
(`sdk.mrf.io`) : `TypeError: Invalid scheme` (forme non couverte
→ cf. §B-11 « autre forme d'entrée à nommer »), `undefined` (×2),
`cannot read hasWebVitals of Undefined` (×2),
`cannot read stableUserId of Undefined`,
`` `post` is not a function (on Object) ``. La console vit —
le vide homepage était donc bien un vide réel.

**Rendu** (`captures/vol82_europe1.png`) : header blanc + logo +
boutons nav (« Émissions » « Direct »), bandeau « Journal
permanent » (rayures bleues — squelette/ticker), pavé noir
(lecteur vidéo), cat-nav complète, hero « Mobilisation du 31
octobre… » + photo. Puits gris vides = slots pub/média.

**Daemon** : `journal DOM rejeté → re-parse (ADR-0025)
why=Fingerprint` ×3 + **`tempête de pompes — disjoncteur armé
(B-41) streak=64`** → `/frame` 204 transitoire ~6 s au scroll
(repli sain, `journal_fallbacks=2`). Applies courts (168-445 ms).

**Scroll ×2 → 1 600** (`captures/vol82_europe1_scroll.png`) :
**tout peint** — titres + vignettes + sidebar. **Pas de §B-8.**

**Clic** titre « Une nouvelle augmentation du Smic… » à
`scroll_y=1 600` → `navigated` vers l'URL exacte de l'article
(`/economie/une-nouvelle-augmentation-du-smic-…-1185423`) →
**contre-exemple §B-40** : le hit-test scrollé fonctionne.

## 3. www.funradio.fr — clone RTL, mêmes deux défauts

`POST /navigate` → `doc_h=12 526` (non clampé, framebuffer
68,4 Mo), titre « Funradio: écouter la radio en direct… ».

**`/console` verbatim** : `pas de journal` — même mensonge :
`eval… tab=3 bytes=4147308` → `ok=false ms=1719`, re-éval
`ok=false ms=1703`. **`js_heap_bytes=0`** — page servie sans
aucun JS. `journal_applies=114`.

**Rendu** (`captures/vol82_funradio.png`) : structure identique
à rtl (header bleu, même burger, même ticker, même « Écouter le
direct » cassé, mêmes puits vides, mêmes cartes au texte rogné)
— confirmation que les défauts sont **ceux de la plateforme**,
pas du contenu.

**Scroll ×2 → 1 600** (`captures/vol82_funradio_scroll.png`) :
même signature §B-8 — 4 vignettes + titre sidebar `fixed`
peints, **tous les textes articles de doc_y 1 935-2 100 absents**
(« Halloween », « Gaming », h3 « "C'est incroyable" : Inoxtag… »
présents dans `/layout`). Retour `scroll_y=0`
(`captures/vol82_funradio_back0.png`) → texte repeint, diff
1,69 % (boîte [83,375→285,817] — marquee dynamique).

**Clic** « Radios » (265,12) → `navigated` → `/radios`.

## Statuts proposés (§8)

| §B | Bras | Verdict ce vol |
|---|---|---|
| §B-51 | évals `ok=false` silencieuses | **confirmé `[V7-main]` ×2** — bundle 4,1 Mo de la plateforme RTL tué au boot (ms≈1700-1775), re-éval post `Fingerprint` idem, `/console` = « pas de journal » des deux côtés ; conséquence : site entier sans JS (`js_heap=0`) |
| §B-8 | frame blanche au scroll | **reproduit `[V7-main]` ×2** — **variante partielle nouvelle** : textes articles non repeints à scroll>0 sur rtl/funradio, images/filets/`fixed` peints ; retour 0 = repeint ; europe1 indemne → site-dépendant, toujours suspect `build_display_list_incremental` |
| §B-40 | hit-test ancre à scroll>0 | **contre-exemple** — clic à scroll_y=1 600 → navigation correcte (europe1, article Smic) ; défaut non universel |
| §B-26 | borne cycle de report | garde-fou tiré ×4 (`dropped=3 path=[0,1]`, rtl/funradio) — **aucun wedge**, RAM contenue |
| §B-41 | tempête de pompes | disjoncteur armé terrain (europe1 `streak=64`) — contenu, page vivante, `/frame` 204 transitoire seul symptôme |
| §B-11 | `Invalid scheme` | +1 forme d'entrée : `unhandledrejection: TypeError: Invalid scheme` via Marfeel SDK (europe1 article) — tracé, console fonctionnelle |
| §B-18 | layout résiduel | +1 instance : runs texte rognés aux bords des cartes plateforme RTL (overflow horizontal interne carte) |
| §B-30 | fuite post-DELETE | **signal favorable** — purge complète à ~430-472 Mo après chaque DELETE, RAM résiduelle 472 Mo en fin de vol |

Pas de nouvelle entrée §B : tout se range dans le catalogue
(§B-8 gagne une variante « partielle », §B-51 deux preuves
terrain supplémentaires).

## Hygiène de vol

- Onglets 1 (rtl), 2 (europe1), 3 (funradio) — tous `DELETE` ;
  `{"tabs":[]}` final, purge mémoire complète à chaque fois.
- Daemon tué par **son PID 19652** seul — jamais `/IM` ; les
  daemons voisins (PID 7877, 7996, 8011, 8415) intacts.
- Artefacts : `captures/vol82_{rtl,rtl_scroll,rtl_actu,
  europe1,europe1_scroll,funradio,funradio_scroll,
  funradio_back0}.png` ; `logs/vol82_*_{dom,layout,mem}.txt/json`
  + `logs/vol82_europe1_article_console.json` ;
  `logs/daemons/_vol82_daemon.log`.

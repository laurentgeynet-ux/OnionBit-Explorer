<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 62 — Culture / institutions (louvre · musee-orsay · culture.gouv) (2026-10-27)

- **Binaire** : `[V4-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (WIP `depth_limit` embarqué), `--port 9444`, `EXPLORER_VIEWPORT=1366x900`,
  journaux `logs/daemons/_vol62_daemon.log` (PID 11212, tué en urgence
  §B-26/§B-30 à 2,35 Go) et `logs/daemons/_vol62_daemon2.log`
  (PID 18316, vol culture.gouv.fr après redémarrage)
- **Cibles** : www.louvre.fr · www.musee-orsay.fr · www.culture.gouv.fr —
  **3/3 volées**
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de `SYNTHESE.md`
- **RAM daemon 1 (PID 11212)** : ~375 Mo boot → 975 Mo (louvre, images
  354 Mo) → **976 Mo → 2,2 Go en ~5 min sous musee-orsay (fuite
  non bornée)** → 2,23 → **2,35 Go post-DELETE avec `{"tabs":[]}`**
  → kill au seuil.
- **RAM daemon 2 (PID 18316)** : ~400 Mo boot → 1,11 Go pic → **416 Mo
  post-DELETE, stable** — libération propre (contraste §B-30).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| www.louvre.fr | **Excellent** — rendu fidèle, JS sain (29/29 évals ok), clic→nav `target=_blank` réelle | `pas de journal` | §B-13 tient ; cosmétique : boutons du panneau « Bienvenue » débordent hors viewport à droite ; micro-chevauchements de runs dans la carte info (page agenda) — recoupe §B-18 résiduel |
| www.musee-orsay.fr | **Wedge §B-26 — NOUVEAU PORTEUR + §B-30 recoupé** | `setAttribute` of Undefined · `appendChild` is not a function (on HTMLDocument) — Drupal | 4ᵉ apply arrêté à `layout`, jamais `repaint`/`fin` → `/dom` 404, `/frame` 204, `dom_nodes=0` terminal ; fuite ~5-10 Mo/s jusqu'au kill ; **`pump.js_run_sync` cadence ~5 s persistante** ; croissance poursuivie après DELETE |
| www.culture.gouv.fr | **Bon avec une anomalie hit-test nouvelle** | `pas de journal` au 1er chargement (3 évals `ok=false` silencieuses) ; 3 exceptions tracées au 2ᵉ | **NOUVEAU §B-40 proposé — clic à travers un overlay `position:fixed`** : clic « Tout accepter » → navigation vers le lien `Audiovisuel` *sous* le bandeau ; méga-menu peint déployé (recoupe §B-17) ; boutons tarteaucitron blanc-sur-gris (style manquant, cosmétique) |

Fait marquant n°1 : **musee-orsay.fr est un nouveau porteur §B-26** —
signature textbook (apply stoppé après `layout ms=968`, DOM détaché
terminal) avec une variante : la pompe JS continue de battre
(`pump.js_run_sync` ~8 ms toutes les ~5 s) pendant que la fuite mémoire
croît, et **la croissance survit au DELETE de l'onglet** (recoupement
direct §B-30).

Fait marquant n°2 : **un clic qui navigue vers la mauvaise cible** — la
preuve la plus précise à ce jour d'un hit-test traversant un overlay
`position:fixed` : géométrie vérifiée dans le dump layout (voir §3).

---

## 1. www.louvre.fr — excellent, §B-13 tient

- `POST navigate` → retour en ~20 s : `doc_h=6 532`, titre « Site
  officiel du musée du Louvre », `url=https://www.louvre.fr/`.
- **Console : `pas de journal`** (`logs/vol62_louvre_console.json`).
  Journal daemon : **29 évals `ok=true`, 0 échec** — pipeline JS sain.
- **Frame** (`captures/vol62_louvre.png`) : header complet (Rechercher,
  Agenda, Français, E-Boutique, Billetterie), wordmark LOUVRE, nav
  principale (Visiter, Expositions et événements, Découvrir, Voir plus),
  **photo hero du musée avec pyramide peinte**, panneau « Bienvenue au
  Louvre / Le musée est fermé aujourd'hui », bandeau « Réservation
  obligatoire du 17 octobre au 1er novembre 2026 » — rendu très fidèle.
  - Cosmétique : les boutons « Réserver un… » / « Préparer sa… » du
    panneau Bienvenue sont **clippés au bord droit du viewport**
    (x>1 366) — le panneau semble layouté plus large que la fenêtre.
    Le tiers inférieur du viewport est noir à scroll 0 (fond sombre du
    site — le contenu se peint normalement au scroll).
- **Scroll ×2** → `scroll_y=1 600` (`captures/vol62_louvre_scroll.png`) :
  nature morte Zurbarán peinte + bloc « En direct — Regardez la
  présentation de l'exposition "Zurbarán 1598-1664" sur Louvre + ».
  Repaint correct, pas de frame blanche (§B-8 non reproduit).
- **Télémétrie** : `dom_nodes=1 160`, `layout_boxes=771`,
  `display_items=534`, `js_heap=16,1 Mo`, `images_bytes=354 Mo`,
  `total≈432 Mo`, `journal_fallbacks=0`.
- **Clic** (820, 760) sur le bloc « Regardez la présentation… » →
  `{"result":"navigated"}` → **nouvel onglet** (id=2,
  `target=_blank`) vers `/agenda/presentation-de-l-exposition-zurbaran-1598-1664`
  — §B-13 confirmé sain (clic→vraie navigation + ouverture d'onglet).
- Page fils (`captures/vol62_louvre_click.png`, console vide) : fil
  d'Ariane, titre « Présentation de l'exposition "Zurbarán, 1598 –
  1664" », onglets Présentation/Le live, carte info. **Micro-
  chevauchements de runs inline** : glyphe « D » superposé à « Durée
  moyenne », « Aud…/…du … » clippé à droite de la carte — recoupement
  bénin §B-18 (famille runs superposés, non bloquant).
- **Hygiène** : DELETE onglets 2 puis 1 → RAM 975 → 405 Mo — libération
  complète et immédiate (contraste net avec le §B-30 observé plus bas).

## 2. www.musee-orsay.fr — §B-26 confirmé, nouveau porteur (+ §B-30)

- `POST navigate` → **retourné normalement** (~35 s) : `doc_h=10 372`,
  titre « Musée d'Orsay », URL finale `https://www.musee-orsay.fr/fr`
  (redirect /fr honoré). La page a donc commité avant de se figer.
- **Console** (`logs/vol62_orsay_console.json`) — erreurs Drupal
  modestes, non fatales en apparence :
  - `uncaught: TypeError: cannot read 'setAttribute' of Undefined`
    (bundle `js_1AdR…js` agrégé, traces pile complètes) ;
  - `exception js : TypeError: 'appendChild' is not a function (on
    HTMLDocument)` — `document.appendChild` appelé par un helper `t`
    ×N — recoupe la matrice §B-14 (API DOM manquante sur `HTMLDocument`).
- **Signature §B-26 textbook** (journal `_vol62_daemon.log`) :
  - 12 évals dont 11 `ok=true` (jusqu'à 1,1 Mo en 838 ms) + 1 `ok=false`
    (script 246 904 o en 90 ms — l'erreur `setAttribute` ci-dessus) ;
  - applies 1-3 complets (`fin tab=3 ms=1040`, `repaint ms=1134`,
    `repaint ms=1059`) ;
  - **4ᵉ apply à 10:14:07,95 : `cascade ms=893 → boxtree ms=922 →
    layout ms=968` puis jamais `repaint`/`fin`** — wedge.
- **DOM détaché terminal** : `/dom` → 404, `/layout` → 404, `/frame` →
  204, `dom_nodes=0, layout_boxes=0, display_items=0` sur 3 tentatives
  espacées (artefacts `logs/vol62_orsay_{dom,layout}.txt` = « pas de
  DOM »/« pas de layout », `captures/vol62_orsay.png` = 0 o).
  `journal_fallbacks=0` — pas même un repli tenté.
- **Variante « pompe battante »** : `pump.js_run_sync` ~7,6-8,5 ms
  toutes les ~5 s pour `tab=3` **pendant tout le wedge** (10:14:32,
  10:18:47, 10:18:53, 10:18:58 — cadence timer/intervalle, probablement
  le Drupal keepalive) — le JS tourne encore sur un DOM détaché.
- **Fuite mémoire** : 976 Mo → 2,15 → 2,21 Go (mesures toutes les ~5 s,
  ~5-10 Mo/s soutenu avec à-coups). **Post-DELETE** (`{"tabs":[]}` à
  11:19:26) : 2,23 → 2,26 → **2,35 Go au kill ~45 s plus tard** —
  recoupement direct §B-30 : la croissance survit à la destruction de
  l'onglet (tâche navigate/pompe/apply orpheline).
- **API vivante tout du long** : `/tabs`, `/mem`, `/dom`, `/frame`,
  `DELETE` ont toujours répondu — à l'opposé du bras bfmtv (API morte).
- Kill d'urgence conforme : seuil 2 Go franchi → DELETE + `taskkill
  //PID 11212 //F`. Daemon redémarré pour la cible suivante.

## 3. www.culture.gouv.fr — rendu correct, **hit-test à travers overlay fixed (§B-40 proposé)**

- `POST navigate` → `doc_h=21 733` (`doc_h_clamped=true`), titre
  « Ministère de la Culture ». **Frame** (`captures/vol62_culture.png`) :
  bloc marque État, champ « Rechercher », nav (Actualités, À la Une,
  Dossiers), **méga-menu entièrement déployé en colonne gauche** (Nous
  connaître → DRAC → Thématiques — recoupe §B-17 : panneau JS-closable
  peint ouvert), bandeau cookies tarteaucitron centré.
- **État JS** : 3 évals `ok=false` au boot (5 035 o / 124 917 o /
  1 013 395 o — translator.min.js, translations, main.js) **sans aucune
  entrée `/console`** (« pas de journal » au 1er chargement —
  `logs/vol62_culture_console.json`). Les **mêmes erreurs apparaissent
  tracées** au 2ᵉ chargement (post-clic) : `TypeError: cannot read
  'replace' of Undefined` (translator.min.js) → cascade
  `ReferenceError: Translator is not defined` ×2 (translations,
  main.js) — **trou d'observabilité §B-4-adjacent : un éval `ok=false`
  au boot ne tombe pas dans le ring console** (ou est reset), alors
  que l'exception identique y est tracée au chargement suivant.
  `js_heap=0` au premier `/mem` (15,9 Mo après la 2ᵉ nav — heap trimmé
  ou jamais rapporté, bénin).
- **Scroll ×2** → `scroll_y=1 600` (`captures/vol62_culture_scroll.png`) :
  le menu déroule (DRAC, Thématiques…), **le bandeau cookies reste
  peint à position viewport fixe** — le fix « `fixed` échappe au clip
  overflow ancêtre » fonctionne pour la peinture.
- **Clic (804, 809) sur « Tout accepter » → `{"result":"navigated"}`
  vers `/thematiques/audiovisuel`** — **la mauvaise cible**. Preuve
  géométrique (`logs/vol62_culture_layout.txt`) :
  - `text " Tout accepter "` enregistré à `[754.3, 796, 108×24]` —
    élément `position:fixed`, peint à viewport y≈796-820 quel que soit
    le scroll : le clic (804,809) est **dedans** ;
  - mais le hit-test a résolu en **coordonnées document** (804,
    809+1 600=2 409) → `block<a> [0 2386.22 1366x48]` — ancre
    pleine-largeur dont le run fait **78,106 px = largeur exacte du
    texte « Audiovisuel »** (l.1185) → navigation
    `/thematiques/audiovisuel` ;
  - conclusion : **l'overlay `fixed` (peint, visible) n'intercepte pas
    le hit-test — le clic traverse jusqu'au contenu scrollé dessous**.
    Variante active de §B-13/§B-17 : au lieu d'`ignored`, on obtient
    une **navigation vers un élément invisible sous l'overlay**.
- **Post-navigation** : DOM 200, frame transitoire 204→200, page
  Audiovisuel commitée (`logs/vol62_culture_click_dom.txt`, titre
  vérifié). Curiosité : la frame PNG post-clic est **byte-identique**
  à la frame homepage (2 276 669 o) — plausible (même header + même
  méga-menu déployé + même bandeau cookies dominent le viewport) mais
  un `diff` frame stale ne peut être exclu.
- **Télémétrie** : `dom_nodes=2 991→2 480`, `layout_boxes=1 539→1 283`,
  `journal_applies=55`, `journal_fallbacks=5` — WARN ADR-0025
  `Fingerprint`/`UnmappedKey` ×4, chacun suivi d'un apply sain (bénin).
- Cosmétique : boutons « Tout refuser »/« Tout accepter » peints en
  **blanc sur fond gris clair** (texte quasi illisible — fond de bouton
  non appliqué, famille §B-css résiduel).
- **Hygiène** : DELETE → RAM 1,11 Go → 416 Mo stable, `{"tabs":[]}`.

## Statuts proposés (§8)

| §B | Bras | Verdict ce vol |
|---|---|---|
| §B-13 | hit-test `ignored` / clic→nav | **sain sur louvre `[V4-main]`** (clic→`navigated` + `target=_blank`) ; **variante ouverte sur culture** → §B-40 |
| §B-14 | matrice APIs DOM | **+1 entrée `[V4-main]`** : `document.appendChild` « is not a function (on HTMLDocument) » (Drupal orsay) |
| §B-17 | menus JS-closables peints ouverts | **recoupement `[V4-main]`** — méga-menu culture.gouv.fr peint déployé en flux |
| §B-18 | runs inline superposés | **recoupement bénin `[V4-main]`** — carte info louvre agenda (« D » sur « Durée », « Aud… » clippé) |
| §B-26 | wedge DOM détaché post-apply | **confirmé `[V4-main]` — nouveau porteur musee-orsay.fr** : 4ᵉ apply stoppé à `layout`, `/dom` 404 `/frame` 204 terminal, `journal_fallbacks=0`, variante « pompe `js_run_sync` battante » |
| §B-30 | fuite post-DELETE sans borne | **confirmé `[V4-main]`** — orsay : `tabs=[]` → RAM 2,23→2,35 Go en ~45 s (croissance post-destruction) |
| §B-4 | morts/échecs silencieux | **recoupement `[V4-main]`** — culture : 3 évals `ok=false` au boot **sans entrée `/console`** ; mêmes erreurs tracées au chargement suivant |
| **§B-40 (NOUVEAU proposé)** | **Hit-test traverse un overlay `position:fixed`** — clic sur bouton peint de l'overlay résolu en coords document → navigation vers l'ancre sous-jacente à la position scrollée | `explorer-core`/hit-test (+`layout` pour la requête point→boîte) — preuve géométrique complète §3, culture.gouv.fr `[V4-main]` |

## Hygiène de vol

- Daemon 1 (PID 11212, port 9444) : onglets 2 (louvre agenda), 1
  (louvre), 3 (orsay) supprimés par `DELETE /api/tabs/{id}` —
  `{"tabs":[]}` confirmé ; tué par **PID** à 2,35 Go — seuil 2 Go
  franchi, arrêt d'urgence conforme §B-26/§B-30.
- Daemon 2 (PID 18316, port 9444) : onglet 1 (culture) supprimé —
  `{"tabs":[]}`, RAM retombée à 416 Mo ; tué par **PID**. Jamais
  `taskkill /IM`. Un autre `explorer-daemon.exe` (PID 17544, ~1 Go —
  vol/daemon tiers) n'a **pas** été touché.
- Artefacts : `captures/vol62_{louvre,louvre_scroll,louvre_click,
  orsay(0 o — 204),culture,culture_scroll,culture_click,
  culture_click2}.png` ; `logs/vol62_{louvre_console,
  louvre_click_console,louvre_dom,louvre_layout,louvre_mem,
  orsay_console,orsay_dom(404),orsay_layout(404),orsay_mem,
  culture_console,culture_dom,culture_layout,culture_mem,
  culture_mem2,culture_click_dom}.{json,txt}` ; journaux
  `logs/daemons/_vol62_daemon{,2}.log`.
- Limites honnêtes : la cause interne du wedge orsay (point exact entre
  `layout` et `repaint`) n'est pas bissectée depuis l'API — la métrique
  proposée `apply_started_without_fin` l'aurait marqué ; la lecture «
  hit-test ignore les fixed » est déduite de la géométrie layout, pas
  d'une instrumentation interne ; l'identité byte-à-byte des frames
  culture pré/post-clic laisse ouverte l'hypothèse d'un frame servi en
  cache.

<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 58 — Sites dev (chrome.dev · vercel · caniuse) (2026-10-27)

- **Binaire** : `[V4-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (WIP `depth_limit` embarqué), `--port 9440`,
  `EXPLORER_VIEWPORT=1366x900`, journal `_vol58_daemon.log`
  (racine, PID 10764)
- **Cibles** : developer.chrome.com · vercel.com (re-vol §B-18) ·
  caniuse.com — **3/3 volées** ; API réactive tout le vol, **deadlock
  au DELETE de clotûre** (cf. « Arrêt propre », §B-36 proposé)
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de
  `SYNTHESE.md`
- **RAM daemon (PID 10764)** : ~585 Mo (chrome.dev + io.google) →
  1 233 Mo (caniuse, `js_heap` 76 Mo) → **1 092 Mo final** — sous le
  seuil 2 Go, baisse observée après GC JS ; aucune rafale §B-26/§B-30.
- **Journal daemon** : 244 189 lignes (156k en vol, +88k pendant le wedge DELETE) ; **30 rejets de journal DOM
  (ADR-0025)** — tab1 : 1 Fingerprint + 2 UnmappedKey ; tab2 : 1
  UnmappedKey ; **tab3 (caniuse) : 24 Fingerprint + 2 UnmappedKey** ;
  **1 panic contenu** (cf. §3, proposition §B-34).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| developer.chrome.com | **Bon** — héro sombre fidèle, clic→nav réelle | `pas de journal` | **nouveau → §B-35 proposé** : item flex héro à 5 839 px → logo Chrome en ellipse aplatie + nav expulsée hors viewport (peinte en filet) ; 2 clics `ignored` marginaux (visée) |
| vercel.com | **Dégradé** — page claire au lieu de sombre, marquee cassé | `pas de journal` | **§B-18 confirmé ×3** (marquee dupliqué, tagline animée superposée, dark theme absent) ; **§B-17 confirmé variante** : méga-menu invisible intercepte le hit-test (CTA `Deploy now` → `ignored`) |
| caniuse.com | **Excellent rendu** — table CSS Grid quasi irréprochable, clic→nav + back/bfcache OK | `pas de journal` | **CRITIQUE nouveau → §B-34 proposé** : 24 rejets Fingerprint en rafale puis **panic `index out of bounds` jscore `dom.rs:2691`** (NodeId obsolète ?) → disjoncteur ouvert, JS de l'onglet mort |

Faits marquants du vol : **panic jscore rattrapé** (§B-34 proposé —
second panic du catalogue après §B-3, le premier contenu par le
garde-fou) sur re-parse ADR-0025 en rafale → `event.dispatch` ; et
**deadlock au DELETE de clotûre** (§B-36 proposé — mémoire libérée,
CPU nul, API muette).

---

## 1. developer.chrome.com — héro fidèle, sizing flex du header cassé

- `POST navigate` → `doc_h=8 336` (non clampée), titre « Chrome for
  Developers ». Console `pas de journal` ; `js_heap=0` au premier
  `/mem` puis 3,7 Mo après le premier input — instanciation paresseuse
  du moteur JS (rien d'évalué avant interaction ?).
- **Frame** (`captures/vol58_chrome_dev.png`) : thème sombre, « A
  Powerful Web. Made Easier. », carte « 15 updates from Google I/O
  2026 » — rendu convaincant.
- **Télémétrie** : `dom_nodes=2 466`, `layout_boxes=691`,
  `images_bytes=24 Mo`, `journal_fallbacks=1`.
- **Scroll ×2** → `scroll_y=1 600`, repaint correct
  (`captures/vol58_chrome_dev_scroll.png`).

### Constats

- **NOUVEAU → §B-35 proposé** — `explorer-layout`/`css` : le premier
  item flex du header est layouté à **5 839 px de large** —
  `block<div> [24 6 5839x36]` → `block<a> [24 10 5839x28]` →
  `block<img> [24 10 5839x28]` (`logs/vol58_chrome_dev_layout.txt`
  l.11-14). Conséquences visibles : le logo Chrome est peint en
  **ellipse aplatie** pleine largeur (`captures/vol58_chrome_dev_
  header.png`) et la nav est expulsée à **x=5 879, hors viewport**
  (`block<nav> [5879 0 197x24]` l.18) — on n'en voit qu'un filet de
  traits à droite. Suspect : `flex-grow`/`min-content` ou un `calc`
  déchaîné sur l'item flex ; la valeur 5 839 ressemble à une somme
  d'intrinsèques.
- **Clics** : deux `ignored` à (65,393) et (70,450) sur « Watch here »
  — boîte du lien `[32 2051 77×20]` soit viewport y 451–471 à
  scroll_y=1 600 : visée marginale en bord de boîte, **pas** un retour
  §B-13. Le troisième clic (70,460), centré dans la boîte →
  `navigated` → `io.google/2026` (vraie navigation, `can_back=true`).
- **io.google/2026** (`captures/vol58_io_google.png`) : rendu très
  fidèle (logo dégradé, nav, « Catch up on Google I/O »). Console :
  `unhandledrejection: TypeError: cannot read 'call' of Undefined
  (×3)` → **§B-14**, cascade à nommer.
- `journal_fallbacks` : 3 rejets bénins (Fingerprint ×1, UnmappedKey
  ×2) — repli re-parse fonctionnel, aucun impact visible (famille
  ADR-0025, recoupe §B-26 **sans** wedge).

## 2. vercel.com — re-vol §B-18 : marquee, tagline et dark theme toujours ouverts

- `POST navigate` → `doc_h=7 186`, titre « Agentic Infrastructure -
  Vercel ». Console `pas de journal`, `js_heap=2,7 Mo`,
  `dom_nodes=1 337`, `images_bytes=135 Mo` (!).

### Constats

- **§B-18 confirmé `[V4-main]` — marquee dupliqué** — le bandeau de
  logos clients (Meta, DoorDash, OpenAI, SpaceX, The Weather Company,
  Polymarket) est **peint en double se chevauchant** au centre du
  viewport (`captures/vol58_vercel.png`) : la 2ᵉ copie du marquee n'est
  pas concaténée mais recouvre la 1ᵉ.
- **§B-18 confirmé — runs superposés** — la tagline animée « To ship
  apps and agents / For coding agents » : **3 variantes de
  l'animation** sont toutes layoutées et peintes au même endroit
  (`logs/vol58_vercel_layout.txt` l.560-567 : `inline<span>` y=91,
  `block<span>` y=88, `block<span>` 1×1 y=111) → texte brouillé
  « To Bhipeodipg aod agents » à l'écran.
- **§B-18 confirmé — dark theme absent** — vercel.com est sombre en
  réalité ; le moteur peint en clair (classe/theme posé par JS ou
  `prefers-color-scheme` non appliqué).
- **§B-17 confirmé `[V4-main]` — variante hit-test** — le méga-menu
  des onglets Products/Resources est **layouté mais invisible** :
  `block<nav> [51 -114.5 1904×293]` (l.13) couvre le héro jusqu'à
  y=178,5. Un clic **centré** sur le CTA `<a> Deploy now`
  `[461 156 444×40]` à (683,176) → **`ignored`** : l'overlay
  transparent (probablement `opacity:0`/état JS fermé) reste
  hit-testé par-dessus le lien. Recoupe §B-13 (élément sous overlay
  reste `ignored`) — ici l'overlay n'est même pas peint.
- **Widget « New AI chat »** peint ouvert sur ~75 % de la largeur en
  bas de page (`captures/vol58_vercel.png` + `_scroll.png`) — sur le
  site réel c'est une pilule repliée → famille §B-17 (état JS non
  appliqué).
- Scroll ×2 OK ; contenu défilé rendu correctement mais sections
  « Features » aplaties en liste simple (grilles/cartes du site réel
  non reproduites — dégradation, pas un blocage).

## 3. caniuse.com — meilleur rendu du vol… et panic contenu dans jscore

- `POST navigate` → `doc_h=2 836`, titre « Can I use… Support tables
  ». Console `pas de journal` ; `dom_nodes=754`, `js_heap=76 Mo`
  (le plus élevé du vol).
- **Frame** (`captures/vol58_caniuse.png`) : **très fidèle** — thème
  sombre natif, bannière orange « Can I use ? », trois colonnes
  Latest/Most searched/Test a feature, liens orange. L'`<input>`
  recherche est layouté `[502 92 510×61]` mais peint invisible
  (transparent sur orange — fidèle au site).
- **Clic « CSS Grid »** (`[498 461 67×22]`, clic (532,472)) →
  `navigated` → `caniuse.com/css-grid` — hit-test sain sur cette
  cible.
- **Table CSS Grid** (`captures/vol58_caniuse_grid.png`) : **quasi
  irréprochable** — cellules vertes/rouges par version sur 15
  navigateurs, « Global usage 97.25 % + 0.21 % = 97.46 % », underline
  de l'input visible. Résidu : débordement léger à droite (colonne
  Baidu tronquée), chevauchements mineurs d'en-têtes (« UC Browser
  for Android »).
- **Back** → bfcache restauré (`can_forward=true`), page repeinte.

### Constats

- **NOUVEAU CRITIQUE → §B-34 proposé** — `explorer-jscore` : pendant
  le séjour sur caniuse, le journal enregistre **24 rejets
  `why=Fingerprint`** (ADR-0025 → re-parse à quasi chaque cycle
  apply, ~toutes les 4-11 s), puis à 09:44:15.5 :

  ```
  panic panicked at crates\explorer-jscore\src\dom.rs:2691:33:
  index out of bounds: the len is 6 but the index is 187
  ```

  dans la phase `event.dispatch` (tab=3, `pump.offlock` 80 ms juste
  avant, `event.dispatch` 523 ms, `event.mirror_sync` 458 ms).
  **Le panic est rattrapé** : processus et API indemnes — mais le
  disjoncteur moteur s'ouvre → 7 `script sauté — disjoncteur moteur
  ouvert` (66 o à 331 Ko), `js_heap` retombe à 0 : le JS de l'onglet
  est mort. Hypothèse : un `NodeId` du mini-DOM jscore émis avant le
  re-parse est réutilisé après — index 187 dans un vecteur de 6 —
  violation de l'invariant ADR-0008 (« handle obsolète = erreur
  typée, jamais d'accès invalide »). Repro : `logs/
  vol58_caniuse_dom.txt` + `_vol58_daemon.log` l.73617-73652.
  **Second panic jscore du catalogue** après §B-3 — le premier
  rattrapé, sur un chemin neuf (re-parse + dispatch, pas récursion).
- Les **24 rejets Fingerprint** eux-mêmes méritent attention :
  caniuse fait muter son DOM en continu (JS) → chaque apply est
  rejeté → re-parse complet ×24 sur ~30 s — coût + risque (c'est ce
  bras qui a mené au panic). Même famille ADR-0025 que §B-26 mais en
  version « rafale » — à corréler avec les métriques proposées
  (`apply_started_without_fin`).

## Arrêt propre — et wedge de clotûre

- `DELETE /api/tabs/1` émis → **jamais retourné (>6 min)**. Pendant
  ~4 min le journal a produit 88 000 lignes `sync_profile`
  (libération tab), puis silence total à 10:49:55. Mesures après :
  **RAM 1 092 → 55 Mo** (mémoire bien libérée), **CPU figé**
  (125,7 s constants sur 8 s — aucun spin), port 9440 toujours
  LISTENING… mais `GET /api/tabs` **timeout >20 s** et le DELETE
  en vol ne rend jamais. Diagnostic : **deadlock** — handler DELETE
  bloqué sur un verrou/canal, tous les endpoints sérialisés derrière.
  → **§B-36 proposé** (distinct de §B-33 `nav_paint` : ici ni spin ni
  rafale — processus garrotté à sec après libération mémoire ;
  à rapprocher aussi de §B-30, DELETE en vol, mais sans fuite).
- Arrêt du daemon garrotté par **son PID** (10764) —
  `Stop-Process -Id 10764`. Aucun autre process touché (daemons
  voisins d'autres sessions épargnés).

## Fichiers produits

| Fichier | Contenu |
|---|---|
| `captures/vol58_chrome_dev.png` / `_scroll.png` / `_header.png` | héro sombre, zoom header (logo 5 839 px, nav hors viewport) |
| `captures/vol58_io_google.png` | navigation réelle post-clic |
| `captures/vol58_vercel.png` / `_scroll.png` | marquee dupliqué, tagline superposée, méga-menu invisible, chat ouvert |
| `captures/vol58_caniuse.png` / `_scroll.png` / `_grid.png` | home + table CSS Grid quasi fidèle |
| `logs/vol58_{chrome_dev,vercel,caniuse}_{dom,layout}.txt` | télémétrie — preuves §B-34/§B-35/§B-18/§B-17 |
| `_vol58_daemon.log` | 244 189 lignes — panic l.73652, rejets journal, disjoncteur |

## Propositions §B (numérotation suivante)

| # proposé | Anomalie | Crate | Statut proposé |
|---|---|---|---|
| **§B-34** | **Panic `index out of bounds` jscore `dom.rs:2691` après rafale de re-parses ADR-0025 (Fingerprint)** — NodeId obsolète indexé (len 6, idx 187) pendant `event.dispatch` ; rattrapé mais disjoncteur ouvert → JS de l'onglet mort | `explorer-jscore` | OUVERT — terrain `[V4-main]` ; viol ADR-0008 (handle obsolète doit être erreur typée) ; repro caniuse.com ~30 s |
| **§B-35** | **Item flex du header à 5 839 px** — `block<a>`/`<img>` logo étirés hors de tout conteneur (nav expulsée à x=5 879) | `explorer-layout`/`css` | OUVERT — terrain `[V4-main]` ; repro developer.chrome.com |
| **§B-36** | **DELETE onglet → deadlock daemon** — mémoire libérée (1 092→55 Mo), CPU nul, port LISTENING, API muette >20 s | `explorer-core`/`api` | OUVERT — terrain `[V4-main]` ; à distinguer de §B-33 (pas de spin) et §B-30 (pas de fuite) |

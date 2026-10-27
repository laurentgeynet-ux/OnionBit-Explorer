# Rapport de vol — Agent 37 — Divers (e-commerce anti-bot, SPA, legacy, crypto-app)

- **Date** : 2026-10-27
- **Binaire** : `[V4-main]` = `C:/obe-vol-target/release/explorer-daemon.exe`
  (build 08:35, HEAD `f5825f5`)
- **Viewport** : 1366×900 (`EXPLORER_VIEWPORT`) — port 9422 — daemon PID 11848
- **Protocole** : `docs/vol-agent.md` — frame PNG = verdict terrain
- **Caveat fraîcheur (§8)** : l'arbre principal porte du **WIP non commité**
  dans `crates/explorer-core/src/session/net.rs` + `tests/session.rs`
  (chantier J3 — modules `data:` décodés inline dans le MapLoader), timestamps
  08:35:30 / 08:36:24 **à cheval sur la build du binaire** (08:35). Le binaire
  peut donc contenir tout ou partie de ce chantier : les constats touchant
  §B-5/§B-6 (MapLoader, `data:`) sont à re-valider sur arbre propre.
- **Contexte processus** : deux daemons tiers actifs en parallèle
  (PID 17828 :9460, PID 14552 :9421) — non touchés ; mon daemon = PID 11848.

---

## 1. leboncoin.fr — https://www.leboncoin.fr

- **navigate** → `{"error":{"handled":true,"message":"HTTP 403"}}` — Datadome.
- **Titre** : « Erreur de chargement » ; `dom_nodes`=0, `layout_boxes`=18.
- **Console** (verbatim, 1 entrée) : `HTTP 403`
- **Constats mesurés** (`vol37_leboncoin.png`) :
  - Page d'erreur interne du moteur **proprement peinte** : « Page
    inaccessible — La page https://www.leboncoin.fr/ n'a pas pu être chargée.
    Le serveur a répondu « HTTP 403 ». Réessayez plus tard, ou revenez à la
    page précédente. » — dégradation gracieuse correcte, aucun artefact.
  - Scroll → `scroll_y` reste 0.0 (page = viewport) ; clic → `ignored`
    (aucune cible cliquable, comportement normal).
- **Classification** : externe — **Datadome confirmé, catalogue à jour**
  (§B-externe : « Datadome (leboncoin) »). Aucun constat moteur.
- **Sévérité** : nulle (hors moteur).

## 2. figma.com — https://www.figma.com

- **Titre** : « Figma: The collaborative canvas for design, code, and AI »
- **doc_h** : 7 344,6 px (non clampé) ; `dom_nodes`=1 687, `js_heap`=78,5 Mio,
  `images_bytes`=60,7 Mio, 108 `<script>` dans le DOM.
- **Console landing** : **404 « pas de journal »** — zéro entrée malgré 108
  scripts ; le log daemon montre des rafales `script: eval fin ok=true`
  (~1 Ko par éval, modules) → le JS **tourne partiellement sans erreur
  tracée**, pas une mort au premier script.
- **Constats mesurés** (`vol37_figma.png`, `vol37_figma_scroll.png`) :
  - **Méga-menus peints ouverts** dès l'atterrissage : colonne gauche des
    rubriques (Design systems, Prototyping, UX design, Web design,
    Wireframing, Online whiteboard, Agile, Strategic planning, Brainstorming,
    Diagramming, Product development, Web development, Design handoff…)
    étalée en plein écran + colonne droite de cartes produits (FigJam,
    Figma Slides, Figma Draw, Figma Sites Beta, Figma Buzz Beta, Figma
    Motion, Figma Make, Figma Weave). Au scroll : second menu
    (Community/Resources : Education, Government, Events, User groups,
    Customer stories, Plugins, UI kits…) également ouvert. → **§B-17**
  - **Chevauchements massifs de runs inline** : « Prototyping » peint sur
    « Design syste… », « Strategic plannir » sur « rategic plannin »,
    « Product developmen… » sur « Web developeme… », « Figma Support »
    sur « Figma Make », « Plugins »/« UI kits »/« Figma Motion »
    superposés — même classe qu'excalidraw/gitlab/DW/deezer. **Reproduit
    sur V4.** → **§B-18**
  - **Troncatures de colonnes** : « Design syste », « Prototypin »,
    « UX design » clippés à ~100-140 px (overflow partiellement appliqué).
  - **Vraies images peintes** : tuiles produits illustrées (Facet, widgets
    « +64° -36° -78° », « Field Greens »), illustration « berceau de
    Newton » au scroll — sous-ressources images OK.
  - Hero visible sous les overlays : « The full-stack creative canvas »,
    « Move fast in the right direction on an AI-native canvas. Teammates
    and AI agents work in the same space… ».
- **Clics** : (222,411 sur carte FigJam) → `ignored` ; (75,259 sur
  « Best practices ») → **`{"result":"navigated"}`** →
  `https://www.figma.com/best-practices/` → **§B-13 confirmé sain** (clic =
  vraie navigation sur V4 aussi).
- **Page /best-practices/** (`vol37_figma_click.png`) : rendu **net** — nav
  repliée correcte (« Products Solutions Community Resources Pricing » +
  « Log in »/« Contact sales »), H1 « Figma Best Practices », section
  « Best Practice Guides », 3 cartes guides en bas (fond noir, images non
  résolues). Au scroll (`vol37_figma_bp_scroll.png`) : méga-menus
  Community/Resources encore ouverts par-dessus le contenu + illustration
  mains/nœuds peinte.
- **Console post-nav** (verbatim, 1 entrée) :

```
unhandledrejection: TypeError: `call` is not a function (on Array)
    at <anonymous> (inline:10:1:69900)
    at <anonymous> (inline:10:1:69841)
    at <anonymous> (inline:10:1:69900)
    at <anonymous> (inline:10:1:70065)
    at <anonymous> (inline:1:18:62075)
    at t (inline:1:18:61453)
```

  Signature nouvelle dans la matrice → **§B-14** (cascade à nommer :
  `call` invoqué sur Array — `Function.prototype.call` manquant ou
  dispatch `.call` mal résolu sur un tableau).
- **§B-5 non reproduit** : aucun « module non chargé » en console
  (avec le caveat WIP J3 ci-dessus).
- **Classification** : SSR + images fonctionnels ; rendu saboté par overlays
  ouverts + collisions inline ; JS vivant mais hydratation incomplète.
- **Sévérité** : haute (page illisible sous les menus).

## 3. meteociel.fr — https://www.meteociel.fr

- **Titre** : « Meteociel - Météo - observations météo en temps réel et
  prévisions météo pour la France »
- **doc_h** : 4 164,9 px ; `dom_nodes`=2 217, **13 `<table>`** / 26
  cellules tr/td — vrai site à tables héritées ; `journal_fallbacks`=1
  (warn `journal DOM rejeté → repli re-parse (ADR-0025) why=UnmappedKey`
  dans le log — repli exercé, page servie).
- **Console** : vide (« pas de journal ») — landing **et** page article ;
  site quasi-statique, bilan normal.
- **Constats mesurés** (`vol37_meteociel.png`, `vol37_meteociel_scroll.png`) :
  - Chrome du site rendue : fond bleu dégradé, logo « Meteociel.fr » en
    script (image peinte), slogan « Observations, Prévisions, Modèles En
    temps réel », icônes Twitter/Facebook peintes, compteur « 12715
    visiteurs » (puis « 12661 » sur l'article — valeur dynamique servie).
  - **Menu latéral déplié en colonne centrée** (~150 px de large au centre
    du viewport) : Accueil, Inscription, Se connecter, Meteociel
    Android/iOS, Forums, Tchat météo (4), Temps réel, Observations, Poster
    vos obs., Température/min/max/mer, Point de rosée, Windchill, Humidex,
    Isobares, Pression, Vent moyen, Rafales, Nébulosité, modèles NWP
    (WRF ARW 0.08°, ALARO 4km, ICON-D2/-RUC, ICON CH1/CH2, ICON 2I/BOLAM,
    UKMO HD, HARMONIE, HRRR 3km) **avec drapeaux peints** (FR/BE/ES/DE/UK/US
    — vraies images). Le site réel place cette nav en **colonne gauche**
    d'une table ; ici la colonne est centrée et le contenu passe
    en dessous en pleine largeur → la structure à deux colonnes de la
    table externe s'est **empilée** au lieu de s'aligner. → **§B-18**
  - **Table des actualités correctement rendue** (bordures, lignes) :
    « 24/08/2026 : Tous les outils pour suivre les orages en direc … »,
    « 01/08/2026 : Eclipse solaire du 12 août 2026 », « 07/01/2026 : Jeudi
    8 janv. 2026 : Tempête Goretti », « 05/01/2026 : Ajout du modèle
    CEP/ECMWF 9km », « 15/12/2025 : Ajout du modèle AIGFS de la NOAA »…
  - **Carte d'observations temps réel réellement peinte** : synoptique
    France avec flèches de vent, températures chiffrées, légende H/H-1,
    « Lundi 27 octobre 2026 08:40 Observations » — pipeline image OK sur
    carte météo dynamique.
  - Section « Observations météo » : onglets France / Régions France /
    DOM- / Europ / Reste du monde + pastilles régions (Auvergne-Rhône-Alpes,
    Bretagne, PACA…) — **troncatures de libellés** (« Franc », « Europ »,
    « Archives » coupés) ; léger chevauchement « Coupes ARPEGE » /
    « sondages ARPEGE » sur les lignes d'actus à droite.
- **Clic** : (300,385 ligne actu « Tempête Goretti ») →
  **`{"result":"navigated"}`** →
  `/actu/38/jeudi_8_janv__2026___tempete_goretti/` — article rendu, même
  sidebar centrée (`vol37_meteociel_click.png`).
- **Classification** : bon rendu legacy — tables, images, carte synoptique,
  navigation ; défaut principal = colonnade de la table externe empilée.
- **Sévérité** : moyenne (lisible, navigable, mise en page altérée).

## 4. cryptpad.fr — https://cryptpad.fr

- **Titre** : « CryptPad: Collaboration suite, encrypted and open-source »
- **doc_h** : **8,0 px** au retour de navigate — coquille vide, app JS
  non montée ; `dom_nodes`=124, `layout_boxes`=22, `display_items`=10,
  `js_heap`=59,7 Mio, `journal_fallbacks`=2 (`why=Fingerprint` dans le log).
- **Console** : vide à +12 s ; à +30 s, **1 entrée** :

```
unhandledrejection: RangeError: eval deadline exceeded
    at <anonymous> (/lib/less.min.js?ver=2026.9.0-1792764980734:11:136874)
    at <anonymous> (/lib/less.min.js?ver=2026.9.0-1792764980734:11:137825)
    at <anonymous> (/lib/less.min.js?ver=2026.9.0-1792764980734:11:94927)
    at <anonymous> (/lib/less.min.js?ver=2026.9.0-1792764980734:11:74148)
    at <anonymous> (/lib/less.min.js?ver=2026.9.0-1792764980734:11:73342)
    at <anonymous> (/lib/less.min.js?ver=2026.9.0-1792764980734:11:73117)
    at <anonymous> (/lib/less.min.js?ver=2026.9.0-1792764980734:11:135199)
    at <anonymous> (/lib/less.min.js?ver=2026.9.0-1792764980734:11:135693)
```

- **Constats mesurés** (`vol37_cryptpad.png`, `vol37_cryptpad_2.png`) :
  - **Écran de pré-chargement peint et figé** : « Loading… », logo
    CryptPad, « End-to-end encrypted » + cadenas — l'app ne dépasse
    jamais le splash (frames identiques à +12 s et +30 s).
  - **RequireJS s'exécute et injecte ses modules** : le dump DOM contient
    55 `<script>` dont les balises runtime `data-requirecontext="_"` /
    `data-requiremodule` (`/common/boot.js`, `/api/config`, `boot2.js`,
    `requireconfig.js`, `application_config.js`…) — l'injection dynamique
    de scripts dans le DOM fonctionne.
  - **La deadline wall-clock d'éval (JS-C164/J1) tue `less.min.js`** :
    le compilateur LESS in-browser dépasse le budget et son rejet est
    correctement tracé en `unhandledrejection`. **L'API reste intégralement
    responsive pendant et après** — le garde-fou fait son office (pas de
    starvation type §B-1), mais le boot CryptPad dépend de cette compile :
    l'app reste sur « Loading… ».
  - Scroll ×2 → `scroll_y` 0.0 ; clic centre → `ignored` (splash sans
    lien). Daemon sain, autres onglets servis en parallèle.
  - AES-GCM : non atteint (le boot meurt avant WebCrypto) ; phase
    `commit.dl_crypto` présente dans le profil de pompe, rien d'anormal.
- **Classification** : boot JS interrompu par le garde-fou — **constat
  nouveau** : la deadline protège le daemon mais ampute un workload
  légitime de compilation → proposition **NOUVEAU §B-23** (« deadline
  d'éval tue les évals longues légitimes — less.min.js cryptpad ;
  étalonner le seuil ou exempter la phase de boot »), `explorer-core`/
  `explorer-jscore`, en lien direct J1/N3.
- **Sévérité** : haute pour le site (inutilisable), positive pour le
  garde-fou (démontré en terrain : traçage + survie API).

---

## Constats actionnables

1. **NOUVEAU §B-23 — `eval deadline exceeded` tue un workload légitime**
   (`explorer-core`/`explorer-jscore`, OUVERT) : cryptpad.fr figé sur
   « Loading… » après `RangeError: eval deadline exceeded` dans
   `less.min.js` — JS-C164/J1 démontré en terrain (rejet tracé, API
   100 % responsive, autres onglets servis) mais le seuil coupe un script
   de boot indispensable. Étalonner le budget (compilation LESS in-page =
   workload normal) ou exempter/majorer les évals de la phase de boot.
   Console `logs/vol37_cryptpad_console.json`, capture
   `captures/vol37_cryptpad.png`.
2. **§B-17/§B-18 reproduits sur V4** (`explorer-layout`/`jscore`) :
   méga-menus figma peints ouverts (2 menus distincts, landing + page
   interne) + chevauchements massifs de runs inline — captures
   `vol37_figma.png`, `vol37_figma_scroll.png`, `vol37_figma_bp_scroll.png`.
3. **§B-14 — nouvelle signature** : `TypeError: 'call' is not a function
   (on Array)` (figma /best-practices/) — à ajouter à la matrice J5.
   Console `logs/vol37_figma_console.json`.
4. **§B-18 — table externe empilée** (meteociel) : la colonne nav se peint
   centrée au lieu de la cellule gauche — layout table 2 colonnes non
   reproduit. Capture `vol37_meteociel.png`.
5. **§B-13 confirmé sain** : clics → vraies navigations ×2 (figma
   /best-practices/, meteociel actu) — cohérent « PROBABLEMENT RÉSOLU ».
6. **§B-externe confirmé** : Datadome leboncoin → 403 + page d'erreur
   interne propre. Capture `vol37_leboncoin.png`.
7. **Points positifs mesurés** : deadline = starvation évitée (J1 livré,
   validé terrain) ; RequireJS injection dynamique DOM OK ; tables héritées
   + images + carte synoptique + drapeaux peints (meteociel) ; console
   reset/404-propre ; `journal_fallbacks` ADR-0025 exercés sans casse
   (UnmappedKey ×1, Fingerprint ×2).

## Arrêt

- Onglets 1-4 supprimés via `DELETE /api/tabs/{id}`.
- Daemon **PID 11848** tué par PID — daemons tiers 17828/14552 intacts.
- Aucun code modifié. Artefacts : `captures/vol37_*.png` (10),
  `logs/vol37_*_{dom,layout,console,mem}.*`, daemon `_vol37_daemon.log`
  (racine dépôt).

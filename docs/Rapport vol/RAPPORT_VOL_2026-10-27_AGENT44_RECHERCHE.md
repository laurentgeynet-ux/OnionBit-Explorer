<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 44 — Moteurs de recherche & streaming (2026-10-27)

- **Binaire** : `[V4-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (WIP `depth_limit` embarqué, PID 14576)
- **Daemon** : `--port 9426`, `EXPLORER_VIEWPORT=1366x900`, journal `_vol44_daemon.log`
- **Cibles** : deezer.com/fr/ (re-vol boot) · qwant.com (champ recherche,
  §B-13) · lite.duckduckgo.com (formulaire minimal, soumission)
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de `SYNTHESE.md`

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| deezer.com/fr/ | **Boot progressé — page réelle + clic→nav** | `unhandledrejection` ×3 + 1 `exception js` (tiers survicate) | §B-18 résiduel (chevauchements) ; boot non silencieux |
| qwant.com | **Contenu réel, champ occlus par hero `<img>`** + **nav auto script** | vide | §B-18 overlap → §B-13 conséquence ; **NOUVEAU §B-24** redirection `location` non sollicitée |
| lite.duckduckgo.com | **Succès complet** : saisie → Enter → SERP → clic → Wikipedia | vide | §B-13 RÉSOLU confirmé ; soumission GET validée bout-en-bout |

Acquis majeurs : **soumission de formulaire validée de bout en bout pour la
première fois en vol agent** (focus par clic → 8 frappes `edited` → Enter
→ `navigated` → SERP réelle → clic résultat → article Wikipedia) ; hit-test
sain sur ancres réelles ×3 (deezer offres, ddg SERP, ddg→wikipedia) ;
`unhandledrejection` fidèle sur deezer ; zéro panic, zéro freeze, API
responsive en continu.

---

## 1. deezer.com/fr/ — re-vol : le boot a progressé `[V4-main]`

- Navigate : `doc_h=8085.77`, titre réel « Deezer Musique | 2 Mois
  gratuit | App de streaming ».
- `/console` verbatim :

```
unhandledrejection: TypeError: cannot read `settings` of Undefined
    at <anonymous> (…/widget_core-29.0.15.js:1:493282)  [survicate-cdn]
unhandledrejection: TypeError: `filter` is not a function (on function n@1:356480)
    at <anonymous> (…/widget_core-29.0.15.js:1:328687)  [survicate-cdn] (×2)
[post-nav /fr/offers]
exception js : TypeError: cannot read `name` of Undefined
    at AR (inline:7:1:41376) at rc (inline:7:1:77610)
```

- Frame (`captures/vol44_deezer.png`) : header sombre complet (nav
  Offres/Découvrir/Musique, boutons Connexion/Inscription), hero « Là où
  ta musique prend vie », CTA violet, étoiles d'avis — fidèle au site.
- Scroll ×2 → `scroll_y=1600` réel (`vol44_deezer_scroll.png`) :
  cartes d'offres peintes (✓/✗ par plan), décor SVG, sections — contenu
  réel en profondeur.
- **Clic « Découvrir toutes nos offres » (683,283) → `navigated`** :
  `/fr/offers`, titre « Deezer Abonnement | 2 Mois gratuits », hero
  « Profite de 2 mois Premium offerts » peint (`vol44_deezer_offres.png`).
  → **§B-13 : confirmation de résolution `[V4-main]`**.
- Mem : 1 832 nœuds DOM, 596 boîtes, images 77 Mo, heap JS 38 Mo.

**Constat** : le boot JS n'est plus silencieux ni bloquant — les seules
erreurs proviennent du **widget tiers survicate** (rejets tracés via
`unhandledrejection`, JS-C154) et d'une exception inline post-nav ;
l'application elle-même rend, route et navigue. Chevauchements résiduels
visibles : bandeau sticky « S'inscrire gratuitement » peint par-dessus le
contenu au scroll, bouton « Explorer notre catalogue » superposé au titre
→ `explorer-layout` → **§B-18** (famille chevauchements / `sticky`
repaint), mineur.

## 2. qwant.com — champ recherche présent mais occlus `[V4-main]`

- Navigate : `doc_h=1678`, titre réel. `/console` : **vide** (« pas de
  journal ») — tous les scripts évalués `ok=true` côté journal daemon.
- Frame (`captures/vol44_qwant.png`) : logo Qwant⁺, tagline, hero
  smartphone (vraie image), panneau de suggestions, section « Discover
  the Qwant Chat » — contenu réel ; layout dégradé (sidebar déplacée,
  chevauchements).

### Champ recherche : éditable mais **inatteignable par clic** → §B-18 → §B-13

- DOM : `<form>` + `<input type="search" role="searchbox"
  aria-label="Enter your search term">` présents (dom ligne 1132+).
- Layout : `block<input> [802 228 614x56]` sous `block<form> [782 228
  690x56]` — boîte correctement dimensionnée.
- **Mais** `block<img> [490.8 68 884x772]` (hero smartphone, ligne
  layout 241 — sous-arbre postérieur, donc peint **au-dessus**) recouvre
  la quasi-totalité du formulaire.
- Mesures hit-test :
  - `mousemove`/`click` (1000,256) → `editable:false` / `ignored`,
    journal `hit=true` — le hit-test tombe sur l'image (comportement
    correct : il teste ce qui est peint dessus) ;
  - `click` (1395,256) sur la bande libre 1375-1416 → `ignored`,
    `hit=false` — **rien** sous ce point : l'input ne produit aucun
    display item hors de son texte placeholder (fond transparent) →
    zone morte.
- Conséquence : le champ **ne peut pas recevoir le focus par clic** →
  saisie et Enter non testables sur cette cible. Le bug racine est
  **layout** (chevauchement hero/formulaire → §B-18), pas hit-test —
  §B-13 reste sain, c'est sa conséquence visible ici.

### NOUVEAU — navigation script spontanée vers `/mobile-apps` → §B-24 (proposition)

- Sans aucun input utilisateur, l'onglet est passé de `qwant.com/` à
  `qwant.com/mobile-apps?utm_campaign=w2m&utm_source=qwantcom
  &utm_medium=panorama` (`can_back:true`, titre « Search safely with the
  Qwant app! » — constaté au relevé d'onglets ~4 min après nav).
- Nav **unique**, API restée responsive, pas de boucle → §B-2
  (`max_auto_navs`) tient — ce n'est pas une régression de la boucle.
- Le site réel ne redirige pas un desktop vers la page app : suspect =
  **détection d'environnement JS** (UA `Explorer/0.1`, API
  `navigator.*`/`userAgentData` absente ou stub retournant une valeur
  qui arme la branche mobile) → `explorer-jscore`/glue, à bissecter ;
  recoupe la matrice §B-14. Page cible elle-même bien rendue
  (`vol44_qwant_mobileapps.png`, badges stores, footer).
- Scroll ×2 : `scroll_y=1600` — captures `vol44_qwant_scroll.png` et
  `vol44_qwant_mobileapps.png` **byte-identiques** → la nav auto a eu
  lieu entre le clic (1395) et les scrolls ; la vue scrollée montrait
  déjà la page mobile-apps (repeint correct après nav).

## 3. lite.duckduckgo.com — formulaire validé bout-en-bout `[V4-main]`

- Navigate : `doc_h=507.9`, titre réel, `/console` vide.
- Frame (`vol44_ddg.png`) : logo, `<input>` bordé (337,434 — 614×40)
  et bouton « Search » — conforme.

### Séquence complète : focus → saisie → Enter → SERP

| Étape | Mesure |
|---|---|
| `click` (600,455) | `{"result":"focused"}` — champ éditable trouvé |
| 8 × `key` « explorer » | 8 × `{"result":"edited"}` — insertion réelle |
| Frame intermédiaire | `vol44_ddg_typed.png` : « explorer\| » peint, **caret visible** |
| `key` Enter | `{"result":"navigated"}` — soumission implicite `<form>` |
| Résultat | titre « explorer at DuckDuckGo », `lite.duckduckgo.com/lite/` — **SERP réelle** |

- SERP (`vol44_ddg_serp.png`) : 7+ résultats réels (Internet Explorer,
  Explorer++, File Explorer…) avec liens bleus, extraits, URLs — rendu
  propre.
- **Clic résultat « Internet Explorer - Wikipedia » (160,734) →
  `navigated`** → `en.wikipedia.org/wiki/Internet_Explorer` rendu
  quasi-parfait (`vol44_ddg_wikipedia.png`) : TOC latérale, infobox avec
  vraies images (logo IE + screenshot), liens inline — seul résidu :
  chevauchement hatnote « Appearance / For other uses » en tête
  (§B-18, mineur).
- bfcache : 2 pages retenues, `can_back:true` — historique fonctionnel.
- Scroll : non applicable (doc_h < viewport sur l'accueil) ; scrolls
  testés sur SERP implicites via nav.

**Constat** : chaîne formulaire complète **RÉSOLUE confirmée
`[V4-main]`** — §B-13 fermé sur les deux jambes (hit-test champ +
ancres), soumission GET `?q=` opérationnelle, navigation SERP→résultat
réelle. Première validation agent du pipeline `click→focus→key×N→Enter
→submit→nav`.

## 4. Télémétrie / observabilité

- `_vol44_daemon.log` : **0 ERROR / 0 panic** ; 5 `WARN` = rejets
  journal ADR-0025 (`Fingerprint` ×1, `UnmappedKey` ×4, tab 1) — repli
  re-parse fonctionnel à chaque fois.
- `/dom`, `/layout`, `/frame`, `/console`, `/mem` : 200 systématiques,
  aucune alternance 404.
- **RAM daemon (PID 14576)** : 364 Mo au boot → 769 Mo (deezer) →
  1 349 Mo (deezer+qwant) → 1 573 Mo (3 onglets, post-wikipedia).
  Croissance continue mais linéaire avec le nombre d'onglets lourds
  (deezer seul = 77 Mo d'images + 44 Mo framebuffer + 38 Mo heap JS) ;
  API restée responsive — surveillance tenue, pas de fuite explosive
  observée. Point de vigilance post-crash noté.
- `sync_profile` : phases `commit.*` à quelques centaines de µs —
  aucune saturation post-apply (§B-1 non reproduit sur ces cibles).

## 5. Actions proposées

| # | Proposition | Cible |
|---|---|---|
| §B-24 | **Redirection `location` spontanée** (qwant → /mobile-apps sans input — détection env JS suspecte) — nouvelle entrée | `explorer-jscore`/glue |
| §B-18 | + hero `<img>` recouvrant le `<form>` recherche (qwant) ; bandeau sticky peint sur contenu + bouton sur titre (deezer) ; hatnote Wikipedia | `explorer-layout`/`css` |
| §B-13 | **RÉSOLU confirmé `[V4-main]`** — clic→nav ×3 (deezer offres, SERP ddg→wiki, ancres standard) + champ éditable focus/saisie/Enter → SERP | `explorer-layout`/`core` |
| §B-4 | Deezer : boot progressé, erreurs tiers tracées — pas de mort silencieuse | `explorer-jscore` |

**Statuts** : §B-13 `RÉSOLU — confirmé [V4-main]` (soumission incluse) ·
§B-18 `PARTIEL — recoupe V4` (overlap formulaire = nouveau sous-cas
bloquant UX) · §B-4 `EN DÉCROISSANCE` · §B-24 `OUVERT — NOUVEAU`.

### Fichiers produits

- `captures/vol44_deezer.png` · `vol44_deezer_scroll.png` ·
  `vol44_deezer_offres.png` · `vol44_qwant.png` ·
  `vol44_qwant_scroll.png` · `vol44_qwant_mobileapps.png` ·
  `vol44_ddg.png` · `vol44_ddg_typed.png` · `vol44_ddg_serp.png` ·
  `vol44_ddg_wikipedia.png`
- `logs/vol44_deezer_{dom,layout}.txt` · `vol44_qwant_{dom,layout}.txt` ·
  `vol44_ddg_{dom,layout}.txt` · `vol44_ddg_serp_{dom,layout}.txt`
- `_vol44_daemon.log` (racine)

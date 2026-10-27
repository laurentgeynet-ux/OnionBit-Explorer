<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 47 — Clôture §B-10 DSFR (2026-10-27)

- **Binaire** : `[V4-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (WIP `depth_limit` embarqué), `--port 9429`, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol47_daemon.log` (racine)
- **Cibles** : legifrance.gouv.fr · service-public.fr · caf.fr —
  re-vols de clôture du bras DSFR de §B-10 (impots RÉSOLU au vol 45)
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de `SYNTHESE.md`

## Synthèse

| Cible | Verdict §B-10 | Console | Anomalies |
|---|---|---|---|
| legifrance.gouv.fr | **PARTIEL — mur fatal levé, instrumentation quasi nulle** | **vide** | §B-4(c) recoupe + §B-17/§B-18 |
| service-public.gouv.fr | **RÉSOLU — DSFR initialise *et* interactionnel** | 1 erreur `replaceChild` ×6 | §B-14 (`replaceChild: not a node`) + §B-17/§B-18 |
| caf.fr | **NON TRANCHABLE — page jamais servie** | vide (1 éval `ok=false` non tracée) | **§B-24 proposé** : wedge journal/détachement DOM |

Fait marquant : pour la première fois sur une cible DSFR, un **clic
déclenche le cycle de dépliage DSFR** — `aria-expanded` bascule
`false→true`, classe `fr-collapse--expanded` injectée, variables CSS
`--collapser`/`--collapse` calculées par le JS (service-public,
menu « Fiches pratiques par événement de vie »). L'erreur historique
`value is not a function` (tuée par JS-C150) ne revient nulle part.

⚠ **PRUDENCE déclenchée** : RAM daemon 8616 franchie à **2,77→2,93 Go**
pendant le wedge caf (reload = second heap + DOM détaché, GC sans
rendu) — onglet supprimé puis daemon arrêté par sécurité.

---

## 1. legifrance.gouv.fr — DSFR amorcé mais quasi inerte → §B-10 PARTIEL

- `POST navigate` → `doc_h=8626`, titre réel « Légifrance », URL
  `/` — **page réelle servie** (contre-preuve : `curl -A Mozilla`
  direct reçoit un **challenge Cloudflare « Just a moment »** que le
  moteur n'a pas subi — la page institutionnelle est passée).
- `/console` verbatim : `pas de journal` — **0 erreur**.
- Journal daemon : **42/42 évals `ok=true`, 0 échec** — dont les
  15 `*.module.min.js` DSFR (core, scheme, card, accordion, navigation,
  breadcrumb, sidemenu, modal, header, display, tag, tile, toggle,
  checkbox, tooltip) + tarteaucitron + Eulerian (iframe injectée).
- `/dom` 288 Ko (~1550 nœuds), `/layout` 58 Ko.

### Instrumentation DSFR mesurée (DOM sérialisé, stable à +15 s)

```
data-fr-js="true"          sur <html>      ×1  ← dsfr API initialisée
data-fr-scheme="system"    sur <html>      ×1  ← scheme injecté (ou source)
data-fr-js-follow-actionee                 ×1  ← 1 instance « follow »
data-fr-opened="false"                     ×2  ← 2 déclencheurs de modale
                                               (fr-btn--menu menu-modal,
                                                fr-btn fr-icon-theme-fill)
```

Per documentation DSFR officielle : `data-fr-js="true"` sur `<html>`
est posé par l'API JS à son initialisation ; `data-fr-opened` sur les
déclencheurs et `data-fr-js-<instance>` sur chaque élément instancié
sont injectés par les instances. Boot DSFR donc **franchi** (le mur
§B-10 est tombé ici aussi), mais l'instanciation est **quasi nulle** :

- 5 `fr-collapse`/méga-menus et 5 `fr-nav__btn` : **aucun**
  `data-fr-js-collapse`/`-navigation-item`/`-modal` — là où
  service-public en porte 168 (§2).
- Clic `fr-nav__btn` « Droit national » (315,156 — centre exact de
  `block<button> [208 128.5 214x56]`) → `ignored`, `aria-expanded`
  reste `false` ×5, menu jamais ouvert (`vol47_legifrance_navclick.png`
  identique à l'initiale).
- Clic bouton « Paramètres d'affichage » (1151,64 — pourtant
  `data-fr-opened="false"` instrumenté) → `ignored`, pas de bascule.
- Contre-preuve hit-test : clic `<a>` « Accueil » (137,156) →
  **`navigated`** — le hit-test est sain ; l'event `click` est dispatché
  au miroir JS (`dispatch_dom_event`), les handlers DSFR ne répondent
  simplement pas.

→ **§B-10 bras DSFR : mur fatal levé (console vide, `data-fr-js` posé,
2 déclencheurs initialisés) mais instanciation résiduelle minimale** —
recoupe §B-4(c) « exécution réussie sans effet » : 42/42 évals ok,
zéro rejet, effet quasi nul. Hypothèses ouvertes : `register()` des
modules composants sans effet (ordre d'éval parallèle des modules ?),
inspection `querySelectorAll` partielle, ou instrumentation serveur
des `data-fr-opened`/`follow-actionee` (non prouvable — source
Cloudflare-bloquée).

### Rendu / layout → §B-17 + §B-18

- Frame (`captures/vol47_legifrance.png`) : page **largement
  déstylée** — nav DSFR non flexée (items étalés), méga-menus
  fermés avec contenu déployé, texte brut lisible sur 8 626 px
  (`vol47_legifrance_scroll.png`). Le document complet rend, la
  grille DSFR ne s'applique pas → §B-18.
- Layout : enfants des `fr-collapse` fermés déployés à
  **`y=-98 518`** (`block [0 -98518.5 249x228]`, bouton « Fermer »
  `-98502`) — rejet off-écran mais peint/posé → sous-cas §B-17
  (panneaux fermés déployés dans le layout ; ici non interceptants
  grâce au y négatif, contrairement à ratp vol 45).

## 2. service-public.fr — DSFR pleinement instrumenté + dépliage → §B-10 RÉSOLU

- `POST navigate` → redirect `service-public.gouv.fr`, titre réel
  « Accueil | Service Public », `doc_h=6373`.
- `/console` verbatim (intégral) :

```
unhandledrejection: TypeError: replaceChild: not a node
    at <anonymous> (…/assets/dsfr-1.14/dist/dsfr.module.min.js:2:45290)
    at <anonymous> (…/assets/dsfr-1.14/dist/dsfr.module.min.js:2:45170)
(×1 … ×6)
```

→ **`Node.replaceChild` rejette « not a node »** dans le bundle DSFR
1.14 (probablement le composant inject-svg — `data-fr-js-inject-svg`
×6, même cardinal) — **nouvelle entrée matrice §B-14**
(`explorer-jscore`). Tracé propre par `unhandledrejection` + dédup ×N.

### Instrumentation massive (168 attributs injectés)

```
data-fr-js="true" + data-fr-scheme="light" + data-fr-theme="light"
    + style="color-scheme:" sur <html>      ← core + scheme exécutés
data-fr-js-footer-link-actionee            ×41
data-fr-js-navigation-link-actionee        ×22
data-fr-js-card-actionee                   ×18
data-fr-js-button-actionee                 ×17
data-fr-js-ratio                           ×13
data-fr-js-link-actionee                   ×10
data-fr-js-navigation-item                  ×7
data-fr-js-inject-svg                       ×6
data-fr-js-collapse-button                  ×4   data-fr-js-collapse ×4
data-fr-js-modal-button                     ×4   data-fr-js-modal ×2
data-fr-js-navigation-section-actionee      ×3   (+ header-*, follow…)
data-fr-opened="false"                      ×2   aria-expanded="false" ×4
```

### Interaction réelle — premier dépliage DSFR mesuré

- `scroll` ×2 → `scroll_y=1600` réel (`vol47_servicepublic_scroll.png`).
- Clic `fr-nav__btn` « Fiches pratiques par événement de vie »
  (565,164 — `block<button> [405 136 321x56]`, porteur
  `data-fr-js-collapse-button`) → outcome `ignored` (pas de nav,
  normal pour un bouton JS) **mais effet DOM réel** :

```diff
  <button aria-controls="menu-sp-1"
-         aria-expanded="false">
+         aria-expanded="true">          ← listener DSFR déclenché
  <div class="fr-collapse fr-menu
+            fr-collapse--expanded"      ← classe injectée par le JS
+       style="--collapser: ; --collapse: -8px">
```

Frame post-clic (`vol47_servicepublic_navclick.png`) : bouton peint
avec fond bleu clair = état déplié DSFR appliqué. **Le cycle
disclose/expand DSFR fonctionne** — c'est la preuve d'interaction
que le vol 45 ne pouvait pas produire sur impots (pas de méga-menu).

→ **§B-10 bras DSFR : RÉSOLU `[V4-main]` sur service-public.fr** —
initialisation complète + interaction fonctionnelle (modulo le
`replaceChild` isolé).

### Rendu → §B-18

- Frame : rendu dégradé type legifrance — nav non flexée (boutons
  empilés `[0 216 369x56]`, `[0 720 368x56]`), contenu du menu fermé
  déployé (liens `fr-menu__list` peints en ligne dès l'initiale),
  recherche off-screen à `y=-97 729`. Contenu complet, mise en page
  DSFR non reproduite → §B-18 (flex/nav) + §B-17 (menu fermé déployé).
- Journal : `journal DOM rejeté → repli re-parse why=UnmappedKey` ×4
  sur tab=3 — **repli non mortel ici** (page livrée), contraste avec
  caf (§3).

## 3. caf.fr — wedge journal/détachement : §B-10 non tranchable → §B-24 proposé

- `POST navigate` → titre réel « Page d'accueil Allocataires | CAF »,
  `doc_h=900` (= viewport — contenu jamais livré).
- JS vivant : `/mem` `js_heap=23,2 Mo`, 11 évals `ok=true`
  (bundle 385 Ko compris), **1 éval `ok=false`** (182 Ko, 55 ms —
  erreur **non remontée au ring console** : `pas de journal` → trou
  d'observabilité, le PipelineError n'a pas été émis en état détaché).
- **DOM détaché permanent** : `/dom` → 404 `pas de DOM` en continu
  (>2 min 15, ~15 sondes), `/frame` → **204**, `/layout` →
  `pas de layout`, `dom_nodes=0`, `journal_applies=0`,
  `journal_fallbacks=5`.
- `journal DOM rejeté → repli re-parse (ADR-0025) why=UnmappedKey` ×1
  à 08:55:19 — puis plus aucune ligne d'apply : le repli re-parse n'a
  **jamais délivré** (sur service-public le même WARN ×4 a délivré —
  le repli n'est donc pas systématiquement mortal).
- Inputs : `scroll` accepté en projection (`scroll_y=0`), `click` →
  différé puis `ignored` (« paint en vol » — paint_detached figé).
- `POST /reload` → nouvelle rafale d'évals (385 Ko ré-évalué ok) mais
  DOM **toujours détaché** ; RAM daemon 2,77→2,93 Go (double heap +
  arbres orphelins ?) → **arrêt de sécurité au seuil PRUDENCE** :
  DELETE onglet puis kill PID.

→ **Nouvelle entrée §B-24 proposée** — `explorer-core` :
« rejet journal `UnmappedKey` → repli re-parse sans délivrance :
onglet détaché permanent, page jamais servie, JS vivant » —
signature distincte de §B-1 (pas de saturation cœur : API et pumps
répondent, évals courtes) et de §B-4 (le DOM *existe*, il n'est
jamais réinséré). §B-10 non tranchable sur cette cible (le DSFR de
caf n'a jamais été observable).

## 4. Télémétrie / observabilité

- RAM daemon (PID 8616, `tasklist`) : 375 Mo départ → 1,49 Go après
  legifrance+service-public → **1,35 Go après DELETE tabs 2/3**
  (le delete libère) → 2,77 Go au reload caf → **2,93 Go et montée
  continue après DELETE tab 4** (GC sans rendu — croisé §B-perf/T3
  « heap gelé » : le wedge fait fuiter).
- `/dom` sur tab=4 : 404 stricts en série — **pas** l'alternance
  200/404 documentée sur github : ici le détachement est terminal.
- `journal_fallbacks` visible dans `/mem` — métrique utile pour
  détecter le wedge (5 sans aucune livraison = signal fort).
- `/console` : vide sur legifrance et caf ; service-public montre le
  dédup `×N` fonctionnel (6 occurrences groupées).
- Deux autres daemons coexistaient (PIDs 4944/17288 puis 17216) —
  aucun touché ; seul le mien (9429) a été arrêté.

## 5. Actions proposées

| # | Proposition | Cible |
|---|---|---|
| §B-10 | **Bras DSFR RÉSOLU `[V4-main]` sur service-public.fr** (168 `data-fr-js-*`, dépliage collapse réel : `aria-expanded`/`--expanded`/`--collapse`) ; **PARTIEL sur legifrance** (boot OK : `data-fr-js`+2 déclencheurs, mais instances absentes — recoupe §B-4c) ; **non tranchable caf** (§B-24). Bilan bras : 2/3 cibles avec boot sain, 1/3 interactionnelle | `explorer-jscore` — JS-C150 validé terrain ×2 |
| §B-24 | **NOUVEAU** : wedge « journal rejeté `UnmappedKey` → repli re-parse sans délivrance » — DOM détaché permanent, `/frame` 204, inputs différés→`ignored`, reload impuissant, fuite RAM post-delete | `explorer-core` (apply/journal ADR-0025) |
| §B-14 | + `Node.replaceChild: not a node` (service-public dsfr-1.14 `inject-svg` ?) ×6 via `unhandledrejection` | `explorer-jscore` — matrice J5 |
| §B-4(c) | Recoupe : legifrance 42/42 évals ok, 0 rejet, instances DSFR non créées — « exécution réussie sans effet » partielle confirmée sur cible réelle | `explorer-jscore`/`core` — détecteur T2 |
| §B-17 | Recoupe ×2 : méga-menus fermés déployés (legifrance y=-98518 ; service-public liens `fr-menu__list` peints dès l'initiale) ; non interceptants sur legifrance (y négatif) | `explorer-layout`/`css` |
| §B-18 | + nav DSFR non flexée (items empilés en bloc), pages gouvernementales déstylées vs impots ~95 % fidèle — même famille CSS, résultats divergents à expliquer | `explorer-css`/`layout` |
| obs | `script: eval ok=false` en état détaché **non remonté** au ring `/console` (caf : 1 échec invisible) | `explorer-core` — télémétrie |

**Statuts proposés** : §B-10 `RÉSOLU [V4-main] sur service-public —
bras DSFR en clôture à 2/3 cibles, PARTIEL legifrance, bloqué caf` ·
§B-24 `OUVERT` (nouveau) · §B-14 `PARTIEL — +1 entrée matrice` ·
§B-17/§B-18 `PARTIEL — recoupes V4` · §B-4(c) enrichi.

### Fichiers produits

- `captures/vol47_legifrance.png` · `vol47_legifrance_scroll.png` ·
  `vol47_legifrance_navclick.png` · `vol47_servicepublic.png` ·
  `vol47_servicepublic_scroll.png` · `vol47_servicepublic_navclick.png`
  (aucune frame caf — `/frame` 204, wedge §B-24)
- `logs/vol47_legifrance_{dom,dom2,dom_afterclick,layout}.txt` ·
  `logs/vol47_servicepublic_{dom,dom_open,layout}.txt` ·
  `logs/vol47_caf_{dom,layout}.txt` (« pas de DOM/layout » — preuve du wedge)
- `_vol47_daemon.log` (racine, 21 Mo)

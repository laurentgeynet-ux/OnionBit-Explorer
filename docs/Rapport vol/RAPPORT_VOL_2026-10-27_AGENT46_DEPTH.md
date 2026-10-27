<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 46 — Re-vol §B-3 stack overflow + frameworks SPA

- **Date** : 2026-10-27
- **Binaire** : `C:/obe-vol-target/release/explorer-daemon.exe` — tag
  **`[V4-main]`** (inclut le WIP `depth_limit`/`stack_guard` jscore de
  l'équipe fix — JS-C166/`deleg_hop`, J2). Binaire du 2026-10-27
  09:32, pré-livré par l'opérateur.
- **Viewport** : 1366×900 (`EXPLORER_VIEWPORT`) — port 9428.
- **Daemons du vol** : PID 15824 (instance 1 — **morte sur alpinejs**,
  journal `logs/daemons/_vol46_daemon.log`) ; PID 4944 (instance 2 —
  tailwind+vuejs, `logs/daemons/_vol46_daemon2.log`, arrêt propre par
  PID après DELETE des onglets).
- **Protocole** : `docs/vol-agent.md` §8 — chaque constat porte `crate`
  + ID §B ; statuts canoniques §G.
- **Mission** : re-vol §B-3 « stack overflow `explorer-jscore` »
  (statut `FIX LIVRÉ` en attente de confirmation terrain) + deux
  frameworks SPA témoins (tailwindcss.com, vuejs.org).
- **RAM** : surveillée par `tasklist` à chaque étape (seuil 2,5 Go).
  Relevés : 366 Mio au boot des deux instances ; 603 Mio après
  tailwind ; 679 Mio après vuejs. **Aucune rampe mémoire avant le
  crash alpine** — mort ~1,3 s après la navigation, trop rapide pour
  un relevé intermédiaire ; ce n'est pas une fuite mais un débordement
  de pile sec.

## Verdicts en une table

| Cible | ID §B | Verdict `[V4-main]` |
|---|---|---|
| alpinejs.dev | §B-3 stack overflow jscore | **TOUJOURS PRÉSENT — crash process** : thread `explorer-jscore` mort en débordement de pile natif **pendant l'éval** d'un script de 55 935 o ; aucune erreur typée, process entier tué ~110 ms après le début des évals JS |
| tailwindcss.com | — (témoin SPA) | **Sain** : SSR peint (hero, mur de logos, sections), 178 évals `ok=true` cumulés sur le vol, console vide, scroll 800→1600 |
| vuejs.org | — (témoin SPA) | **Très bon** : rendu quasi fidèle (nav dropdowns, hero, boutons, 3 colonnes, footer complet), console vide, scroll 800→1243 (doc_h 2215) |

## §B-3 — Verdict : TOUJOURS PRÉSENT (crash)

Le fix JS-C166 (`deleg_hop` borné, `stack_guard` plancher pile native,
sondes `depth_probe` 47/47) est présent dans ce binaire — **mais ne
couvre pas le chemin réel d'alpinejs.dev**.

Preuve verbatim (`logs/daemons/_vol46_daemon.log`, intégralité = 37
lignes) :

```
2026-10-27T08:40:44.076387Z  INFO explorer_daemon: explorer-daemon prêt addr=127.0.0.1:9428
2026-10-27T08:41:04.524953Z  INFO script: eval… tab=1 bytes=208        → ok=true ms=0
2026-10-27T08:41:04.527478Z  INFO script: eval… tab=1 bytes=159822    → ok=true ms=34
[… 12 autres scripts évalués ok=true, dont 133 239 o, 38 866 o, 41 063 o …]
2026-10-27T08:41:04.634940Z  INFO script: eval… tab=1 bytes=55935     ← jamais terminé
thread 'explorer-jscore' (16988) has overflowed its stack
```

Constats mesurés :

1. **Le process entier meurt** (PID 15824 absent de `tasklist` 1 s
   après) — un débordement de pile Windows est un abort non rattrapable
   (`STATUS_STACK_OVERFLOW`), pas un panic Rust : **pas d'erreur
   typée**, pas de `JsError`, l'API REST meurt avec lui (curl exit 56).
2. **L'overflow survient pendant l'éval, pas le parse** : 14 scripts
   sont déjà passés `ok=true`, dont deux bundles bien plus gros
   (159 822 o en 34 ms, 133 239 o en 30 ms). Le script fatal fait
   55 935 o — ce n'est donc pas la descente syntaxique sur un gros
   littéral (bornée par C134/depth_limit) mais une **récursion à
   l'exécution** : chemin de délégation/proxy/`bind`×N, getter/proto
   chaîné, ou drop récursif en fin d'éval — hors périmètre des
   transporteurs déjà bornés.
3. **`stack_guard` n'a pas tiré** : soit la récursion entre par un
   point d'entrée sans garde (callback hors `call_depth`, cf. note
   §B-3 « forEach→callback » déjà traitée — reste d'autres portes),
   soit le plancher est sous-dimensionné pour la profondeur réelle
   rencontrée.
4. **RAM non en cause** : 366 Mio au boot, crash ~110 ms après le
   premier eval — signature pile pure, pas fuite.

**Recommandation §B-3** : repasser le statut de `FIX LIVRÉ` à
**OUVERT** (ou `FIX LIVRÉ — insuffisant terrain`). Pistes pour NAV-JS :
attacher l'URL du script aux lignes `script: eval…` (aujourd'hui
seule la taille identifie le fautif — 55 935 o sur alpinejs ≈ le
bundle core ou un plugin docs) et instrumenter un compteur de
profondeur de récursion d'exécution (pas seulement d'AST) :
`Drop` récursif, `instanceof`/`bind`×N restant, getters proto,
`toString`/coercions chaînées. Sondes `depth_probe` vertes sur
chemins synthétiques ≠ couverture du chemin réel.

## 1. tailwindcss.com — sain (`crate`: témoin, aucun §B nouveau)

- **Titre** : « Tailwind CSS - Rapidly build modern websites without
  ever leaving your HTML. » — doc_h 9675 px non clampé.
- **Frame** `captures/vol46_tailwind.png` : hero peint correctement
  (titre, paragraphe avec spans `code` colorés, boutons « Get
  started »/recherche, bloc sombre). Défauts visibles : les liens de
  nav tiennent en un bloc compact « DocsPlayBlogShowcasePartnersPlus »
  (gap flex/espacement non reproduit) et des noms de classes
  (`text-8xl text-gray-950…`) apparaissent en texte — ce sont des
  exemples de code du site, fidèles à la source.
- **Scroll** 800→1600 : `captures/vol46_tailwind_scroll.png` — mur de
  logos sponsors rendu proprement (Syntax, Sanity, KIRO, Lovable,
  PostHog…), section « Built for the modern web ».
- **Console** : « pas de journal » (`logs/vol46_tailwind_console.txt`).
- **Télémétrie** (`logs/vol46_tailwind_mem.txt`) : DOM 3202 nœuds,
  2609 boîtes, 1288 items, heap JS 2,1 Mio, images 30,4 Mio,
  total réservoir 90 Mio — daemon à 603 Mio.
- **Journal daemon** : 0 `ok=false` sur tout le vol ; seul bruit =
  lignes `sync_profile` (instrumentation phase sync) — ~65 k lignes
  tab=1, surtout de la télémétrie, pas un symptôme.

## 2. vuejs.org — très bon (`crate`: témoin, aucun §B nouveau)

- **Titre** : « Vue.js - The Progressive JavaScript Framework |
  Vue.js » — doc_h 2215 px.
- **Frame** `captures/vol46_vuejs.png` : **fidelity élevée** — nav
  complète avec dropdowns (Docs, Ecosystem, About, Support), hero
  « The Progressive JavaScript Framework », 4 boutons (Why Vue,
  Get Started, Install, Get Security Updates for Vue 2), bandeau
  « Special Sponsor slot », 3 colonnes Versatile/User-friendly/
  Efficient.
- **Scroll** 800→1243 (clampé par doc_h) : `captures/vol46_vuejs_
  scroll.png` — footer complet (Docs, Support, Resources, Official
  Libraries, Video Courses, Help, News, copyright).
- **Console** : « pas de journal ».
- **Télémétrie** : DOM 786 nœuds, 640 boîtes, heap JS 3,1 Mio,
  `journal_fallbacks: 1` — corrélé à l'unique WARN du vol :
  `journal DOM rejeté → repli re-parse (ADR-0025) tab=2
  why=Fingerprint` (comportement prévu, dégradation gracieuse).

## Constats annexes

- **0 `ok=false` sur 178 évals** (daemon 2) — les boots JS des deux
  témoins courent à terme ; pas de mort silencieuse sur ces cibles
  (recoupe la décroissance §B-4).
- **Arrêt propre** : DELETE tab1+tab2, `{"tabs":[]}`, puis kill du
  **PID 4944 uniquement**. Les autres `explorer-daemon.exe` du poste
  (PID 14576/16604 au lancement, puis 8616/17288 — ~1,8 Go chacun)
  appartiennent à d'autres sessions : **non touchés**, mais proches
  du seuil d'alerte 2,5 Go — signalé à l'opérateur.
- Logs archivés : `logs/daemons/_vol46_daemon.log` (crash alpine,
  preuve §B-3), `logs/daemons/_vol46_daemon2.log` (vol nominal).

## Propositions §B

- **§B-3 → rouvrir** : « FIX insuffisant — overflow à l'**exécution**
  (script 55 935 o, alpinejs.dev), transporteurs C134/JS-C166 +
  `stack_guard` ne couvrent pas ce chemin. Ajouter : traçabilité URL
  par éval + garde sur la récursion d'exécution (drop récursif,
  proto chaîné, coercions). »

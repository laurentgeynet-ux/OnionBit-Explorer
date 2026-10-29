<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->
---
name: vol
description: Campagne de vols agents autonome et continue — boucle 1 vol = 1 rapport + synchro SYNTHESE.md, sélection des cibles par priorité (re-vols §B/§D d'abord, recommandations propres ensuite), sans intervention jusqu'à arrêt de l'opérateur
argument-hint: "[N-agent-de-départ]  ex. /vol ou /vol 96"
allowed-tools:
  - read
  - write
  - edit
  - exec
  - grep
  - find_file_by_name
---

# Skill `vol` — campagne de vols agents en boucle autonome

Une invocation = **une campagne continue** : l'agent enchaîne les
vols sans attendre l'opérateur. Chaque itération fait exactement :

> **1 vol → 1 rapport → 1 synchro `SYNTHESE.md` →
> vol suivant** (preuves écrites dans `docs/Rapport vol/` —
> dossier local **non suivi**, rien à committer).

Il n'y a **pas d'étape de fin** : la campagne tourne jusqu'à ce que
l'opérateur l'interrompe, ou jusqu'à épuisement des cibles ET
absence d'anomalie ouverte (cap asymptotique, §9 AGENTS).

Références contractuelles (lire en début de campagne, puis relire
`SYNTHESE.md` à chaque itération — il évolue avec les vols des
autres pistes) :

- `docs/vol-agent.md` — règles de vol (endpoints, §5 « Règles de
  vol », §7 outillage).
- `docs/Rapport vol/SYNTHESE.md` — §B catalogue des anomalies,
  §D priorités, §E-bis file de tâches, §G protocole de synchro.

## Numérotation et mission

- `N-agent` : dernier numéro AGENTNN dans `docs/Rapport vol/` + 1
  (ou l'argument fourni).
- `mission` : code en majuscules reflétant le contenu du vol —
  `REVALID<n>` pour une vague de re-vols, un code thématique
  (`WEDGES`, `ECOM`, `MEDIA`…) pour une exploration nouvelle.
- Un vol couvre **5 à 10 cibles** (budget daemon : au-delà, la
  RAM et le risque de wedge dégradent les mesures).

## Sélection des cibles — par priorité stricte

À chaque itération, composer la liste de vol dans cet ordre :

1. **Re-vols dus** — toute entrée §B au statut
   `CORRECTIF <sha> à re-voler` **dont le fix est dans le binaire
   courant** (comparer le sha à `git log`), plus la file
   « Re-vols reportés » du rapport précédent. Priorisées par §D.
2. **Re-vols de confirmation** — anomalies `OUVERT`/`INTERMITTENT`
   dont la reproduction est incertaine (non re-testées sur le tag
   de binaire courant), anomalies `PARTIEL`/`EN DÉCROISSANCE` à
   trancher.
3. **Recommandations propres** — si aucun re-vol n'est dû,
   choisir de nouvelles cibles selon la couverture :
   - catégories du web réel sous-représentées dans le corpus
     (~180 cibles volées) : secteur absent ou 1 seul site ;
   - cibles à risque structurel connu mais non catalogué :
     flux réseau continu (§B-54), boots multi-chaînes lourds
     (§B-23), bundles minifiés massifs (§B-12), challenges
     anti-bot (§B-6), cartes/tuiles (§B famille OSM) ;
   - sites réels représentatifs du web français + international
     (mix historique des vagues : presse, e-commerce, admin,
     banque, santé, voyage, dev, forums, apps).
   - **Jamais** un site déjà volé sur le même tag de binaire
     sans raison §B ; jamais de doublon avec les rapports
     récents sauf re-vol justifié.
- Justifier en une ligne dans le rapport (« cibles choisies
  parce que… »).

## Itération de vol

### a) Binaire frais (verdict = binaire frais, §8 AGENTS)

1. `git status` : un verdict visuel n'est valable que sur un
   **binaire buildé après le dernier commit touché et sur arbre
   sans WIP** (vol-agent §intro). Worktree WIP → builder depuis
   `git worktree add <dir> HEAD`.
2. Si `HEAD` n'a pas bougé depuis le vol précédent **et** qu'aucun
   re-vol n'exige un binaire plus récent, réutiliser le binaire
   courant ; sinon `cargo build --release -p explorer-daemon`
   et **attendre `Finished`** (leçon R-75 — deux fausses
   validations sur binaire périmé).
3. Relever le tag `[V<n>-main]` courant et les commits
   postérieurs au binaire.

### b) Daemon dédié

```powershell
$env:EXPLORER_VIEWPORT="1366x900"
.\target\release\explorer-daemon.exe --port <PORT> *> _vol<N>_daemon.log
```

- Port 9385+, jamais 9377 (vol humain) ; choisir un port libre,
  noter le **PID** (arrêt par PID uniquement — jamais
  `taskkill /IM`, les daemons voisins appartiennent à d'autres
  sessions).
- **Watchdog mémoire obligatoire** : lancer en fond dès le
  démarrage — `pwsh -File scripts/vol_memwatch.ps1 -DaemonPid
  <PID>` (seuil 5 120 Mo, kill par PID). Un daemon wedged à 7 Go
  a crashé la station (vol 106).
- **Un daemon par vol.** Si le vol précédent s'est terminé
  proprement, le daemon peut être réutilisé seulement s'il reste
  sain (`GET /api/tabs` répond < 5 s, RAM stable).

### c) Vol des cibles

Pour chaque cible (`docs/vol-agent.md` §2-§3) :

```powershell
$BASE = "http://127.0.0.1:<PORT>"
$tab = (Invoke-RestMethod "$BASE/api/tabs" -Method Post).id
Invoke-RestMethod "$BASE/api/tabs/$tab/navigate" -Method Post `
  -ContentType 'application/json' -Body '{"url":"<URL>"}'
```

- **Yeux** : `GET /api/tabs/{id}/frame` →
  `docs/Rapport vol/captures/vol<N>_<site>.png` — puis
  `pwsh -File scripts/shrink_capture.ps1 <png>` et **lire
  uniquement `_view_<nom>.png`** (PNG brut 2-6 Mo = crash de
  session garanti à la lecture, garde vol-agent §7). Zoom :
  `?x=&y=&w=&h=` ; document : `?full=1`.
- **Premier réflexe** : `GET /api/tabs/{id}/console` **avant** le
  DOM (erreurs persistées, ring 512).
- **Télémétrie** : `/mem`, `/dom`, `/layout` — un **404
  transitoire** sur page à JS continu se retente, ce n'est pas
  un « DOM absent ».
- **Mains** : `POST /input` (`scroll`/`click`/`mousemove`/`key`),
  `/back` `/forward` `/reload`. Un clic doit produire une vraie
  navigation — « clic ignoré » est un constat.
- **Mesurer, jamais "ça a l'air"** : doc_h, scroll_y, ms d'éval,
  js_heap, occurrences ×N, captures jointes.
- **Surveiller le journal** : phases `event.poll_net` /
  `event.dispatch` > ~15 s = suspect §B-54 ; `disjoncteur armé`
  = tir §B-41 à noter ; évals `ok=false ms=0` = §B-51.
- **Entre cibles** : `DELETE /api/tabs/{id}`, vérifier purge
  mémoire (signal §B-30).
- **Daemon wedged** (API sans réponse > ~15 s) : constater §B-54,
  arrêt forcé **par PID**, archiver le journal, relancer un
  daemon neuf sur un autre port et **continuer le vol** avec les
  cibles restantes — le wedge termine la cible, pas la campagne.
- **Fraîcheur au retour du vol** — à la fin de chaque vol,
  `git log -1 main` : si HEAD a bougé depuis le build du daemon
  **et** que les nouveaux commits portent des fixes
  (`fix(`/`feat(`) touchant le catalogue §B, le vol suivant
  **repart sur un binaire neuf** : kill PID, worktree à jour,
  rebuild (`Finished` attendu), daemon neuf, re-sélection des
  cibles (les re-vols dus ont changé — les anomalies corrigées
  passent en priorité 1). Pas de contrôle entre les cibles
  d'un même vol — un vol se joue entier sur son binaire ;
  un constat fait juste avant le commit d'un fix reste une
  preuve valable (noter « constat pré-`<sha>` »).

### d) Rédiger LE rapport du vol

`docs/Rapport vol/RAPPORT_VOL_<AAAA-MM-JJ>_AGENT<N>_<MISSION>.md`
(français, en-tête de licence, modèle =
`RAPPORT_VOL_2026-10-28_AGENT95_REVALID5.md`) :

- en-tête : tag binaire + HEAD + commits post-binaire, mission,
  justification des cibles, PID/port/viewport/journal, RAM ;
- verdict global (3-5 constats) ;
- table **Synthèse** : une ligne par cible ;
- **Preuves marquantes** : verbatim console/log par anomalie ;
- chaque constat porte `crate` + ID §B (`→ §B-5`) ; trou nouveau =
  proposition §B au numéro suivant ;
- section **Re-vols reportés au vol N+1** (cibles non volées,
  correctifs hors binaire).

### e) Synchro `SYNTHESE.md` (protocole §G)

`SYNTHESE.md` = source de vérité des statuts ; le rapport est une
preuve immuable.

- §A : entrée « **Agent N (`[tag]`)** : … » datée.
- §B : statuts démontrés uniquement — `RÉSOLU` **seulement** par
  re-vol sur binaire post-fix ; `NON REPRODUIT` si diagnostic
  infirmé ; `PARTIEL`/`EN DÉCROISSANCE`/`INTERMITTENT`/
  `CORRECTIF <sha> à re-voler` selon le cas ; tag de binaire sur
  chaque anomalie.
- §B : nouvelles entrées au numéro suivant pour les trous nouveaux.
- §E-bis : colonne État des lignes concernées ; §D : réordonner
  si la gravité change ; en-tête : comptage de documents.

### f) Clôture du vol — puis **repartir**

1. `DELETE` les onglets restants, vérifier purge, tuer le daemon
  par son PID (ou le garder s'il est sain et réutilisé au vol
  suivant).
2. Archiver le journal dans `docs/Rapport vol/logs/daemons/`.
3. **Enchaîner immédiatement** : N+1, re-sélection des cibles
   (les « re-vols reportés » du rapport qui vient d'être écrit
   alimentent la priorité 1), retour à l'étape a).

## Interdits de vol

- Jamais de verdict visuel sur binaire non re-buildé ou arbre WIP.
- Jamais de vol sur le daemon humain :9377.
- Jamais `taskkill /IM` — PID ciblé uniquement.
- Jamais §B « RÉSOLU » sans re-vol post-fix.
- Jamais « ça a l'air de marcher » — mesurer.
- Jamais deux missions dans un même rapport.
- Jamais un % sans dénominateur dans les synthèses (§8 AGENTS).
- Jamais `read` sur une capture PNG brute (> ~1 Mo) — vignette
  `_view_*` via `scripts/shrink_capture.ps1` uniquement.
- Jamais de pause en attente de confirmation entre deux vols —
  la campagne est autonome ; l'opérateur interrompt quand il veut.

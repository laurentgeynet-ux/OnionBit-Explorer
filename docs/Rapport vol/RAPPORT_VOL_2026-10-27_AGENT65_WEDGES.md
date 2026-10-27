<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 65 : re-test des wedges §B-26/30/33 `[V5-main]`

- **Binaire** : `[V5-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (build 2026-10-27 13:10 local, post-`2fc40043`) — inclut les FIX
  `0ffca689`+`b408570b` (§B-30), `20f2336c` (§B-33 borne 30 s),
  `17f5cfb5`+`b93107c5` (§B-41), `b9189d5e` (§B-39 plafond 45 s),
  `3f4ac8d0` (témoins §B-26).
- **Cibles** : www.figaro.fr · www.musee-orsay.fr · meteofrance.com
  (+ www.lefigaro.fr en complément — cible canonique du vol 49).
- **Daemons** : 4 lancements (ports 9447, 9465, 9477) — détails §6.
- **Fenêtre** : 12:12 → 12:50 UTC.
- **Verdict** : **§B-26 REPRODUITS ×3** (orsay, meteofrance, lefigaro),
  **§B-30 PARTIELLEMENT RÉSOLU** (purge ok, fuite résiduelle +2 Mo/s),
  **§B-33 non déclenché**, figaro.fr inatteignable (TLS) → §B-43 proposé.

---

## 1. www.figaro.fr — TLS `UnrecognisedName`, page jamais fetchée → §B-43 ?

`POST /navigate` → **502 en 0,12 s** :

```
fetch : TLS vers https://www.figaro.fr:443 : received fatal alert: UnrecognisedName
```

Erreur **gérée** (page d'erreur peinte : `/frame` 200, PNG 1,87 Mo,
`logs/vol65_figaro_mem.json`, `captures/vol65_figaro.png`). Aucun wedge.
Le serveur rejette au handshake (SNI/routage CDN) — la cible wedge
historique est **lefigaro.fr** (vol 49), pas `www.figaro.fr` ; ce bras
TLS n'apparaît dans aucune entrée §B → **proposition §B-43**
(`explorer-net`, handshake TLS refusé `UnrecognisedName` — mesurer la
surface : autres hôtes `www.*` concernés ?).

## 2. www.musee-orsay.fr — §B-26 REPRODUITS `[V5-main]`

`POST /navigate` → **200 en 9,8 s** (→ `/fr`, titre « Musée d'Orsay »,
`doc_h` 10 051). Puis la signature exacte §B-26 :

```
12:13:00.716  apply: cascade ms=809
12:13:00.749  apply: boxtree ms=842
12:13:00.797  apply: layout  ms=891      ← dernier apply, jamais repaint/fin
12:13:34.717  pump.js_run_sync us=14967
12:13:34.717  pump.drains us=210        ← puis silence TOTAL (0 WARN/ERROR)
```

| Instant | RAM daemon (PID 3628) |
|---|---|
| 12:12:58 (fin nav) | 724 Mo |
| 12:13:58 | 970 Mo |
| 12:14:23 | 1 090 Mo |
| 12:14:49 | 1 215 Mo |

**Fuite ~4,4 Mo/s**, non bornée. `/frame` → 204, `/dom` → 404,
`/mem` : tout détaché (`dom_nodes:0`, `layout_boxes:0`,
`js_heap` 33 Mo), **`journal_fallbacks=0`** (variante figaro du vol 49 :
mort d'apply entre `layout` et `repaint`, **sans** rejet journal).
Différence avec vol 62 : pas de boucle `pump.js_run_sync` ~8 ms/5 s —
ici le pump se tait après 34 s ; la fuite continue dans le silence.

## 3. meteofrance.com — §B-26 ×2 + escalade fatale (API morte, process tué)

Nav lancée 12:15:31 sur tab 2 **pendant la fuite orsay** (tab 1) :

- Côté serveur : fetch + 3 évals ok (dont bundle 807 Ko en 521 ms), puis
  `apply: layout ms=250` à 12:15:34 — **même arrêt avant `repaint`**.
- `POST /navigate` **jamais retourné** (timeout curl 150 s) — API
  affamée par la double fuite.
- Dernière ligne : `pump.js_run_sync us=40 238 584` (**40,2 s** de run
  synchrone) à 12:19:04 puis silence.

| Instant | RAM | Vitesse |
|---|---|---|
| 12:14:49 | 1 215 Mo | — |
| 12:18:07 | **4 231 Mo** | ~15 Mo/s |
| 12:18:42 | **4 892 Mo** | ~38 Mo/s (accélère) |

`DELETE /api/tabs/1` → timeout 20 s (API entière morte — escalade
§B-26→§B-1 identique à bfmtv vol 49). Daemon **retrouvé mort** à
12:28:40 : aucun événement Application/Système (ni OOM loggé, ni crash
WerFault) et **tous** les explorer-daemon de la machine (3 vols
parallèles) absents en même temps — mort externe probable
(taskkill /IM ou watchdog ; mon 3628 à 4,9 Go en était la cible
plausible). Hypothèse OOM-abort propre non exclue (dernière alloc
~4,9 Go + ~38 Mo/s → dépassement commit plausible en minutes).

## 4. www.lefigaro.fr — §B-26 REPRODUIT, variante `UnmappedKey`

Daemon propre PID 18780, port 9465 (voir §6 pour le rebond de port) :

- `POST /navigate` → **200 en 19,3 s**, titre OK, `doc_h` 71 703 clampé.
- `12:35:52` — `WARN journal DOM rejeté → repli re-parse (ADR-0025)
  why=UnmappedKey` → `journal_fallbacks=1`.
- `apply: cascade ms=319 → boxtree ms=401 → layout ms=614` (12:35:53.474)
  — **arrêt au même point : jamais `repaint`**.
- `12:36:02` — dernier log (`pump.drains us=267676`) puis silence.
- `/frame` 204, `/dom` 404, `js_heap` 63 Mo, `journal_fallbacks=1`.

RAM : 1 122 → 1 166 → 1 194 → 1 232 → 1 283 → **1 322 Mo** (12:37:37)
→ **fuite ~2,5-3 Mo/s**, soutenue. Daemon retrouvé mort vers 12:44 —
deuxième disparition **simultanée de tous** les explorer-daemon de la
machine (même signature externe que §3 ; 18780 n'était qu'à 1,3 Go).

Console lefigaro : `unhandledrejection: TypeError: cannot read
'hasWebVitals' of Undefined` ×2 (sdk.mrf.io → initial-vendors.js) —
à rapprocher §B-14/`unhandledrejection` (déjà tracé, pas un silence).

## 5. §B-30 — PARTIELLEMENT RÉSOLU `[V5-main]` : purge ok, fuite résiduelle

Protocole canonique vol 53/59 : navigate + DELETE avec onglet actif.
Daemon propre PID 9080, port 9477, lefigaro.fr :

- nav → 200 en 15,2 s (12:46:35) ; `DELETE` → **204 en 0,65 s** (12:46:47).
- **La purge fonctionne** : RAM 1 253 → **585 Mo** en ~20 s
  (`{"tabs":[]}` vérifié).
- **MAIS** : 3 s après le DELETE, `UnmappedKey` re-parse + chaîne apply
  complète pour l'onglet mort (`apply: cascade → boxtree → layout
  ms=1115` à 12:46:51 — encore jamais `repaint`) puis silence.
- Fuite **post-DELETE** : 585 → 615 → 645 → 674 → 704 → 793 → 833 →
  872 → 955 Mo (12:47:09→12:50:18) ≈ **+2 Mo/s, zéro onglet, sans borne**.
- **Aucun** `WARN nav orpheline stoppée` : le garde `nav_alive` couvre
  `sub.run` (fetch sous-ressources) mais **pas** les jobs
  apply/journal post-commit — la nav avait déjà commité sa tête.

Verdict : le bras « navigate in-flight au DELETE » est bouché (purge
667 Mo), mais **le bras « apply/re-parse en vol au DELETE » reste
ouvert** — mêmes symptômes §B-30 en atténué (+2 vs +28 Mo/s).

## 6. Incidents de vol (traçabilité)

1. **Port 9447 repris** : après la mort de mon daemon 3628, un
   daemon parallèle (PID 22040, même binaire) a bind 9447 — mon second
   lancement a échoué (`bind impossible`, log
   `daemons/_vol65_daemon2_9447_bindfail.log`). Le **profil partagé**
   (`%APPDATA%/explorer`) a restauré mon onglet figaro dans ce daemon
   étranger ; ma nav lefigaro.fr (12:32:33, 200 en 6,5 s) s'est exécutée
   chez lui — RAM 437→1 279 Mo en ~90 s puis chute à 852 Mo quand
   **l'autre agent a supprimé l'onglet** (`/mem` → `onglet inconnu`).
   Mesures marquées contaminées — non comptées dans les verdicts.
   Recommandation : profil par port/daemon ou `--profile` isolé pour
   les vols parallèles.
2. **Deux extinctions de masse** (~12:19-12:28 et ~12:37-12:44) :
   tous les explorer-daemon de la machine morts simultanément, sans
   événement journal Windows — suspect externe (kill /IM/watchdog).
   Mon PID 3628 (4,9 Go) était une cible légitime de la première.
3. Daemon 9080 (§B-30) tué proprement par **son PID** à 12:50 —
   `taskkill //PID 9080 //F` ; aucun taskkill /IM de ma part.

## 7. Synthèse des statuts

| ID | Verdict `[V5-main]` | Preuve |
|---|---|---|
| **§B-26** | **REPRODUIT ×3 — reste CRITIQUE** — orsay (fallbacks=0, +4,4 Mo/s), meteofrance (→4,9 Go, API morte, process mort), lefigaro (`UnmappedKey`, fallbacks=1, +2,5-3 Mo/s). Signature invariante : apply journalisé jusqu'à `layout`, **jamais `repaint`/`fin`**, DOM détaché, silence. Les témoins `apply_started_without_fin`/`paint_in_flight_age` ne remontent pas dans `/mem` (à exposer ?) | `explorer-core` (`session::apply`, ADR-0025) |
| **§B-30** | **PARTIEL** — purge au close libère 667 Mo et `nav_alive` couvre `sub.run`, mais les jobs apply/journal d'un onglet supprimé continuent : +2 Mo/s sans borne avec `{"tabs":[]}`. **Étendre `wait_tab_dead`/le garde aux phases post-commit** (re-parse, apply, pump) | `explorer-core` (`session::{mod,shared,apply}`) |
| **§B-33** | **Non déclenché** — aucun `nav_paint wedge — détenteur mort, réarmé` dans 4 journaux ; les navigate complétés ont tous rendu la main (6,5-19,3 s). Le timeout 30 s n'a jamais eu à tirer (les wedges §B-26 tuent l'apply **après** la restitution nav_paint) | `explorer-core` (`session::shared`) |
| **§B-41** | **Non reproduit** — aucune boucle `event.*` ; meteofrance = `pump.js_run_sync` unique de 40 s (signature §B-26, pas §B-41) | `explorer-core` |
| **§B-43 ?** | **Nouveau proposé** — TLS `UnrecognisedName` sur `www.figaro.fr:443`, page jamais fetchée, erreur gérée (502 propre + page d'erreur) | `explorer-net` |

## 8. Artefacts

- `captures/vol65_figaro.png` (page d'erreur TLS, 1,87 Mo) ;
  `vol65_orsay.png` / `vol65_lefigaro.png` = 0 o (`/frame` 204 — wedge).
- `logs/vol65_{figaro,orsay,meteo,lefigaro}_*.json/txt` (mem, console,
  dom — `lefigaro_{console,mem,mem_wedge}` produits par le vol parallèle
  sur daemon 22040, convergence : même état détaché, js_heap ~63 Mo).
- `logs/daemons/_vol65_daemon{,_9447_orsay_meteo,_2_9447_bindfail,
  _3_9465_lefigaro,_4_9477_b30}.log`.

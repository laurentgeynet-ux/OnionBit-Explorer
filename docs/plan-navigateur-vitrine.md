# Plan NAVIGATEUR — « YouTube vitrine » + surface navigateur

> Document de pilotage — 2026-11-12, rév. 2. **Source de
> vérité du détail** : la section « Plan NAVIGATEUR — vitrine
> YouTube + surface navigateur » de
> `roadmap-equipe-navigateur.md` ne garde que les cases de
> suivi (contrat §4.5) et renvoie ici — les tables ne se
> dupliquent pas. Complète les roadmaps
> (`roadmap-equipe-navigateur.md` piste produit/JS) ; chaque
> tâche cite ses fichiers, son verrou et sa condition de
> sortie. Rédigé à partir de l'état mesuré (journal crans
> ≤ 86, `css.values` 834/2273, `api` 1510/8099 = 18,6 %).
> Protocole de vol : `docs/vol-youtube.md` (worktree de vol
> dédié, artefacts sous `C:\OnionBit-vol\` — `.eval/` reste
> du triage local, jamais suivi).

## 0. Objectif et définition de « fait »

**Produit** : un screenshot YouTube digne d'une vitrine GitHub —
feed `ytd-browse` monté (miniatures + titres), masthead complet
(logo, recherche, icônes), navigation `/watch` fonctionnelle,
zéro littéral `[[…]]` visible, rendu rejouable hors-ligne.

**Condition de sortie mesurable** :

1. `explorer-cli shot https://www.youtube.com` sur daemon frais
   → PNG contenant ≥ 1 `ytd-rich-item-renderer` monté
   (`appKids` > 0 stable à +60 s), titre page `YouTube`,
   aucun `[CRIT]` moteur dans le vol.
2. Rejeu `DirTransport` du vol → même rendu (vitrine
   déterministe, corpus de régression).
3. `yt-navigate-finish` émis sur clic vidéo OU mur
   explicitement documenté (BotGuard) avec éléments de preuve.

**Non-objectifs** : lecture vidéo (`poToken`/DRM hors scope),
score BCD global (outil de pilotage, pas un but).

## 1. Carte des murs (état mesuré)

| Mur | État | Chaîne causale prouvée |
|---|---|---|
| Hydratation `ytd-*` | ✅ soldé | 1038 `ce.define`, watch complète (cran 78/83) |
| Consentement | ✅ soldé | `SOCS` scopé + cookie jar local (É220, JS-C486) |
| Stamping templates | ✅ soldé | JS-C496 — clone détaché récolté par le GC pendant `ce_upgrade_subtree` → `pin_node` ; `importNode`/`localName` → 0 en vol (cran 87) |
| **BotGuard `pNM`** | 🔴 ouvert | `trayride.a`→`mr` tourne, `g` jamais appelé → `pNM` pend → `bb` → `v8Q` morte → `/youtubei` gatées |
| `dyn.err dyn:7` | 🔴 ouvert | script dynamique, syntax 1:12 — **à corréler avec la fenêtre `mr`** : si émis par BG, c'est la cause première de `pNM`, pas un mur parallèle |
| `fn.restricted` ×4 | 🔴 ouvert | `caller`/`arguments` sur fonctions strictes — probes BG probables (cran 84), fingerprint candidat |
| `api` 18,6 % | 🟡 croissance | `AbstractRange`/sensors en tête de miss-list |
| `prop_scan` 72 M | 🟡 perf | J17 cran 70 — goulot CPU #1 |

## 2. Jalons

### M0 — Reproductibilité d'abord (0,5-1 j)
| ID | Tâche | Fichiers | Verrou | DoD |
|---|---|---|---|---|
| N0.0 | **Hygiène `.eval/` — risque déjà réalisé** : le dossier est partiellement **tracké** (`cookies.json`, `history.json`, `hsts.json`, `dns_cache.json` en `M` dans `git status` — session YouTube commitée) et pèse 1,8 Go. `git rm -r --cached .eval` + ligne `.eval/` dans `.gitignore` (convention vol §8 : corpus `C:\OnionBit-vol\`, rejeux promus `fuzz/`) | `.gitignore` | `git status` propre | plus aucun fichier `.eval/` suivi |
| N0.1 | Vol YouTube complet enregistré via `EXPLORER_NET_RECORD` ; rejeu `DirTransport` vérifié. **Limite connue** : `key_of` = `(méthode, url)` sans corps POST (`replay.rs:46`) — les `/youtubei` ne se distinguent que par body et l'ordre des requêtes peut varier run-to-run → hasher le body POST dans la clé, ou DoD « même rendu » plutôt que hash strict | vol enregistré, `explorer-net/replay.rs` | test `dir_transport_replays_youtube_flight` | rejeu déterministe, corpus versionné |
| N0.2 | Snapshot J17 de référence sur le vol rejeué (`prop_scan`, `env_ns`, `call_nat`) | `jscore/lib.rs` compteurs | — | baseline perf archivée dans `.eval/` |
| N0.3 | **Shot de contrôle sur rejeu** — `explorer-cli shot` + `scripts/geo_diff.mjs` vs capture référence : mesure l'écart visuel résiduel **avant** d'investir M1 (trous `css.values` côté PIPE vs murs JS côté NAV) — réordonne M1/M3 si l'écart dominant est côté PIPE | `explorer-cli`, `geo_diff.mjs` | — | écart documenté au journal, file réordonnée si besoin |

### M1 — BotGuard : lever `pNM`/`bb` (cœur du chantier)
| ID | Tâche | Détail | Verrou | DoD |
|---|---|---|---|---|
| N1.1 | **`Error().stack` top-level** | 8c vs ~200c V8. Options : (a) `Expr::pos` sur les nodes chauds (invasif), (b) position d'appel-site au `CallExpr` seul (borné : une frame synthétique `at <anonymous>:l:c`). Choisir (b) d'abord — 80 % de la forme pour 20 % du coût | `stack_format_v8_like` (frames `at x:l:c`) | `new Error().stack` contient `at ` + position |
| N1.2 | **`Error.stackTraceLimit` réel : défaut 10, honoré** | Diagnostic corrigé : la propriété **n'existe pas** dans le moteur — seule la const interne `STACK_TRACE_LIMIT = 10` (`interp/mod.rs`). Le « 50 » lu en vol est l'assignement kevlar `Error.stackTraceLimit=50` collé comme propriété own ordinaire. En V8 : own property défaut 10 **live** — `stack_text`/`captureStackTrace` doivent la lire ; une valeur figée serait elle-même fingerprintable. Vérifier aussi la présence de `Error.captureStackTrace` | test prop + test live | ctx frais `=== 10` ; `=3` → `stack` borné à 3 frames |
| N1.3 | **Attribution du stall `mr`** | spy `g` armé + compteur d'appels internes `trayride.*` + trace `fn.ret` : le programme s'arrête sur un `throw`, un `await` pendu, ou une fin « clean » anti-bot. Instrumenter `promise` settle + `unhandled_reports` sur la coroutine BG. **Corréler `dyn.err dyn:7` avec la fenêtre `mr`** : si le script fautif est émis par BG → N1.6 devient cause première | sonde `EXPLORER_CTX_PROBE` étendue | **cause nommée** (fingerprint fautif identifié, `dyn:7` prouvé dans le chemin BG, OU fin anti-bot prouvée) |
| N1.4 | **Boucle d'élimination fingerprints** | après N1.1/N1.2 : re-vol, `g` appelé ? → sinon diff suivant (`t.m`=41 = compteur interne BG, **`fn.restricted` ×4** = probes `caller`/`arguments` sur fonctions strictes (cran 84), `performance.now` précision, `Intl`/`Date` granularité, présence `Error.captureStackTrace`) | vol mesuré | `pNM` résout OU 3+ fingerprints éliminés documentés |
| N1.5 | **JS-C496 validation en vol** | `importNode`/`localName` erreurs → 0 ; stamping `dom-if`/`dom-repeat` effectif | vol + suite `dom` | erreurs absentes du log |
| N1.6 | **`dyn.err dyn:7`** | isoler le script dynamique fautif (`eval(dyn:N)` log) → repro minimal via `scripts/ddmin.mjs` → fix tokenizer/parser ; passe en tête de file si la corrélation BG (N1.3) est prouvée | repro + verrou | erreur disparue |

### M2 — `/youtubei` + nav (dépend de M1)
| ID | Tâche | DoD |
|---|---|---|
| N2.1 | Post-clic `/watch` : `yt-navigate-start` → requêtes `/youtubei` → `yt-navigate-finish` | émis en vol |
| N2.2 | Shot vitrine : `shot` sur rejeu M0 → PNG feed+masthead | critères §0 |

### M3 — `api` Pareto (parallélisable, puits sans fin borné)
| ID | Tâche | Portée | DoD |
|---|---|---|---|
| N3.1 | `Range` complet | `startContainer`/`endContainer`/`startOffset`/`endOffset`/`collapsed`, `createRange`, `setStart/End`, `cloneContents`, `toString`, `AbstractRange` IDL | les ~30 clés BCD `Range`/`AbstractRange` ok |
| N3.2 | `Selection` | `getSelection`, `anchorNode`/`focusNode`/`isCollapsed`, `addRange` | clés `Selection` ok |
| N3.3 | IDL membres restants par fréquence BCD | `Element`/`Document`/`Window` membres manquants (`getAnimations`, `caretRangeFromPoint`…), `CSSStyleDeclaration` accesseurs, `fetch`/`Response` membres, observers membres | `api` ≥ 1700 (+200) |
| N3.4 | Hors-périmètre déclaré | sensors (`Accelerometer`, `Gyroscope`, `AbsoluteOrientationSensor`, `LinearAccelerationSensor`, `Magnetometer`, `ProximitySensor`), WebAuthn, WebTransport, BT/USB/NFC/Serial | sous-ensemble documenté → `na` ou file assumée, dénominateur nettoyé |

### M4 — Perf jscore (post-M1, mesuré par J17)
| ID | Tâche | Hypothèse | DoD |
|---|---|---|---|
| N4.1 | `prop_scan` : armer l'index au **miss répété** (compteur par objet, seuil ~3) — les objets gros lus en boucle s'indexent sans taxer les 500 k closures jetables | `prop_ns` dominant post-C490 | `prop_scan` −50 % à volume égal, shot/vol de contrôle |
| N4.2 | Inline-cache `(site, env, nom) → cellule` | plancher actuel = 2 sondes de table ~50 ns ; IC saute tout. **Garde obligatoire** : vérification de hit façon `RESOLVE_VERIFY` (postmortem C490 cran 82 — même classe de bug : clé aplatie → liaison silencieuse fausse) | `env_ns`/get −30 %, staleness bornée documentée |
| N4.3 | Gate perf : pas de régression > 5 % `env_ns`/`prop_ns` sur vol de référence | — | tableau comparatif par commit |

## 3. Ordonnancement (DAG)

```
M0.0 (hygiène .eval) ── M0.1 (rejeu) ──┬── M0.2 (baseline J17) ──────── M4
                                      ├── M0.3 (shot contrôle) ──→ réordonne M1/M3
                                      └── M1.3 (sondes) ──┬── M1.1 ──┐
M1.5 (JS-C496, parallèle)               │                 ├── M1.2 ──┼── M1.4 (boucle) ── M2 ── SHOT
                                      │                 └── M1.6 ────┘   (dyn:7 en tête si corrélation BG)
M3 (api) ──── file parallèle, aucune dépendance
```

**Semaine type** : M0.0→M0.3 d'abord (rejeu + shot de contrôle
avant tout investissement BG) → M1.3 sondes → M1.1+N1.2 en
parallèle → boucle M1.4 (2-3 itérations max, timebox). M3
absorbe les creux entre vols (chaque vol = ~10 min de mesure
passive).

## 4. Risques

| Risque | Prob. | Mitigation |
|---|---|---|
| `.eval/` partiellement tracké — cookies/historique de session commités, 1,8 Go | **réalisé** | N0.0 : `git rm -r --cached .eval` + `.gitignore` ; rejeux promus `fuzz/`, jamais `git add .eval` |
| Rejeu `DirTransport` flaky : clé `(méthode,url)` sans body POST, ordre de requêtes variable | moyenne | body-hash dans `key_of` ou DoD « même rendu » — tranché à N0.1 |
| Le `g` non-appelé est une fin anti-bot pure (notre engine détecté sur un critère non encore isolé) | moyenne | Timebox M1.4 : 3 itérations puis verdict « mur BG documenté » — le shot reste jouable sur rejeu (feed initial hydraté) |
| `Expr::pos` invasif casse le resolveur C490 | moyenne | Option (b) d'abord ; suites `lang`/`dom`/`c49x` systématiques |
| M3 absorbe tout le temps (puits IDL infini) | haute | cap +200 clés N3.3, puis ordre = census réel uniquement |

## 5. Suivi

- **Tableau de bord par commit** : `api` x/8099, `css.values`
  x/2273 (PIPE), métriques J17 du vol rejeu (`prop_scan`,
  `env_ns`), état BG (`g` appelé oui/non).
- **Journal** : crans numérotés dans `interface-equipes.md` ;
  chaque vol = daemon + profil + métriques tracées. Ce doc =
  le détail ; les cases de suivi vivent dans
  `roadmap-equipe-navigateur.md` §Plan (qui renvoie ici).
- **Gates** : `lang` 313 + `dom` 263 + fumées (`c49x`,
  `lazy_gen`) par commit ; `bcd_coverage --gate
  scripts/bcd_baseline.json` en régression ; clippy
  `-D warnings`.
- **Revue jalon** : à chaque M terminé → entrée CHANGELOG +
  case roadmap dans le même commit.

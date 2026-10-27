<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 53 — E-commerce grand public FR (2026-10-27)

- **Binaire** : `[V4-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (WIP `depth_limit` embarqué), `--port 9435`,
  `EXPLORER_VIEWPORT=1366x900`, journal `_vol53_daemon.log`
  (racine, copié `logs/daemons/_vol53_daemon.log`)
- **Cibles** : www.fnac.com · www.darty.com · www.leroymerlin.fr —
  **3/3 bloquées HTTP 403** → substitution www.manomano.fr (même
  famille e-commerce bricolage, précédent vol 51)
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de
  `SYNTHESE.md`
- **RAM daemon (PID 16776)** : 384 Mo au boot → **1 701 Mo** pendant
  l'éval JS manomano → 2 428 Mo → **DELETE onglet 4 → 2 977 Mo** →
  DELETE onglets restants → **croissance continue +~28 Mo/s jusqu'à
  4,27 Go — seuil 2,5 Go franchi → daemon tué par PID**. Fuite
  post-DELETE aggravée : voir proposition §B-30.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| www.fnac.com | **HTTP 403 — page d'erreur moteur** | `HTTP 403` | ⚪ externe anti-bot — nouvelle entrée 403 |
| www.darty.com | **HTTP 403 — page d'erreur moteur** | `HTTP 403` | ⚪ externe anti-bot — nouvelle entrée 403 |
| www.leroymerlin.fr | **HTTP 403 — page d'erreur moteur** | `HTTP 403` | ⚪ externe anti-bot — nouvelle entrée 403 |
| www.manomano.fr (subst.) | **Wall « navigateur incompatible » peint ~95 %** | `RangeError: expression too deeply nested` ×3 + `unexpected '\'` | §B-25 confirmé ×2 · §B-3 garde opérationnelle · §B-9 verbatim · **§B-30 proposé (fuite post-DELETE croissante)** |

Acquis : zéro panic, zéro stack overflow (le garde `depth_limit`
produit un `RangeError` rattrapable là où §B-3 tuait le process) ;
pages d'erreur moteur peintes proprement ×4. Faits marquants :
**manomano redirige spontanément vers `/navigateur-incompatible`**
(2ᵉ cas §B-25) et la fermeture de tous les onglets n'a **pas**
arrêté la croissance mémoire — le daemon a gagné ~1,3 Go après le
dernier DELETE, en silence total.

---

## 1-3. fnac.com / darty.com / leroymerlin.fr — 403 externe ×3

Même signature sur les trois cibles, relevées indépendamment :

- `POST navigate` → `{"error":{"handled":true,"message":"HTTP 403"}}`.
- `/console` verbatim : `{"console":["HTTP 403"]}`.
- `/dom` → **404** (page synthétisée) ; `/mem` : 18 boîtes /
  29 display items, `dom_nodes:0`, `js_heap:0` — page d'erreur
  interne présente en layout.
- Frames (`captures/vol53_{fnac,darty,leroymerlin}.png`) : page
  d'erreur moteur localisée « Page inaccessible — Le serveur a
  répondu « HTTP 403 ». Réessayez plus tard… » peinte proprement —
  dégradation gracieuse conforme (AGENTS §1).
- Attribution **⚪ externe** : fnac et darty (même groupe
  Fnac-Darty) filtrent en amont ; leroymerlin.fr (groupe Adeo)
  applique la même politique. Signature catalogue connue
  (UA `Explorer/0.1`, pas de `sec-ch-ua`/`sec-fetch-*`,
  fingerprint rustls).
  → **+3 entrées pour la liste 403** (§B ⚪) : `www.fnac.com`,
  `www.darty.com`, `www.leroymerlin.fr`.

*Scroll/clic produit non applicables : pages d'erreur synthétiques
sans contenu réel.*

## 4. www.manomano.fr — wall compatibilité peint, JS récursif, fuite post-DELETE

*(substitution pour les cibles 403 — même famille « e-commerce
bricolage » que leroymerlin)*

- `POST navigate` → **jamais retourné (>2 min, curl abandonné)** —
  la session a néanmoins servi l'onglet (titre « Ah. Votre
  navigateur n'est pas compatible. », URL finale
  `https://www.manomano.fr/navigateur-incompatible`).
- `/console` verbatim (4 entrées, `logs/vol53_manomano_console.json`) :
  ```
  [0] exception js : RangeError: expression too deeply nested
      at ToObject (cdnjs.cloudflare.com/polyfill/v3/polyfill.min.js?…:48:1067)
      at e (…polyfill.min.js…:111:98)
      at Get (…polyfill.min.js…:46:702)
      at e (…polyfill.min.js…)   [trace complète dans le JSON]
  [1] exception js : RangeError: expression too deeply nested   [polyfill]
  [2] syntaxe js 40:4585 : unexpected `\` — manomano.fr/assets/_next/
      static/chunks/pages/_app-b6d595afab675cf2.js — src
      « (self.webpackChunk_N_E=self.webpackChunk_N_E||[]).push([[90636],{92:(r,n,… »
  [3] exception js : RangeError: expression too deeply nested   [polyfill]
  ```
- `/mem` (fenêtre intermédiaire) : 126 nœuds DOM, 0 boîtes layout,
  `js_heap` 9,2 Mo, `journal_applies:0` — page wall encore en cours
  de pose au moment du relevé.
- `/dom` → 200, 372 lignes : contient « navigateur-incompatible »,
  « ManoMano », « Votre navigateur », 3 refs polyfill.

### Rendu — wall ~95 % fidèle

- `captures/vol53_manomano.png` : logo ManoMano centré, titre
  « Ah. Votre navigateur n'est pas compatible. », sous-titre
  « Faut dire qu'il n'est pas tout jeune. », instructions Chrome /
  Firefox / Safari / IE complètes, carte blanche sur fond gris —
  layout centré correct, typographie propre.

### §B-25 confirmé — navigation `location` spontanée

Redirection **non sollicitée** de `www.manomano.fr` vers
`/navigateur-incompatible` — 2ᵉ cas terrain après qwant
(vol 44). Feature-detect UA/`navigator.*` côté site qui choisit
le wall plutôt que la page. **OUVERT — recoupement `[V4-main]`.**

### §B-3 — signal positif : le garde `depth_limit` convertit le crash

Les trois `RangeError: expression too deeply nested` tracés sur
`polyfill.min.js` (`ToObject` → récursion) montrent le garde-fou
**à l'exécution** produisant une erreur JS rattrapable — le
process a survécu là où alpinejs.dev tuait le thread
(`STATUS_STACK_OVERFLOW`). À nuancer : ce n'est pas un re-vol
d'alpine — c'est la preuve que *une* forme de récursion est bornée.
**Statut inchangé** (crash réel non re-testé), mais la direction
WIP est validée sur le terrain.

### §B-9 — verbatim parseur nouveau

`syntaxe js 40:4585 : unexpected '\'` dans le chunk webpack
`_app-b6d595afab675cf2.js` (Next.js) — séquence `\` non acceptée
en position où un parseur conforme continue (regex literal ?
escape dans template ?). Verbatim archivé → recoupe §B-9.

### ⚠ §B-30 (proposé) — fuite mémoire post-DELETE en croissance continue

Chronologie mesurée (WorkingSet64 PID 16776) :

| Heure | Événement | RAM |
|---|---|---|
| boot | daemon prêt | 384 Mo |
| ~09:19 | évals manomano (script 1,26 Mo, `pump.drains` 165 ms) | 1 701 Mo |
| +40 s | pendant collecte télémétrie | 2 428 Mo |
| ~09:21 | `DELETE /api/tabs/4` | 2 977 Mo |
| +15 s | `DELETE` onglets 1-3 (pages 403) | 3 406 Mo |
| 09:21:28→37 | **aucun onglet**, API `{"tabs":[]}`, zéro log | 4 035 → 4 275 Mo (**+~28 Mo/s**) |
| 09:21:40 | `Stop-Process 16776` | — |

Distinct de §B-26 (caf.fr : RAM haute **stable** non libérée) :
ici l'allocation **continue après fermeture de tous les onglets**,
sans aucune entrée de journal — suspect : tâche orpheline du
navigate encore in-flight (le POST navigate n'a jamais retourné)
continuant d'allouer après destruction de l'onglet, ou boucle
post-apply survivant à la session. Croissance linéaire non bornée
→ serait monté à l'OOM. Crate `explorer-core`/daemon.

---

## Statuts proposés (§8)

| §B | Bras | Verdict ce vol |
|---|---|---|
| §B ⚪ | liste 403 anti-bot | +3 entrées : `www.fnac.com`, `www.darty.com`, `www.leroymerlin.fr` |
| §B-25 | `location` spontanée non sollicitée | **confirmé ×2 `[V4-main]`** — manomano → `/navigateur-incompatible` |
| §B-3 | stack overflow jscore | signal positif — `RangeError` rattrapable sur récursion `ToObject` polyfill.io, process survivant ; crash alpine non re-testé |
| §B-9 | parser résiduel | verbatim archivé — `unexpected '\'` webpack `_app` `[V4-main]` |
| §B-26 | RAM non libérée au DELETE | recoupement aggravé — voir §B-30 |
| §B-30 (proposé) | **fuite mémoire en croissance continue post-DELETE** — +~28 Mo/s avec 0 onglet, zéro log, navigate POST jamais retourné | **NOUVEAU — `[V4-main]`** (manomano, crate `explorer-core`/daemon — tâche orpheline suspectée) |

## Hygiène de vol

- Onglets 1-4 fermés par `DELETE /api/tabs/{id}` (dans l'ordre
  4 puis 1-3) — la fermeture n'a **pas** contenu la fuite.
- Daemon tué par son **PID 16776** (`Stop-Process -Id`) ; le
  daemon voisin PID 3444 (:9434, autre vol) n'a pas été touché.
- Vol écourté : scrolls ×2 et clic produit non joués (cibles
  originales = murs 403 ; manomano = wall compat + urgence RAM).
  Substitutes ikea/amazon non tentées par prudence post-fuite.
- Artefacts : `captures/vol53_{fnac,darty,leroymerlin,manomano}.png`,
  `logs/vol53_{fnac,darty,leroymerlin,manomano}_{dom,mem}.txt`,
  `logs/vol53_*_console.json`,
  `logs/vol53_{fnac,leroymerlin}_layout.txt` (stubs 404),
  `logs/daemons/_vol53_daemon.log`.
  NB : `vol53_manomano_layout.txt` non capturé (course contre la
  montée RAM) ; mem/dom/frame intacts.

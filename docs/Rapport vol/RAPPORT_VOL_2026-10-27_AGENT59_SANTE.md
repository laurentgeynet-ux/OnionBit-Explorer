<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 59 — Santé / services (ameli · meteofrance · doctolib) (2026-10-27)

- **Binaire** : `[V4-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (WIP `depth_limit` embarqué), `--port 9441`, `EXPLORER_VIEWPORT=1366x900`,
  journaux `_vol59_daemon.log` (PID 2056, tué en urgence §B-26/§B-30) et
  `_vol59_daemon2.log` (PID 17780, vol doctolib après redémarrage)
- **Cibles** : www.ameli.fr (re-vol §B-14 Bootstrap `call of Undefined`) ·
  meteofrance.com · www.doctolib.fr (re-vol wedge historique vol 29) —
  **3/3 volées**
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de `SYNTHESE.md`
- **RAM daemon 1 (PID 2056)** : ~520 Mo boot → 834 Mo (ameli) →
  **4,36 → 4,92 Go en ~5 s sous meteofrance (+~110 Mo/s)** → dent de scie
  2,17 → 0,53 Go → re-croissance 2,3 → 4,1 Go **après DELETE des onglets**
  → **5,10 Go au kill avec `{"tabs":[]}`** — franchissement du seuil 2 Go :
  DELETE + arrêt d'urgence appliqués.
- **RAM daemon 2 (PID 17780)** : 394 Mo stable sur tout le vol doctolib.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| www.ameli.fr | **Excellent** — rendu fidèle, JS sain, clic→nav réelle | `pas de journal` | **§B-14 bras ameli RÉSOLU** (`call of Undefined` + `instanceof Element` disparus) ; §B-13 tient (clic → `target=_blank` → nouvel onglet) ; timeout connect externe annuairesante.ameli.fr (handled) |
| meteofrance.com | **Wedge §B-26 + §B-30 — NOUVEAU PORTEUR** | `writeEmbed is not defined` (iframe `javascript:`), `unhandledrejection: invalid token` ×3 | apply journalisé jusqu'à `layout`, jamais `repaint` ; `/dom` 404, `/frame` 204, `dom_nodes=0` ; churn mémoire en dent de scie jusqu'à 5,1 Go, **poursuivi après DELETE de l'onglet** |
| www.doctolib.fr | **Disjoncteur validé terrain — daemon survit** ; site toujours inutilisable | `pas de journal` | 1 éval 1,5 Mo → 30 012 ms `ok=false` → disjoncteur ouvert, 7 scripts (~4,1 Mo) sautés ; `navigate` retourné en 64 s (vs ~4 min + mort au vol 29) ; `doc_h=0`, page blanche — recoupe §B-23 |

Fait marquant : **le disjoncteur post-commit (`a6fb473`) fait exactement son
travail sur doctolib** — un seul timeout de 30 s au lieu de huit, puis
court-circuit. Et **meteofrance est un nouveau porteur §B-26** avec une
variante de signature nouvelle : la fuite n'est plus monotone mais en
dent de scie, et surtout **la pompe de l'onglet supprimé continue de
tourner** (`pump.commit_lock` ≈ 90 s journalisé 12 min après DELETE) —
recoupement fort de §B-30.

---

## 1. www.ameli.fr — §B-14 bras ameli : RÉSOLU confirmé `[V4-main]`

- `POST navigate` → `doc_h=4 608`, titre « ameli, le site de l'Assurance
  Maladie en ligne | ameli.fr | Assuré », retour en 5,3 s.
- **Console : `pas de journal`** (`logs/vol59_ameli_console.json`) —
  zéro erreur JS. Au vol 29 : `TypeError: cannot read 'call' of
  Undefined` (Bootstrap) + `instanceof Element` rejeté (menu.js/jQuery).
  Journal daemon : ~15 évals `ok=true` (jusqu'à 188 Ko, 40 ms), **aucun
  échec** — la correction JS-C162 (`querySelector*`/`getElementsBy*` sur
  `Element.prototype`) a tué la cascade ameli comme elle l'avait fait
  pour OSM.
- **Frame** (`captures/vol59_ameli.png`) : header (logo Assurance
  Maladie + ameli.fr, « Rechercher », bouton « Se connecter »), hero
  vaccination grippe avec photo, rangée de 5 raccourcis (« Payer en
  ligne », « Complémentaire santé solidaire », « Attestation de droits »,
  « Carte Vitale », « Envoi d'une feuille de soins »), panneau « Comment
  faire ? » — rendu fidèle.
- **Scroll ×2** → `scroll_y=1 600`, repaint correct
  (`captures/vol59_ameli_scroll.png`) : header sticky conservé, cartes
  « Vos outils et services » avec **icônes peintes** (calculette,
  stéthoscope, checklist — carrés vides au vol 29), bandeau jaune « Les
  maladies de A à Z ».
- **Télémétrie** : `dom_nodes=1 530`, `layout_boxes=734`,
  `js_heap=23,4 Mo`, `total≈61 Mo`, `journal_fallbacks=1`.
- **Clic** (570, 460) sur la carte « Annuaire des professionnels de
  santé » → `{"result":"navigated"}` → **nouvel onglet ouvert**
  (`target=_blank` → `https://annuairesante.ameli.fr/`) — §B-13 confirmé
  sain sur cette cible (les clics étaient tous `ignored` au vol 29).
- L'onglet fils échoue proprement : `fetch : timeout connect après
  10 000 ms vers https://annuairesante.ameli.fr` → titre « Erreur de
  chargement », erreur **handled**, API indemne — ⚪ externe/net,
  pas de wedge.
- **WARN bénins ADR-0025** : `journal DOM rejeté → repli re-parse`
  `why=UnmappedKey` ×2 + `why=Fingerprint` ×1 (`journal_fallbacks=3` au
  second `/mem`) — chaque repli suivi d'un apply sain, **sans wedge**
  (contraste §B-26).
- Cosmétique : petit fragment de run tronqué au sommet du hero
  (~x=700, y=295) — vraisemblablement légende/pagination du carrousel,
  marginal.

## 2. meteofrance.com — §B-26 confirmé, nouveau porteur (+ §B-30)

- `POST navigate` → **jamais retourné** (timeout curl `-m 90`). L'onglet
  a néanmoins commité partiellement : titre « PREVISIONS METEO FRANCE -
  Site Officiel de Météo-France… », `can_back=true` (chaîne de redirect).
- **Console** (`logs/vol59_meteo_console.json`) :
  `exception js : ReferenceError: writeEmbed is not defined — iframe —
  src «writeEmbed();»` + `unhandledrejection: invalid token` ×3 —
  erreurs modestes, non bloquantes en apparence.
- **Signature §B-26 textbook** (journal `_vol59_daemon.log`) :
  apply n°2 à 09:44:42 — `cascade ms=166 → boxtree ms=191 →
  layout ms=253` puis **jamais `repaint`/`fin`** ; 3 évals JS ok avant
  (dont 806 Ko en 517 ms). `/dom` → **404**, `/frame` → **204**,
  `/mem` : `dom_nodes=0, layout_boxes=0, display_items=0`,
  `journal_fallbacks=3` — DOM détaché terminal.
- **Variante RAM en dent de scie** (PID 2056) : 4,36 → 4,92 Go (+~110 Mo/s
  sur 5 s, signature radiofrance) **puis** 2,17 → 0,53 Go — des
  allocations géantes transitoires, pas une fuite monotone bfmtv-style.
- **§B-30 recoupé — pompe orpheline post-DELETE** : `DELETE tab 3`
  accepté à ~09:46, mais le journal montre `pump.commit_lock
  us=89 918 900` (~90 s !) **à 09:58:16 pour `tab=3`** — la pompe d'un
  onglet supprimé tient encore des sections ~90 s 12 min après sa
  destruction ; la RAM repart 2,3 → 4,1 → **5,10 Go au kill alors que
  `{"tabs":[]}`**. Confirme le suspect §B-30 (tâche navigate/pompe
  orpheline allouant après destruction) **et** la métrique proposée
  `apply_started_without_fin`.
- **API vivante tout du long** : `/tabs`, `/mem`, `/dom`, `/frame`,
  `DELETE` tous répondus — contrairement à bfmtv vol 49 (API morte).
- Vue externe : la dernière éval connue est `bytes=806 828 ok=true` ;
  le wedge intervient au 2ᵉ apply, probablement déclenché par les
  mutations DOM de ce bundle (météo = page à widgets/iframes pubs,
  famille §B-29).

## 3. www.doctolib.fr — disjoncteur validé, bootstrap toujours tué

- `POST navigate` → **retourné en 64 s** (vol 29 : ~4 min de verrou
  puis daemon mort — 8 évals × 30 s séquentiels). Titre « Doctolib :
  Prenez rendez-vous en ligne chez un soignant », `doc_h=0`.
- **Disjoncteur `a6fb473` en conditions réelles** (journal
  `_vol59_daemon2.log`) : 8 évals `ok=true` (jusqu'à 143 Ko en 891 ms),
  puis `eval… bytes=1 543 906` → `eval fin ms=30 012 ok=false` →
  `WARN moteur figé — disjoncteur ouvert, appels JS court-circuités`
  → 7 scripts sautés (~4,1 Mo, dont un bundle de 3,6 Mo) en ~15 ms,
  apply final `repaint ms=6` — **un seul timeout de 30 s au lieu de
  huit**. Le wedge vol 29 est **RÉSOLU** au sens « daemon tué » :
  process sain, API 100 % responsive, RAM 394 Mo stable.
- **Mais la page reste blanche** (`captures/vol59_doctolib.png`) :
  `doc_h=0`, layout = `block<html> [0 0 1366x0]` (1 boîte),
  `display_items=0`, `dom_nodes=442` parsés (dump 1,68 Mo —
  `logs/vol59_doctolib_dom.txt`), `js_heap=0` (moteur désarmé).
  L'éval de 1,5 Mo qui dépasse 30 s est le bootstrap applicatif —
  sans lui, rien ne se construit. Même état final qu'au vol 29.
- **Console : `pas de journal`** — l'échec est dans le journal daemon
  (`ok=false` + disjoncteur), pas remonté au ring console.
- Scroll → `scroll_y=0` (doc_h=0), clic (683, 450) → `ignored` (rien
  à toucher) — comportements cohérents avec une page vide, pas des bugs.
- **Attribution ouverte** : l'éval de 1,5 Mo > 30 s est soit une
  compile/exécution légitime trop lente (**recoupe §B-23** « eval
  deadline trop agressif au boot » — seuil à étalonner/exempter en
  phase boot, cf. JS-C163 non livré), soit un vrai hang moteur sur ce
  bundle — non tranché depuis l'API. Le framing honnête : **le
  garde-fou tient, le site ne passe pas**.

## Statuts proposés (§8)

| §B | Bras | Verdict ce vol |
|---|---|---|
| §B-14 | `call of Undefined` Bootstrap + `instanceof Element` (ameli) | **RÉSOLU confirmé `[V4-main]`** — console vide, ~15 évals ok, icônes peintes, clic→nav |
| §B-13 | hit-test `ignored` en série | **confirmé sain `[V4-main]`** — clic carte ameli → `navigated` + ouverture onglet `target=_blank` |
| §B-26 | wedge DOM détaché post-apply + fuite mémoire | **confirmé `[V4-main]` — nouveau porteur meteofrance** ; variante dent de scie (pics 5,1 Go), API vivante, `pump.commit_lock` ~90 s visible dans `sync_profile` |
| §B-30 | fuite post-DELETE sans borne | **confirmé `[V4-main]` — reproduction directe** : `tabs=[]` → RAM 2,3→5,1 Go, pompe `tab=3` active 12 min après DELETE |
| §B-23 | `eval deadline` au boot | **recoupement `[V4-main]`** — doctolib : éval 1,5 Mo tuée à 30 012 ms ; disjoncteur épargne le daemon mais le bootstrap meurt → page blanche persistante |
| ⚪ externe | timeout connect annuairesante.ameli.fr (10 s) | erreur typée handled, pas de wedge — vraisemblablement refus côté serveur/réseau |
| ADR-0025 | rejets journal `UnmappedKey`/`Fingerprint` | recoupement bénin — 3 fallbacks ameli, tous replis sains |

## Hygiène de vol

- Daemon 1 (PID 2056) : onglets 2 (annuaire), 3 (meteo), 1 (ameli)
  supprimés par `DELETE /api/tabs/{id}` — `{"tabs":[]}` confirmé ;
  tué par **PID** (`taskkill //PID 2056 //F`) à 5,10 Go — seuil 2 Go
  franchi, arrêt d'urgence conforme.
- Daemon 2 (PID 17780) : onglet 1 (doctolib) supprimé, tué par
  **PID** à 385 Mo — jamais `taskkill /IM`.
- Artefacts : `captures/vol59_{ameli,ameli_scroll,ameli_click,
  ameli_click2,annuaire,meteo(204 vide),doctolib}.png` ;
  `logs/vol59_{ameli_console,ameli_console2,ameli_dom,ameli_layout,
  ameli_mem,ameli_mem2,annuaire_console,meteo_console,meteo_dom(404),
  meteo_mem,doctolib_console,doctolib_dom,doctolib_layout,doctolib_mem,
  doctolib_mem2}.{json,txt}` ; journaux `_vol59_daemon.log` +
  `_vol59_daemon2.log` (racine).
- Limites honnêtes : la cause exacte de la boucle d'allocation
  meteofrance n'est pas bissectée (pompe vs apply vs retry) — la
  signature `pump.commit_lock ≈ 90 s` post-DELETE est la piste la plus
  chaude ; la nature « deadline vs hang réel » de l'éval doctolib
  reste non tranchée.

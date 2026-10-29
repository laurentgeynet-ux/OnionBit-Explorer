<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 78 : speedtest · messenger · twitch `[V6-main]`

- **Binaire** : `[V6-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (contient la borne §B-26 `emit_scoped_at` `1e24942`, le fix
  §B-45 `34a019b`, §B-47 `e268ec7`/`8ddd26d`, §B-49 `741f810`,
  et — découverte de ce vol — **l'instrument T2
  `evals_sans_mutation` livré** : le WARN se déclenche seul sur
  twitch).
- **Mission** : 3 cibles — speedtest.net (terrain §B-16 WebGL/mesure),
  messenger.com (nouvelle cible), twitch.tv (re-vol §B-4c
  « exécution sans effet » : React monte-t-il ?).
- **Daemon** : PID 8656, port 9461, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol78_daemon.log`. **RAM max ~1 246 Mo** — sous le seuil
  de prudence 1,5 Go, stable en fin de vol (~1 224 Mo avec 3
  onglets vivants dont deux SPA JS actives).
- **Fenêtre** : ~15:10 → 16:15 UTC (~65 min — deux fenêtres de
  collecte : 15:10-15:13 puis 16:11-16:15).
- **Verdict global** : **2 applications React réellement montées et
  interactives** (speedtest : app complète + clic GO → mutation DOM ;
  messenger : consentement cookies fonctionnel, formulaire de
  connexion complet), **1 reproduction verbatim de §B-4c** (twitch :
  44/44 évals `ok=true` — le même nombre qu'au vol initial —, 0
  mutation DOM, shell SSR jamais remplacé ; désormais **auto-détecté**
  par le WARN `evals_sans_mutation`).

---

## 1. speedtest.net — app montée, GO cliquable, §B-26 tiré, APIs UA manquantes

`POST /navigate` → **7,0 s** — titre complet « Speedtest by Ookla —
The Global Broadband Speed Test », `doc_h=2 266` (`clamped:false`),
URL stable, 0 nav auto.

**Rendu** (`captures/vol78_speedtest.png`) : page réelle — header
SPEEDTEST (logo, Menu, English, bascule thème soleil/lune peinte,
Download, Sign In), onglets Results/Settings avec icônes vectorielles
(coche, engrenage — SVG fonctionnels), état dynamique **« Finding
provider » / « Finding optimal server… »**, bouton GO en dégradé
cyan→bleu, section Connections. Scroll ×2 sain (`scroll_y=800` puis
`1 366` = bas de doc) : `vol78_speedtest_scroll.png` peint le pied de
page complet (Speedtest Global Index™, Performance Directory™, Ookla
Research, marques Downdetector®/Ekahau®/RootMetrics®, liens Apps
Android/Apple TV/iOS/macOS/Windows, copyright) — texte net, pas de
tofu PUA (fix icônes tient).

**Clic GO (612,540)** → `edited` (livré au DOM) puis **mutation
réelle** : `vol78_speedtest_go.png` montre l'app passée en état
« test » — boîte bordée cyan apparue au-dessus du GO (zone du cadran
— canvas/WebGL resté vide → famille **§B-16**, attendu hors scope),
options « Multi »/« Single » révélées. Le handler React a donc
exécuté un vrai changement d'état. « Finding optimal server… »
persiste : la sonde serveur Ookla (XHR temps réel) n'aboutit pas —
cause non qualifiée (fetch/API réseau côté jscore, même famille que
les timeouts N2 ou la géoloc §B-14).

**`/console` verbatim** (`logs/vol78_speedtest_console.json`) —
4 signatures, toutes ad-tech Amazon (`c.amazon-adsystem.com`) :
- `uncaught: TypeError: cannot set 'amazonStatus' of Undefined`
  (inline:6, ×2) — objet global absent, cascade honnête ;
- `unhandledrejection: 'push' is not a function (on Num(1.0))` —
  apstag.js ;
- `unhandledrejection: 'getHighEntropyValues' is not a function
  (on Object)` ×2 — **`navigator.userAgentData` absent** → entrée de
  plus pour la matrice **§B-14** (`explorer-jscore` : APIs
  navigateur manquantes — la feature-detect UA-CH d'Amazon jette).

**Télémétrie** (`logs/vol78_speedtest_mem.json`) : `dom_nodes=1 120`,
`layout_boxes=609`, `display_items=263`, `frame_bytes=12 381 624`,
`images_bytes=23 829 108`, `js_heap=36 127 754` (~34 Mio — l'app
tourne), `journal_fallbacks=0`.

**Borne §B-26 tirée ×1 sur cette cible** : `cycle de report cassé
dropped=1 path=[0,0,0,0,0,0]` à 15:10:56 — le filet tient, apply
mené à terme, process sain (comportement nominal du fix `1e24942`).

**Constats** : §B-14 (`userAgentData`), §B-16 (cadran canvas vide —
attendu), sonde serveur bloquée (à rapprocher N2/§B-14 géoloc) ;
**le reste est un succès de bout en bout** — une SPA de mesure
temps réel monte, rend et répond au clic.

## 2. messenger.com — React monté, consentement cookies fonctionnel (succès)

`POST /navigate` → **5,1 s** — titre « Messenger », `doc_h=554,9`
(< viewport 900 → scroll légitimement à 0 ; les deux `scroll dy=800`
retournent `scrolled scroll_y=0`, pas un blocage).

**Rendu** (`captures/vol78_messenger.png`) : vraie page localisée
français — logo Messenger, hero (« Soyez au plus près de vos êtres
chers au quotidien », « Créez des communautés… »), bandeau de
consentement complet (« Voulez-vous autoriser l'utilisation des
cookies par Messenger sur ce navigateur ? », sections « À propos des
cookies », boutons « Refuser les cookies optionnels »), formulaire
de connexion, badge App Store, « from ∞ Meta ». Quelques dégradations
visuelles : hero tronqué en haut, illustrations des cartes absentes
(images décoratives), champs e-mail/mot de passe compressés →
famille **§B-18** (layout résiduel).

**Clic « Refuser les cookies optionnels » (680,862)** → `edited` —
**le bandeau disparaît réellement** (`vol78_messenger_click.png`) :
le handler React s'est exécuté, consentement enregistré, re-layout
propre. Derrière : formulaire complet (« Adresse e-mail ou mobile »,
« Mot de passe », « Se connecter », « Mot de passe oublié ? »,
« Rester connecté(e) »), badges App Store/Google Play, nav
« Confidentialité et sécurité / Pour les développeur·ses / Pages
d'aide », pied de page (Politique de confidentialité, Conditions
générales, « Français (France) ▼ »). Preuve layout : bouton à
`[556 853 220×20]` dans `logs/vol78_messenger_layout.txt` — clic
centré dessus, `x=680 y=862` ∈ boîte.

**`/console` verbatim : `pas de journal`** — et pourtant le journal
daemon compte **7 évals `ok=false ms=0`** : le même script inline de
**1 202 octets** ré-évalué après chaque re-parse `UnmappedKey` et
échouant instantanément, **sans jamais produire d'entrée console**.
105 évals `ok=true` (bundle principal ~469 Ko ré-évalué à chaque
re-parse — coût non trivial). **Anomalie d'observabilité** : un
échec d'éval qui n'atteint ni `/console` ni un WARN — nouvelle
variante possible de mort silencieuse → **proposition §B-51**
(`explorer-core`/`jscore` : router les `ok=false` post-re-parse
vers la console, ou logger la raison).

**Télémétrie** (`logs/vol78_messenger_mem.json`) : `dom_nodes=1 138`,
`layout_boxes=541`, `display_items=3 384`, `js_heap=48 756 076`
(~46 Mio), `images_bytes=23 384 436`.

**Re-parses `UnmappedKey` ×10** sur cet onglet en ~1 h (ADR-0025) —
tous récupérés (`apply: fin` journalisé après chaque repli, DOM
toujours réattaché, aucune oscillation §B-50). Rythme : 4 au boot,
puis rafales de 2-3 pendant la fenêtre de collecte 16:12 — corrélé
aux apply déclenchés par l'activité JS résiduelle de l'onglet.
**Borne §B-26 tirée ×3** (`dropped=1 path=[0,1]` à 15:12:43,
15:13:09, 16:12:19) — le filet absorbe, le site reste interactif.

**Constats** : succès majeur (app Meta montée + interactive) ;
§B-18 (hero/champs) ; §B-51 proposé (évals `ok=false` muettes) ;
UnmappedKey ×10 récupérés + §B-26 ×3 nominal.

## 3. twitch.tv — §B-4c reproduit verbatim : 44/44 évals ok, React jamais monté

`POST /navigate` → **5,0 s** — titre « Twitch », `doc_h=900,0`
(= viewport : page shell à hauteur fixe).

**`/console` verbatim : `pas de journal`** — zéro erreur, comme au
vol initial.

**Journal daemon — le constat §B-4c à l'identique** : **44 évals,
44× `ok=true`, 0 `ok=false`** — exactement le même décompte que le
porteur initial (44/44, bundles ~355 Ko). Le runtime webpack
s'exécute : l'éval de 356 941 o (`ms=40`) pose la table des chunks
`assets/` (visible dans le dump DOM : manifest complet sites.sunlight/
features.chat/pages.channel…). **Et zéro mutation DOM** :
`dom_nodes=246`, `layout_boxes=22`, `display_items=18`,
`js_heap=7 264 444`.

**Nouveauté de ce binaire — l'instrument T2 est livré et fonctionne** :
```
WARN explorer_core::session::net: evals_sans_mutation — boot
possiblement muet (aucune mutation DOM) tab=3 evals=44
```
Le détecteur proposé dans §B-4c (« instrument T2 `evals_sans_mutation`
proposé comme détecteur ») tire automatiquement — la mort silencieuse
n'est **plus silencieuse côté journal** (reste absente de `/console` —
le WARN daemon seul ; remonter en PipelineError ferait le lien T2).

**Rendu** (`captures/vol78_twitch.png`) : shell SSR seul — logo
Twitch, barres squelettes du `shell-nav` (gris, sans texte), un tofu
au centre, tout le reste blanc. Détail pipeline : le `.shell-loader`
est `opacity:0` + `animation:shell-display .1s .3s forwards` —
l'animation CSS n'étant pas exécutée, le conteneur reste à
`opacity:0` **définitivement** (observation : les animations CSS ne
tournent pas → `fill-mode:forwards` jamais atteint — cosmétique,
famille §B-17/§B-18, à confirmer sur repro minimal).

**Scroll ×2** : `scrolled scroll_y=0` ×2 (doc_h=900=viewport —
légitime). **Clic (100,200)** → `edited` ; frame post-clic
**byte-identique** (`vol78_twitch_click.png` ≡ `vol78_twitch.png`) —
rien ne monte, rien ne s'anime.

**Télémétrie** (`logs/vol78_twitch_mem.json`) : `dom_nodes=246`,
`layout_boxes=22`, `display_items=18`, `frame_bytes=4 917 600`,
`js_heap=7 264 444`, `journal_fallbacks=10` *(compteur remonté par
`/mem` — au vu du journal daemon, les 10 rejets `UnmappedKey` de la
session sont tous `tab=2` ; le compteur `/mem` semble session-wide,
pas per-tab — à clarifier, entrée d'observabilité)*.

**Statut §B-4c : TOUJOURS PRÉSENT `[V6-main]`** — reproduction
verbatim (même nombre d'évals), mais **désormais instrumentée** :
T2 livré, le WARN `evals_sans_mutation` qualifie le bras (c) sans
bissection manuelle.

---

## Synthèse des constats

| Cible | Résultat | IDs §B | Crate |
|---|---|---|---|
| speedtest.net | App montée, interactive ; `userAgentData` absent ; cadran canvas vide ; sonde serveur bloquée | §B-14 (entrée `getHighEntropyValues`/`userAgentData`), §B-16 (terrain attendu), sonde → N2/§B-14 | `explorer-jscore` |
| messenger.com | **Succès** — React monté, consentement fonctionnel ; hero/champs dégradés ; évals `ok=false` ×7 muettes ; UnmappedKey ×10 récupérés | §B-18 ; **§B-51 proposé** (échecs d'éval sans trace console) | `explorer-layout` ; `explorer-core`/`jscore` |
| twitch.tv | §B-4c **reproduit verbatim** (44/44 ok, 0 mutation, React absent) — **auto-détecté** par `evals_sans_mutation` | §B-4c (statut : TOUJOURS PRÉSENT, instrumenté) ; T2 livré | `explorer-core`/`jscore` |

### Proposition d'entrée §B — numéro suivant (51)

| # | Anomalie | Crate | Preuve |
|---|---|---|---|
| 51 | **Évals `ok=false` post-re-parse sans trace** — après repli `UnmappedKey`, un script ré-évalué échoue (`ms=0`, répété ×7) sans entrée `/console` ni WARN ; à distinguer : erreur non routée vs éval sautée | `explorer-core`/`jscore` | messenger `[V6-main]` vol 78 — `_vol78_daemon.log` evals 1 202 o `ok=false` ×7, `/console` « pas de journal » |

### Mises à jour de statut suggérées (SYNTHESE.md)

- **§B-4c twitch** : annoter « reproduit verbatim `[V6-main]` vol 78
  (44/44 `ok=true` identiques) — **T2 `evals_sans_mutation` livré et
  déclenché** (WARN daemon ; reste à remonter en PipelineError
  `/console`) ».
- **§B-14** : ajouter `navigator.userAgentData`/
  `getHighEntropyValues` (speedtest apstag `[V6-main]` vol 78).
- **§B-16** : terrain — l'app speedtest monte et répond au clic ; la
  zone cadran (canvas/WebGL) reste vide, conforme au stub.
- **T2 (§E-bis Terrain)** : peut passer « livré » — validé en
  conditions réelles.
- **`journal_fallbacks` `/mem`** : vérifier sémantique per-tab vs
  session (twitch remonte 10 alors que les rejets sont tous tab=2).
- **§B-30** : signal favorable de plus — DELETE des 3 onglets →
  purge immédiate 1 224→**133 Mo stable** (+0 fuite sur 8 s
  d'observation), API responsive ; cohérent avec orsay vol 67.

## Artifacts

- Captures : `captures/vol78_speedtest.png`, `vol78_speedtest_scroll.png`,
  `vol78_speedtest_go.png`, `vol78_messenger.png`,
  `vol78_messenger_scroll.png` (scroll_y=0, identique),
  `vol78_messenger_click.png`, `vol78_twitch.png`,
  `vol78_twitch_click.png` (byte-identique).
- Logs : `logs/vol78_{speedtest,messenger,twitch}_{console.json,
  dom.txt,layout.txt,mem.json}` ; journal daemon
  `logs/daemons/_vol78_daemon.log` (~360 k lignes —
  `commit.eval_mod` de messenger en rafale).
- Arrêt propre : `DELETE` des onglets 1-3 puis kill du PID 8656.

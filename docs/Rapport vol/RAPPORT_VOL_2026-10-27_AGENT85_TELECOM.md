<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 85 : free · sosh · edf `[V7-main]`

- **Binaire** : `[V7-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (horodaté 27/10 18:30).
- **Mission** : 3 cibles télécom/énergie — www.free.fr · www.sosh.fr ·
  www.edf.fr. Prudence §B respectée (aucune cible liste
  26/30/33/41/43/46/48/50 ni RTL/funradio).
- **Daemon** : PID **10548**, port 9468, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol85_daemon.log` (65 541 lignes) → `logs/daemons/`.
- **RAM** : base ~371 Mo → pics 905 (free) / 452 (sosh) / 677 Mo (edf) —
  **seuil 1,5 Go jamais franchi**, purge complète ~420-446 Mo après
  chaque DELETE (aucune fuite §B-30).
- **Verdict global** : **2/3 sites volés utiles, 0 wedge**. free.fr =
  meilleure cible (8/8 évals ok, clic → vraie navigation). **sosh.fr =
  coquille vide** : unique bundle applicatif 863 Ko `ok=false ms=92`
  **silencieux** (§B-51 confirmé) → page blanche à jamais. **edf.fr =
  rendu ~85 % mais page verrouillée** : la popin consentement
  TarteAuCitron est peinte **décalée +~50 % viewport à droite**
  (anomalie nouvelle → proposition §B-52), ses boutons hors viewport,
  scroll légitimement bloqué (`body.tc-modal-open{position:fixed}`) —
  page inutilisable ; second `ok=false` silencieux (§B-51 ×2).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| www.free.fr | Rendu ~80 % — header/hero/cartes peints ; clic « Freebox » → **nav réelle `/freebox`** | `pas de journal` — 8/8 évals `ok=true` (bundles 485+503 Ko) | §B-18 (chevauchements runs hero /freebox) + icônes tofu · garde-fou §B-26 tiré ×2 |
| www.sosh.fr | **Coquille vide** — bandeau bleu 50 px + filet, DOM 38 nœuds, `doc_h=101` | `pas de journal` — **mensonger** : bundle `hptu.*.js` 863 560 o `ok=false ms=92` | **§B-51 confirmé** → app jamais montée (§B-4 consequence) |
| www.edf.fr | Rendu ~85 % — hero + cartes peints, **mais page verrouillée** par popin consentement décalée hors viewport | `pas de journal` — mensonger : 605 454 o `ok=false ms=664` + 3 009 o `ok=false ms=0` | **§B-51 ×2** + **§B-52 nouveau** (popin `left:683px`+translate → x=1046, déborde 320 px) + §B-42/43-famille (backdrop avale clics, 3/3 `edited`) |

Points marquants :

- **§B-51 devient le défaut le plus fréquent du vol** : 3 cibles,
  **2 bundles applicatifs tués au boot sans aucune trace** — sosh
  (863 Ko, ms=92 — échec *précoce*, probablement parse/resolve) et
  edf (605 Ko, ms=664). `/console` = « pas de journal » des deux
  côtés alors que `eval fin … ok=false` est bien journalisé. Sur
  sosh la conséquence est terminale : DOM = 38 nœuds de coquille,
  `js_heap=2,3 Mo`, `prerenderReady=false` jamais levé, sonde
  anti-bot `sonde-z_img` sans suite.
- **edf.fr : popin consentement décalée hors viewport** — le
  dialogue `#popin_tc_privacy` (`role="dialog"`,
  `style="top:450px; left:683px"` posé par JS) est layouté/peint à
  **`[1046 450 640x765]`** au lieu de ~`[363 450 …]`
  (1366/2 − 640/2 = 363) : débordement de **320 px** à droite,
  boutons « Accepter » (x=1374) et « Continuer sans accepter »
  (x=1479) hors viewport. Le décalage ≈ +50 % de la largeur
  suggère un `translate(-50%)` centrage appliqué en sens inverse
  (ou base de % erronée) sur élément `position:fixed`.
- **Verrouillage en cascade** : `body.tc-modal-open{position:fixed;
  overflow:hidden}` → `scroll_y=0` aux deux envois de dy=800
  (comportement *correct* — le site verrouille volontairement),
  `.tc-privacy-overlay` fixe `[0 0 1366x900]` z=999997 → les
  3 clics testés (carte hero, header, **lien à l'intérieur de la
  popin**) retournent `edited` — la popin elle-même semble
  sous-couverte par son overlay (z 999998 vs peinture : la popin
  *est* peinte par-dessus → écart ordre paint/hit-test, famille
  §B-42/§B-43). Résultat : page réelle mais totalement bloquée.
- **Re-parse `UnmappedKey` ×4 absorbés** sur edf — `journal DOM
  rejeté → repli re-parse (ADR-0025)`, `journal_fallbacks=4`,
  `apply: fin ms=175`, page vivante (signature §B-50 saine,
  correctif tient).
- **free.fr valide le couple SSR+hydratation légère** : les 2 gros
  chunks Next.js (485/503 Ko) passent `ok=true`, clic header
  sticky à `scroll_y=1600` → `navigated` → `/freebox`
  (contre-exemple §B-40 confirmé à nouveau).

---

## 1. www.free.fr — Next.js sain, navigation réelle

`POST /navigate` → `doc_h=6 467` non clampé, titre « Freebox,
offres internet Très Haut Débit fibre, ADSL - Free ».

**`/console` verbatim** : `pas de journal` — honnête cette fois :
8/8 évals `ok=true` (`script: eval fin tab=1 ms=0-3`), dont les
chunks applicatifs de 485 435 o et 503 290 o (au boot et après
re-nav). DOM = coquille Next.js (`self.__next_f.push` ×N dans le
dump) **hydratée correctement**.

**Rendu** (`captures/vol85_free.png`) : header (logo « free »
rouge, nav Freebox/Forfaits mobiles/Téléphones/TV/Bons Plans,
« Free Pro · Assistance · Boutiques · Actualités », « Espace
Abonné »), bandeau bleu « Appelez le 1044 » (2 icônes absentes =
tofu PUA muet connu), hero carrousel « Hello, hello. » + bouton
« Commander » + mentions légales, section « Nos offres internet
et mobiles à la une. ». Grande zone grise = piste de carrousel
(partielle — transition/slides non hydratés, logo Free FLEX
coupé à mi-droite — §B-18-adjacent).

**Télémétrie** (`logs/vol85_free_mem.json`) : `dom_nodes=2 592`,
`layout_boxes=1 494`, `display_items=1 697`, `frame_bytes=35,3 Mo`,
`images_bytes=100,4 Mo`, `js_heap=1,9 Mo`, `journal_fallbacks=0`.
Daemon : `cycle de report cassé (§B-26) dropped=1 path=[0,0]` ×2 —
**borné, aucun wedge**.

**Scroll ×2 → `scroll_y=1 600`** (`captures/vol85_free_scroll.png`)
: tout peint — header sticky repeint, section « Nos smartphones. »
complète (cartes iPhone 18 Pro / Galaxy S26 Ultra + badges
réparabilité), « Les bons plans Free. ». **Pas de §B-8.**

**Clic** « Freebox » (205,62) à scroll_y=1 600 → `navigated` →
`https://www.free.fr/freebox` (`captures/vol85_free_freebox.png`).
Rendu de la page interne ~80 % : hero « Combo gagnant » peint
mais **chevauchement de runs** — bandeau rouge traversant
« sur le Forfait Free 5G+ » et prix « €/mois » tronqué en haut
(→ §B-18, chevauchements inline confirmés). Console post-nav :
toujours vierge. RAM pic 905 Mo → purge 420 Mo au DELETE.

## 2. www.sosh.fr — bundle mort en silence, coquille vide (§B-51)

`POST /navigate` → retour normal `200` mais `doc_h=101`,
titre `"‍"` (invisible). À +10 s : inchangé.

**`/console` verbatim** : `pas de journal` — **mensonger §B-51** :
le journal daemon montre `script: eval… tab=2 bytes=495` →
`ok=true`, puis `script: eval… tab=2 bytes=863560` →
**`eval fin tab=2 ms=92 ok=false`** — le bundle applicatif
`hptu.8c8ac3d0da6da8c725fc.js` (unique script defer du site)
meurt en 92 ms, **aucune entrée console**. Échec précoce
(<100 ms) → suspect parse/resolve plutôt qu'exécution profonde.

**Rendu** (`captures/vol85_sosh.png`) : bandeau bleu ~50 px +
filet à ~100 px, **reste blanc total** — DOM 38 nœuds :
`<header id="s-header">` avec 2 placeholders `oecs__*`,
`<main id="sg1-sections">` vide, `<img id="sonde-z_img"
style="display:none">` (sonde anti-bot Orange, meta
`property="orange: sonde"`). Script inline `window.prerenderReady
= false` — jamais levé (le JS qui devait peupler est mort).

**Télémétrie** (`logs/vol85_sosh_mem.json`) : `dom_nodes=38`,
`layout_boxes=6`, `display_items=6`, `js_heap=2,3 Mo`,
`journal_fallbacks=0`, phases pump ~200 µs (page inerte).

**Scroll ×2** : `scroll_y=0` (rien à défiler). **Clic** centre
(683,450) → `ignored` (rien sous le curseur). Comportement
API correct — la page est simplement vide.

## 3. www.edf.fr — popin consentement hors viewport → page verrouillée

`POST /navigate` → `doc_h=3 747`, titre « EDF France | Groupe
EDF » (site corporate groupe).

**`/console` verbatim** : `pas de journal` — **mensonger §B-51
×2** : 14 évals, 12 `ok=true`, **2 `ok=false` non tracés** —
3 009 o `ms=0` et **605 454 o `ms=664`** (2ᵉ bundle applicatif).
`js_heap=35,7 Mo` → JS substantiel mais partiellement mort.

**Rendu** (`captures/vol85_edf.png`) : logo EDF, nav (« Vous
êtes », « Aide & Contact », « fr », « Mon espace client »),
hero « Ensemble, accélérons l'électrification pour un avenir
bas carbone » + photo + paragraphe + 3 cartes-liens. **La popin
consentement TarteAuCitron déborde à droite** : texte tronqué au
bord du viewport (« Un petit mot su… », bouton « Personnalis… »
coupé) et **toute la page assombrie** par l'overlay.

**Preuve layout** (`logs/vol85_edf_layout.txt`) :
`#popin_tc_privacy` `style="top:450px; left:683px"` → boîte
`[1046 450 640x765]` — déborde de **320 px** à droite alors que
le centrage attendu est x=363. Boutons :
`block<button> [1479.6 466 …]` « Continuer sans accepter »
(hors écran), `inline<button> [1083 1169.3 …]` « Personnaliser »,
`inline<button> [1374 1169.3 …]` « Accepter » (hors écran).
`.tc-privacy-overlay` = `block<div> [0 0 1366x900]` z=999997 —
backdrop plein viewport.
`body.tc-modal-open{position:fixed; overflow:hidden}` présent
dans la feuille → **scroll verrouillé est comportement site**.

**Scroll ×2** : `scroll_y=0` les deux fois, PNG byte-identique
(5 475 238 o). Verrou cohérent avec `position:fixed` sur body —
**pas un bug** en soi, mais rend l'échec de la popin fatal.

**Clics 3/3 `edited`** (« livré au DOM — aucune action par
défaut », journal `session::input`) :
- carte hero « Nos aides pour passer à l'électrique » (308,671)
  — ancre réelle `href="https://www.jepassealelectrique.fr/…"
  target="_blank"` ;
- header « Mon espace client » (1200,23) — `<button>` (edited
  légitime) ;
- **lien « sites edf.fr » à l'intérieur de la popin** (1210,601)
  — `<a href>` réel, aurait dû naviguer : intercepté par
  l'overlay bien que la popin soit peinte au-dessus → écart
  ordre de peinture vs ordre de hit-test (famille §B-42/§B-43).

**Re-parse** : `journal DOM rejeté → repli re-parse why=UnmappedKey`
×4 → `journal_fallbacks=4`, `apply: fin ms=175`, aucune mort
(→ §B-50 tient). RAM pic 677 Mo → purge 446 Mo au DELETE.

## Statuts proposés (§8)

| §B | Bras | Verdict ce vol |
|---|---|---|
| §B-51 | évals `ok=false` silencieuses | **confirmé `[V7-main]` ×2 cibles / 3 évals** — sosh 863 560 o `ms=92` (coquille vide, conséquence terminale), edf 605 454 o `ms=664` + 3 009 o `ms=0` ; `/console` vierge à chaque instant ; **premier cas à effet observable total** (sosh = site entier absent) |
| §B-52 **(nouveau)** | **`position:fixed` + centrage translate mal résolu** | **proposition d'entrée** — popin `#popin_tc_privacy` `left:683px` (50 % de 1366) layoutée `[1046 450 640x765]` au lieu de x≈363 : décalage +363 ≈ +50 % de sa largeur → suspect `translate(-50%)` appliqué en sens inverse (ou % sur mauvaise base) ; bloque tout le site consentement ouvert |
| §B-42/43 | hit-test vs overlays `fixed` | **+1 instance écart paint/hit-test** — popin peinte au-dessus de l'overlay mais clics intra-popin avalés (`edited`) : ordre de hit-test ≠ ordre de peinture sur éléments z-index voisins (999997/999998) |
| §B-40 | hit-test ancre scrollée | **contre-exemple ×2** — free : clic header sticky à scroll_y=1 600 → nav `/freebox` correcte ; le défaut trainline non universel |
| §B-18 | layout résiduel | **+2 instances** — chevauchements runs hero `/freebox` (bandeau rouge traversant le titre, prix tronqué) ; carrousel free.fr partiel (slide coupée) |
| §B-26 | borne cycle de report | garde-fou tiré ×2 (`dropped=1 path=[0,0]`, free) — borné, aucun wedge |
| §B-50 | re-parse post-apply | **signal favorable** — `UnmappedKey` ×4 (edf) absorbés proprement (`apply: fin ms=175`, page vivante) |
| §B-30 | fuite post-DELETE | **signal favorable ×3** — purge complète ~420-446 Mo après chaque DELETE, `{"tabs":[]}` final |
| §B-4 | morts silencieuses | **bras (b)/(c) recoupés** — sosh : app jamais montée sans erreur tracée ; edf : JS partiel (35,7 Mo heap) mais popin jamais centrée (la cause exacte de `ok=false` reste invisible) |

**Nouvelle entrée §B-52 proposée** : « Popin/dialog `fixed`
centré par `left:50%`+`translateX(-50%)` peint à `left:50%` +
**50 % de largeur** → déborde hors viewport » — `explorer-layout`
/`css` — preuve `logs/vol85_edf_layout.txt` + `vol85_edf.png` —
**OUVERT — terrain V7** (bloquant : consentement inatteignable →
site verrouillé entier).

## Hygiène de vol

- Onglets 1 (free), 2 (sosh), 3 (edf) — tous `DELETE` ;
  `{"tabs":[]}` final, purge mémoire complète à chaque fois.
- Daemon tué par **son PID 10548** seul — jamais `/IM` ; les
  daemons voisins (PID 7296, 13000, 16516) intacts.
- Artefacts : `captures/vol85_{free,free_scroll,free_freebox,
  sosh,edf,edf_scroll}.png` ;
  `logs/vol85_*_{console,dom,layout,mem}.json/txt` ;
  `logs/daemons/_vol85_daemon.log`.

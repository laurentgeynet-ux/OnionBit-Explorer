<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 83 : conseil-etat · cnil · data.gouv `[V7-main]`

- **Binaire** : `[V7-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (buildé 2026-10-27 18:30, HEAD `09695ad1` docs-only post-build).
- **Mission** : 3 cibles « service public » — conseil-etat.fr,
  cnil.fr, data.gouv.fr (**régression** — ex-« meilleur vol »
  équipe 8 : 34 modules Nuxt évalués sans erreur en 2,3 s).
- **Daemon** : PID 13000, port 9466, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol83_daemon.log` → `logs/daemons/_vol83_daemon.log`
  (~259 373 lignes).
- **RAM** : base ~380 Mo → **pic ~784 Mo** (conseil-etat, stable sous
  le wedge — pas de fuite) → purge complète **423-443 Mo**
  post-DELETE ×3. Seuil 1,5 Go jamais approché.
- **Fenêtre** : ~17:44 → 18:31 UTC.
- **Verdict global** : **3/3 pages servies et peintes**, rendu
  fidèle partout — mais **2 constats majeurs** : (1) conseil-etat
  = boucle re-parse/cascade en escalade (**signature §B-44** :
  cascade ms=5 → ~14 600 ms plateau) + phases de starvation
  §B-41 (`poll_net` jusqu'à 89 s, `drains` jusqu'à 92 s) → API
  intermittente (scroll honoré en 57,7 s, `/frame` timeout 90 s) ;
  (2) data.gouv = **régression confirmée** : l'entrée Nuxt
  `type="module"`+importmap **jamais fetchée/évaluée — zéro trace**
  (§B-4 bras a, verbatim windy/pinterest) alors que le vol 4
  l'évaluait en 2,3 s.

---

## 1. conseil-etat.fr — rendu fidèle, wedge starvation §B-41 × escalade §B-44

`POST /navigate` → ~20 s — `doc_h=3 403,0`, titre « Accueil -
Conseil d'État ». Premier apply sain (17:44:38) : `cascade ms=5`,
`display_list items=753`, `repaint ms=2 281`.

**9 scripts évalués** (17:45:00-01) : 938 Ko `ok=8 ms`, 574 Ko
`ok=163 ms`, **502 941 o `ok=false ms=273`** (bundle webpack — et
**`/console` = « pas de journal »** : échec non tracé → §B-51,
même signature que micromania vol 81).

**Puis escalade §B-44 sur les re-parses (ADR-0025, mutations JS
continues)** — cascade mesurée : ms=114 → 1 133 → 3 084 → 6 455 →
12 954 → **plateau ~13 300-14 600 ms** (×10 en 3 min, ×2 900 vs
apply initial). Aucun `UnmappedKey` (0 occurrence) — déclencheur =
mutations DOM par le JS vivant (tarteaucitron/Matomo/webpack),
pas un rejet.

**Phases de starvation §B-41 mesurées** : `commit.drains` 8,3 →
37,4 → **66,7 s** ; `pump.drains` 25,7 → 55,8 → **91,9 s** ;
`event.poll_net` 71,3 → 74,5 → 78,8 → 84,0 → **89,4 s** ;
`pump.commit_lock` jusqu'à 66,7 s. Différence avec le porteur
flixbus/credit-agricole : **les phases finissent** (le wedge n'est
pas terminal — chaque apply aboutit à `repaint`, l'API répond par
fenêtres).

**Impact vol mesuré** : scroll ×1 → `scrolled scroll_y=1 600`
**en 57,7 s** ; `/frame` → 204 (apply en vol) puis timeout curl
90 s puis 200 (4,3 Mo) ; clic (300,200) → livré au DOM **après
~2 min** (« aucune action par défaut » — bande sombre non-lien) ;
`/dom` → 200 **1 789 318 o** (19 149 nœuds — DOM massif) puis
timeout 60 s en rafale ; `/layout` 200 (64 571 o) ; `/mem` :
`dom_nodes=19 149`, `js_heap=123 058 505` (vs 4-5 Mo sur les
autres cibles — JS continu confirmé), `display_items=587`.

**Rendu** (`captures/vol83_conseilet.png`) : page réelle peinte —
hero « Nuit du droit 2026 : plus de 500 étudiantes et étudiants
en immersion au Conseil d'État ! » (dégradé skyline peint),
colonne navigation gauche **par-dessus le hero** (méga-menu peint
ouvert — famille §B-17 : menus fermés par JS restants ouverts),
panneau recherche droit peint ouvert (« Thème recherché »,
« Rechercher », « Recherches courantes »), « En savoir plus »
chevauchant « Publications & Collections » (chevauchement inline —
§B-18), cartes actualités peintes en bas.
`captures/vol83_conseilet_scroll.png` (scroll_y=1 600) : section
« Juridictions administratives » + **carte SVG de France par
départements peinte** (noir sur blanc) ; bande sombre ~440 px en
haut sans texte visible (section fond sombre — texte potentiellement
absent ou couleur).

**Purge post-DELETE saine** (§B-30 favorable) : DELETE en 171 ms
pendant l'activité → 784 → 570 → **423 Mo**, `{"tabs":[]}`, CPU
retombé, API responsive.

**Constats** : escalade apply §B-44 **nouvelle variante — déclenchée
par mutations DOM continues, sans UnmappedKey ni cycle de report**
(borne §B-26 non tirée : `dropped=0` attendu, mécanisme « coût
par apply qui grandit » identique à radiofrance mais déclencheur
différent) ; starvation §B-41 **non-terminale** (phases 25-92 s
mais aboutissantes) ; §B-51 +1 (éval 502 Ko `ok=false ms=273`
muette) ; §B-17/§B-18 résiduels visuels ; §B-30 signal favorable.

## 2. cnil.fr — quasi-parfait, stretched-link symptom

`POST /navigate` → **50,6 s** (redirect `/fr`, fetch complet) —
`doc_h=4 328,4`, titre « Particulier | CNIL ».

**Rendu** (`captures/vol83_cnil.png`) : **très fidèle** — onglets
Particuliers/Professionnels/Presse, logo CNIL + baseline, menu
catégories (Téléphones · Objets connectés · Enfants et ados ·
Santé · Social · Logement · Travail · IA · Achats · Banque ·
Citoyenneté · Sport), « Gestionnaire de cookies », 3 sections
illustrées (Je m'informe / J'agis / J'ai besoin d'aide), carte
édito « La CNIL publie le tome 2 de L'Agence Privacy » avec image.

Défaut visible : le menu catégories **déborde le viewport des deux
côtés** — items layoutés à x<0 (`block<a> [-159.1228 220 …]`,
`[-43.5 …]` dans `vol83_cnil_layout.txt`), premiers libellés
coupés (« t », « ux », « ix » en bord gauche) — carrousel horizontal
sans mécanisme de défilement peint.

**Scrolled** (`captures/vol83_cnil_scroll.png`, scroll_y=1 600) :
bandeau alerte rouge « restez vigilants face aux offres
d'accompagnement » + illustration carte bancaire, grille
« Actualités » (6+ cartes avec images/tags/dates toutes peintes),
bouton « Besoin d'aide » sticky. Le bloc onglets header reste
peint **par-dessus** le bandeau en position scrollée (sticky
plausible — le titre du bandeau est partiellement occlus).

**JS sain** : 68 évals, **0 `ok=false`**, `/console` = « pas de
journal ». Télémétrie : `dom_nodes=1 651`, `js_heap=4 874 228`,
`display_items=859`, images 12,5 Mo.

**Interactions** : scroll ×2 → `scroll_y=800` puis `1 600` en
**0,11 s / 0,08 s** (immédiat). Clic carte actualité (230,590 →
doc 2190, image) → **`edited`** : l'`<img>` de la carte
`.all-wrapper` n'est **pas** dans une ancre (DOM : `<img>` puis
`div.element-bg > div.element-titre > <a href>` — le site réel
étend le clic à toute la carte, pattern stretched-link/overlay)
→ **symptôme §B-24 terrain** (carte incliquable hors titre).
Clic titre article (230,680) → **`navigated`** vers
`/fr/journee-rgpd-2026-poitiers` (titre page complet, `can_back`) —
navigation réelle validée.

**Constats** : site visitable de bout en bout ; §B-24 symptom
(image de carte non cliquable — seul le titre/tag naviguent) ;
débordement horizontal du menu catégories (cosmétique, overflow
non borné).

## 3. data.gouv.fr — SSR intact mais **module Nuxt inerte — régression §B-4(a)**

`POST /navigate` → **8,1 s** — `doc_h=5 558,6`, titre
« data.gouv.fr : Plateforme ouverte des données publiques
françaises ».

**Rendu** (`captures/vol83_datagouv.png`) : **fidèle** — marianne
RF, logo datagouv, barre recherche + bouton, nav (Données · API ·
Réutilisations · Organisations · Nouveautés · Nous écrire), hero
« La plateforme des données publiques françaises » + 2 CTA,
badge « En ce moment ». Micro-défaut : icônes « Se connecter »/
« S'enregistrer » peintes **sur** le texte (chevauchement icône/
libellé — §B-18 résiduel léger).

**Scrolled** (`captures/vol83_datagouv_scroll.png`, scroll_y=1 600) :
section « Réutilisations » (carrousel : carte COVID Tracker avec
graphiques + Eco-Score), « Une communauté dynamique et engagée »
(6K/83K/18K + logos Le Monde/Google Maps/Libération/Wikimédia),
section sombre « La mission data.gouv.fr ».

**RÉGRESSION §B-4 bras (a) — drain module inerte** : le DOM sert
`<script type="module" crossorigin src="/_nuxt/DL4muOy-.js">` +
`importmap {"imports":{"#entry":"/_nuxt/DL4muOy-.js"}}` + ~34
`modulepreload` — **et le journal ne contient AUCUNE trace** :
0 fetch, 0 éval, 0 erreur, 0 warn (`grep DL4muOy|module` = vide
hors phases `eval_mod`). Seules **2 évals** sur la home (202 o +
6 367 o inline, `ok=true`), +2 sur /reuses. `js_heap=4 050 860`
seul. Le vol 4 (équipe 8, 2026-10-26) mesurait pourtant « module
_nuxt évalué sans erreur en 2,3 s (34 sources prefetchées),
zéro exception JS » — **la référence positive §C est cassée** :
l'hydratation Nuxt ne démarre plus (recherche/autocomplete/carrousels
inertes) alors que le SSR peint toujours tout. Même signature
verbatim que windy/pinterest/GFG (« `type="module"`+importmap
0 trace »).

**Interactions** : scroll ×2 → `scroll_y=1 600` immédiat (0,16/0,20 s) ;
clic « Voir les réutilisations » (750,471) → **`navigated`** vers
`/reuses` en 61 ms (« Catalogue des réutilisations publiques ») —
les ancres SSR suffisent à naviguer, l'hydratation morte ne bloque
pas le parcours.

**Constats** : **régression §B-4(a) sur cible ex-« meilleur vol »**
— module `type="module"` jamais démarré, silence total ;
cosmétique §B-18 léger (icônes header). SSR-first passe (§C
inchangé visuellement).

---

## Synthèse des constats

| Cible | Résultat | IDs §B | Crate |
|---|---|---|---|
| conseil-etat.fr | SSR peint fidèle ; **escalade apply §B-44 variante mutations** (cascade 5→14 600 ms) + **starvation §B-41 non-terminale** (`poll_net` 89 s, `drains` 92 s) — scroll 57,7 s, frame timeout 90 s, clic livré ~2 min ; 1 éval `ok=false` muette | §B-44 (+variante déclencheur) ; §B-41 (variante non-terminale) ; §B-51 +1 ; §B-17/§B-18 visuels ; §B-30 favorable | `explorer-core`/`layout`/`render` |
| cnil.fr | Quasi-parfait : rendu fidèle, 68 évals 0 erreur, scroll instant, **clic titre → navigated** ; image de carte → `edited` ; menu déborde x<0 | §B-24 (symptôme stretched-link) ; overflow cosmétique | `explorer-layout`/`css` |
| data.gouv.fr | SSR intact mais **entrée Nuxt `type="module"`+importmap jamais évaluée — 0 trace** : régression vs vol 4 | **§B-4 bras (a) — régression cible ex-référence** ; §B-18 léger | `explorer-core`/`explorer-jscore` (drain modules) |

### Mises à jour de statut suggérées (SYNTHESE.md)

- **§B-44** : annoter « +1 porteur `[V7-main]` vol 83 :
  conseil-etat.fr — escalade cascade ms=5 → ~14 600 ms plateau,
  déclenchée par **mutations DOM continues** (tarteaucitron/Matomo/
  webpack, `js_heap` 123 Mo, DOM 19 149 nœuds), **sans UnmappedKey
  ni cycle de report** — même mécanisme de coût croissant par
  apply, déclencheur différent des porteurs précédents ; RAM
  stable ~784 Mo, purge post-DELETE complète ».
- **§B-41** : annoter « +1 variante `[V7-main]` vol 83 :
  conseil-etat.fr — starvation **non-terminale** : `event.poll_net`
  71→89 s, `commit.drains`/`pump.drains` 8→92 s, `commit_lock`
  67 s, mais chaque phase aboutit → API intermittente (scroll
  57,7 s, `/frame` timeout 90 s → 200, clic livré ~2 min,
  `/dom` 200 puis timeout) ; RAM stable, purge propre ».
- **§B-51** : +1 — conseil-etat `[V7-main]` : éval initiale
  502 941 o `ok=false ms=273` (bundle webpack) sans trace
  `/console` (« pas de journal »).
- **§B-4 bras (a)** : **régression** — data.gouv.fr `[V7-main]`
  vol 83 : ex-« meilleur vol » (34 modules Nuxt évalués en 2,3 s,
  vol 4) désormais **0 fetch/éval du `type="module"` entry**,
  silence total — à investiguer en priorité (le drain modules
  a pu casser entre vol 4 et V7, ou la condition de drain a
  changé).
- **§B-24** : +1 symptôme `[V7-main]` vol 83 — cnil.fr : image
  de carte `.all-wrapper > <img>` hors ancre → clic `edited`
  (seuls titre/tag naviguent) ; famille clic-carte non étendu.
- **§B-30** : +1 signal favorable ×3 (vol 83 : DELETE pendant
  wedge conseil-etat → purge 784→423 Mo ; cnil/data.gouv
  DELETEs propres).
- **Liste prudence** : ajouter **conseil-etat.fr** (starvation
  §B-41 + escalade §B-44 garantis — navigable mais API lente).

## Artifacts

- Captures : `captures/vol83_conseilet.png` (hero + menus peints
  ouverts), `captures/vol83_conseilet_scroll.png` (carte SVG
  France + bande sombre), `captures/vol83_cnil.png` (home fidèle),
  `captures/vol83_cnil_scroll.png` (bandeau + grille Actualités),
  `captures/vol83_datagouv.png` (hero fidèle),
  `captures/vol83_datagouv_scroll.png` (Réutilisations + mission).
- Logs : `logs/vol83_conseilet_{console.json,dom.txt(1,79 Mo),
  layout.txt,mem.json}`, `logs/vol83_cnil_{console.json,dom.txt,
  layout.txt,mem.json}`, `logs/vol83_datagouv_{console.json,
  dom.txt,layout.txt,mem.json}` ; journal daemon
  `logs/daemons/_vol83_daemon.log` (259 k lignes — phases
  `sync_profile` complètes du wedge conseil-etat).
- Arrêt propre : DELETE onglets 1-3 (204 ×3, purge mémoire
  complète) puis kill du PID 13000 par `Stop-Process -Id`.

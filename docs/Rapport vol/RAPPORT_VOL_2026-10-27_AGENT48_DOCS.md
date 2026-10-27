<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — AGENT 48 — Docs + presse française (2026-10-27)

- **Binaire** : `[V4-main]` — `C:/obe-vol-target/release/explorer-daemon.exe`
  (WIP `depth_limit` embarqué), `--port 9430`, `EXPLORER_VIEWPORT=1366x900`,
  journal `_vol48_daemon.log` (racine)
- **Cibles** : developer.mozilla.org (régression) · lemonde.fr (re-vol §B-10
  `lmd` + images §B-20) · ouest-france.fr (re-vol §B-10 `firstparty.push`)
- **Protocole** : `docs/vol-agent.md` — traçabilité §8, IDs §B de `SYNTHESE.md`
- **RAM daemon (PID 17216)** : pic mesuré ~1,4 Go avec 3 onglets — sous le
  seuil de vigilance 2 Go ; retombée à 450 Mo après fermeture des onglets.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| developer.mozilla.org | **Rendu quasi-parfait** | **vide** | §B-17 (panneau/menu peint ouvert au scroll, éléments de sélecteurs flottants) |
| lemonde.fr | **Dégradé sévère — disjoncteur ouvert** | `moteur js inerte` | §B-22 (éval 254 Ko tuée à 30 s → disjoncteur → tout JS sauté) ; §B-18 **reproduit** (runs inline superposés + colonne décalée) ; §B-20 partiel ; §B-10 `lmd` **non re-vérifiable** (moteur mort avant) |
| ouest-france.fr | **Bon rendu structurel, JS sain** | **vide** | §B-10 `firstparty.push` **non reproduit → RÉSOLU ici** ; §B-18 ×2 (runs en-tête superposés, titre hero en overflow-x) ; proposition §B-25 (image plein écran pixelisée au scroll) |

Acquis : 3/3 pages servies avec contenu réel ; **zéro panic, zéro freeze,
API responsive en continu** ; scrolls réels (`scroll_y` effectif, repeintes).
Le disjoncteur « moteur figé » a fait exactement son office sur lemonde :
éval monstrueuse tuée, session reprise, télémétrie intacte — mais il
transforme désormais la mort JS en `inerte`, ce qui **masque la
re-vérification des bras §B-10** sur cette cible.

---

## 1. developer.mozilla.org — régression : aucune `[V4-main]`

- `POST navigate` → `doc_h=2433`, titre « MDN Web Docs », URL finale
  `/en-US/` (redirect locale honorée).
- `/console` verbatim : `pas de journal` — **0 erreur**.
- `/mem` : 2 497 nœuds DOM, 538 boîtes, heap JS 47,6 Mio, images 7,0 Mio.
- Frame (`captures/vol48_mdn.png`) : header nav complet (HTML/CSS/
  JavaScript/Web APIs/Learn/Tools/About/Blog), hero « Resources for
  Developers, by Developers », recherche, 4 cards « Featured articles »
  fidèles — **~95 % fidèle**, pas de régression vs état connu.
- Scroll ×2 → `scroll_y=1533` réel, repeint (`vol48_mdn_scroll.png`) :
  « Contributor Spotlight » complet, footer 3 colonnes + copyright
  — scroll sain, aucun blanc (non-repro §B-8).

### §B-17 — résiduel overlay confirmé

- Frame 1 : textes « Search », « MDN », « Theme », « English (US) » peints
  **flottants** en tête (y≈75-115) hors de tout conteneur — fragments de
  sélecteurs langue/thème rendus hors flux.
- Frame scroll : un panneau sombre (liste « mdn/translated-content …
  8 hours ago » ×3 lignes) peint **au-dessus** du hero en tête de
  viewport — vraisemblablement un menu/notifications qui devrait être
  fermé par JS. Cohérent avec §B-17 (menus fermés par JS restés peints).
  Console vide → le JS a tourné ; le bras « CSS corrigé, JS mort » ne
  s'applique pas ici : ce serait plutôt un toggle d'état non propagé ou
  un `hidden` posé puis retiré.

## 2. lemonde.fr — disjoncteur ouvert, §B-18 reproduit, §B-10 masqué

- `POST navigate` → `doc_h=16 584` (`doc_h_clamped=true`), titre réel.
- `/console` verbatim : `{"console":["moteur js inerte"]}` — unique entrée.
- `/mem` : 13 124 nœuds DOM, 3 601 boîtes, **js_heap_bytes=0**, images
  155,7 Mio.

### §B-22 — nouveau cas terrain : éval de 254 Ko tuée à la deadline

Journal daemon tab=2 :

```
08:58:32  script: eval…  bytes=254226
08:59:02  script: eval fin  ms=30013 ok=false
08:59:02  moteur figé — disjoncteur ouvert (éval dépassée)
08:59:02  script sauté — disjoncteur moteur ouvert ×5 (1992/1175/154/20028/162 octets)
```

Un seul script de **254 Ko** a consommé la deadline de 30 s → disjoncteur
ouvert → les 5 scripts restants sautés → moteur signalé `inerte`
(`js_heap=0` au dump). Deux hypothèses ouvertes, à trancher par
bissection du script : (a) compile/parse légitime trop lente → deadline
30 s insuffisante sur gros bundle (cf. cryptpad §B-22) ; (b) wedge réel
type boucle infinie → disjoncteur correct. Dans les deux cas c'est le
**2ᵉ cas terrain §B-22** — la famille « deadline agressive » s'étoffe.

### §B-10 `lmd` — bras non re-vérifiable sur cette cible

L'erreur historique `lmd is not defined` ×10 n'apparaît **pas** — mais
le moteur a été tué avant que les scripts consommateurs de `lmd` ne
tournent. Le silence est donc un artefact du disjoncteur, pas une preuve
de résolution. **Statut inchangé** ; un re-vol exige soit un budget éval
plus grand, soit le retrait temporaire du script fautif.

### §B-18 — REPRODUIT : chevauchements de runs inline + colonne décalée

- `captures/vol48_lemonde.png` : header fidèle (masthead, Menu, FR/EN),
  puis **~400 px de blanc** et le contenu éditorial **décalé à droite** —
  la colonne article démarre à x≈1040 et déborde le bord droit
  (« Parcoursup, effectifs, bac… » tronqué hors viewport) : ~75 % du
  viewport est vide à gauche.
- `captures/vol48_lemonde_scroll.png` : **runs inline superposés**
  flagrants — « À Marseille, Emmanuel Macron a fait appuyé… » : deux
  passes de texte peintes l'une sur l'autre (interférences de mots). C'est
  la **première reproduction claire** du bras « chevauchements de runs
  inline » du §B-18 (catalogué « non reproduit » sur excalidraw/gitlab/
  DW/deezer).
- La cause probable est double : layout (colonne mal positionnée, runs
  doublés) *et* JS mort (classes/grid appliquées par JS absentes) —
  à ré-attribuer après bissection.

### §B-20 — images partielles

`images_bytes=155,7 Mio`, 170 `<img>` dans le dump — les `<img>` se
sérialisent **sans `src`** (attribut absent du dump : `<img>` nu) ;
seuls 5 `data-src` subsistent (`data-src="https://www.lemonde.fr"` —
marqueurs non-image). **2 photos éditoriales peintes** (ministres,
tag lycéen) — le fetch+decode image fonctionne — mais les vastes zones
blanches suggèrent que la majorité des visuels dépend du JS mort
(lazy/cascade), pas du repli `data-src` : §B-20 reste **présent en
substance**, sous forme « repli inopérant car pipeline d'attributs
différent ».

## 3. ouest-france.fr — §B-10 `firstparty.push` non reproduit → RÉSOLU ici

- `POST navigate` → `doc_h=41 203` (`doc_h_clamped=true`), titre réel.
- `/console` verbatim : `pas de journal` — **0 erreur**.
- `/mem` : 13 839 nœuds DOM, 7 039 boîtes, heap JS 34,2 Mio.
- Journal daemon : **23 évals, 23 `ok=true`, 0 échec** — aucun script
  sauté, disjoncteur fermé.

### §B-10 — bras `firstparty.push` : RÉSOLU `[V4-main]`

Historique : `firstparty.push` ×3 (appel sur undefined → scope global
suspect). Ici : le dump DOM montre `window.firstparty=[],window.
thirdparty=[]` posé par un script top-level, **et zéro erreur console**
sur les scripts consommateurs. Le global partagé tient (cohérent avec la
sonde « VÉRIFIÉ SAIN » du catalogue) ; le bras historique est **clos sur
cette cible** — les scripts définisseurs s'exécutent et les liaisons
traversent. Reste à re-voler `lmd` (bloqué ci-dessus par §B-22) et
`OSM.i18n` pour solder §B-10.

### §B-18 — deux reproductions

- En-tête (`vol48_ouestfrance.png`) : **runs inline superposés** dans la
  barre utilitaire — « Newslette**Pour** », « utili**sateur**/secteur »,
  « S'iden…**ger** » + « NOVO19 » en collision ; deuxième site du vol
  avec ce symptôme → le bras « chevauchements de runs inline » est
  désormais **confirmé terrain ×2** et mérite un issue dédiée.
- Titre hero « REPORTAGE. « Il faut virer Netanyahou »… » **déborde
  le bord droit** (tronqué hors viewport) → bras `overflow-x` non clippé
  (déjà vu guardian).

### Proposition §B-25 — image plein écran pixelisée au scroll

`vol48_ouestfrance_scroll.png` : à `scroll_y=1800`, une **image en
mosaïque géante** occupe la quasi-totalité du viewport (blocs ~30 px) —
un visuel manifestement sous-échantillonné étiré sur une boîte immense,
ou un interstitiel/bloc pub positionné en overlay. Ni le DOM (13 839
nœuds) ni la console ne signalent d'anomalie. Suspects : intrinsic size
d'un thumbnail respecté au lieu de la boîte CSS, ou pub sticky non
masquée. → **proposition d'entrée §B-25** (crate : `explorer-layout`/
`render` ou glue images), à confirmer par inspect de la boîte en
`logs/vol48_ouestfrance_layout.txt`.

---

## Statuts proposés (§8)

| §B | Bras | Verdict ce vol |
|---|---|---|
| §B-10 | `firstparty.push` (ouest-france) | **RÉSOLU `[V4-main]`** — confirmé sur binaire post-fix |
| §B-10 | `lmd` (lemonde) | inchangé — masqué par §B-22, re-vol à replanifier |
| §B-17 | overlays (mdn) | OUVERT — confirmé `[V4-main]` (panneau peint ouvert, console vide : variante « JS vivant, état non propagé ») |
| §B-18 | runs inline superposés | OUVERT — **reproduit ×2 `[V4-main]`** (lemonde, ouest-france) |
| §B-18 | `overflow-x` clip | OUVERT — reproduit `[V4-main]` (titre hero ouest-france) |
| §B-20 | images `data-src` (lemonde) | OUVERT — présent en substance (repli inopérant, `<img>` sans `src`) |
| §B-22 | deadline éval agressive | OUVERT — 2ᵉ cas terrain `[V4-main]` (lemonde, 254 Ko/30 s) |
| §B-25 (proposé) | image plein écran pixelisée au scroll | NOUVEAU — `[V4-main]` (ouest-france scroll_y=1800) |

## Hygiène de vol

- Onglets 1/2/3 fermés par `DELETE /api/tabs/{id}`.
- Daemon tué par son PID (17216) — `taskkill //PID` (jamais `/IM`) ;
  le daemon tiers voisin (PID 8616) n'a pas été touché.
- Artefacts : `captures/vol48_{mdn,mdn_scroll,lemonde,lemonde_scroll,
  ouestfrance,ouestfrance_scroll}.png`,
  `logs/vol48_{mdn,lemonde,ouestfrance}_{dom,layout,mem}.txt`,
  journal `_vol48_daemon.log` (racine).

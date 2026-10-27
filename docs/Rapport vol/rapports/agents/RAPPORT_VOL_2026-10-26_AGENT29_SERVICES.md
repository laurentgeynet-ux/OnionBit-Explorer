<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 29 — sites services / santé / admin

- **Date** : 2026-10-26
- **Binaire** : `C:/obe-vol-integ/target/release/explorer-daemon.exe` (release, intégration V2)
- **Viewport** : `EXPLORER_VIEWPORT=1366x900`, port 9413
- **Cibles** : doctolib.fr, ameli.fr, impots.gouv.fr, laposte.fr (fallback caf.fr non nécessaire)
- **Reprise** : l'agent précédent est mort en cours de mission ; ses artefacts
  (captures ameli/impots/laposte + logs console/dom/layout, ~21:09–21:14)
  ont été réutilisés, doctolib a été rejoué intégralement.

## Synthèse

| Cible | Verdict | Gravité max |
|---|---|---|
| doctolib.fr | **Inutilisable** — page blanche, moteur JS figé | Bloquant |
| ameli.fr | Rendu correct, JS cassé partiel, clics morts | Majeur (clic) |
| impots.gouv.fr | Rendu correct, défauts cosmétiques | Mineur |
| laposte.fr | Rendu correct, défauts cosmétiques | Mineur |

**Régression confirmée** : `POST /input {"type":"click"}` répond
`{"result":"ignored"}` sur **4/4 clics**, dont 3 sur des éléments réels
d'une page pleinement rendue (ameli, doc_h=4608) : bouton « Se connecter »,
« Rechercher », lien hero « En savoir plus ». Aucune navigation déclenchée.
Documenté sans re-diagnostic (consigne).

**`unhandledrejection`** : jamais observé dans les consoles de ce vol —
aucune promesse rejetée n'est survenue sur les 4 sites ; le fix ne peut
être ni confirmé ni infirmé sur cet échantillon.

---

## 1. doctolib.fr — https://www.doctolib.fr (bloquant)

**Rendu** : page entièrement blanche (`captures/vol29_doctolib.png`).
DOM parsé présent (dump 1 687 341 octets, `logs/vol29_doctolib_dom2.txt`,
html/head/meta/scripts visibles) mais **layout racine à hauteur nulle** :
`block<html> [0 0 1366x0]` (`logs/vol29_doctolib_layout2.txt`).

**Erreurs verbatim** (console : `pas de journal` — aucune erreur loguée
malgré le gel) ; journal daemon (`_vol29_daemon2.log`) :

```
WARN explorer_core::session::js_glue: moteur figé — disjoncteur ouvert,
  appels JS court-circuités tab=1
  error=exception js : moteur figé — éval dépassée
INFO ... script: eval fin tab=1 ms=30011 ok=false   (×8, 30 s chacun)
```

**Mesures** : 16 scripts évalués au boot, **8 évals tombées au timeout
30 s** (dont un bundle de 3 605 005 octets et un de 1 543 906 octets) ;
~4 min de blocage. Sur le premier daemon (session précédente) la file
d'évals ne se vidait pas : **le daemon entier est resté wedge** —
`/api/tabs`, `/frame`, `/dom`, `DELETE /api/tabs/2` tous en timeout
(code curl 000), process tué par PID. Sur le second daemon la file
s'est vidée et l'onglet redevient réactif (`scroll` accepté mais
`scroll_y` reste `0.0`, page à hauteur 0 ; `click` → `ignored`), mais
la page n'est jamais peinte.

**Classification** : moteur JS — évals qui dépassent le budget 30 s en
chaîne ; ensuite rendu absent (layout h=0 avec DOM complet).
**Sévérité** : bloquante — site majeur de prise de RDV santé
inutilisable, et le gel peut verrouiller le daemon entier.

## 2. ameli.fr — https://www.ameli.fr (majeur)

**Rendu** : correct (`captures/vol29_ameli.png`, `_scroll.png`). Header
Assurance Maladie, hero « la grippe nous rappelle la nécessité de se
faire vacciner », rangée d'accès rapides, cartes « Vos outils et
services » (Simulateur, Annuaire des professionnels de santé,
Questionnaire handifaction), section « Les maladies de A à Z ».
`doc_h=4608`. Scroll fonctionnel (`scroll_y: 800.0`).

**Erreurs verbatim** (`logs/vol29_ameli_console.json`/`_console2.json`,
reproduites en session 2) :

```
exception js : TypeError: cannot read `call` of Undefined
    at <anonymous> (…/js_F2tPDUV4q1oiljMY…js?scope=footer…:5:250)
    at <anonymous> (…:5:16637)
    at i (…:5:5239)
exception js : TypeError: The element must be an instance of Element
    at u (…/js_TC3tWBNaXmO3HKwoNn5QqiT6…js?…:3:335)
    … at <anonymous> (…/menu.js?v=1.0.3:1:1493)
    at <anonymous> (…/jquery.min.js?v=4.0.0:2:37293) …
```

**Constats mesurés** : exceptions répétées du bundle webpack
(`cannot read 'call' of Undefined` — module/runtime) et du menu jQuery
(`instanceof Element` rejeté) ; icônes/glyphes absents (carrés vides sur
« Rechercher », « Se connecter ») ; fragments de carrousel chevauchés en
haut de page (« automnale », « de tumer ») ; clics 3/3 → `ignored`.

**Classification** : JS (bootstrap webpack + DOM `instanceof`) — le
contenu statique CSS tient, les interactions JS sont mortes.
**Sévérité** : majeure (page lisible, rien de cliquable ni de
fonctionnel côté scripts).

## 3. impots.gouv.fr — https://www.impots.gouv.fr (mineur)

**Rendu** : correct (`captures/vol29_impots.png`, `_scroll.png`).
Bandeau Marianne « République française », nav complète (Accueil,
Particulier, Professionnel, Partenaire, Collectivité, Statistiques et
Données, International), liens « Connexion à mon espace Finances
publiques » / « espace professionnel », article « aide de 100 € pour
les grands rouleurs » avec bouton « En savoir plus », section
« À la Une ».

**Erreurs verbatim** : console `pas de journal` (aucune erreur).

**Constats mesurés** : bouton de recherche tronqué — texte visible
« echer » au lieu de « Rechercher » (clip du libellé dans le bouton) ;
icône loupe absente du champ ; libellés nav soudés
« AccueilParticulier » (espacement/marge ignoré entre deux liens
adjacents) ; icône étoile « Votre avis sur le site » ok.

**Classification** : layout/inline — clipping de texte de bouton et
marges inter-liens. **Sévérité** : mineure (page entièrement
exploitable visuellement ; clics morts par la régression globale).

## 4. laposte.fr — https://www.laposte.fr (mineur)

**Rendu** : correct (`captures/vol29_laposte.png`, `_scroll.png`).
Bandeau app « Suivi, courrier, colis avec l'application La Poste »,
logo, champ « N° de suivi, rechercher un produit ou un service »,
nav (Le courrier, Le colis, Le transfert de courrier, Les services,
Tous nos produits, Laposte.net), hero « Le collector de 8 timbres
Route du Rhum », 3 cartes jaunes (ENVOIS COLIS / INTERNATIONAL /
PROMOTION −15 %) avec images produits.

**Erreurs verbatim** : console `pas de journal`.

**Constats mesurés** : image de fond du hero absente (bloc bleu uni,
seule la vignette latérale peinte — probablement image en CSS
`background` ou lazy-load non résolue) ; cases carrées vides rendues
dans la nav à la place d'icônes ; après clic (`vol29_laposte_click.png`)
la barre de nav **perd ses libellés** et le bandeau « Télécharger
l'application » disparaît — re-layout dégradé post-input ;
« Télécharger l'application » affiché avec un trait (style bizarre).

**Classification** : rendu (background-image/icones) + possible
invalidation layout sur input. **Sévérité** : mineure.

## 5. caf.fr — fallback non joué

Les 4 cibles primaires ont été mesurées ; fallback non nécessaire.

---

## Constats actionnables

1. **[Bloquant] doctolib.fr fige le moteur JS puis le daemon** — 8 évals
   consécutives au timeout 30 s (bundles 1,5 Mo et 3,6 Mo) ; dans un cas
   la file ne s'est jamais vidée et **tout le daemon devient muet**
   (API, DELETE inclus). Le disjoncteur se ré-ouvre à chaque script au
   lieu de couper la queue : prévoir un abandon de la file d'évals
   restante quand le moteur est déclaré figé, pour ne pas wedge le
   daemon. Repro : `_vol29_daemon.log` / `_vol29_daemon2.log`.
2. **[Bloquant] Page à DOM complet jamais peinte** — doctolib : DOM
   1,7 Mo parsé mais `block<html> [0 0 1366x0]`, frame blanche,
   `scroll_y` 0. Le layout n'a jamais produit de hauteur (probablement
   dépendant du JS figé / `<html>` sans contenu rendu). Vérifier le
   chemin « DOM prêt, layout vide ».
3. **[Majeur] Régression clics `ignored`** — 4/4 clics répondent
   `{"result":"ignored"}` y compris sur liens/boutons réels d'ameli
   (doc_h=4608). Aucun hit-test ne trouve de cible. Suspecté global
   (consigne : documenté, pas re-diagnostiqué).
4. **[Majeur] `TypeError: cannot read 'call' of Undefined`** — ameli :
   échec du runtime de modules webpack (`at i (…:5239)`), tuant tout le
   JS de page ; vraisemblablement un appel de module/chunk manquant dans
   explorer-jscore.
5. **[Majeur] `instanceof Element` rejeté** — ameli menu.js via jQuery :
   `TypeError: The element must be an instance of Element` sur un nœud
   pourtant élément — interop DOM↔JS (cross-realm ? wrapper ?).
6. **[Mineur] Texte de bouton clippé** — impots : « Rechercher » rendu
   « echer » (clip gauche+droite dans le bouton submit) ; marges
   inter-liens nav ignorées (« AccueilParticulier » soudé).
7. **[Mineur] Icônes/glyphes absents → carrés vides** — ameli
   (Rechercher, Se connecter), laposte (nav) : fontes d'icônes non
   résolues ou fallback glyph manquant.
8. **[Mineur] `background-image` hero non peinte** — laposte : bloc
   hero bleu vide alors que la vignette `<img>` latérale est peinte.
9. **[Mineur] Re-layout dégradé post-input** — laposte : après un
   `click`, la nav perd ses libellés et le bandeau app disparaît
   (`vol29_laposte.png` vs `vol29_laposte_click.png`).
10. **[Info] `unhandledrejection` non testable** — aucune promesse
    rejetée observée sur les 4 sites ; le fix ne peut pas être confirmé
    par ce vol.

## Artefacts

- Captures : `captures/vol29_{doctolib,ameli,ameli_scroll,ameli_click,
  impots,impots_scroll,impots_click,laposte,laposte_scroll,
  laposte_click,ameli_click2}.png`
- Logs : `logs/vol29_{doctolib,ameli,impots,laposte}_{console,console2,
  dom,layout,dom2,layout2}.{json,txt}`
- Journaux daemon : `_vol29_daemon.log` (daemon 1, wedge),
  `_vol29_daemon2.log` (daemon 2)
- Nettoyage : onglets supprimés, daemons tués par PID (15800, 14136).

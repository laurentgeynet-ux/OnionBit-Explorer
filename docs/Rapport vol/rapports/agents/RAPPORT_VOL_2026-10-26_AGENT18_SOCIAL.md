<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 18 — Sites sociaux / communauté

Date : 2026-10-26 — Daemon `explorer-daemon` release (ob-vol-head), port 9402,
viewport 1366×900. Cibles : x.com, pinterest.com, quora.com,
tumblr.com/explore, bsky.app (fallback). Aucun code modifié.

Captures : `captures/vol18_<cible>.png` ; télémétrie :
`logs/vol18_<cible>_{console,dom,layout}.*`.

---

## 1. https://x.com — mur d'inscription SSR partiellement rendu

- URL effective : `https://x.com/` — titre « X. It's what's happening / X ».
- `doc_h=908`, DOM 62 Ko, console : **« pas de journal »** (aucune erreur
  loguée).
- **Rendu** : la page d'accueil déconnectée (SSR) s'affiche :
  « Happening now », boutons « Continue with phone / Google / Apple »,
  logo X SVG géant à droite, liens de pied de page. Fond blanc, styles
  partiels (bouton « Continue » gris pleine largeur, espacements absents —
  flexbox/gap manifestement non appliqués).
- **Constat mesuré — commentaires rendus comme texte** : le texte littéral
  `<#comment>` apparaît à l'écran à 6 reprises (haut de page, dans le titre
  « Happening now<#comment>. », en bas). Le layout le confirme :
  `text "<#comment>" [8 8 92.3584x20]`. Les nœuds commentaires du DOM sont
  exposés comme runs de texte → **bug du pipeline parse→layout** (les
  commentaires ne doivent jamais produire de glyphes).
- **Interaction** : clic sur « Continue with phone » → `{"result":"navigated"}`
  → page blanche (uniquement 4 lignes `<#comment>`). Le flux d'inscription
  est piloté en JS (React) : squelette servi, app jamais hydratée.
- Scroll : `scroll_y` plafonne à 8 px (doc_h ≈ viewport).
- Classification : **rendu partiel** (SSR visible, layout dégradé,
  commentaires fuitant à l'écran) + SPA morte après navigation.
- Sévérité : **élevée** pour le bug `<#comment>` (visible sur toutes les
  cibles), moyenne pour la suite (mur de login attendu sans JS complet).

## 2. https://www.pinterest.com — SPA morte, regex Unicode fatale

- URL effective : `https://www.pinterest.com/` — titre « Pinterest ».
- `doc_h=38` — page quasi vide. DOM 1,57 Mo (le document brut est livré,
  avec `id="__PWS_ROOT__"` vide, `__PWS_INITIAL_PROPS__`, `__PWS_DATA__`
  présents).
- **Première erreur fatale (verbatim)** :
  `syntaxe js 1:35998 : invalid regex /[\$\+<->\^\`\|~\xA2-\xA6\xA8\xA9\xAC\xAE-\xB1\xB4\xB8\xD7\xF7\u02C2-\u02C5…` —
  une longue classe de caractères Unicode (plages de symboles/ponctuation,
  incluant des paires de substitution `\uD800[\uDD37-…]`, `\uD802[…]`, etc.)
  est **rejetée au parse** par explorer-jscore → le bundle principal ne
  s'exécute pas → `__PWS_ROOT__` reste vide.
- Layout : `block<html> [0 0 1366x38]` — seuls 4 runs `<#comment>` (même
  fuite de commentaires, en gras).
- Scroll : `scroll_y=0` ; clic : `{"result":"ignored"}` (rien à cliquer).
- Classification : **SPA morte — erreur de syntaxe JS (regex Unicode)**,
  squelette DOM présent mais jamais hydraté.
- Sévérité : **critique** — la grammaire regex d'explorer-jscore ne couvre
  pas les classes de caractères Unicode étendues / surrogates échappées,
  bloquant les bundles webpakés modernes.

## 3. https://www.quora.com — page d'erreur servie côté serveur

- Redirection vers `https://fr.quora.com/` — titre « Quora - Un lieu pour
  partager le savoir… ». `doc_h=950`, DOM 87 Ko, console : « pas de journal ».
- **Rendu** : icône « ! », message « Un problème est survenu. Veuillez
  patienter et réessayer plus tard. », bouton « Réessayer » **pleine
  largeur** (devrait être un petit bouton centré — CSS de centrage/largeur
  non appliqué).
- Le DOM contient `id="error"` et `"shouldServerRender": false` dans
  `window.ansFrontendGlobals.earlySettings` : Quora a **décidé côté
  serveur** de servir la page d'erreur (probable détection navigateur non
  supporté / échec d'assets — les scripts portent
  `onerror="addAssetErr(this.src)"`).
- Clic « Réessayer » → `navigated` → même page d'erreur.
- Classification : **blocage amont (anti-bot/fallback serveur)** +
  rendu CSS dégradé du bouton.
- Sévérité : moyenne — la cause première est côté serveur, mais le style
  du bouton plein cadre confirme des trous CSS (centrage/flex).

## 4. https://www.tumblr.com/explore — CMP injecté, app absente, iframe vide

- URL effective : `https://www.tumblr.com/explore` — titre « Tumblr ».
- `doc_h=9`, DOM 62 Ko, console : « pas de journal ».
- **Rendu** : fond noir Tumblr + **boîte blanche 766×786 centrée** = la
  modale de consentement (`cmp-app-container`, iframe CMP). Le JS du
  bandeau CMP **s'exécute** : il a injecté `#cmp-app-container`, les
  focus-guards, l'iframe `__tcfapiLocator` et le style
  `.cmp-components-modal__*` dans le DOM.
- **Mais** : l'iframe de contenu CMP reste **blanche** (contenu jamais
  chargé) et l'app principale Tumblr n'apparaît pas : le boot s'appuie sur
  `document.write('<script src="…/modern/main-…js" …>')` dans un script
  inline (sniff `typeof Array.prototype.at === "function"`). Aucun bundle
  `main` n'a produit de DOM → `document.write` manifestement inopérant ou
  les scripts injectés jamais chargés/exécutés.
- Scroll : `scroll_y=0` ; clic : `ignored`.
- Classification : **SPA morte — `document.write` / scripts injectés non
  exécutés** + **iframes sans navigation** (iframe CMP rendue comme cadre
  vide).
- Sévérité : élevée — deux trous distincts et mesurés.

## 5. https://bsky.app — SPA morte, regex emoji fatale

- URL effective : `https://bsky.app/` — titre « Bluesky ». `doc_h=900`,
  DOM 30 Ko (`id="root"` présent, à 0 px de hauteur, + `<noscript>`).
- **Première erreur fatale (verbatim)** :
  `syntaxe js 2136:14296 : invalid regex /[#*0-9]\uFE0F?\u20E3|[\xA9\xAE\u203C\u2049\u2122\u2139\u2194-\u2199\u21A9\u21AA\u231A\u231B\u2328\u23CF\u23ED-\u23EF\u23F1\u23F2\u23F8-\u23FA\u24C2\u25AA\u25AB\u…`
  — regex de graphemes emoji (même famille que Pinterest) rejetée au
  parse → bundle mort → `#root` vide → page blanche.
- Classification : **SPA morte — erreur de syntaxe JS (regex Unicode
  emoji)**. Squelette présent, jamais hydraté.
- Sévérité : **critique** — deuxième occurrence du même trou : les regex
  Unicode à plages étendues (ponctuation/symboles/emoji, surrogates)
  bloquent les bundles de deux SPA majeures distinctes.

---

## Constats actionnables

1. **Commentaires HTML rendus comme texte `<#comment>`** — mesuré à
   l'écran sur x.com (6×, dont « Happening now<#comment>. ») et
   pinterest.com (4 runs dans le layout). Les nœuds commentaire
   produisent des runs de texte affichables : corriger le mapping
   parse→layout (un commentaire ne génère aucune boîte/glyphe).
   **Périmètre PIPELINE, sévérité élevée, reproduction immédiate.**

2. **Grammaire regex JS : classes de caractères Unicode étendues
   rejetées** — erreur fatale verbatim `syntaxe js …: invalid regex`
   sur pinterest.com (plages `\xA2-\xA6 … \uD800[\uDD37-…]`) et
   bsky.app (regex emoji `[#*0-9]\uFE0F?\u20E3|[\xA9\xAE…]`). Deux SPA
   majeures mortes à la **compilation** du bundle. Couvrir : plages
   `\xXX-\xXX`/`\uXXXX-\uXXXX` longues, séquences de surrogates
   `\uD800[\uDD37-\uDD3F…]`, `?` sur `\uFE0F`, classes négatives.
   **Périmètre JS (explorer-jscore), sévérité critique.**

3. **`document.write` / scripts injectés par document.write non
   exécutés** — tumblr.com boote tout son app via
   `document.write('<script src=… defer>')` : aucune trace d'exécution
   (fond noir, app absente), alors que le script inline et le bundle CMP
   tournent. **Périmètre NAVIGATEUR (js/core), sévérité élevée.**

4. **Iframes jamais naviguées** — tumblr : l'iframe de la modale CMP
   (766×786) se monte mais reste blanche ; `__tcfapiLocator` présent.
   Le pipeline iframe→navigation semble absent. **Sévérité élevée** (le
   bandeau cookies est le premier élément visible de nombreux sites).

5. **CSS : centrage/boîtes flex non appliqués** — quora : bouton
   « Réessayer » pleine largeur au lieu d'un bouton centré ; x.com :
   espacements de colonnes/boutons absents sur le mur SSR. Constats
   visuels convergents sur des trous flexbox/marges auto.
   **Périmètre PIPELINE, sévérité moyenne.**

6. **Pas de télémétrie d'erreur pour les navigations JS** — clic
   « Continue with phone » sur x.com → navigation → page blanche, et la
   console reste « pas de journal » : soit les erreurs du nouveau
   document sont perdues au changement de page, soit aucun script n'a
   tourné. À instrumenter (les erreurs doivent survivre à la nav ou être
   horodatées). **Sévérité moyenne (observabilité).**

7. Quora sert volontairement une page d'erreur (shouldServerRender:false,
   `onerror=addAssetErr`) — **blocage amont côté serveur**, non
   actionnable dans le moteur ; à re-tester après correction des trous
   1-4 (l'UA/les capacités détectées changeront le verdict serveur).

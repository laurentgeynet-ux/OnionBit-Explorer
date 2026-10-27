# Rapport de vol — Agent 33 — Sites musique/audio

- **Date** : 2026-10-26
- **Binaire** : `C:/obe-vol-integ/target/release/explorer-daemon.exe` (intégration V2)
- **Viewport** : 1366×900 — port 9417
- **Protocole** : `docs/vol-agent.md` — frame PNG = verdict terrain
- **Note contexte** : clics `ignored` sur contenu non hydraté = limitation
  connue, documentée sans re-diagnostic. `unhandledrejection` visible en
  console = fix récent confirmé (les rejets de promesses sont bien
  remontés).

---

## 1. Deezer — https://www.deezer.com/fr/

- **Titre** : « Deezer Musique | 2 Mois gratuit | App de streaming »
- **doc_h** : 8 171,7 px (non clampé)
- **Console** (verbatim, 5 entrées) :

```
unhandledrejection: TypeError: cannot read `settings` of Undefined
    at <anonymous> (https://surveys-static-prd.survicate-cdn.com/widget_core-29.0.15.js:1:493282)
    ...
unhandledrejection: TypeError: `filter` is not a function (on function n@1:356480)
    at <anonymous> (https://surveys-static-prd.survicate-cdn.com/widget_core-29.0.15.js:1:328687)
    ... (×2, widget Survicate)
```

- **Constats mesurés** (`vol33_deezer.png`, `vol33_deezer_scroll.png`) :
  - Page **bien rendue** : header noir avec nav (« Offres / Découvrir
    Deezer / Musique / Connexion / Inscription »), hero « Là où ta
    musique prend vie », bouton violet « S'inscrire gratuitement »,
    étoiles d'avis. Thème sombre appliqué.
  - **Chevauchement de texte** : le sous-titre « Le streaming musical
    par Deezer » recouvre le bas du titre hero (ligne de texte posée
    sur le « prend vie »).
  - Après scroll 1 800 px : section sombre avec rangée d'ellipses
    violettes (décor SVG rendu), bannière « Vis ta musique avec
    Deezer » correcte ; **chevauchement** : bouton « Explorer notre
    catalogue » posé sur le paragraphe « Tu le trouveras sur Deezer
    en haute qualité sonore ! » dont les lignes se coupent mutuellement
    (hauteur de ligne ou positionnement absolu non résolu).
  - Aucune image photographique/hero visible — probablement des images
    chargées via CSS/JS non abouties (fonds unis à la place).
  - Scroll OK (`scroll_y` 900 → 1800). Clic sur « S'inscrire
    gratuitement » → `{"result":"ignored"}` (comportement connu).
- **Classification** : rendu fonctionnel avec défauts de layout.
- **Sévérité** : moyenne (page lisible et navigable ; chevauchements
  de texte et images absentes).

## 2. Last.fm — https://www.last.fm

- **Titre** : « Last.fm | Play music, find songs, and discover artists »
- **doc_h** : 2 449 px
- **Console** : **vide** (« pas de journal ») — aucune erreur JS.
- **Constats mesurés** (`vol33_lastfm.png`, `vol33_lastfm_scroll.png`) :
  - Rendu correct : header noir (Music / Events / Charts / Log In /
    Sign Up), hero « Music Counts. Track, Find and Rediscover music. »,
    bouton rouge « Start Now », section « Your music stats. Live. »
    avec rouge d'accent.
  - Après scroll 1 550 px : section « last.fm PRO » sur fond noir,
    footer complet et correctement stylé (colonnes Company/Help/
    Goodies/Account/Follow Us, sélecteur de langue, copyright).
  - **Boîte bleue orpheline** en bas à gauche du footer (~56×56 px,
    bleu vif) — probablement une image (avatar/logo) non résolue
    rendue comme bloc de couleur de repli.
  - Image hero absente (zone blanche à droite du titre où se trouve
    normalement le visuel d'accueil).
  - Scroll OK. Clic sur « Start Now » → `{"result":"ignored"}`.
- **Classification** : rendu largement conforme.
- **Sévérité** : faible (décor/images manquants, structure intacte).

## 3. Radio Garden — https://radio.garden

- **Titre** : « Radio Garden – Explore live radio by rotating the globe »
- **doc_h** : 900 px (= hauteur viewport)
- **Console** : **vide** — aucune erreur JS rapportée.
- **Constats mesurés** (`vol33_radiogarden.png`,
  `vol33_radiogarden_click.png`) :
  - Page blanche avec **un unique point noir** (~50 px) décalé vers
    la gauche du centre — le loader animé de Radio Garden figé (la
    suite de l'animation « ... » ne s'affiche pas).
  - DOM (29 962 octets) : 33 `<script>`, 1 `<canvas>`, mentions
    « globe », « Loading » — le DOM applicatif est présent, l'app
    ne s'initialise pas visiblement.
  - Clic centre → `{"result":"ignored"}`, frame inchangée après 3 s.
- **Classification** : échec d'amorçage applicatif (app JS/canvas,
  probablement WebGL/boot asynchrone) — page inutilisable **sans
  erreur console** : l'échec est silencieux.
- **Sévérité** : critique.

## 4. Genius — https://genius.com

- **Titre** : « Genius | Song Lyrics & Knowledge »
- **doc_h** : 43 893 px — **`doc_h_clamped: true`** (clamp de
  document atteint).
- **Console** : **512 entrées** (cap atteint ?), toutes identiques :

```
unhandledrejection: ReferenceError: r is not defined
    at <anonymous> (https://assets.genius.com/javascripts/compiled/reactAds-iceVJr5s.js:1:31073)
    at <anonymous> (https://assets.genius.com/javascripts/compiled/reactAds-iceVJr5s.js:1:20617)
    at <anonymous> (https://assets.genius.com/javascripts/compiled/reactAds-iceVJr5s.js:1:20574)
    at <anonymous> (https://assets.genius.com/javascripts/compiled/reactAds-iceVJr5s.js:1:45756)
    at <anonymous> (19:37)
    at <anonymous> (https://assets.genius.com/javascripts/compiled/reactAds-iceVJr5s.js:1:42093)
    at <anonymous> (18:20)
```

- **Constats mesurés** (`vol33_genius.png`,
  `vol33_genius_scroll.png`) :
  - Page rendue **sans aucun CSS externe** : styles navigateur par
    défaut (fond blanc, liens bleus soulignés, Times pour les
    paragraphes), logo « GENIUS » affiché pixelisé (police bitmap de
    repli ?), titres d'articles empilés verticalement.
  - Contenu présent (articles news, « Charts », champ « Search
    lyrics & more », « Sign Up ») — SSR HTML livré, la cascade CSS
    absente.
  - **Cause mesurée dans le DOM** : `0` élément `<link
    rel="stylesheet">` ; 56 `rel="modulepreload"`, 2 `rel="preload"`
    et seulement 2 `<style>` inline. Le CSS arrive par le pipeline JS
    (modules) qui ne s'exécute pas correctement (cf. 512
    `ReferenceError` sur reactAds).
  - Scroll 800→1 600 OK ; frame scrollée (204 transitoire puis 200 —
    retry effectué). Clic sur un titre → `{"result":"ignored"}`.
- **Classification** : CSS non chargé → rendu dégradé majeur + boucle
  d'erreurs JS (rejets en rafale).
- **Sévérité** : haute.

---

## Constats actionnables

1. **Chevauchements de texte sur Deezer** : sous-titre hero posé sur
   le titre ; bouton « Explorer notre catalogue » superposé au
   paragraphe dont les lignes se coupent (scroll 1 800 px). Suspect :
   hauteur de ligne/`line-height` ou positionnement absolu de
   superposition non résolu. Sévérité moyenne.
   (captures `vol33_deezer.png`, `vol33_deezer_scroll.png`)
2. **Radio Garden ne démarre pas et ne dit rien** : loader figé (1
   point noir), 0 erreur console malgré 33 scripts + canvas. L'absence
   de diagnostic console sur un boot JS mort rend la bissection aveugle
   — vérifier le support des APIs exigées au boot (canvas 2d/WebGL,
   modules, `fetch` streaming). Sévérité critique.
   (capture `vol33_radiogarden.png`, log `vol33_radiogarden_dom.txt`)
3. **Genius servi sans CSS** : 0 `rel=stylesheet`, 56
   `rel=modulepreload` — feuilles de style dépendantes du bootstrap de
   modules JS qui échoue (`r is not defined` ×512 sur reactAds).
   Deux sous-problèmes : (a) `ReferenceError` en rafale infinie (512
   entrées, probablement un retry/intervalle non borné — risque de
   spam console) ; (b) styles CSS-in-JS jamais injectés. Sévérité
   haute. (capture `vol33_genius.png`)
4. **`doc_h_clamped: true` sur Genius** (43 893 px) : document très
   long tronqué — vérifier le seuil de clamp face aux pages
   éditoriales longues. Informatif.
5. **Images d'arrière-plan/hero absentes** sur Deezer et Last.fm :
   les visuels non-`<img>` (CSS `background-image` ou injection JS)
   ne peignent pas ; les blocs de repli (boîte bleue Last.fm)
   restent visibles. Sévérité faible-moyenne.
6. **`unhandledrejection` remonté correctement** : les rejets
   Survicate (Deezer) et reactAds (Genius) apparaissent avec pile
   complète — le fix est validé en conditions réelles.

## Arrêt

Onglets 1-4 supprimés via `DELETE /api/tabs/{id}` ; daemon arrêté par
PID propre (19228). Aucun code modifié.

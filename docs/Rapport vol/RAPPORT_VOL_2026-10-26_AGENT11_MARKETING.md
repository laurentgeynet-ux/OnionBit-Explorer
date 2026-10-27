# Rapport de vol — Agent 11 — Sites marketing (2026-10-26)

Mission : voler sur des sites marketing au CSS sophistiqué, mesurer les
trous layout/CSS/DOM. Daemon `explorer-daemon` (binaire
`C:/obe-vol-head/target/release/`) sur `127.0.0.1:9395`,
viewport 1366×900, frames = rendu pleine page du moteur.

## Synthèse

| Cible | URL finale | doc_h | Console | État global |
|---|---|---|---|---|
| Apple | https://www.apple.com/ | 6 766 px | 1 exception JS | Largement lisible, défauts localisés |
| Stripe | https://stripe.com/fr | 12 645 px | « pas de journal » | **Très dégradé** — texte en colonnes ~1 mot/ligne, hero cassé |
| Mozilla | https://www.mozilla.org/ | 2 568 px | « pas de journal » | Lisible mais **commentaires peints en texte** + nav chevauchée |
| Vercel | https://vercel.com/ | 7 393 px | « pas de journal » | Thème sombre absent, `<#comment>` partout, hero chevauché |

## 1. Apple — https://www.apple.com/

**Console (verbatim)** :

```
exception js : TypeError: `registerHelper` is not a function (on u)
    at <anonymous> (https://www.apple.com/ac/localeswitcher/4/en_US/scripts/localeswitcher.built.js:1:116504)
    … — localeswitcher.built.js — src «!function e(t,n,r){function i(s,a){if(!n[s])…»
```

**Constats mesurés** (frame 1366×6766, captures `vol11_apple*.png`) :

- Hero iPad mini : images produit rendues correctement des deux côtés ;
  le titre « iPad mini » est **superposé par un second texte** (« mintv »
  fantôme, probablement un `<span>` gradient-text dupliqué) et le sous-titre
  « More than a little awesome. » **chevauche** le bloc « Pre-order starting
  at 10:10 a.m. PT on 10.23 / Available starting 11.6 » (~4 lignes de texte
  empilées au même emplacement).
- Élément de navigation déroulante **visible en permanence** : liste
  verticale « Mac / iPad / iPhone / Watch / Vision » flottant au-dessus du
  hero sous la barre de nav ; une mini-barre de recherche + icônes orphelines
  est aussi peinte en haut à gauche (menus qui devraient être
  `display:none`/JS-togglés restent peints).
- Section « iPhone 18 Pro » (fond noir pleine largeur) : bien rendue, texte
  centré, image produit correcte.
- Tuiles du grid bas (Watch Series 12, HomeView, Watch Ultra 4, AirPods Pro 3,
  Upgrade, Card) : structure 2 colonnes respectée, images OK, mais **textes
  en colonnes très étroites** (~90 px, un mot par ligne : « The most /
  accurate / heart rate / sensing in a / wearable.¹ ») — dimensionnement
  min-content au lieu de la largeur de la colonne.
- Footer : parfaitement rendu (colonnes de liens, mentions légales).
- Interactions : scroll ×3 OK ; clic à (660,200) → `{"result":"ignored"}`
  (zone non cliquable). Console identique après interactions (1 erreur).

**Classification** : layout (chevauchement de textes hero, colonnes
min-content), CSS (visibilité des menus — éléments masqués peints), JS
(`registerHelper` manquant → TypeError).

## 2. Stripe — https://stripe.com → redirection /fr

**Console** : aucune erreur rapportée (« pas de journal »).

**Constats mesurés** (frame 1366×12645, `vol11_stripe*.png`) :

- **Hero quasi illisible** : le titre et les paragraphes d'accroche sont
  rendus en **colonne d'environ 60-90 px de large** (un mot, voire un
  fragment de mot par ligne, lettres partiellement superposées :
  « Part Une Une / du infrastru/cture »…). Le fameux fond gradient
  animé du hero Stripe est **absent** (fond blanc).
- Bouton « Se connecter » : boîte noire sans libellé lisible + un lien
  violet « Contacter notre équipe » à sa droite (styles de bouton partiels).
- **Bandeau logos** (OpenAI, Amazon, Mistral AI, NVIDIA, Le Monde, Figma) :
  rendu correct.
- Section « Des solutions flexibles… » : titre bien posé, mais les 5 blocs
  de features sont à nouveau en **colonnes min-content** (~90 px, texte
  éclaté verticalement). Une barre gradient multicolore (décorative) rendue.
- Bandeau statistiques « 135+ / 1,9 Bn $US / 99,999 % / 200 M+ » : fond navy
  rendu, chiffres lisibles, mais un bouton vidéo orphelin (« Regarder la
  vidéo ») flotte en haut à gauche hors de son conteneur.
- Grande section navy (~2 800 px de haut) **majoritairement vide** : les
  cartes/grids de contenu attendues ne sont pas peintes (textes blancs
  visibles seulement aux extrémités). Un carrousel de témoignages (Hertz,
  Le Monde) est **empilé/chevauché** sur une image de kiosque.
- Footer : réduit à ~3 lignes (« Français / Stripe Inc / © 2026 Stripe,
  LLC ») — les colonnes de liens du footer réel sont **absentes ou
  effondrées**.
- Interactions : scroll ×3 OK ; **clic à (683,450) → `{"result":"navigated"}`
  vers `https://stripe.com/fr/atlas`** (capture `vol11_stripe_atlas.png`,
  2,1 Mo) — les hit-tests de liens fonctionnent malgré le layout cassé.

**Classification** : layout (min-content systématique dans les containers
flex/grid — constat dominant), CSS (gradient/clip-path hero absent,
visibilité de contenus, carrousel empilé), rendu correct des images.

## 3. Mozilla — https://www.mozilla.org/

**Console** : aucune erreur (« pas de journal »).

**Constats mesurés** (frame 1366×2568, `vol11_mozilla*.png`) :

- **`<#comment>` peint en texte visible** : des dizaines de nœuds
  commentaires HTML s'affichent littéralement (« <#comment> <#comment>… »)
  dans le header, dans le footer et entre les sections. **Vérifié dans le
  layout tree** : `vol11_mozilla_layout.txt` contient 35 boîtes
  `text "<#comment>"` (ex. ligne 4 : `text "<#comment>" [0 0.31 93.38x20]`) —
  les nœuds commentaires du DOM deviennent des **inline boxes textuels**
  au lieu d'être ignorés au layout.
- La page servie est le sélecteur de langue (« Choose your language or
  locale to browse Mozilla.org ») — grille de ~90 langues parfaitement
  rendue en 6 colonnes.
- Header/nav **chevauché** : « Products / Firefox / Mozilla… » superposés
  en ~40 px de hauteur, textes illisibles les uns sur les autres.
- « Our Mission » dans une boîte bordée, décalée — structure visible mais
  position approximative.
- Footer noir : bien rendu (colonnes Company/Support/Developers, icônes
  sociales, bouton Donate, logo géant « Mozilla » en bas de page).
- Interactions : scroll ×3 OK ; clic à (130,660) → `ignored`.

**Classification** : DOM→layout (**fuite des commentaires dans l'arbre de
boîtes** — hautement actionnable), layout (nav chevauchée).

## 4. Vercel — https://vercel.com/

**Console** : aucune erreur (« pas de journal »).

**Constats mesurés** (frame 1366×7393, `vol11_vercel*.png`) :

- **Même fuite `<#comment>`** : 28 boîtes `text "<#comment>"` dans
  `vol11_vercel_layout.txt` ; le **pied de page est littéralement un écran
  de `<#comment>`** (~15 lignes) + « Loading state » + logo triangle seul.
- **Thème sombre absent** : le site réel est quasi intégralement noir ;
  ici le fond body reste blanc et seules quelques bandes noires (sections
  média) apparaissent, souvent **vides** (vidéos/canvas absents, contenu
  blanc-sur-noir partiellement invisible ailleurs).
- Hero : « Agentic Infrastructure » rendu, mais accroche **chevauchée**
  (« Powered … Drop to deploy / To ship apps and a…Loading » — plusieurs
  lignes empilées) ; bouton « Deploy now » noir OK, « Talk to sales » OK.
- Marquee de logos (Meta, DoorDash, OpenAI, SpaceX…) : **dupliqué et
  superposé** (deux pistes du marquee peintes l'une sur l'autre).
- Sections produit : les captures d'écran intégrées (Notion AI chat, Zapier,
  Mintlify) sont bien rendues — images et ombres de cartes OK — mais le
  texte d'habillage est réduit à de fines colonnes à gauche, et les
  légendes contiennent encore « <#comment> » inséré **au milieu des
  phrases** (« Zapier serves over 100 million <#comment> monthly website
  visits »).
- Skip-links visibles (« Skip to content / main navigation » en surimpression
  haut-gauche) — éléments visually-hidden peints.
- Interactions : scroll ×3 OK ; clic à (683,450) → `ignored`. Aucune erreur
  console en fin de vol.

**Classification** : DOM→layout (commentaires peints), CSS (fond body du
dark theme non appliqué, visually-hidden non masqué, marquee dupliqué),
layout (hero chevauché).

## Constats actionables

1. **[Critique — DOM/layout] Les nœuds commentaires deviennent des boîtes
   texte peintes.** Preuve : `vol11_vercel_layout.txt` lignes 4-7 et 564+,
   `vol11_mozilla_layout.txt` ligne 4 — `text "<#comment>" [x y 93×24]`.
   Visuel : lignes de `<#comment>` dans header/footer Mozilla, footer Vercel
   entier, et insertion en pleine phrase sur Vercel. Les commentaires ne
   doivent jamais produire d'inline box.
2. **[Majeur — layout] Colonnes min-content systématiques** dans les
   containers flex/grid sophistiqués : texte éclaté à ~60-90 px/un mot par
   ligne sur Stripe (hero + features + colonnes) et tuiles Apple. Suspect :
   résolution de `flex-basis`/`grid` ou mesure intrinsèque qui retombe sur
   min-content faute de `width`/`fr` résolu.
3. **[Majeur — CSS] Éléments masqués peints** : menu déroulant Apple
   visible en permanence, skip-links Vercel affichés, bouton « Regarder la
   vidéo » Stripe orphelin. Les techniques `visually-hidden`/`clip`/
   `display:none` togglées par JS ne sont pas honorées (ou un état initial
   supposé masqué ne l'est pas).
4. **[Majeur — CSS] Chevauchements de texte dans les heroes** : Apple
   (« iPad mini » / « mintv », sous-titre sur pre-order), Vercel (accroche
   empilée), Mozilla (nav superposée). Probable éléments `position:absolute`
   ou line-height/transform non résolus.
5. **[Majeur — CSS/rendu] Décoratifs absents** : gradient animé Stripe,
   fond noir du dark theme Vercel (body background non propagé ?),
   bandes noires vides (vidéos/canvas sans fallback).
6. **[Mineur — JS]** `registerHelper is not a function` sur le bundle
   localeswitcher Apple — surface JS manquante (probablement un registre
   de helpers type Handlebars/module loader).
7. **[Positif]** Hit-testing fonctionnel (clic Stripe → navigation
   /fr/atlas), redirection locale Stripe honorée, images produits et
   screenshots bien rasterisées, footers Apple/Mozilla quasi parfaits,
   aucune erreur console sur 3 cibles, aucun crash (tolérance OK).

## Annexes

- Captures : `captures/vol11_{apple,stripe,stripe_atlas,mozilla,vercel}.png`
  (+ découpes `_pN.png` pour lecture).
- Logs : `logs/vol11_{cible}_{console.json,dom.txt,layout.txt}`.
- Daemon : `explorer-daemon --port 9395`, log `_vol11_daemon.log`,
  arrêté en fin de mission, onglets fermés.

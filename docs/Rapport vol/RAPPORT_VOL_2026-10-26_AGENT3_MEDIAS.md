# Rapport de vol — Agent 3 (médias / social / archives)

- Date : 2026-10-26
- Daemon : `C:/obe-vol-head/target/release/explorer-daemon.exe`, port 9387,
  viewport 1366×900 (HEAD 7a01534)
- Protocole : `docs/vol-agent.md`

Résumé : 4/4 sites injoignables → aucun n'est réellement utilisable.
Le motif dominant est **JS** (modules ES / APIs manquantes) aggravé par
un **bug de parsing HTML récurrent** : les commentaires `<!-- -->`
fuient dans le texte rendu sous la forme `<#comment>`.

---

## 1. https://mastodon.social/explore

- **URL finale** : `https://mastodon.social/explore` (pas de redirection), titre « Mastodon.social »
- **Erreurs console (1)** :
  1. `exception js : TypeError: cannot load module \`./axios-Blsu9zSB.js\`: module non chargé : https://mastodon.social/packs/axios-Blsu9zSB.js — module «https://mastodon.social/packs/application-CBLJxgs9.js»`
- **Constats visuels** (`captures/vol3_mastodon.png`, identique après
  scroll/clic dans `vol3_mastodon_after.png`) :
  - Page quasiment blanche (fond `#f8f9fb`), aucune UI de l'app.
  - Le contenu du `<noscript>` est rendu **en texte brut** : la balise
    `<img alt="Mastodon" src="/packs/assets/logo-DXQkHAe5.svg" />` et
    `<div>` apparaissent *littéralement* à l'écran, suivies du message
    « To use the Mastodon web application, please enable JavaScript… ».
    Alors que JS est actif dans le moteur, le `<noscript>` ne devrait
    rien afficher — et son contenu est sérialisé au lieu d'être parsé.
- **Layout** : `block<html> 1366x900` ; le `<noscript>` occupe
  1366×54.4 en haut ; le reste (divs, svg) à 0×0 — l'app ne s'est
  jamais montée.
- **Classification** :
  - JS (majeur) : échec de chargement d'un module ES relatif
    (`./axios-*.js` importé par `application-*.js`) → toute l'app morte.
  - DOM/HTML (majeur) : gestion de `<noscript>` incorrecte (contenu
    affiché + sérialisé en texte alors que le scripting est actif).
- **Sévérité** : bloquant — page inutilisable.

## 2. https://old.reddit.com

- **URL finale** : `https://old.reddit.com/login/?reason=lor2&dest=…`
  (redirection serveur vers login ; Reddit sert en fait la coquille
  « shreddit » web-components, même sur old.*)
- **Erreurs console** : 0 (« pas de journal » — aucune exception
  rapportée, le silence lui-même est suspect : les scripts modules
  shredit n'ont visiblement pas produit d'UI).
- **Constats visuels** (`captures/vol3_reddit.png`, `vol3_reddit_after.png`) :
  - Page blanche à ~95 %. Seul le logo Reddit s'affiche en haut à
    gauche, dans un cadre 616×57 ; le wordmark « reddit » est rendu
    **écrasé/déformé** (lettres noir/orange aplaties, SVG mal mis à
    l'échelle).
  - Deux occurrences du texte `<#comment>` visibles (lignes ~y=68 et
    ~y=890) : les commentaires HTML `<!-- #comment -->` du document
    sont **rendus comme du texte**.
  - Aucun formulaire de login, aucun lien, rien du contenu shreddit.
- **Layout** : `shreddit-app 1366x941` ; header 616×57 ; tous les
  custom elements (`faceplate-loader`, `alert-controller`, …) sont
  `inline` à 0×0 — non stylés, non upgradés.
- **Classification** :
  - DOM/HTML (majeur) : fuite des nœuds commentaires en texte rendu
    (`<#comment>`).
  - JS (majeur, probable) : web components jamais upgradés
    (customElements/ESM), contenu applicatif absent — sans erreur
    console rapportée.
  - Layout/CSS (mineur) : mise à l'échelle SVG du logo cassée.
- **Sévérité** : bloquant — page inutilisable.

## 3. https://bandcamp.com/discover

- **URL finale** : `https://bandcamp.com/discover`, titre correct.
- **Erreurs console (1)** :
  1. `exception js : TypeError : \`supportedValuesOf\` is not a function (on Object) — module «https://s4.bcbits.com/vite/bc-discover-page/client/v1/en/entrypoints/DiscoverApp.vue_en-46QVaVXQ.js»`
- **Constats visuels** (`captures/vol3_bandcamp.png`,
  `vol3_bandcamp_after.png`) :
  - Chrome de la page correctement rendue : header avec champ
    « Search and discover music », « Sign up / Log in », pied de page
    complet (About, réseaux sociaux, Bandcamp for Artists…), fond
    gris foncé conforme.
  - **Résultats absents** : zone de résultats = un spinner figé au
    centre — `Intl.supportedValuesOf` manquant tue le bootstrap Vue.
  - **Chips de filtres cassées** : les étiquettes de genres
    (« electronic », « rock », « hip-hop/rap », « ambient »…) se
    **chevauchent mutuellement** dans leur rangée ; idem « vinyl /
    compact discs / cassettes / t-shirts » — texte qui déborde des
    boîtes, chevauchement horizontal net.
- **Layout** : 25 176 octets, structure riche ; doc 1366×1472.
- **Classification** :
  - JS (majeur) : `Intl.supportedValuesOf` non implémenté.
  - Layout/inline (moyen) : chips de tags qui se chevauchent
    (flex-wrap / inline-block metrics).
- **Sévérité** : majeur — page présentable mais fonctionnalité cœur
  (liste de résultats) morte.

## 4. https://archive.org

- **URL finale** : `https://archive.org/`, titre correct.
- **Erreurs console** : 0 (« pas de journal ») — et pourtant rien ne
  s'exécute : silence suspect, possiblement des scripts modules dont
  l'échec n'est pas rapporté.
- **Constats visuels** (`captures/vol3_archive.png`,
  `vol3_archive_after.png`) :
  - **Page blanche totale** sauf le texte `<#comment>` en haut à
    gauche — même fuite de commentaire HTML que Reddit.
  - `doc_h` rapporté = 15.4 px : le document mis en page ne fait
    qu'une ligne de haut.
- **DOM** : shell Angular de 5.7 KB seulement (`<app-root>` + 7
  `<script>` + `<noscript>` « Javascript is required… »). Le DOM
  n'a pas grossi après 10 s + interactions → bootstrap Angular
  jamais exécuté, aucun script ayant produit d'effet.
- **Layout** : `app-root inline 58×14`, `header` et `a` à 0×0.
- **Classification** :
  - JS (bloquant) : scripts jamais exécutés ou échec silencieux non
    rapporté dans le ring console (à investiguer : ESM, importmap,
    ou erreur avalant l'exception).
  - DOM/HTML (majeur, récurrent) : `<#comment>` rendu en texte.
- **Sévérité** : bloquant — page blanche.

---

## Constats actionnables

1. **[HTML/DOM — priorité haute, récurrent]** Les commentaires HTML
   `<!-- … -->` fuient dans l'arbre de texte et sont **rendus** à
   l'écran sous la forme littérale `<#comment>` (observé sur
   old.reddit.com ×2 et archive.org). Vérifier le tokenizer/tree-builder
   : un nœud commentaire ne doit jamais produire de boîte ni de run
   de texte — le bug semble transformer le token commentaire en
   character token.
2. **[HTML/DOM — haute]** `<noscript>` est rendu (et son contenu
   sérialisé en texte brut, balises visibles) alors que le scripting
   est **actif** (mastodon.social). Respecter le flag scripting du
   tree-builder : `noscript` en scripting-enabled doit être parsé en
   rawtext → jamais affiché.
3. **[JS — bloquant, pattern ESM]** Échec de chargement d'un import
   relatif de module : `./axios-Blsu9zSB.js` depuis
   `/packs/application-CBLJxgs9.js` (mastodon). Vérifier la
   résolution d'URL relative des modules ES et le suivi des échecs
   de fetch de sous-modules.
4. **[JS — haute]** `Intl.supportedValuesOf` absent (bandcamp). Stub
   minimal ou implémentation à ajouter à explorer-jscore (retourner
   au moins `[]`/listes connues pour `calendar`, `currency`,
   `numberingSystem`, `timeZone`, `unit`).
5. **[JS — investigation]** Les apps web-components/Angular ne
   bootstrap pas **sans aucune erreur en console** (reddit shreddit,
   archive.org). Soit les scripts `type=module` ne s'exécutent pas
   silencieusement, soit l'exception est perdue — le ring console
   doit capturer les échecs de fetch/parse/exécution de modules.
6. **[Layout — moyen]** Chevauchement des chips/tags inline
   (bandcamp : étiquettes de genres se recouvrant) — métriques
   inline-block/flex-wrap à vérifier.
7. **[Rendu — mineur]** Mise à l'échelle de SVG inline défaillante
   (wordmark « reddit » écrasé dans un header 616×57).

## Artefacts

- Captures : `docs/Rapport vol/captures/vol3_{mastodon,reddit,bandcamp,archive}[_after].png`
- Logs : `docs/Rapport vol/logs/vol3_<site>_{console.json,dom.txt,layout.txt}`
- Journal daemon : `_vol3_daemon.log` (vide — tracing non configuré)

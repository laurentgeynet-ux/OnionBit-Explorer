<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 25 — sites voyage/immobilier

- **Date** : 2026-10-26
- **Binaire** : `C:/obe-vol-integ/target/release/explorer-daemon.exe`
  (binaire d'intégration, ~20 fixes intégrés)
- **Viewport** : `EXPLORER_VIEWPORT=1366x900`, port API 9409
- **Captures** : `captures/vol25_*.png` — **Logs** : `logs/vol25_*_{dom,layout}.txt`
- **Daemon log** : `_vol25_daemon.log`

## Synthèse

| Cible | Verdict | Cause principale |
|---|---|---|
| booking.com | ❌ page blanche | Challenge AWS WAF non résoluable (JS) |
| airbnb.fr | ⚠️ coquille SSR | Skeleton rendu, hydratation absente, aucun log JS |
| tripadvisor.fr | ❌ HTTP 403 | Blocage bot côté serveur (page d'erreur OK) |
| seloger.com | ❌ HTTP 403 | Blocage bot côté serveur (page d'erreur OK) |
| leboncoin.fr (fallback) | ❌ HTTP 403 | Blocage bot côté serveur |
| kayak.fr (extra) | ⚠️ partiel | **Moteur JS figé → API bloquée ~90 s**, layout cassé |

Aucun site n'est navigable. Trois blocages réseau (403), un WAF-JS,
une hydratation morte, un gel du moteur JS qui prend l'API en otage.

---

## 1. booking.com — page blanche (WAF)

- URL : `https://www.booking.com/` — `doc_h: 8.0` à la nav, title null.
- **Console verbatim** :
  - `exception js : TypeError: cannot read \`contentWindow\` of Undefined
    at <anonymous> (.../challenge.js:1:510440) ...`
  - `exception js : ReferenceError: AwsWafIntegration is not defined —
    src «let newHref; try { setTimeout(() => { try { reportChallengeError(
    "Reloaded without updating url"…»`
- **DOM** : page-challenge `awsWafCookieDomainList = ['booking.com']` —
  c'est bien la page interstitielle AWS WAF, pas la home.
- **Frame** : 1366×900 intégralement blanche (`vol25_booking.png`).
- Classification : bloquant externe (anti-bot) + trou JS
  (`iframe.contentWindow` non résolu → le challenge ne peut jamais
  aboutir, même côté humain ça tournerait en boucle).
- **Sévérité : haute** — sans `contentWindow`, aucun site derrière AWS
  WAF ne passe.

## 2. airbnb.fr — coquille SSR sans hydratation

- URL : `https://www.airbnb.fr/` — titre correct
  (« Airbnb : locations de vacances… »), `doc_h: 900.0` (== viewport :
  contenu principal à hauteur nulle).
- **Console : vide** (`"pas de journal"`) — **aucune erreur JS rapportée**
  alors que les bundles n'ont manifestement pas hydraté la page.
  Soit les scripts ne sont pas exécutés, soit ils échouent en silence :
  dans les deux cas, **l'absence totale de télémétrie est un trou**.
- **Frame** (`vol25_airbnb.png`) : en-tête rendu (logo SVG airbnb,
  onglets Logements/Hôtels/Expériences/Services), barre de recherche
  à moitié dessinée (champ vide + bouton loupe magenta seul, libellés
  « Destination / Dates / Voyageurs » absents), 3 barres grises de
  skeleton à gauche, bannière cookies lisible et bien composée en bas
  (« Accepter tout », « Gérer les préférences »). Aucune carte
  d'annonce — la grille produit est entièrement absente.
- **Interactions** :
  - scroll ×2 (+1600 px) → frame **strictement identique** (même
    taille PNG 2 009 622 o) : `doc_h == viewport`, rien à scroller.
  - click sur « Accepter tout » (150, 758) → `{"result":"ignored"}`.
  - mousemove sur le champ recherche (180, 127) →
    `{"editable":false,"result":"hover"}` ; click → `ignored` ;
    touche `a` → `unfocused`.
  - Frame après clic : identique octet pour octet — **zéro réaction**,
    aucun handler JS n'est actif.
- Classification : hydratation JS morte silencieuse (constat le plus
  important du vol — un site entier qui « marche visuellement » à 10 %
  sans aucune erreur remontée).
- **Sévérité : haute**.

## 3. tripadvisor.fr — HTTP 403

- Réponse navigate : `{"error":{"handled":true,"message":"HTTP 403"}}`.
- Console : `["HTTP 403"]`. DOM/layout : 404 (pas de document).
- **Frame** : page d'erreur interne correctement rendue —
  « Page inaccessible / La page https://www.tripadvisor.fr/ n'a pas pu
  être chargée. Le serveur a répondu « HTTP 403 ». Réessayez plus tard… »
  — texte net, mise en page propre (**la page d'erreur moteur fonctionne,
  fix confirmé dans son périmètre**).
- Classification : blocage anti-bot externe (hors moteur).
- Sévérité : n/a moteur ; à noter pour la politique UA/fingerprint.

## 4. seloger.com — HTTP 403

- Identique à TripAdvisor : `HTTP 403`, page d'erreur interne rendue,
  pas de document. Blocage anti-bot (DataDome probable).
- Sévérité : n/a moteur.

## 5. leboncoin.fr (fallback) — HTTP 403

- Identique : `HTTP 403`, page d'erreur interne.
- Trois 403 sur les cibles FR → le fingerprint du client (TLS/UA/en-têtes)
  est détecté par les WAF français. Piste connue, pas un fix moteur.

## 6. kayak.fr (cible extra) — gel du moteur JS + layout cassé

- URL : `https://www.kayak.fr/` — titre OK.
- **Le POST /navigate est resté muet >10 s** et **`GET /api/tabs` a mis
  ~90 s à répondre** pendant que le moteur JS tournait : le gel d'une
  éval bloque toute l'API REST — constat de disponibilité, pas juste de
  rendu.
- **Console verbatim** :
  - `exception js : moteur figé — éval dépassée — src «window.manifest =
    JSON.parse(document.getElementById('jsonData_manifest').innerHTML)…»`
  - `exception js : moteur figé — éval dépassée — src «window.__vite_
    plugin_react_preamble_installed__ = true; …»`
  - `exception js : moteur figé — éval dépassée — module
    «https://content.r9cdn.net/frontier/assets/BLZheESR3U.js»`
  - `moteur js inerte` **×16** — après 3 évals dépassées, le moteur se
    déclare inerte et tout script ultérieur est ignoré.
- **Frame** (`vol25_kayak.png`) : header partiel (logo KAYAK en pastilles
  orange, « Assistant IA », sélecteur EUR, icône compte), chip
  « Marseille (MRS) », **deux énormes flèches ⇄ pixelisées** au centre
  (SVG/icône swap rendu à une échelle démesurée — constat rendu),
  titre « L'organisation de votre voyage commence ici ».
- **Scroll +700** (`vol25_kayak_scroll.png`, `scroll_y: 700`) : liste de
  liens SSR brute, non stylée — « VOITURES • VOLS • HÔTELS », hôtels
  avec prix (« Central Park West Hostel51 €+ » — espace insécable mangée
  entre libellé et prix), vols avec prix, section « Lisbonne », chevron
  SVG géant en bas. **Layout entièrement linéarisé à gauche** : grille/
  flex non appliquées (CSS moderne Kayak : vars + grid).
- Classification : gel JS (perf) + couverture CSS insuffisante.
- **Sévérité : critique** pour le gel API ; haute pour le layout.

---

## Constats actionnables

1. **[CRITIQUE] Une éval JS figée bloque toute l'API REST ~90 s.**
   Sur kayak.fr, 3 scripts dépassent le budget d'éval → `moteur figé —
   éval dépassée`, puis `moteur js inerte` ×16. Pendant ce temps
   `GET /api/tabs` ne répond plus. Le budget temps JS doit être borné
   *par* éval sans monopoliser la boucle d'événements ; l'API doit
   rester réactive pendant l'exécution d'un script hostile. (Kayak,
   logs `vol25_kayak_*`.)
2. **[HAUTE] Hydratation morte en silence sur Airbnb.**
   `console` vide (`pas de journal`), DOM de 3 398 lignes = squelette
   SSR, `doc_h` == viewport, clics `ignored`, champ `editable:false`,
   touche `unfocused`, frame immuable. Soit les scripts ne tournent pas,
   soit ils meurent avant le premier handler — et rien n'est remonté.
   Il faut au minimum une erreur « script non exécuté » dans `/console`.
   (`vol25_airbnb*.png`, `vol25_airbnb_dom.txt`.)
3. **[HAUTE] `iframe.contentWindow` non résolu → AWS WAF infranchissable.**
   `TypeError: cannot read 'contentWindow' of Undefined` dans
   challenge.js de booking.com : tout site derrière AWS WAF (booking,
   et une bonne part du web commercial) s'arrête sur une page blanche.
   (`vol25_booking*`.)
4. **[MOYEN] Fingerprint client détecté : 403 systématique sur les WAF
   français** (tripadvisor, seloger, leboncoin). Hors moteur pur mais
   bloque tout le segment immobilier/voyage FR — à suivre côté
   empreinte TLS/UA de `explorer-net`.
5. **[MOYEN] SVG/icônes rendus à échelle géante.**
   Flèches ⇄ ~350 px pixelisées au lieu de ~24 px sur kayak (viewport
   SVG non borné ?), chevron « V » géant après scroll. Voir
   `vol25_kayak.png` / `vol25_kayak_scroll.png`.
6. **[MOYEN] Layout Kayak entièrement linéarisé** : grille/flex + CSS
   custom properties du design system non appliquées — tout le contenu
   SSR s'empile à gauche, texte brut bleu-lien. Espace insécable
   perdue avant les prix (« Hostel51 €+ » au lieu de « Hostel 51 €+ »).
   (`vol25_kayak_scroll.png`.)
7. **[INFO / fix confirmé] Page d'erreur HTTP interne rendue
   proprement** (titre, URL, code, conseil) sur les trois 403 —
   le chemin « handled error » fonctionne.
8. **[INFO] Clics « ignored » non diagnostiqués** : sur Airbnb,
   `click` renvoie `{"result":"ignored"}` sans raison (hit-test raté ?
   pas de handler ? overlay ?) — exposer le motif dans la réponse
   aiderait le vol agent à distinguer « zone non cliquable » de
   « handler absent ».

## Hygiène de vol

- Onglets créés : 1 (booking), 2 (airbnb), 3 (tripadvisor),
  4 (seloger), 5 (leboncoin), 6 (kayak) — tous fermés via `DELETE`
  avant arrêt.
- Daemon arrêté par son PID (11443) — pas de `taskkill /IM`.
- Aucun fichier de code modifié.

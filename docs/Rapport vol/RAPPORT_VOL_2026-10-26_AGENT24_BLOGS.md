# Rapport de vol — Agent 24 — Plateformes de blogs/CMS

- **Date** : 2026-10-26
- **Binaire** : `C:/obe-vol-integ/target/release/explorer-daemon.exe` (binaire d'intégration, ~20 fixes)
- **Viewport** : `EXPLORER_VIEWPORT=1366x900`, port API 9408
- **Cibles** : medium.com, substack.com (+ article), blogger.com, wordpress.com/fr, ghost.org (fallback)
- **Note reprise** : l'agent précédent est mort en erreur interne ; des fichiers
  `vol24_medium_*` orphelins de sa tentative coexistent avec les miens dans `logs/`.

## Synthèse

| Cible | Chargement | Rendu | Console |
|---|---|---|---|
| medium.com | **HTTP 403** | page quasi blanche (message d'erreur) | `HTTP 403` |
| substack.com | OK | home rendue, texte + structure | 6× image 404 + 1 exception JS |
| article Substack | OK (clic → navigation) | article lisible | idem |
| blogger.com | OK (redir. `/about/?bpli=1`) | page rendue, 42 `<img>` | aucune nouvelle erreur |
| wordpress.com/fr | OK | page rendue, 30 `<img>` | RangeError (budget JS) + TypeError |
| ghost.org | OK | page rendue, 73 `<img>` | aucune nouvelle erreur |

## medium.com — https://medium.com

- `navigate` → réponse `{"error":{"handled":true,"message":"HTTP 403"}}`.
- `GET /dom` → `pas de DOM` ; `GET /layout` → `pas de layout`.
- `GET /console` verbatim : `{"console":["HTTP 403"]}`.
- Frame (`vol24_medium.png`, 1366×900) : fond blanc à 99 % avec quelques
  pixels de texte sombre (~3 400 px noirs/gris) — page d'erreur moteur
  minimaliste ou corps du 403 rendu sans style.
- **Constat** : Medium répond 403 à notre client (protection anti-bot
  classique — un navigateur réel reçoit du HTML). Le moteur dégrade
  proprement, pas de crash. Le corps d'erreur serveur est-il rendu ?
  Il semble afficher du texte brut — acceptable.
- Classification : **infrastructure réseau / blocage serveur**, sévérité
  moyenne (le site est inutilisable, mais hors portée moteur sauf si on
  veut travailler le fingerprint client).

## substack.com — https://substack.com

- Navigate OK → titre `Substack`, `doc_h = 4728.68`, non clampé.
- Console verbatim (cumulatif — voir constat transverse) :
  - 6× `sous-ressource en échec (HTTP 404) : image https://substackcdn.com/image/fetch/$s_!oaAR!`
    (variantes `!40EB!`, `!vPFG!`, `!mwzW!`, `!zVGl!`, `!vDAU!`)
  - `exception js : TypeError: value is not a function: undefined — src «if (window.matchMedia) { const match = window.matchMedia('(prefers-color-scheme: dark)'); match.…»`
- **Constat mesuré — URLs d'images littérales `$s_!xxx!`** : les `src` des
  images Substack contiennent des placeholders `$s_!oaAR!` que le JS de la
  page est censé réécrire. Notre fetch part sur l'URL brute → 404
  systématique. Le script de réécriture échoue (le `matchMedia(...)` du
  TypeError est probablement `matchMedia().addListener` ou similaire non
  implémenté → le script suivant qui fixerait les URLs ne s'exécute pas).
  Layout : 0 nœud `image` peint dans le dump home (les `<img>` existent
  mais leurs boîtes n'apparaissent pas sous ce nom — à vérifier ; en tout
  cas **aucune photo d'article visible**, 184 couleurs sur la frame initiale
  vs 625 après scroll, du texte essentiellement).
- Scroll ×3 OK (`scroll_y` 800→2400), frame scroll capturée.
- **Clic sur un article** (x=350,y=770 après scroll 2400, sur un `<a>`
  document-coords [300 3158 206×107]) → `{"result":"navigated"}` vers
  `https://soniamanginalegriagogia.substack.com/p/adopte-un-jeu-de-plateau-idees-dutilisation`
  — navigation par clic fonctionnelle ✓.
- Article : layout contient le titre, l'auteure, le corps ; seulement
  **3 `<img>`** dans tout l'arbre (un article photo-illustré en aurait
  davantage — le même problème `$s_!` frappe probablement).
- Classification : **JS — API manquante** (TypeError) + **images non
  résolues** (placeholders `$s_!`), sévérité haute pour la fidélité.

## blogger.com — https://www.blogger.com

- Navigate → redirection suivie vers `https://www.blogger.com/about/?bpli=1`
  (titre correct « Blogger.com - Create a unique and beautiful blog
  easily. »). `doc_h = 5538.6`.
- Console : aucune erreur nouvelle (les entrées affichées sont le reliquat
  cumulatif — voir constat transverse).
- Layout : textes rendus (« Create your blog », « Publish your passions,
  your way »), header avec `<svg>` inline, skip-link « Skip to content »
  bien en haut, **42 `<img>`**. Frame colorée (moyenne RGB ~92,131,118 —
  fond hero verdâtre cohérent avec la vraie page).
- Scroll ×3 OK.
- Classification : **bon rendu**. Pas de constat bloquant.

## wordpress.com/fr — https://wordpress.com/fr/

- Navigate OK → titre FR correct, `doc_h = 5182.83`.
- Console (erreurs nouvelles) :
  - `exception js : RangeError: script execution limit exceeded` — le
    bundle combiné `_static/??-eJylz…` dépasse le budget d'exécution JS
    (comportement voulu côté budgets, mais le script n'aboutit pas).
  - `exception js : TypeError: cannot read 'accounts' of Undefined at <anonymous> (14:37)`
- Layout : navigation FR rendue (« Produits », « Ressources »,
  « Assistance », « Tarification », « Connexion »), 30 `<img>`.
- Frame moyenne ~247 → page claire cohérente.
- Classification : **fonctionnel avec JS partiellement tué par le budget**
  — sévérité moyenne (le contenu statique tient, l'interactivité est
  inconnue).

## ghost.org — https://ghost.org (fallback)

- Navigate OK → titre correct, `doc_h = 10578.2` (longue page), non clampé.
- Console : **aucune erreur nouvelle** — JS propre.
- Layout : sections « For Creators / For Publishers / For Business »
  rendues, **73 `<img>`**, 1 081 lignes de boîtes.
- Classification : **bon rendu**.

## Constats transverses

1. **`GET /console` n'est jamais vidé à la navigation** : le journal est
   cumulatif par onglet — après ghost.org il contenait encore les 404
   substackcdn et le TypeError wordpress (11 entrées au total). Les
   erreurs sans URL de page d'origine rendent l'attribution ambiguë.
   → actionable : préfixer chaque entrée par l'URL de la page émettrice,
   ou vider le journal à la navigation.
2. **URLs d'images avec placeholders `$s_!xxx!` non réécrites**
   (substackcdn) — lié à l'échec du script `matchMedia(...)`.
   → actionable : implémenter l'API manquante signalée par
   `TypeError: value is not a function` (`matchMedia(...).addListener` ou
   équivalent), ou confirmer en test quel membre manque.
3. **Medium 403** → rendu d'une page d'erreur quasi blanche.
   → actionable (produit) : accepter/documenter ; côté moteur, vérifier
   que le corps du 403 est bien affiché (2 % de pixels non blancs seulement).

## Fixes non régressés observés

- Clic → navigation inter-sites ✓
- Scroll multipalier ✓ (`scroll_y` exact)
- Suivi de redirection (blogger.com → /about/?bpli=1) ✓
- Titres de page extraits correctement sur 4/4 pages chargées ✓
- Aucun crash, aucun panic sur 5 sites hostiles ✓

## Fichiers produits

- Captures : `vol24_medium.png`, `vol24_substack.png`,
  `vol24_substack_scroll.png`, `vol24_substack_click.png`,
  `vol24_blogger.png`, `vol24_blogger_scroll.png`, `vol24_wordpress.png`,
  `vol24_ghost.png` (dans `captures/`)
- Logs : consoles/dom/layout `vol24_*` (dans `logs/`)
- Journal daemon : `_vol24_daemon.log` (racine du repo)

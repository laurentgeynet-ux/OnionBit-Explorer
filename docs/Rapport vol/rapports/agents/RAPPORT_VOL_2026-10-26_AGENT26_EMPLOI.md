# Rapport de vol — AGENT26 — Sites emploi / pro

- **Date** : 2026-10-26
- **Binaire** : `C:/obe-vol-integ/target/release/explorer-daemon.exe` (intégration V2 : unhandledrejection→console, disjoncteur moteur figé, ellipsis, hit-test transform, reset console à la nav)
- **Daemon** : PID 10340, port 9410, `EXPLORER_VIEWPORT=1366x900`
- **Captures** : `captures/vol26_*.png` — **Logs** : `logs/vol26_*_{dom,layout}.json`

## Synthèse des fixes V2

| Fix | Verdict |
|---|---|
| Reset console à la nav | **Confirmé** — onglet 4 : `["HTTP 403"]` après apec.fr, puis `pas de journal` après navigation francetravail.fr |
| Ellipsis | **Confirmé** — francetravail.fr : « relationne… », « les besoins … », « p… » tronqués proprement dans les cartes |
| Disjoncteur moteur figé | **Confirmé** — 5 navigations, 4 onglets, daemon resté sain de bout en bout, aucune gélure |
| Hit-test transform | **Infirmé / régression possible** — **tous** les clics répondent `{"result":"ignored"}`, tous les survols `{"editable":false}` (voir constat n°1) |
| unhandledrejection→console | **Non déclenché** — aucune entrée console sur aucune cible (`pas de journal` partout) ; impossible de confirmer ni d'infirmer |

## Cible 1 — welcometothejungle.com/fr

- **URL** : `https://www.welcometothejungle.com/fr` → titre « Choisissez l'entreprise faite pour vous », `doc_h=10002.98`
- **Console** : `pas de journal` (verbatim) — après nav, après scrolls, après clics
- **Constats mesurés** :
  - Premier viewport : page éclatée — titre h1 rendu en très gros corps débordant l'écran (« Ne cherchez plus n'importe quel job, trouvez votre place. » tracé sur ~7 lignes géantes), liens de nav superposés (« Je recrute » / « Trouver un job » chevauchés, barre de sélection erratique horizontale), aucune image, aucune couleur de marque.
  - Scroll y=1600 : une **carte de poste parfaitement rendue** — « Product Manager Senior / Good company », badges (`82K€/an`, `Senior`, `Paris`, `Plutôt au bureau`, `IT`, `Foodtech`, `B2C`), boutons « Je sauvegarde » / « Je passe » avec arrondis et jaune de marque, carte empilée sur fond violet. Contraste total avec le viewport 0 : le CSS se comporte sous la ligne de flottaison.
  - Clics (230,55 puis 320,170 sur la carte) → `{"result":"ignored"}` ; hover → `{"editable":false,"result":"hover","url":null}` — pourtant la carte est un `<a>` dans le DOM (38 `<a>`, 0 `<input>` dans le DOM SSR ; barre de recherche absente du markup servi).
- **Classification** : moteur (layout/cascade du premier écran ; hit-test).
- **Sévérité** : élevée (premier écran illisible) mais contenu profond correct.

## Cible 2 — indeed.fr

- **URL** : `https://www.indeed.fr` → `https://fr.indeed.com/`, titre « Emplois | Indeed », `doc_h=4638.7`
- **Console** : `pas de journal`
- **Constats mesurés** :
  - **Formulaire de recherche correctement rendu** : deux champs (« Intitulé de poste, mots-clés ou entreprise », « Ville, département, code postal ou « Télétravail » ») + bouton « Rechercher » bleu — structure, libellés et couleurs fidèles.
  - **Glyphes SVG géants pixelisés** : formes noires massives (icône/illustration mise à l'échelle sans borne) recouvrant le centre de page, texte « Trouvez votre prochain emploi ici » et « Créez un compte… » à moitié masqué dessous, bouton « Commencer » avec flèche géante pixelisée.
  - **Champ inutilisable** : `mousemove (220,65)` sur le champ → `editable:false` ; `click` → `ignored` ; `key d/e/v` → `unfocused`. Impossible de saisir une recherche.
  - Bandeau cookies : rendu, texte complet, bouton « Autoriser tous les co… » rogné à droite du viewport (bouton déborde la fenêtre — la moitié est hors-champ).
  - Scroll y=1600 : listes de liens de tendances rendues en colonne unique brute (flex/grid non appliqué — 30+ liens « winter season », « freelance relecteur… », « Lieux populaires » empilés), bandeau cookies `position:fixed` correct (reste en bas).
- **Classification** : moteur (dimensionnement SVG, hit-test/focus champ, layout flex/grid en seconde partie).
- **Sévérité** : élevée — la fonction cœur (recherche) est visible mais non actionnable.

## Cible 3 — glassdoor.fr

- **URL** : `https://www.glassdoor.fr` → `…/index.htm`, titre « Recherche d'emploi Glassdoor | Découvrez l'entreprise qui vous mérite », `doc_h=5524.1`
- **Anti-bot** : **aucun blocage observé** — HTML complet servi (le payload Next embarque `"cloudflare":{"botScore":"87","verifiedBot":"false","threatScore":"0"}` — score élevé mais page livrée quand même).
- **Console** : `pas de journal`
- **Constats mesurés** :
  - **Meilleur rendu du vol** : logo Glassdoor vert, nav (Emplois/Entreprises/Salaires/Pour les employeurs), titre en vert de marque, illustrations SVG dessinées fidèlement (avion en papier, personnages, icônes cerclées), section « Prenez une longueur d'avance » avec 4 tuiles icône+libellé.
  - **Barre de recherche absente du DOM servi** : 0 `<input>` — zone héro blanche là où le formulaire devrait être (rendu JS attendu).
  - **BUG MAJEUR — contenu posé mais jamais peint** : le layout contient 200+ textes de y=895 à y=5520 (« Commencez votre recherche », 30+ « Offres d'emploi populaires », footer « Copyright © 2008-2026… »), or à `scroll_y=1600` la frame est **intégralement blanche**. Les boîtes existent, le raster ne suit pas — clip de display list ou invalidation de paint hors viewport initial.
  - Clics sur « Se connecter » (1250,27) et « Emplois » (220,10) → `ignored` ; hovers `editable:false`.
- **Classification** : moteur (paint/clip au scroll) + JS (champ recherche injecté).
- **Sévérité** : critique — page à moitié invisible malgré un layout correct.

## Cible 4 — apec.fr

- **URL** : `https://www.apec.fr` → **HTTP 403** (anti-bot, WAF côté serveur — hors périmètre moteur).
- **Console** : `{"console":["HTTP 403"]}` (verbatim)
- **Constats mesurés** : page d'erreur moteur **propre et informative** — « Page inaccessible / La page https://www.apec.fr/ n'a pas pu être chargée. / Le serveur a répondu « HTTP 403 ». / Réessayez plus tard, ou revenez à la page précédente. ». DOM = 10 octets (page synthétique). Dégradation gracieuse conforme à la règle « tolérant, jamais crash ».
- **Sévérité** : externe (blocage serveur), pas un trou moteur.

## Cible 5 (fallback) — francetravail.fr

- **URL** : `https://www.francetravail.fr/accueil/` — titre « Accueil | France Travail », `doc_h=4606`
- **Console** : `pas de journal` — le `["HTTP 403"]` précédent a été purgé à la nav → **fix reset console confirmé**.
- **Constats mesurés** :
  - Structure fidèle : carte « Recherche d'offres — 751106 offres d'emploi disponibles », deux libellés de champ (« Métier, mot-clé, n° d'offre », « Lieu de travail »), bouton « Rechercher » bleu nuit, 4 cartes de service (Estimer/S'inscrire/Actualisation fermée « Ouverture le 28/10/2026 »/Se connecter), 3 colonnes d'accordéons.
  - **Voile gris global** : toute la page (y compris le scroll y=1600) est peinte sous une pellicule semi-transparente — un backdrop modal/consentement reste tracé par-dessus le document.
  - **Icônes manquantes** : chevrons d'accordéon et puces rendus en **boîtes tofu `□`** (glyphe de fallback — police d'icônes ou caractère non couvert).
  - **Champs sans boîte** : les deux inputs n'affichent que leur label, aucun cadre visible.
  - Sous la ligne de flottaison : **grille de cartes avec vraies images JPEG chargées** (EHPAD, bus, kiné…), ellipsis actif sur les résumés tronqués — fix confirmé en situation réelle.
  - Clics (champ recherche, carte « Se connecter ») → `ignored`.
- **Classification** : moteur (backdrop modal, polices d'icônes, bordures d'input, hit-test).
- **Sévérité** : moyenne-élevée — lisible mais voilée et non interactive.

## Constats actionnables

1. **[Critique] Hit-test : tous les clics `ignored`, tous les hovers `editable:false`** sur 4 sites réels (champs `<input>`, `<a>`, `<button>`). Le hit-test transform semble avoir régressé ou ne trouve jamais de cible interactive — aucune navigation par clic n'a été possible pendant tout le vol. À reproduire : `POST /input {"type":"click",…}` sur un lien réel.
2. **[Critique] Paint au-delà du viewport initial absent (Glassdoor)** : layout complet jusqu'à y=5520, frame intégralement blanche à scroll_y=1600. Display list ou clip raster borné à la fenêtre initiale.
3. **[Élevé] SVG sans intrinsic size mis à l'échelle sans borne (Indeed)** : illustrations/icônes peintes en géant pixelisé par-dessus le contenu.
4. **[Élevé] Layout du premier écran éclaté (WTTJ)** : h1 en taille démesurée multi-lignes, nav superposée — `clamp()`/`min()`/typo fluide ou grid probablement ignorés.
5. **[Moyen] Backdrop modal permanent (francetravail)** : voile semi-transparent peint sur tout le document, y compris après scroll — élément de consentement jamais masqué (ou z-order/mask mal résolu).
6. **[Moyen] Icônes en boîtes tofu (francetravail, Indeed)** : glyphes d'icônes (chevrons, puces) non résolus — fallback police d'icônes/PUA.
7. **[Moyen] Bordures/boîtes des `<input>` invisibles (francetravail, partiellement Indeed OK)** — les champs n'ont que leur label.
8. **[Faible] Bouton cookie Indeed rogné hors viewport** — « Autoriser tous les co… » déborde à droite (overflow du bandeau).
9. **[Info] Absence de `<input>` dans le SSR (WTTJ, Glassdoor)** : barres de recherche injectées par JS — attendu ; souligne que le click-to-focus n'a pas pu être testé sur un vrai champ éditable non-React.

## Fixes confirmés

- Reset console à la nav (mesuré en direct sur l'onglet 4).
- Ellipsis (`…`) sur cartes réelles francetravail.
- Disjoncteur : aucun figeage, 5 navigations propres.
- Page d'erreur HTTP native (403) — dégradation gracieuse correcte.

## Arrêt

Onglets DELETE (1-4), daemon PID 10340 tué par PID. Aucun code modifié.

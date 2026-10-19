# ADR-0014 — Champs de formulaire rendus par le moteur

Statut : Acceptée (2026-10-19).

## Contexte

L'étape 38 (`<form>`, GET/POST, « champs natifs rendus moteur
ou délégués UI ») impose de choisir qui dessine et qui fait
vivre les contrôles de formulaire : le moteur (layout →
display list → raster) ou l'app Flutter (overlay natif par-
dessus la frame PNG).

Contraintes :

1. **Une frame = un état cohérent** (AGENTS.md §8) : un champ
   peint par l'UI devrait suivre le scroll/zoom du PNG servi
   — synchronisation fragile entre deux plans de rendu.
2. **Le contrat REST/SSE/PNG reste la couture unique**
   (ADR-0003) : déléguer les champs à l'UI exigerait un
   protocole de géométrie/focus additionnel (hit regions par
   champ, IME, sélection…) — beaucoup de surface pour un
   palier MVP.
3. Le DOM est déjà mutable côté core (`set_attribute`,
   `set_text` — étape 31) : la valeur vit dans le document,
   le moteur peut la peindre.

## Décision

**Les champs natifs sont rendus par le moteur** ; la
délégation UI est limitée aux interactions qu'elle fait déjà
(coordonnées de clic, clavier, molette).

- **Peinture** : `input`/`select` produisent une boîte
  atomique avec un enfant `Text` *synthétique* (valeur
  `value`, `placeholder` en repli, libellé `value` des
  boutons, texte de l'option `selected`) — le texte passe
  par le pipeline shaping/lignes existant, sans nouvel item
  d'affichage. `checkbox`/`radio` peignent une marque pleine
  centrée (`LayoutBox.control_checked` → `FillRect` à ~55 %
  du plus petit côté). `<textarea>` peint ses enfants texte
  comme avant.
- **Soumission** : clic sur `<button>` (hors
  `type=button|reset`) ou `<input type=submit|image>` dans un
  `<form>`, ou `Enter` dans un `<input>` focalisé →
  événement `submit` dispatché dans le miroir JS
  (`preventDefault` annule), collecte des contrôles
  *successful* (nommés, non `disabled`, cases cochées
  seulement, bouton = le submitter seul), `GET` → query sur
  l'`action` résolue, `POST` → corps
  `application/x-www-form-urlencoded`. La réponse emprunte le
  pipeline de navigation complet (`navigate_request`) et
  pousse l'URL d'action dans l'historique.
- **Interactions** : clic sur `checkbox`/`radio` bascule
  `checked` (exclusivité `radio` par `name` dans le
  formulaire), clic sur `<select>` **cycle** ses options,
  `label[for]` délègue au contrôle nommé.

## Conséquences

Acceptées (déviations MVP documentées) :

- ~~Le sélecteur d'options natif (popup `<select>`) est remplacé
  par le cycle au clic~~ — **levée par la révision étape 54**
  ci-dessous.
- `POST` dans l'historique retient l'URL sans le corps :
  `reload`/`back`/`forward` refont un `GET` (pas de
  re-soumission, ni d'avertissement).
- ~~`enctype` : seul `application/x-www-form-urlencoded` est
  émis — `multipart/form-data` (fichiers) n'est pas tenté ;
  `input[type=file]` n'est ni rendu ni successful~~ — levée
  partiellement par la révision étape 54 (encodage et entrée
  `File` implémentés, le choix de fichier réel reste absent).
- `form=` (association hors-arbre), `<fieldset disabled>`
  hérité et `<object>` ne sont pas traités par la collecte.
- `input[type=password]` peint sa valeur en clair (la fonte
  fixture rend tous les glyphes identiques — masquage à
  revoir avec de vraies fontes).
- ~~Les événements clavier (`keydown`/`input`/`change`) ne sont
  pas dispatchés au JS~~ — levée à l'étape 52.

## Révision — étape 54 (2026-10-19)

Le palier « formulaires v2 » remplace le cycle `<select>` et
ajoute la validation de contraintes, `multipart` et la
soumission script :

- **Popup `<select>` moteur** : clic → overlay peint
  chrome-side en fin de display list (`FillRect` + `Border` +
  lignes `Placeholder`), au-dessus de tout contenu — la même
  approche « rendu moteur » que la décision initiale, la
  listbox n'est qu'un overlay de la frame (aucun nouveau
  contrat REST/Flutter). L'état vit hors DOM dans
  `Tab.popup` (`SelectPopup`) : options figées à
  l'ouverture, `<select>` retenu par **chemin** `(tag,index)`
  (re-résolution au choix — le nœud peut mourir d'un
  re-parse de synchro ; un re-parse referme le popup).
  `ArrowDown`/`ArrowUp` surlignent, `Enter` choisit,
  `Escape` referme, clic hors popup referme (clic consommé).
  Bornée à 8 lignes visibles (pas de défilement interne).
- **`multipart/form-data`** (RFC 7578) : `enctype` honoré en
  `POST` ; `input[type=file]` devient *successful* et émet
  une partie `filename=` vide — **pas de sélecteur de
  fichier réel** (aucune surface UI pour choisir un fichier ;
  le hook de test pose le nom via `value=`).
- **Validation de contraintes** (HTML §4.10.21.2) avant tout
  submit (clic, `Enter` implicite avec bouton par défaut
  comme `submitter`) : `required`, `minlength`, `maxlength`,
  `pattern` (correspondance entière), `min`/`max`/`step`
  numériques. Contrôles « barred » exclus (`disabled`,
  `hidden`, boutons, `file`). `invalid` (non-annulable) part
  sur chaque fautif ; la soumission est bloquée.
  `novalidate`/`formnovalidate` sautent la validation.
- **API JS** : `validity` (drapeaux), `validationMessage`,
  `willValidate`, `checkValidity`/`reportValidity`,
  `setCustomValidity`, `form.submit()`/`requestSubmit()`.
  Le moteur JS est autonome — les règles sont **réimplémentées
  côté miroir** (même logique, doublon assumé et documenté).
  `setCustomValidity` transite miroir→DOM réel par l'attribut
  `data-explorer-cv` via la synchro de sérialisation (visible
  dans `outerHTML` — canal minimal, pas de nouvelle couture).
  `form.submit()`/`requestSubmit()` émettent
  `NavRequest::FormSubmit` (champs collectés miroir), drainé
  par la session vers `navigate_request`.

Déviations restantes (étape 54) :

- `:valid`/`:invalid`/`:user-invalid` côté CSS — le moteur de
  sélecteurs relève de PIPELINE (besoin consigné dans
  `docs/interface-equipes.md`).
- `invalid` n'est pas non-bullant comme le veut la spec (le
  dispatcher miroir propage toujours).
- `reportValidity` n'affiche pas de bulle native — identique
  à `checkValidity` (pas de surface UI).
- `pattern` via `regex` côté moteur et `fancy-regex` côté
  miroir : constructs JS `v`-flag non couverts → un pattern
  non compilable ne contraint pas (conforme HTML).
- `option.selected = true` côté script ne désélectionne pas
  les sœurs (single-select non forcé par le setter).
- La ligne de choix `value=` du `<select>` affiché reste le
  rendu boxtree (le popup ne re-peint pas le contrôle avec
  la sélection courante — le relayout le fait).
- `Enter` dans un champ valide soumet ; `input[type=date|email|url]`
  n'ont pas de sémantique de valeur dédiée (typeMismatch,
  badInput = `false` — MVP).

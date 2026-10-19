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

- Le sélecteur d'options natif (popup `<select>`) est remplacé
  par le cycle au clic — une vraie listbox déléguée UI reste
  une évolution possible derrière le même `InputEvent`.
- `POST` dans l'historique retient l'URL sans le corps :
  `reload`/`back`/`forward` refont un `GET` (pas de
  re-soumission, ni d'avertissement).
- `enctype` : seul `application/x-www-form-urlencoded` est
  émis — `multipart/form-data` (fichiers) n'est pas tenté ;
  `input[type=file]` n'est ni rendu ni successful.
- `form=` (association hors-arbre), `<fieldset disabled>`
  hérité et `<object>` ne sont pas traités par la collecte.
- `input[type=password]` peint sa valeur en clair (la fonte
  fixture rend tous les glyphes identiques — masquage à
  revoir avec de vraies fontes).
- Les événements clavier (`keydown`/`input`/`change`) ne sont
  pas dispatchés au JS à cette étape — `click`/`submit`
  seulement.

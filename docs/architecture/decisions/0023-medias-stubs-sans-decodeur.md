# ADR-0023 — `<video>`/`<audio>` : stubs honnêtes + affiche, pas de décodeur

Statut : Acceptée (2026-11-07) — arbitrage exigé par
l'étape 58 (roadmap NAVIGATEUR).

## Contexte

L'étape 58 demandait un arbitrage explicite : soit un
décodage audio/vidéo **pur Rust** (règle §2 — aucun crate
liant du C ne peut entrer dans le workspace), soit des
stubs propres avec affiche.

L'état de l'art est sans ambiguïté :

- **Vidéo** — aucun décodeur H.264/HEVC/VP9/AV1 complet en
  pur Rust. `openh264` lie la lib C Cisco ; `dav1d` est du
  C/assembleur ; les projets purement Rust (`rav1d` porte
  dav1d, encore instable ; `h264-rs`/`vp9-rs` incomplets)
  ne couvrent ni la conformité bitstream ni la robustesse
  attendue d'un navigateur face à du contenu hostile.
- **Audio** — `symphonia` est le seul candidat viable
  (pur Rust, MP3/AAC/Vorbis/FLAC), mais sortir du PCM
  décodé exige une chaîne de lecture (resampling, clock,
  sortie OS) qui n'existe pas : `cpal` est la brique
  standard et elle lie l'audio système — hors règle §2
  pour un simple stub, et la mission n'a pas encore de
  sortie audio du tout.
- Un décodeur from-scratch (écrire H.264 dans ce dépôt)
  est un projet en soi, mesuré en années-homme — pas une
  étape de palier.

## Décision

Pas de décodage. Les éléments médias deviennent des
citoyens de première classe **visuels et scriptables** :

1. **Affiche** — `video[poster]` suit le canal image
   existant (`collect_images` étendu, fetch borné par
   `max_images`, décodage `decode_image`, bitmap remplacé
   peint dans la content-box — le mécanisme était déjà
   générique par `NodeId`). Les affiches `data:` passent
   par `decode_data_url` en complément du collecteur PIPE.
2. **Boîtes de repli** — feuille UA complémentaire posée
   à l'origine `UserAgent` depuis `explorer-core`
   (`MEDIA_UA_CSS` dans `session::apply`) : `video` =
   inline-block 300×150 fond noir, `audio` masqué sans
   `controls` (comme les UA réelles), `audio[controls]` =
   barre 300×54. L'auteur l'emporte toujours — aucun
   comportement n'est forcé.
3. **Surface JS honnête** — stubs `HTMLMediaElement`
   dans `dillo-js` : `play()` rejette `NotSupportedError`
   (même chemin `.catch()` que le blocage autoplay, déjà
   géré par les sites), `canPlayType` répond `""` pour
   tout type, `paused`/`duration`/`readyState`/
   `networkState` reflètent « rien à lire » (NO_SOURCE si
   `src` présent), `buffered`/`played`/`seekable` sont des
   `TimeRanges` vides, les écritures IDL (`volume`,
   `currentTime`…) sont acceptées-ignorées plutôt que
   fautives. Jamais de faux succès : rien ne prétend
   jouer.

## Conséquences

- Les héros vidéo des sites réels ne cassent plus leurs
  scripts (`video.play().catch(…)`, `canPlayType` testé
  avant sélection de source) — bénéfice web réel immédiat,
  zéro codec.
- Le repli reste honnête : `canPlayType("")` + promesse
  rejetée signalent aux pages qu'aucune lecture n'aura
  lieu — leurs fallbacks (image, GIF, message) s'activent.
- Réouverture possible : si `rav1d`/`symphonia` + une
  couture de sortie pure Rust deviennent viables, la
  décision se revisite — le contrat ci-dessus (affiche,
  boîtes, surface JS) reste valable en surcouche.
- Demande PIPE consignée dans
  `docs/interface-equipes.md` : faire de `video`/`audio`
  de vrais remplacés dans `boxtree` (taille intrinsèque
  du poster au lieu du 300×150 forcé) et couvrir
  `video[poster]` dans `collect_data_images` — refinements
  non bloquants, le surcoût NAV du v1 étant minime.

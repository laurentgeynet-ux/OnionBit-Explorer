# ADR-0018 : Mode in-process `explorer-ffi` — frames RGBA brutes via `dart:ffi`

- **Statut** : accepté
- **Date** : 2026-02
- **Contexte** : étape 47 — « texture externe Flutter via FFI ».
  ADR-0004 documentait la voie d'upgrade du pull PNG et sa
  tension avec l'isolation de processus ADR-0003 (une texture
  GPU partagée peut exiger le moteur dans le processus UI).
  Cette ADR tranche : **les deux modes coexistent**, l'isolation
  daemon/REST reste le mode référence, le mode in-process est
  une option de performance activable par configuration.

## Décision

1. **`explorer-ffi` — nouveau crate `cdylib`** produisant
   `explorer_ffi.dll`/`.so`/`.dylib`. Il embarque une
   `Session` complète (pile réseau identique au démon :
   TLS + ALPN h1/h2 + décodage + redirections bornées,
   fabrique JS `DilloEngine`, `DirFontSource`) derrière une
   ABI C volontairement étroite : `create`/`destroy`,
   `open_tab`/`close_tab`, `navigate`/`reload`/`go_back`/
   `go_forward`, `set_viewport`, `input`, `tick`,
   `frame_size`/`frame_copy` (RGBA brut), `tab_url`,
   `last_error` (diagnostic thread-local). Runtime tokio
   privé `current_thread` par handle — les appels FFI
   bloquants de l'UI restent hors du runtime async.
2. **Frames : RGBA brut copié une fois** (`frame_copy` dans un
   buffer alloué par l'appelant), puis
   `ui.decodeImageFromPixels` côté Dart → `ui.Image` affichée
   par `RawImage`. Zéro encodage PNG, zéro socket, zéro
   sérialisation. Une vraie texture GPU partagée (plugin
   natif `PixelBufferTexture`) reste une voie d'upgrade
   ultérieure — non retenue car elle exige un plugin C++
   par plateforme pour un gain marginal à ce stade.
3. **`dart:ffi` + `FfiEngine implements BrowserRepository`** :
   l'UI reste découplée du moteur (règle ADR-0003 préservée
   *côté code* : aucune dépendance Flutter→core, seulement
   l'ABI). `tryLoad` échoue en `null` → repli REST
   transparent ; le mode se choisit par
   `EXPLORER_ENGINE=auto|rest|ffi` et
   `EXPLORER_ENGINE_LIB=<chemin>`.
4. **Pas de canal d'événements** : le SSE
   `frame_invalidated` n'existe pas in-process. Le contrôleur
   sonde `explorer_tick` toutes les 50 ms (animations) et
   tire la frame au retour de chaque action
   (navigate/input) — le pull est le même, seul le signal
   change.
5. **Dérogation `unsafe` unique et bornée.** La règle
   `#![forbid(unsafe_code)]` ne peut pas s'appliquer à une
   frontière C. `explorer-ffi` est le **seul** crate du
   workspace autorisé à contenir `unsafe` (lint recopié en
   `allow` dans son manifeste, le reste des lints inchangé) :
   les blocs sont confinés à la validation des pointeurs
   d'entrée (null/longueur) et aux copies bornées
   (`copy_nonoverlapping` plafonné par `cap`). Aucun `unsafe`
   ne touche la logique moteur ; tout point de sortie écrit
   uniquement dans un buffer dont la capacité a été vérifiée.

## Alternatives écartées

- **Mémoire partagée + daemon** (`CreateFileMapping`/mmap) :
  conserve l'isolation mais exige une glue native de
  mappage côté Flutter (plugin) et une synchronisation
  producteur/consommateur — complexité supérieure au mode
  in-process pour un gain identique (une copie dans les
  deux cas).
- **`ExternalTexture`/`flutter_rust_bridge` GPU** : zéro-copie
  réel mais impose un plugin C++ par plateforme et, sur
  Windows, la même contrainte processus — reporté.
- **PNG ré-encodé côté Dart** pour tout le monde : gardé
  uniquement comme implémentation de `fetchFrame` (contrat
  d'interface), jamais sur le chemin d'affichage rapide.

## Conséquences

- **Isolation dégradée en mode FFI** : un crash moteur
  (incluant un bug dans ses dépendances `unsafe`) emporte
  l'UI ; un exploit de contenu hostile atteint le processus
  UI directement. Le mode `rest` reste le défaut recommandé
  pour la robustesse ; `ffi` est le mode « framerate ».
- **Déviations assumées** : pas de titre d'onglet ni d'URL de
  survol exposés (surface ABI volontairement minimale — la
  barre de statut FFI est silencieuse), pas d'événements SSE
  (polling `tick` 50 ms), `list_tabs` simulée côté client.
- **Testabilité préservée** : les tests Rust appellent les
  fonctions `extern "C"` en-process (pipeline navigate →
  RGBA vérifié pixel à pixel) ; le test Dart charge la vraie
  DLL quand elle est buildée (skip silencieux sinon).
- **Déploiement** : la DLL se place à côté de l'exécutable
  Flutter, ou via `EXPLORER_ENGINE_LIB`, ou en dev dans
  `target/{debug,release}` — détectée automatiquement.

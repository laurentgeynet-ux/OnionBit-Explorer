# ADR-0004 — Transport des frames : PNG tiré sur invalidation SSE

Statut : Acceptée (2026-10-18), voies d'upgrade documentées.

## Contexte

Le moteur produit des framebuffers ; l'UI Flutter doit les afficher.
Transport possible : pull PNG, stream binaire WebSocket, texture
partagée FFI, ou reparenting (écarté par ADR-0003).

## Décision

**MVP : pull PNG.**

- Le moteur rasterise en framebuffer RGBA
  (`explorer-render`, tiny-skia à terme).
- `GET /api/events` pousse `frame_invalidated{tab}` en SSE.
- L'UI tire `GET /api/tabs/{id}/frame` → `image/png`, affichée via
  `Image.memory` ; le redimensionnement UI →
  `POST /api/tabs/{id}/viewport` puis nouvelle invalidation.
- Entrées : `POST /api/tabs/{id}/input` (JSON : type, coords,
  touches) → hit-test dans `explorer-layout`.

**Voies d'upgrade planifiées** :
- **Phase 6/7 (Consolidation UI/API)** : canal direct de pixels bruts
  RGBA/BGRA décompressés via **mémoire partagée** (`CreateFileMapping` /
  `mmap` anonyme) ou **Named Pipe local** (`\\.\pipe\...` sous Windows),
  notifié par l'événement SSE avec les rectangles invalidés (*dirty rects*).
  Cette étape élimine la double surcharge CPU encode/decode PNG tout en
  préservant l'isolation stricte des processus.
- **Phase 10 (Rendu temps réel)** : texture externe Flutter via FFI
  (`ExternalTexture` / `flutter_rust_bridge`) si le framerate élevé (60 FPS,
  canvas/vidéos) exige un pipeline GPU zéro-copie. Toute extension de canal
  est adossée à un ADR dédié (AGENTS.md §7).

## Justification

- Simplicité radicale au MVP : pas de stream binaire complexe, le contrat
  REST existant suffit pour afficher les premières pages.
- Le PNG est décodable par Flutter out-of-the-box, téléchargeable,
  inspectable et testable (golden images).
- L'anticipation d'un transport en mémoire partagée / pixels bruts en
  phase 6/7 garantit que le scrolling et les interactions (hover, frappe)
  ne resteront pas pénalisés par la compression d'image.

## Conséquences

- MVP assumé : scrolling/animations initialement bridés en pull PNG,
  remplacés dès la phase 6/7 par le buffer brut partagé.
- Réduction drastique de la charge CPU lors du passage en mémoire
  partagée : aucun encodage PNG nécessaire pour l'affichage interactif.
- `EventSource` sans en-têtes custom → la clé d'API accepte `?key=`
  (déjà tranché en ADR-0003, compromis y documenté).
- **Mémoire partagée côté Flutter** : Dart ne peut pas mapper le
  segment d'un processus tiers sans glue native — la voie phase 6/7
  exige un mini-plugin FFI (ou `package:win32`/`mmap`) qui copie le
  segment vers une `Texture`. Ce plugin ne fait que transférer des
  pixels : ce n'est pas du code moteur, la frontière ADR-0003 tient.
- **Tension documentée avec l'étape 47** : une `ExternalTexture`
  GPU performante peut exiger que le moteur vive dans le processus
  UI — en conflit frontal avec l'isolation d'ADR-0003. Options à
  trancher par l'ADR de phase 10 : mémoire partagée + copie unique
  (compromis par défaut), mode in-process optionnel dégradant
  l'isolation, ou surfaces GPU partagées si l'écosystème le permet.
  Le zéro-copie cross-process n'est pas promis tant que l'ADR n'est
  pas tranché.


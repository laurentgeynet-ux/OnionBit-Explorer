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

**Voies d'upgrade (non arbitrées)** : WebSocket binaire pour les
frames (moins de latence), texture FFI si le pull devient le
goulot, diff-tiles (envoyer les rects modifiés). Toute extension de
canal passe par un ADR — AGENTS.md §7.

## Justification

- Simplicité radicale : pas de stream binaire, pas de codec vidéo,
  pas de shared memory — le contrat REST existant suffit.
- Le navigateur cible est documentaire : pages essentiellement
  statiques ; 1 frame PNG par invalidation est largement
  suffisante.
- Le PNG est décodable par Flutter out-of-the-box, debuggable
  (téléchargeable), testable (golden images) — un format opaque
  n'apporterait rien au MVP.

## Conséquences

- Scrolling/animations dans la page seront saccadés tant qu'on reste
  en pull-full-frame : connu, assumé, réversible par ADR d'upgrade.
- Coût encode PNG à chaque invalidation : mitigable par dirty-rects
  dans `explorer-render` sans changer le contrat.
- `EventSource` sans en-têtes custom → la clé d'API accepte `?key=`
  (déjà tranché en ADR-0003).

# ADR-0003 — Couture UI : API REST + SSE sur loopback (modèle OnionBit)

Statut : Acceptée (2026-10-18).

## Contexte

Comment l'app Flutter parle-t-elle au moteur Rust ? Options pesées :

1. **Reparenting de fenêtre native** (modèle du shell DilloRust,
   ADR-0006 de ce projet : winit + `SetParent` Win32) — zéro couplage
   mais rendu et chrome irrémédiablement séparés, hacks Windows/X11.
2. **FFI / flutter_rust_bridge** — couplage fort, rendu en texture
   Flutter (perf), mais le moteur vit dans le processus UI et tout
   crash touche l'app.
3. **API de contrôle locale REST + SSE** — le modèle du repo OnionBit
   (`app/` ne consomme que `onionbit-api`, jamais le core).
4. IPC ad hoc (socket/pipe avec protocole maison).

## Décision

Option 3 : `explorer-daemon` expose une API `axum` sur `127.0.0.1` ;
l'app Flutter est un client HTTP/SSE pur.

- REST pour les actions (`/tabs`, `navigate`, `input`).
- SSE `GET /api/events` pour le push (`tab_state`,
  `frame_invalidated`, `title_changed`).
- PNG pour les frames (ADR-0004).
- Erreurs `{"error": {handled, message}}`.
- Bind loopback seul ; clé d'API optionnelle (`x-api-key` ou
  `?key=` — `EventSource` ne pose pas d'en-têtes).
- L'app ne compile/contient **aucun** code moteur.

## Justification

- Réutilise une architecture éprouvée dans l'écosystème de
  l'utilisateur (même modèle mental, mêmes conventions, coût
  cognitif minimal).
- Sépare les cycles de vie : le daemon peut être redémarré, testé,
  débogué sans l'UI — et réciproquement.
- Testable sans Flutter : `curl`/`explorer-cli` suffisent.
- Un crash moteur ne tue pas l'app ; un contenu hostile reste
  confiné au processus daemon (future sandbox possible).

## Conséquences

- Latence frame : le rendu traversé par HTTP n'est pas un GPU
  pipeline — acceptable pour un navigateur document (mitigations en
  ADR-0004).
- Deux processus à orchestrer : l'app supervise `explorer-daemon`
  (spawn, drain stderr, kill propre — leçon du shell DilloRust :
  un stderr non drainé bloque l'enfant par deadlock de pipe).
- La sécurité repose sur loopback + clé optionnelle ; tout bind
  non-loopback est interdit par défaut.

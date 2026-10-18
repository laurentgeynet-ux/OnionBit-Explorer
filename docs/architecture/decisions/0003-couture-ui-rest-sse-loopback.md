# ADR-0003 — Couture UI : API REST + SSE sur loopback

Statut : Acceptée (2026-10-18), justifications réécrites après le
retrait des liens avec l'écosystème historique de l'auteur.

## Contexte

Comment l'app Flutter parle-t-elle au moteur Rust ? Options pesées :

1. **Reparenting de fenêtre native** (`SetParent` Win32 / embed
   X11 — piste déjà éprouvée sur un chantier voisin de l'auteur) —
   zéro couplage mais rendu et chrome irrémédiablement séparés,
   hacks spécifiques par OS.
2. **FFI / flutter_rust_bridge** — couplage fort, rendu en texture
   Flutter (perf), mais le moteur vit dans le processus UI et tout
   crash touche l'app.
3. **API de contrôle locale REST + SSE** — daemon séparé, l'app est
   un client HTTP pur.
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

- Sépare les cycles de vie : le daemon peut être redémarré, testé,
  débogué sans l'UI — et réciproquement.
- Testable sans Flutter : `curl`/`explorer-cli` suffisent.
- Un crash moteur ne tue pas l'app ; un contenu hostile reste
  confiné au processus daemon (future sandbox possible).
- Un protocole HTTP documenté est consommable par n'importe quel
  client futur (TUI, autre UI, scripts) — l'investissement du
  contrat n'est pas perdu si le frontend change.

## Conséquences

- Latence frame : le rendu traversé par HTTP n'est pas un GPU
  pipeline — acceptable pour un navigateur document (mitigations en
  ADR-0004).
- Deux processus à orchestrer : l'app supervise `explorer-daemon`
  (spawn, drain stderr, kill propre — écueil connu : un stderr non
  drainé bloque l'enfant par deadlock de pipe).
- La sécurité repose sur loopback + clé optionnelle ; tout bind
  non-loopback est interdit par défaut.
- **`?key=` est un compromis documenté** : `EventSource` ne pose
  pas d'en-têtes, donc la clé transite par l'URL — elle peut
  apparaître dans des logs/historiques. Acceptable sur loopback
  pour le MVP ; le header `x-api-key` reste le chemin privilégié
  pour REST, et toute exposition non-loopback future imposera un
  vrai mécanisme d'auth (ADR dédié).
- La latence d'entrée : chaque clic, frappe ou mouvement fait un
  round-trip HTTP + hit-test layout. Sur loopback c'est de l'ordre
  de la milliseconde — acceptable, mais le contrat prévoit le
  **coalescing** des événements mouvement côté client, et le canal
  direct d'ADR-0004 pourra véhiculer l'input si le ressenti
  l'exige.

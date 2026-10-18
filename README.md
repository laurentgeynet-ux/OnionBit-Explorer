# OnionBit-Explorer

Navigateur web écrit **from scratch** : moteur 100 % Rust (réseau,
HTML, CSS, layout, rendu, JS) et interface graphique **Flutter**
desktop, reliées par une API de contrôle locale **REST + SSE**
(même modèle que l'écosystème OnionBit — l'app ne touche jamais le
moteur directement).

> État : **phase 0 — cadrage documentaire**. Ce dépôt contient
> pour l'instant les documents de pilotage ; le code arrivera avec
> la phase 1 (moteur d'abord, UI Flutter en parallèle dès que l'API
> minimal existe). Voir `ROADMAP.md`.

## Documents de pilotage

| Fichier | Rôle |
|---|---|
| `AGENTS.md` | Contrat de travail (règles, conventions, workflow) |
| `ROADMAP.md` | Source de vérité de l'avancement, par phases |
| `CHANGELOG.md` | Historique Keep a Changelog |
| `docs/architecture.md` | Cartographie des couches et crates cibles |
| `docs/architecture/decisions/` | ADR numérotés |

## Licence

Double licence **MIT OR Apache-2.0** (ADR-0006) — `LICENSE-MIT` +
`LICENSE-APACHE`.

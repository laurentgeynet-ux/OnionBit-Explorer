# Explorer

Navigateur web écrit **from scratch** : moteur 100 % Rust (réseau,
HTML, CSS, layout, rendu, JS) et interface graphique **Flutter**
desktop, reliées par une API de contrôle locale **REST + SSE**
(l'app ne touche jamais le moteur directement).

> « Explorer » est un nom de travail — le dépôt physique
> (`C:\OnionBit-Explorer`) sera renommé quand l'identité finale sera
> choisie. Les crates `explorer-*` ne changent pas.
>
> État : **phase 0/1 — workspace bootstrappé**. Les 13 crates
> `explorer-*` squelettes compilent et `scripts/verify_all.ps1`
> orchestre les vérifications. Le moteur JavaScript **`dillo-js`**
> (crate autonome de ce workspace, chantier de l'équipe JS dédiée)
> est déjà présent et testé ; son intégration derrière le trait
> `JsEngine` est prévue en phase 8. Voir `ROADMAP.md`.

## Documents de pilotage

| Fichier | Rôle |
|---|---|
| `AGENTS.md` | Contrat de travail (règles, conventions, workflow) |
| `ROADMAP.md` | Source de vérité de l'avancement, par phases |
| `CHANGELOG.md` | Historique Keep a Changelog |
| `docs/architecture.md` | Cartographie des couches et crates cibles |
| `docs/architecture/decisions/` | ADR numérotés |

## Périmètre

Le cap : compatibilité web moderne en **direction asymptotique**
(ADR-0001), mesurée par paliers sur corpus publics —
html5lib-tests, WPT, test262 (ADR-0010).

Exclusions assumées (documentées, non planifiées — ADR dédié si un
besoin réel émerge) : `<iframe>`, shadow DOM, accessibilité native,
téléchargements, zoom page, recherche en page, impression,
extensions, SVG animé (SMIL), isolation multi-processus par site
(ADR-0011).

## Licence

Double licence **MIT OR Apache-2.0** (ADR-0006) — `LICENSE-MIT` +
`LICENSE-APACHE`.

# Explorer

Navigateur web écrit **from scratch** : moteur 100 % Rust (réseau,
HTML, CSS, layout, rendu, JS) et interface graphique **Flutter**
desktop, reliées par une API de contrôle locale **REST + SSE**
(l'app ne touche jamais le moteur directement).

> « Explorer » est un nom de travail — le dépôt physique
> (`C:\OnionBit-Explorer`) sera renommé quand l'identité finale sera
> choisie. Les crates `explorer-*` ne changent pas.
>
> État : **release 0.2.0 livrée** — vidéo native via
> `MediaSurface`, handlers inline `on*`, daemon auto-réparant,
> smoke E2E étendu (liens, scroll, média, find).
> Travail organisé en deux équipes : **PIPELINE** (domaine pur :
> `docs/roadmap-equipe-pipeline.md`) et **NAVIGATEUR**
> (plateforme produit + JS : `docs/roadmap-equipe-navigateur.md`).
> État détaillé : `docs/etat-des-lieux-2026-10-19.md` —
> `ROADMAP.md` pour l'index maître.

## Documents de pilotage

| Fichier | Rôle |
|---|---|
| `AGENTS.md` | Contrat de travail (règles, conventions, workflow) |
| `ROADMAP.md` | Index maître — renvoie vers les roadmaps d'équipe |
| `docs/roadmap-equipe-*.md` | Files de travail PIPELINE / NAVIGATEUR |
| `docs/interface-equipes.md` | Journal des coutures inter-équipes |
| `docs/etat-des-lieux-*.md` | Audit daté de l'avancement mesuré |
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

## Build & vérification

```powershell
pwsh -File scripts/verify_all.ps1   # fmt, clippy, tests, doc, flutter
pwsh -File scripts/smoke.ps1        # test de fumée E2E loopback
pwsh -File scripts/package.ps1      # bundle release dist/explorer-0.2.0
cargo bench -p explorer-core        # benchs pipeline (criterion)
cargo fuzz run tokenizer            # fuzz HTML (aussi : css, url)
```

Le daemon (`explorer-daemon`) expose l'API REST+SSE sur
`127.0.0.1:9377` — l'app Flutter s'y connecte (ADR-0003).

Le dossier `target/` est déporté sur `D:` via `.cargo/config.toml`
(commité — postes de dev sur la même machine). Sur une machine
sans `D:` ou en CI, définir `CARGO_TARGET_DIR` — la variable
d'environnement a priorité sur la config.

## Licence

Double licence **MIT OR Apache-2.0** (ADR-0006) — `LICENSE-MIT` +
`LICENSE-APACHE`.

# ADR-0001 — Moteur web écrit from scratch, périmètre des dépendances

Statut : Acceptée (2026-10-18).

## Contexte

OnionBit-Explorer se donne pour mission un navigateur dont le moteur
est entièrement écrit dans ce dépôt — lisibilité, maîtrise totale du
pipeline, cohérence avec l'écosystème OnionBit. La question posée :
où tracer la frontière entre « moteur écrit ici » et « réutilisation
légitime de crates » ?

## Décision

- **Écrits ici** : URL→fetch, client HTTP, tokenizers et parsers
  (HTML, CSS), DOM, cascade, layout, display list, orchestration.
  C'est le moteur — sa valeur pédagogique et de contrôle est le
  projet lui-même.
- **Réutilisés** (infrastructure pur Rust) : runtime (`tokio`), TLS
  (`rustls`/`webpki-roots`), parsing wire-format borné (`httparse`),
  décodage transfert (`flate2`/équivalent), rasterisation
  (`tiny-skia`, ADR-0005), fontes (crate pur Rust à arbitrer en
  phase 4), décodeurs d'images (phase 9), framework API (`axum`).
- **Exclus** : tout moteur ou sous-système de navigateur existant
  (`html5ever`, `servo`/`stylo`, `kuchiki`, `reqwest` côté moteur…).
  Un crate qui lie du C (`*-sys` sans feature pure Rust) est interdit
  par défaut — exceptions uniquement par ADR.

## Justification

- Le but est un moteur maîtrisé de bout en bout : importer html5ever
  viderait le projet de sa substance.
- TLS, compression et rasterisation sont des commodités de sécurité/
  performance sans valeur différentiante — les réécrire ajouterait
  du risque, pas de la compréhension.
- `#![forbid(unsafe_code)]` dans tous nos crates garde la garantie
  mémoire côté code maison.

## Conséquences

- La pile réseau reste modeste au début : HTTP/1.1 (ADR-0002), HTTP/2
  et QUIC sont explicitement hors scope initial.
- Le web « moderne complet » n'est pas la cible : HTML5/CSS2.1+,
  ES3-subset via dillo-js (ADR-0007) — les pages app le web resteront
  partiellement rendues ; tolérance et dégradation sont la spec.
- Chaque dépendance nouvelle exige justification ADR ou commit
  (AGENTS.md §4.2).

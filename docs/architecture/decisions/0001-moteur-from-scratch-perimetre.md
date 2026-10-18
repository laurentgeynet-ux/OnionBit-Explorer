# ADR-0001 — Moteur web écrit from scratch, périmètre des dépendances

Statut : Acceptée (2026-10-18).

## Contexte

Explorer se donne pour mission un navigateur dont le moteur
est entièrement écrit dans ce dépôt — lisibilité, maîtrise totale du
pipeline. La question posée :
où tracer la frontière entre « moteur écrit ici » et « réutilisation
légitime de crates » ?

## Décision

- **Écrits ici** : URL→fetch, client HTTP, tokenizers et parsers
  (HTML, CSS), DOM, cascade, layout, display list, orchestration.
  C'est le moteur — sa valeur pédagogique et de contrôle est le
  projet lui-même.
- **Réutilisés** (infrastructure pur Rust) : runtime (`tokio`), TLS
  (`rustls`/`webpki-roots`), parsing wire-format borné (`httparse`),
  décodage transfert (`flate2` backend `miniz_oxide`), rasterisation
  (`tiny-skia` — présente décision), fontes (crates pur Rust
  arbitrées par ADR-0009), décodeurs d'images (phase 9), framework
  API (`axum`).
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

- La pile réseau démarre modeste : HTTP/1.1 (ADR-0002) puis HTTP/2
  et QUIC rejoignent la roadmap — le web réel en dépend de plus en
  plus.
- **Le cap est la compatibilité avec le web moderne — comme
  direction, pas comme livrable.** HTML5, CSS complet (flexbox,
  grid…), ES6+ (ADR-0007) restent des objectifs de roadmap, pas des
  exclusions ; mais « tous les sites » est une cible asymptotique —
  aucun navigateur hors des trois grands moteurs n'y parvient
  (Servo, Ladybird inclus, avec des équipes dédiées). La
  progression est donc pilotée par les paliers de la roadmap,
  rendus **mesurables** par les corpus de conformité (ADR-0010).
  Tolérance et dégradation gracieuse restent la spec de
  comportement sur contenu malformé, jamais un verdict sur la
  couverture.
- Chaque dépendance nouvelle exige justification ADR ou commit
  (AGENTS.md §4.2).

# ADR-0006 — Double licence MIT OR Apache-2.0

Statut : Acceptée (2026-10-18).

## Contexte

Licence « à définir » au cahier des charges, arbitrée par
l'utilisateur : **MIT OR Apache-2.0**.

## Décision

Double licence permissive `MIT OR Apache-2.0` :

- `LICENSE-MIT` + `LICENSE-APACHE` à la racine ;
- `license = "MIT OR Apache-2.0"` dans `[workspace.package]` ;
- en-tête SPDX court sur chaque fichier source :
  `SPDX-License-Identifier: MIT OR Apache-2.0` (avec la ligne
  copyright `2026 Laurent Geynet` — convention écosystème) ;
- contributions = acceptation implicite de la double licence
  (standard Rust).

## Justification

- Standard de l'écosystème Rust : compatibilité maximale avec les
  crates consommées et réutilisabilité du code.
- Apache-2.0 apporte la clause brevet que MIT n'a pas ; MIT garde la
  simplicité maximale — la disjonctive couvre les deux publics.
- Contrairement au daemon OnionBit (GPL-3.0, fidélité copyleft
  Tribler), ce navigateur n'hérite d'aucune obligation de licence
  amont : il est écrit ici, from scratch.

## Conséquences

- Code potentiellement réutilisable dans des contextes propriétaires
  — assumé par le choix permissif.
- La couture `dillo-js` (ADR-0007) : vérifier la licence de ce
  sous-projet à l'intégration — si GPL, l'adapter en dépendance
  optionnelle ou processus séparé pour ne pas contaminer ce dépôt.
  **Point ouvert à ré-arbitrer en phase 8.**
- Les dépendances choisies doivent rester compatibles MIT/Apache-2.0
  (c'est le cas de tout l'écosystème Rust courant).

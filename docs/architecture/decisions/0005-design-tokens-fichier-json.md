# ADR-0005 — Design tokens : `app/assets/design/tokens.json` source de vérité

Statut : Acceptée (2026-10-18).

## Contexte

L'utilisateur exige « utilisation fichier token » : un fichier de
design tokens pilote l'UI Flutter (couleurs, espacements, rayons,
typographie). Options : constantes Dart en dur (modèle du projet
OnionBit : `lib/core/design/tokens/*.dart`), fichier JSON chargé au
runtime, ou codegen depuis un fichier source.

## Décision

**`app/assets/design/tokens.json` = source de vérité unique.**

- Format structuré par sections (`color`, `spacing`, `radius`,
  `typography`, `motion`, `elevation`), valeurs nommées
  (`color.surface.base`, `spacing.md`…) — proche esprit W3C Design
  Tokens, versionné en interne (`"version": 1`).
- Chargé au démarrage via `rootBundle` → objet `DesignTokens` typé
  exposé par Riverpod ; le thème `ThemeData` est *dérivé* des tokens.
- Interdiction de valeur en dur dans les widgets : toute couleur/
  espacement/rayon passe par les tokens (lint custom si nécessaire).
- Évolution possible vers codegen si le fichier grossit — le contrat
  « un fichier, une source » ne change pas.

## Justification

- Un fichier éditable sans recompiler Dart : itérations visuelles
  rapides, diff de design reviewable en PR.
- Le même fichier pourra alimenter d'autres consommateurs (site,
  docs, thème du shell OnionBit) — le JSON est portable, pas le
  code Dart.
- Distinction claire design (fichier) / implémentation (widgets) :
  cohérent avec la clean architecture de `app/`.

## Conséquences

- Le chargement est asynchrone : l'app affiche un splash minimal
  pendant la lecture du bundle (déjà nécessaire pour la connexion
  daemon).
- Une entrée absente/invalide doit dégrader vers un thème de repli
  embarqué — jamais de crash sur fichier token (tolérance : même
  règle que le moteur).
- Les tests de widgets peuvent injecter des tokens fixtures sans
  asset bundle.

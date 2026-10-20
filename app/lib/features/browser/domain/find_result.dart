// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Résultat d'une recherche dans la page (étape 57bis,
/// `POST /api/tabs/{id}/find`). Le surlignage est peint
/// côté moteur ; l'app ne transporte que le compte et
/// l'indice courant.
class FindResult {
  const FindResult({required this.count, required this.index, this.query = ''});

  /// Nombre d'occurrences (boîtes de texte contenant la
  /// requête — granularité boîte, v1).
  final int count;

  /// Indice de l'occurrence courante (0-based).
  final int index;

  /// Requête telle qu'envoyée.
  final String query;

  factory FindResult.fromJson(Map<String, dynamic> j, {String query = ''}) =>
      FindResult(
        count: (j['count'] as num?)?.toInt() ?? 0,
        index: (j['index'] as num?)?.toInt() ?? 0,
        query: (j['query'] as String?) ?? query,
      );

  /// Aucun résultat — utilisé par le mode FFI dégradé et les
  /// erreurs tolérées.
  static const empty = FindResult(count: 0, index: 0);
}

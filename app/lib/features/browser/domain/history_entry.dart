// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Entrée d'historique de navigation persistée côté moteur
/// (`GET /api/history` — store du profil). Ordre : le plus
/// récent d'abord, comme renvoyé par l'API.
class HistoryEntry {
  const HistoryEntry({required this.url, this.title = ''});

  final String url;
  final String title;

  /// Reconstruit depuis le DTO JSON (`{url, title}`).
  factory HistoryEntry.fromJson(Map<String, dynamic> j) => HistoryEntry(
    url: (j['url'] as String?) ?? '',
    title: (j['title'] as String?) ?? '',
  );
}

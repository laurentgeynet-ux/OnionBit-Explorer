// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Signet persisté côté moteur (store du profil — ADR-0003,
/// `GET/POST/DELETE /api/bookmarks`). L'app ne garde que le
/// couple url/titre ; le moteur déduplique par URL.
class Bookmark {
  const Bookmark({required this.url, this.title = ''});

  final String url;
  final String title;

  /// Reconstruit depuis le DTO JSON (`{url, title}`) — champs
  /// absents → valeurs neutres (tolérance API).
  factory Bookmark.fromJson(Map<String, dynamic> j) => Bookmark(
    url: (j['url'] as String?) ?? '',
    title: (j['title'] as String?) ?? '',
  );
}

// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Téléchargement direct en vol (étape 176, événements SSE
/// `download_started`/`progress`/`done`/`failed`) : le job
/// moteur streame vers `.part` hors verrou — la fiche vit
/// du `started` au `done`/`failed` et alimente la liste
/// téléchargements + la barre de progression.
class ActiveDownload {
  const ActiveDownload({
    required this.id,
    required this.tab,
    required this.url,
    required this.filename,
    this.total,
    this.received = 0,
    this.finished = false,
    this.error,
  });

  /// Identifiant moteur (espace global — unique entre onglets).
  final int id;

  /// Onglet émetteur.
  final String tab;

  /// URL téléchargée.
  final String url;

  /// Nom de fichier résolu (hint > Content-Disposition > URL).
  final String filename;

  /// Taille totale si connue (`Content-Length`), sinon `null`.
  final int? total;

  /// Octets déjà écrits.
  final int received;

  /// `true` après `download_done`/`download_failed` — la
  /// fiche reste visible un temps puis l'hôte la purge.
  final bool finished;

  /// Message d'échec (`download_failed`), `null` sinon.
  final String? error;

  /// Progression `0..1` — `null` quand `total` est inconnu.
  double? get fraction =>
      total != null && total! > 0 ? received / total! : null;

  ActiveDownload copyWith({
    int? received,
    int? Function()? total,
    bool? finished,
    String? Function()? error,
  }) => ActiveDownload(
    id: id,
    tab: tab,
    url: url,
    filename: filename,
    total: total != null ? total() : this.total,
    received: received ?? this.received,
    finished: finished ?? this.finished,
    error: error != null ? error() : this.error,
  );

  /// Parse le `data:` JSON de `download_started`.
  factory ActiveDownload.fromJson(Map<String, dynamic> j) => ActiveDownload(
    id: (j['id'] as num?)?.toInt() ?? 0,
    tab: '${j['tab']}',
    url: j['url'] as String? ?? '',
    filename: j['filename'] as String? ?? 'download',
    total: (j['total'] as num?)?.toInt(),
  );
}

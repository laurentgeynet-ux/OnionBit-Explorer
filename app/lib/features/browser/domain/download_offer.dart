// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Téléchargement proposé (étape 70, événement SSE
/// `download_offered`) : le corps non affichable est
/// bufferisé côté moteur en attente de décision —
/// `accept` (écriture fichier) ou `decline` (rejeu comme
/// document, « dump texte »).
class DownloadOffer {
  const DownloadOffer({
    required this.id,
    required this.tab,
    required this.url,
    required this.filename,
    required this.mime,
    this.size,
  });

  /// Identifiant moteur du téléchargement.
  final int id;

  /// Onglet de la navigation.
  final String tab;

  /// URL servie.
  final String url;

  /// Nom de fichier suggéré.
  final String filename;

  /// Type MIME déclaré.
  final String mime;

  /// Taille annoncée si connue (sinon `null`).
  final int? size;

  /// Parse le `data:` JSON de l'événement.
  factory DownloadOffer.fromJson(Map<String, dynamic> j) => DownloadOffer(
    id: (j['id'] as num?)?.toInt() ?? 0,
    tab: '${j['tab']}',
    url: j['url'] as String? ?? '',
    filename: j['filename'] as String? ?? 'download',
    mime: j['mime'] as String? ?? 'application/octet-stream',
    size: (j['size'] as num?)?.toInt(),
  );
}

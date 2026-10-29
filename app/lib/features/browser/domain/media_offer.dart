// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:ui' show Rect;

/// Surface média offerte par le moteur — seam `MediaSurface`
/// (étape 71, ADR-0024, contrat `docs/interface-equipes.md`).
///
/// `rect` est en **px CSS espace document** : l'overlay se
/// positionne à `rect.y − scroll_y` (le scroll arrive par
/// `frame_invalidated.scroll_y`, jamais par re-calcul local).
class MediaOffer {
  const MediaOffer({
    required this.id,
    required this.kind,
    required this.rect,
    required this.url,
    this.mime,
    this.controls = false,
    this.autoplay = false,
    this.claimed = false,
    this.mse = false,
  });

  /// Identité stable de l'instance (`media: u64` session).
  final int id;

  /// `"video"` | `"audio"`.
  final String kind;

  /// Boîte en px CSS document — `audio` sans `controls`
  /// est offert à 0×0 (rien à positionner).
  final Rect rect;

  /// URL résolue de la ressource (`src` ou `<source>`).
  final String url;

  /// Attribut `type` ou suffixe d'URL, si connu.
  final String? mime;

  /// `<video controls>` → l'hôte doit montrer son UI.
  final bool controls;

  /// `<video autoplay>` — l'hôte décide de jouer (ou pas).
  final bool autoplay;

  /// Claim posé par l'hôte (REST `…/claim` accepté).
  final bool claimed;

  /// Surface MSE (MSE-2, ADR-0029) : la `src` est un `blob:`
  /// `MediaSource` — le lecteur pull ses octets via
  /// `…/segments` au lieu de fetcher `url`.
  final bool mse;

  bool get isVideo => kind == 'video';

  /// Une surface positionnable : vidéo avec boîte non vide.
  bool get hasOverlay => isVideo && rect.width > 0 && rect.height > 0;

  MediaOffer copyWith({Rect? rect, bool? claimed}) => MediaOffer(
    id: id,
    kind: kind,
    rect: rect ?? this.rect,
    url: url,
    mime: mime,
    controls: controls,
    autoplay: autoplay,
    claimed: claimed ?? this.claimed,
    mse: mse,
  );

  factory MediaOffer.fromJson(Map<String, dynamic> j) {
    final r = j['rect'] as Map<String, dynamic>? ?? const {};
    double d(String k) => (r[k] as num?)?.toDouble() ?? 0;
    return MediaOffer(
      // `media` dans les events SSE, `id` dans `GET …/media`.
      id: ((j['media'] ?? j['id']) as num?)?.toInt() ?? 0,
      kind: j['kind'] as String? ?? 'video',
      rect: Rect.fromLTWH(d('x'), d('y'), d('w'), d('h')),
      url: j['url'] as String? ?? '',
      mime: j['mime'] as String?,
      controls: j['controls'] == true,
      autoplay: j['autoplay'] == true,
      claimed: j['claimed'] == true,
      mse: j['mse'] == true,
    );
  }
}

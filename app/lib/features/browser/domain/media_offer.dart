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
/// Géométrie **peinte** d'une surface — canal `x-explorer-
/// media` REST / `media` du JSON `explorer_frame_pull` FFI
/// (étape 185, M1). Atome de la frame qui l'accompagne :
/// ces valeurs décrivent ce que le raster a fait de la
/// boîte dans CE PNG, pas un échantillon postérieur.
/// `null` sur `MediaOffer.paint` = transport antérieur —
/// l'app retombe sur `rect − scrollY` (comportement pré-185).
class MediaPaintMeta {
  const MediaPaintMeta({
    required this.id,
    required this.doc,
    required this.view,
    required this.fixed,
    required this.transformed,
    required this.visible,
    required this.order,
    this.clip,
    this.clipPath = false,
  });

  /// Identité de l'offre (`i` du filaire = `media` SSE).
  final int id;

  /// Rect px CSS **document** (`r`) — référence du clic
  /// forwardé et de `media_moved`.
  final Rect doc;

  /// Rect px CSS **viewport** (`v`) — position de
  /// l'overlay : `fixed ? r : r − scroll_y` déjà résolu
  /// côté moteur (règle jamais recopiée ici).
  final Rect view;

  /// Ancrée au viewport (`f` — `position:fixed` chaîne).
  final bool fixed;

  /// Transformée (`t` — `transform ≠ none`/`sticky` sur la
  /// chaîne) : l'overlay se masque — un plan non
  /// transformé sur du contenu transformé est un mensonge.
  final bool transformed;

  /// Effectivement peinte (`s`) : `visibility`, `opacity ≤
  /// 0` ancêtre, ou boîte absente (`display:none`).
  /// `false` → masquer (la lecture continue — contrat).
  final bool visible;

  /// Ordre de peinture (`o` — ordre document, approx M1 :
  /// les contextes d'empilement réels ne sont pas
  /// ordonnancés à ce jalon).
  final int order;

  /// Clip **espace viewport** (`c`) — `null` = le peintre
  /// n'a émis aucun clip sur la chaîne : `ClipRect` sur
  /// `view ∩ clip`, intersection vide → masquer.
  final Rect? clip;

  /// `clip-path`/`clip` géométrique présent (`n`) —
  /// modélisé à la border box de son porteur (honnêteté).
  final bool clipPath;

  /// Rect `{"x":…,"y":…,"w":…,"h":…}` → `Rect`, `null`
  /// tolérant (jamais de crash sur méta malformée).
  static Rect? _rectOf(Object? j) {
    if (j is! Map) return null;
    double d(String k) => (j[k] as num?)?.toDouble() ?? 0;
    return Rect.fromLTWH(d('x'), d('y'), d('w'), d('h'));
  }

  /// Une entrée `x-explorer-media` — champs tolérés
  /// absents (un serveur antérieur n'émet pas le header ;
  /// ici on ne parse que des entrées présentes).
  factory MediaPaintMeta.fromJson(Map<String, Object?> j) {
    return MediaPaintMeta(
      id: (j['i'] as num?)?.toInt() ?? 0,
      doc: _rectOf(j['r']) ?? Rect.zero,
      view: _rectOf(j['v']) ?? Rect.zero,
      fixed: j['f'] == true,
      transformed: j['t'] == true,
      visible: j['s'] != false,
      order: (j['o'] as num?)?.toInt() ?? 0,
      clip: _rectOf(j['c']),
      clipPath: j['n'] == true,
    );
  }
}

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
    this.mseBuffers = const [],
    this.mseOffsets = const [],
    this.loops = false,
    this.paint,
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

  /// MIME de chaque `SourceBuffer` (`mseBuffers` du moteur —
  /// index = `?buffer=` du pull) : `> 1` = multi-rail A+V
  /// (cas YouTube/Twitch → `onionmedia_open_mse_multi`).
  /// Vide hors MSE ou mono-buffer.
  final List<String> mseBuffers;

  /// `timestampOffset` de chaque `SourceBuffer` — même ordre
  /// que [mseBuffers] (`ptsOffsetSecs` `MseDesc`, P-54).
  final List<double> mseOffsets;

  /// Attribut `<video loop>` (champ `loop` du moteur, 139k)
  /// — `onionmedia_set_loop(-1)` quand posé ; hors MSE
  /// (source live non seekable, réarmement ignoré).
  final bool loops;

  /// Géométrie peinte de la **dernière frame installée**
  /// (étape 185) — `null` tant qu'aucun pull méta n'a
  /// atterri : l'app retombe sur `rect − scrollY`.
  final MediaPaintMeta? paint;

  bool get isVideo => kind == 'video';

  /// Une surface positionnable : vidéo avec boîte non vide.
  bool get hasOverlay => isVideo && rect.width > 0 && rect.height > 0;

  /// `<audio controls>` : la boîte porte la barre hôte seule
  /// (étape 183) — pas de couche vidéo, le lecteur n'a pas de
  /// `view` pour l'audio.
  bool get hasAudioBar =>
      !isVideo && controls && rect.width > 0 && rect.height > 0;

  MediaOffer copyWith({
    Rect? rect,
    bool? claimed,
    MediaPaintMeta? paint,
    bool clearPaint = false,
  }) => MediaOffer(
    id: id,
    kind: kind,
    rect: rect ?? this.rect,
    url: url,
    mime: mime,
    controls: controls,
    autoplay: autoplay,
    claimed: claimed ?? this.claimed,
    mse: mse,
    mseBuffers: mseBuffers,
    mseOffsets: mseOffsets,
    loops: loops,
    paint: clearPaint ? null : (paint ?? this.paint),
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
      mseBuffers:
          (j['mseBuffers'] as List?)
              ?.map((e) => e.toString())
              .toList(growable: false) ??
          const [],
      mseOffsets:
          (j['mseOffsets'] as List?)
              ?.map((e) => (e as num).toDouble())
              .toList(growable: false) ??
          const [],
      loops: j['loop'] == true,
    );
  }
}

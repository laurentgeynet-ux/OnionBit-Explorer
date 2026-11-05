// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:async';

import 'package:flutter/widgets.dart';

import '../domain/media_offer.dart';

/// Compteurs `frame_stats` du contrat (JS-C334) — miroir de
/// `OnionFrameStats` côté dépôt codec : `presented`/`dropped`
/// servent `totalVideoFrames`/`droppedVideoFrames`,
/// `corrupted` = fautes de conversion/présentation.
class MediaFrameStats {
  const MediaFrameStats({
    required this.presented,
    required this.dropped,
    this.corrupted = 0,
    required this.totalDelaySecs,
  });

  final int presented;
  final int dropped;
  final int corrupted;
  final double totalDelaySecs;
}

/// Instantané de lecture rapporté au moteur — contrat
/// `MediaReport` (étape 71, `docs/interface-equipes.md`) :
/// `position_ms`/`buffered_ms`/`duration_ms` entiers,
/// `state` ∈ playing|paused|ended|error. Extensions ABI 5 :
/// `buffered_ranges` = intervalles réels `[[s,e],…]` ms,
/// `frame_stats` = télémétrie frames.
class MediaSnapshot {
  const MediaSnapshot({
    required this.positionMs,
    required this.bufferedMs,
    required this.state,
    this.durationMs,
    this.error,
    this.bufferedRanges = const [],
    this.frameStats,
  });

  final int positionMs;
  final int bufferedMs;
  final int? durationMs;
  final String state;
  final String? error;

  /// Intervalles tamponnés réels (ms, triés) — occupation
  /// PacketBuffer après seeks disjoints. Vide = hôte
  /// scalaire → le moteur replie sur `[(0, buffered_ms)]`.
  final List<(int, int)> bufferedRanges;

  /// Télémétrie `getVideoPlaybackQuality` — `null` quand
  /// l'hôte n'expose pas `frame_stats` (DLL pré-ABI 5).
  final MediaFrameStats? frameStats;

  Map<String, dynamic> toJson() => {
    'position_ms': positionMs,
    'buffered_ms': bufferedMs,
    'state': state,
    if (durationMs != null) 'duration_ms': durationMs,
    if (error != null) 'error': error,
    if (bufferedRanges.isNotEmpty)
      'buffered_ranges': [
        for (final r in bufferedRanges) [r.$1, r.$2],
      ],
    if (frameStats != null)
      'frame_stats': {
        'presented': frameStats!.presented,
        'dropped': frameStats!.dropped,
        // `decoded` ≈ présentées + sautées + corrompues —
        // toute trame décodée est affichée, sautée ou
        // perdue en présentation (approximation honnête,
        // le dépôt ne compte pas le décodé pur).
        'decoded':
            frameStats!.presented +
            frameStats!.dropped +
            frameStats!.corrupted,
        'corrupted': frameStats!.corrupted,
        'total_delay_ms': frameStats!.totalDelaySecs * 1000,
      },
  };
}

/// Lecteur natif derrière la seam — abstraction pour la
/// testabilité : la vraie implémentation est `OnionMediaPlayer`
/// (FFI `onionmedia_ffi`, codecs OnionCodec), les tests
/// injectent un faux sans codec.
/// Le contrat `state`/`MediaCommand` ne connaît que cette
/// interface (ADR-0024 : hôte-composée).
abstract class MediaPlayer {
  /// Widget vidéo à poser dans l'overlay — audio → `SizedBox`.
  Widget get view;

  /// `true` quand la vidéo est initialisée (premier frame
  /// disponible) — l'overlay affiche un fond noir en attendant.
  bool get ready;

  /// Instantané courant pour `POST …/state`.
  MediaSnapshot get snapshot;

  /// Volume courant `[0..1]` — dernier `setVolume` posé (1.0
  /// au démarrage) : le backend n'expose pas de getter, la
  /// seam trace la valeur pour la barre de contrôles (étape
  /// 183).
  double get volume;

  /// Muet courant — dernier `setMuted` posé (`false` au
  /// démarrage). Même remarque que [volume].
  bool get muted;

  /// Abonnement aux changements (`ValueListenable` du contrôleur
  /// sous-jacent) — le rapporteur borne à ≤ 4 Hz.
  void addListener(void Function() l);
  void removeListener(void Function() l);

  Future<void> play();
  Future<void> pause();
  Future<void> seekTo(Duration pos);

  /// `fastSeek` DOM (JS-C334) — seek approximatif accostant
  /// la keyframe ≤ cible ; repli `seekTo` précis quand le
  /// backend n'expose pas `onionmedia_fast_seek_ms`.
  Future<void> fastSeek(Duration pos);
  Future<void> setVolume(double v);
  Future<void> setMuted(bool on);
  Future<void> setRate(double r);
  Future<void> dispose();
}

/// Fabrique de lecteurs — injectable (fake en test). L'offre
/// complète passe (branche `mse` : le lecteur a besoin du
/// flag MSE + de l'identité de surface, pas juste l'URL).
typedef MediaPlayerFactory = Future<MediaPlayer> Function(
  MediaOffer offer, {
  required String tabId,
  String apiBase,
  String referer,
});

/// `OnionMediaPlayer` (FFI `onionmedia_ffi`) en prod,
/// fake en test.

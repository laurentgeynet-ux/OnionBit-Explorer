// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:video_player/video_player.dart';

/// Instantané de lecture rapporté au moteur — contrat
/// `MediaReport` (étape 71, `docs/interface-equipes.md`) :
/// `position_ms`/`buffered_ms`/`duration_ms` entiers,
/// `state` ∈ playing|paused|ended|error.
class MediaSnapshot {
  const MediaSnapshot({
    required this.positionMs,
    required this.bufferedMs,
    required this.state,
    this.durationMs,
    this.error,
  });

  final int positionMs;
  final int bufferedMs;
  final int? durationMs;
  final String state;
  final String? error;

  Map<String, dynamic> toJson() => {
    'position_ms': positionMs,
    'buffered_ms': bufferedMs,
    'state': state,
    if (durationMs != null) 'duration_ms': durationMs,
    if (error != null) 'error': error,
  };
}

/// Lecteur natif derrière la seam — abstraction pour la
/// testabilité : la vraie implémentation est `video_player`
/// + `fvp` (MDK), les tests injectent un faux sans codec.
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

  /// Abonnement aux changements (`ValueListenable` du contrôleur
  /// sous-jacent) — le rapporteur borne à ≤ 4 Hz.
  void addListener(void Function() l);
  void removeListener(void Function() l);

  Future<void> play();
  Future<void> pause();
  Future<void> seekTo(Duration pos);
  Future<void> setVolume(double v);
  Future<void> setMuted(bool on);
  Future<void> setRate(double r);
  Future<void> dispose();
}

/// Fabrique de lecteurs — injectable (fake en test).
typedef MediaPlayerFactory = Future<MediaPlayer> Function(String url);

/// Implémentation réelle : `VideoPlayerController` (MDK via
/// `fvp` sur desktop — `fvp.registerWith` posé dans `main`).
class VideoMediaPlayer implements MediaPlayer {
  VideoMediaPlayer._(this._c);

  final VideoPlayerController _c;

  /// Volume restauré au dé-mute — `muted` est distinct de
  /// `volume=0` dans le contrat (le moteur attend que le
  /// dé-mute rende le niveau courant, pas un 1.0 aveugle).
  double _savedVolume = 1.0;

  static Future<MediaPlayer> create(String url) async {
    final c = VideoPlayerController.networkUrl(Uri.parse(url));
    await c.initialize();
    return VideoMediaPlayer._(c);
  }

  @override
  Widget get view => VideoPlayer(_c);

  @override
  bool get ready => _c.value.isInitialized;

  @override
  MediaSnapshot get snapshot {
    final v = _c.value;
    final pos = v.position.inMilliseconds;
    final dur = v.duration.inMilliseconds;
    var buffered = 0;
    for (final r in v.buffered) {
      if (r.end.inMilliseconds > buffered) buffered = r.end.inMilliseconds;
    }
    final state = v.hasError
        ? 'error'
        : v.isPlaying
        ? 'playing'
        : dur > 0 && pos >= dur
        ? 'ended'
        : 'paused';
    return MediaSnapshot(
      positionMs: pos,
      bufferedMs: buffered,
      durationMs: dur > 0 ? dur : null,
      state: state,
      error: v.errorDescription,
    );
  }

  @override
  void addListener(void Function() l) => _c.addListener(l);
  @override
  void removeListener(void Function() l) => _c.removeListener(l);

  @override
  Future<void> play() => _c.play();
  @override
  Future<void> pause() => _c.pause();
  @override
  Future<void> seekTo(Duration pos) => _c.seekTo(pos);
  @override
  Future<void> setVolume(double v) => _c.setVolume(v.clamp(0.0, 1.0));
  @override
  Future<void> setRate(double r) => _c.setPlaybackSpeed(r.clamp(0.25, 4.0));
  @override
  Future<void> setMuted(bool on) async {
    if (on) {
      _savedVolume = _c.value.volume;
      await _c.setVolume(0.0);
    } else {
      await _c.setVolume(_savedVolume.clamp(0.0, 1.0));
    }
  }

  @override
  Future<void> dispose() => _c.dispose();
}

// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:async';

import 'package:flutter/widgets.dart';

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

/// `OnionMediaPlayer` (FFI `onionmedia_ffi`) en prod,
/// fake en test.

// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:async';
import 'dart:ui' as ui;

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

/// Piste média du conteneur (Étape 196) — `kind` = famille
/// filaire dépôt : `0` audio, `1` vidéo, `2` sous-titre,
/// `3` autre ; `disposition` = masque `StreamDisposition`
/// amont (1=DEFAULT, 2=FORCED, 4=attaché, 8=malentendant,
/// 16=malvoyant, 32=commentaire). `index` = stream_index
/// conteneur (sert `set_*_stream`). UI-side : les pistes ne
/// partent **pas** dans `toJson`/`MediaReport`.
class MediaTrack {
  const MediaTrack({
    required this.index,
    required this.kind,
    this.codec = '',
    this.language = '',
    this.title = '',
    this.disposition = 0,
  });

  static const int kindAudio = 0;
  static const int kindVideo = 1;
  static const int kindSubtitle = 2;

  final int index;
  final int kind;
  final String codec;
  final String language;
  final String title;
  final int disposition;

  bool get isDefault => disposition & 1 != 0;
  bool get isForced => disposition & 2 != 0;

  /// Étiquette de menu : titre > langue > codec > index.
  String get label => title.isNotEmpty
      ? title
      : language.isNotEmpty
          ? language
          : codec.isNotEmpty
              ? codec
              : '#$index';
}

/// Chapitre du conteneur (Étape 197) — `startMs`/`endMs` en
/// ms (`endMs < 0` = fin non déclarée par le conteneur),
/// `title` `''` si anonyme. UI-side : hors `MediaReport`.
class MediaChapter {
  const MediaChapter({
    required this.index,
    required this.startMs,
    this.endMs = -1,
    this.title = '',
  });

  final int index;
  final int startMs;
  final int endMs;
  final String title;

  /// Étiquette de menu : titre > rang humain (`#1`-base).
  String get label => title.isNotEmpty ? title : '#${index + 1}';
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
    this.tracks = const [],
    this.audioTrack = -1,
    this.subtitleTrack = -1,
    this.subtitle = '',
    this.chapters = const [],
    this.chapter = -1,
    this.loopState,
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

  /// Pistes du conteneur (lue à l'open, statiques) — vide
  /// si l'hôte n'expose pas `stream_*` (menu pistes masqué).
  final List<MediaTrack> tracks;

  /// Piste audio **résolue** courante — `-1` désactivée /
  /// inconnue / hôte sans getter.
  final int audioTrack;

  /// Piste sous-titre courante — `-1` désactivée (ou hôte
  /// sans getter). Séparée de [subtitle] : une piste peut
  /// être sélectionnée sans cue à l'écran.
  final int subtitleTrack;

  /// Texte sous-titre à la position courante (pull
  /// `subtitle_text` — fichier externe activé ou voie
  /// embarquée) ; `''` = rien à afficher.
  final String subtitle;

  /// Chapitres du conteneur (lus à l'open comme [tracks]) —
  /// vide si l'hôte n'expose pas `chapter_*`.
  final List<MediaChapter> chapters;

  /// Index du chapitre à la position courante — `-1` hors
  /// lecture, sans chapitres ou hôte sans getter.
  final int chapter;

  /// Réglage `loop-file` courant (Étape 199) — `-1` infini,
  /// `0` off, `N ≥ 1` lectures. `null` = hôte sans `loop`
  /// (à distinguer de `0` : absent ≠ off).
  final int? loopState;

  /// Fin du segment de cache contenant la tête de lecture
  /// (barre « buffered » — l'extent *contigu en avance*,
  /// honnête après seeks disjoints). Tête hors segment → fin
  /// du dernier intervalle (= le scalaire). Vide → `bufferedMs`.
  int bufferedAheadEnd(int positionMs) {
    for (final (s, e) in bufferedRanges) {
      if (s <= positionMs && e > positionMs) return e;
    }
    return bufferedRanges.isEmpty ? bufferedMs : bufferedRanges.last.$2;
  }

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

  /// Pochette jointe du conteneur (`cover_art_*`, Étape 199)
  /// — image décodée à l'open, `null` quand le média n'en a
  /// pas ou que l'hôte n'expose pas les exports. Sert de
  /// poster avant la première frame (sur MSE, le moteur ne
  /// peint rien sous la surface).
  ui.Image? get coverArt;

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

  /// Pas-à-pas mpv (`frame-step`/`frame-back-step`, dépôt
  /// P-32, Étape 203) : `dir > 0` = trame suivante, `dir < 0`
  /// = trame précédente, `0` = no-op — un pas par appel. En
  /// lecture, le backend pause d'abord (le pas repause
  /// toujours). No-op sur hôte sans les exports — jamais de
  /// seek approximatif fabriqué à la place.
  Future<void> stepFrame(int dir);
  Future<void> setVolume(double v);
  Future<void> setMuted(bool on);
  Future<void> setRate(double r);

  /// `video.loop = …` runtime (étape 193 — l'attribut
  /// `offer.loops` ne couvre que la création) :
  /// `onionmedia_set_loop(-1|0)`, repli no-op DLL < ABI 5.
  Future<void> setLoop(bool on);

  /// Sélection de la piste audio par stream_index
  /// (`≥ 0`), `-1` = désactivée, `-2` = auto — contrat
  /// dépôt : **effet au prochain play** (pas de commutation
  /// à chaud sur la famille). No-op hôte sans l'export.
  Future<void> selectAudioStream(int index);

  /// Sélection de la piste sous-titre embarquée — `-1`
  /// désactivée, `-2` = auto (première FORCED, sinon DEFAULT,
  /// sinon aucune — sémantique `sid` mpv). **Effet
  /// immédiat** sur le pipeline. Bitmaps listées mais
  /// sélection = no-op d'affichage (doc dépôt). No-op sans
  /// l'export.
  Future<void> selectSubtitleStream(int index);

  /// Saut au chapitre `index` (`onionmedia_seek_chapter` —
  /// seek au `start_ms` du chapitre). No-op sans l'export.
  Future<void> seekChapter(int index);
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

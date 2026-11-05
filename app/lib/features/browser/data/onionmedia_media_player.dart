// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:async';
import 'dart:ffi' as ffi;
import 'dart:isolate';
import '../domain/media_offer.dart';
import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:ffi/ffi.dart';
import 'package:flutter/widgets.dart';

import 'media_player.dart';
import '../../../core/config/app_config.dart';

/// Vue `repr(C)` d'`OnionMediaFrame` (onionmedia-ffi P-59,
/// ABI ≥ 3) : `data`/`len` empruntent le snapshot interne et
/// restent valides **uniquement jusqu'à
/// `onionmedia_frame_release(token)`** — lecture zéro-copie,
/// jamais de `calloc` plein cadre par image.
final class _OnionMediaFrame extends ffi.Struct {
  external ffi.Pointer<ffi.Uint8> data;
  @ffi.Size()
  external int len;
  @ffi.Uint32()
  external int width;
  @ffi.Uint32()
  external int height;
  @ffi.Uint64()
  external int seq;
  @ffi.Size()
  external int token;
}

/// Descripteur `repr(C)` `MseDesc` (onionmedia-ffi P-53) —
/// un rail par `SourceBuffer` (`buffer` = `?buffer=` du pull
/// `…/segments`). Layout ABI 4 : base@0, baseLen@8, tab@16,
/// media@24, buffer@32 (+pad), ext@40, extLen@48,
/// ptsOffsetSecs@56 = 64 octets.
final class _MseDesc extends ffi.Struct {
  external ffi.Pointer<ffi.Char> base;
  @ffi.Size()
  external int baseLen;
  @ffi.Uint64()
  external int tab;
  @ffi.Uint64()
  external int media;
  @ffi.Uint32()
  external int buffer;
  external ffi.Pointer<ffi.Char> ext;
  @ffi.Size()
  external int extLen;
  @ffi.Double()
  external double ptsOffsetSecs;
}

/// Sortie `repr(C)` `OnionFrameStats` (onionmedia-ffi P-67,
/// ABI 5) — `presented`/`dropped`/`corrupted` comptés depuis
/// l'ouverture, `totalFrameDelaySecs` = retard cumulé vs
/// l'horloge. `presented + dropped` = `totalVideoFrames`
/// (convention navigateur, notée dans la doc DLL).
final class _OnionFrameStats extends ffi.Struct {
  @ffi.Uint64()
  external int presented;
  @ffi.Uint64()
  external int dropped;
  @ffi.Uint64()
  external int corrupted;
  @ffi.Double()
  external double totalFrameDelaySecs;
}

/// Lecteur `onionmedia_ffi` (OnionMedia/OnionCodec — pur Rust, MIT/Apache)
/// derrière la seam `MediaPlayer` (ADR-0024) — backend
/// via FFI (`onionmedia_ffi.dll`). Les frames RGBA sont
/// acquises **sans copie** (`onionmedia_frame_acquire`/
/// `release`, ABI ≥ 3 — repli `frame_pull`/`pull_ex` sur DLL
/// plus ancienne) puis décodées en `ui.Image` — l'audio joue
/// en natif dans la DLL (cpal). La cadence vient d'un
/// isolate bloqué sur `onionmedia_frame_wait` (repli Timer
/// 33 ms si l'export manque). Aucun codec n'est embarqué
/// côté Dart : un échec d'ouverture remonte en erreur de
/// claim.
class OnionMediaPlayer implements MediaPlayer {
  OnionMediaPlayer._(this._lib, this._h) {
    _optionalBindings();
  }

  final ffi.DynamicLibrary _lib;
  final ffi.Pointer _h;

  final List<void Function()> _listeners = [];
  Timer? _pump;
  Isolate? _waiter;
  StreamSubscription<dynamic>? _waiterSub;
  ReceivePort? _waiterExit;
  ui.Image? _frame;
  int _lastSeq = -1;
  bool _disposed = false;
  bool _pulling = false;

  /// État audio tracé localement (étape 183) — la DLL n'a pas
  /// de getter volume/mute : les setters mémorisent et notifient
  /// pour que la barre de contrôles se redessine.
  double _volume = 1.0;
  bool _muted = false;

  // -- liaisons natives -------------------------------------------------------

  late final _qtavFree =
      _lib.lookupFunction<ffi.Void Function(ffi.Pointer), void Function(ffi.Pointer)>(
          'onionmedia_free');
  late final _qtavOpenUrl = _lib.lookupFunction<
      ffi.Int32 Function(ffi.Pointer, ffi.Pointer<ffi.Char>, ffi.IntPtr),
      int Function(ffi.Pointer, ffi.Pointer<ffi.Char>, int)>('onionmedia_open_url');
  late final _qtavOpenPath = _lib.lookupFunction<
      ffi.Int32 Function(ffi.Pointer, ffi.Pointer<ffi.Char>, ffi.IntPtr),
      int Function(ffi.Pointer, ffi.Pointer<ffi.Char>, int)>('onionmedia_open_path');
  late final _qtavPlay =
      _lib.lookupFunction<ffi.Int32 Function(ffi.Pointer), int Function(ffi.Pointer)>(
          'onionmedia_play');
  late final _qtavPause =
      _lib.lookupFunction<ffi.Int32 Function(ffi.Pointer), int Function(ffi.Pointer)>(
          'onionmedia_pause');
  late final _qtavStop =
      _lib.lookupFunction<ffi.Int32 Function(ffi.Pointer), int Function(ffi.Pointer)>(
          'onionmedia_stop');
  late final _qtavSeek = _lib.lookupFunction<ffi.Int32 Function(ffi.Pointer, ffi.Int64),
      int Function(ffi.Pointer, int)>('onionmedia_seek_ms');
  late final _qtavSetVol = _lib.lookupFunction<ffi.Int32 Function(ffi.Pointer, ffi.Double),
      int Function(ffi.Pointer, double)>('onionmedia_set_volume');
  late final _qtavSetMute = _lib.lookupFunction<ffi.Int32 Function(ffi.Pointer, ffi.Int32),
      int Function(ffi.Pointer, int)>('onionmedia_set_mute');
  late final _qtavSetSpeed = _lib.lookupFunction<ffi.Int32 Function(ffi.Pointer, ffi.Double),
      int Function(ffi.Pointer, double)>('onionmedia_set_speed');
  late final _qtavState =
      _lib.lookupFunction<ffi.Int32 Function(ffi.Pointer), int Function(ffi.Pointer)>(
          'onionmedia_state');
  late final _qtavPos = _lib.lookupFunction<ffi.Int64 Function(ffi.Pointer),
      int Function(ffi.Pointer)>('onionmedia_position_ms');
  late final _qtavDur = _lib.lookupFunction<ffi.Int64 Function(ffi.Pointer),
      int Function(ffi.Pointer)>('onionmedia_duration_ms');
  late final _qtavEof = _lib.lookupFunction<ffi.Int32 Function(ffi.Pointer),
      int Function(ffi.Pointer)>('onionmedia_is_eof');
  late final _qtavSeq = _lib.lookupFunction<ffi.Uint64 Function(ffi.Pointer),
      int Function(ffi.Pointer)>('onionmedia_frame_seq');
  late final _qtavFSize = _lib.lookupFunction<
      ffi.Int32 Function(ffi.Pointer, ffi.Pointer<ffi.Uint32>, ffi.Pointer<ffi.Uint32>),
      int Function(ffi.Pointer, ffi.Pointer<ffi.Uint32>, ffi.Pointer<ffi.Uint32>)>(
      'onionmedia_frame_size');
  late final _qtavFPull = _lib.lookupFunction<
      ffi.Int64 Function(ffi.Pointer, ffi.Pointer<ffi.Uint8>, ffi.IntPtr),
      int Function(ffi.Pointer, ffi.Pointer<ffi.Uint8>, int)>('onionmedia_frame_pull');
  late final _qtavErr = _lib.lookupFunction<
      ffi.Int32 Function(ffi.Pointer<ffi.Uint8>, ffi.IntPtr),
      int Function(ffi.Pointer<ffi.Uint8>, int)>('onionmedia_last_error');

  // -- liaisons optionnelles (ABI ≥ 2/3 — lookup gardé : la
  //    DLL circule copiée à la main, les exports récents
  //    peuvent manquer → replis honnêtes) -------------------

  /// `onionmedia_frame_acquire`/`release` (P-59, ABI 3) —
  /// prêt du snapshot `Arc`, zéro copie de pixel.
  int Function(ffi.Pointer, ffi.Pointer<_OnionMediaFrame>)? _acquire;
  void Function(int)? _frameRelease;

  /// `onionmedia_frame_wait` (P-59) — attente bloquante
  /// `seq > min_seq` ou timeout ; jamais sur le thread UI.
  int Function(ffi.Pointer, int, int)? _frameWait;

  /// `onionmedia_frame_pull_ex` (P-57) — pull atomique :
  /// `buf NULL` = sonde → `-2` + dims/seq de la trame.
  int Function(
      ffi.Pointer,
      ffi.Pointer<ffi.Uint8>,
      int,
      ffi.Pointer<ffi.Uint32>,
      ffi.Pointer<ffi.Uint32>,
      ffi.Pointer<ffi.Uint64>)? _framePullEx;

  /// `onionmedia_player_error` (P-57) — dernière erreur par
  /// handle, insensible aux migrations d'isolate entre
  /// threads OS (le TLS de `last_error` ne l'est pas).
  int Function(ffi.Pointer, ffi.Pointer<ffi.Uint8>, int)? _playerErr;

  /// `onionmedia_video_rotation` — métadonnée `tkhd` en
  /// degrés horaires (vidéos de téléphone) ; `0` inconnu.
  int Function(ffi.Pointer)? _videoRotation;

  /// `onionmedia_buffered_ms` (P-60) — fin de la plage
  /// tamponnée en ms (`cache-end` mpv ≈ `buffered` HTML5) ;
  /// `-1` hors lecture.
  int Function(ffi.Pointer)? _bufferedMs;

  /// `onionmedia_buffer_underrun` (P-31) — `1` = piste
  /// maître affamée → état `buffering` du contrat (139k).
  int Function(ffi.Pointer)? _bufferUnderrun;

  /// `onionmedia_set_loop` (P-29) — `loop-file` mpv ;
  /// `-1` = infini (`<video loop>`).
  int Function(ffi.Pointer, int)? _setLoop;

  /// `frame_seq`/`dropped_frames` — scalaires **pré-ABI 5** :
  /// trames présentées / sautées par la synchro. Repli quand
  /// `frame_stats` est absent (DLL < 5 qui a quand même les
  /// compteurs — `corrupted`/delay non mesurables → 0).
  int Function(ffi.Pointer)? _frameSeq;
  int Function(ffi.Pointer)? _droppedFrames;

  /// Famille `stream_*` / `subtitle_*` (Étape 196 — exports
  /// pré-ABI 5, lookups gardés hors la porte).
  int Function(ffi.Pointer)? _streamCount;
  int Function(ffi.Pointer, int)? _streamKind;
  int Function(ffi.Pointer, int)? _streamDispos;
  int Function(ffi.Pointer, int, ffi.Pointer<ffi.Char>, int)?
      _streamCodec;
  int Function(ffi.Pointer, int, ffi.Pointer<ffi.Char>, int)?
      _streamLang;
  int Function(ffi.Pointer, int, ffi.Pointer<ffi.Char>, int)?
      _streamTitle;
  int Function(ffi.Pointer)? _audioStreamCur;
  int Function(ffi.Pointer)? _subStreamCur;
  int Function(ffi.Pointer, int)? _setAudioStream;
  int Function(ffi.Pointer, int)? _setSubStream;
  int Function(ffi.Pointer, ffi.Pointer<ffi.Char>, int)? _subText;

  /// Famille `chapter_*` (Étape 197 — même vintage pré-5).
  int Function(ffi.Pointer)? _chapterCount;
  int Function(ffi.Pointer, int)? _chapterStartMs;
  int Function(ffi.Pointer, int)? _chapterEndMs;
  int Function(ffi.Pointer, int, ffi.Pointer<ffi.Char>, int)?
      _chapterTitle;
  int Function(ffi.Pointer)? _currentChapter;
  int Function(ffi.Pointer, int)? _seekChapter;

  /// Étape 199 — pochette + `loop` getter + `pause_at_eof`.
  int Function(ffi.Pointer)? _coverArtLen;
  int Function(ffi.Pointer, ffi.Pointer<ffi.Char>, int)?
      _coverArtData;
  int Function(ffi.Pointer)? _loopGet;
  int Function(ffi.Pointer, int)? _setPauseEof;

  /// Pochette décodée à l'open (`cover_art_*`) — poster
  /// avant la première frame, MSE compris.
  ui.Image? _coverArt;

  /// Pistes lues à l'open (statiques pour un conteneur) —
  /// `stream_*` absents → vide (menu masqué).
  List<MediaTrack> _tracks = const [];

  /// Chapitres — même régime de lecture que [_tracks].
  List<MediaChapter> _chapters = const [];

  /// `onionmedia_abi_version` — la DLL s'auto-identifie
  /// (5 = surface P-63→P-71). Absente = socle ≤ ABI 4 :
  /// le bloc ABI 5 entier est sauté d'un coup. Doctrine
  /// dépôt : les symboles restent découverts par lookup —
  /// la version borne, elle ne prouve pas.
  int Function()? _abiVersion;

  /// `onionmedia_fast_seek_ms` (P-68) — accoste à la
  /// keyframe ≤ cible (approximation DOM `fastSeek`).
  int Function(ffi.Pointer, int)? _fastSeek;

  /// `onionmedia_buffered_ranges` (P-70) — filaire CSV
  /// `s-e,s-e` en **ms**, contrat `copy_out_str`
  /// (`buf NULL`/`0` → taille requise).
  int Function(ffi.Pointer, ffi.Pointer<ffi.Char>, int)? _bufferedRanges;

  /// `onionmedia_frame_stats` (P-67) — compteurs
  /// `getVideoPlaybackQuality` dans un `OnionFrameStats`.
  int Function(ffi.Pointer, ffi.Pointer<_OnionFrameStats>)? _frameStats;

  /// `onionmedia_hw_codecs` — `tag[@WxH];…` issu de
  /// l'interrogation D3D11 réelle ; global (sans handle),
  /// convention `copy_out_str`.
  int Function(ffi.Pointer<ffi.Char>, int)? _hwCodecs;

  /// `onionmedia_set_max_pixels` (P-64) — cap de géométrie
  /// consommé à `open_*` : sélecteur de représentation
  /// HLS/DASH direct (no-op progressif sans manifeste et
  /// `open_mse*` hors `IoConfig`).
  int Function(ffi.Pointer, int)? _setMaxPixels;

  /// `onionmedia_set_framedrop_auto` (P-63) — échelle auto
  /// de saut de trames (`window_ms == 0` désarme).
  int Function(ffi.Pointer, int, int, int, int, int)? _framedropAuto;

  /// `onionmedia_set_preload` (P-71) — politique 0/1/2
  /// (`auto`/`metadata`/`none`) aux `open_*` suivants.
  int Function(ffi.Pointer, int)? _setPreload;

  void _optionalBindings() {
    try {
      _acquire = _lib.lookupFunction<
          ffi.Int32 Function(ffi.Pointer, ffi.Pointer<_OnionMediaFrame>),
          int Function(ffi.Pointer, ffi.Pointer<_OnionMediaFrame>)>(
          'onionmedia_frame_acquire');
      _frameRelease = _lib.lookupFunction<ffi.Void Function(ffi.Size),
          void Function(int)>('onionmedia_frame_release');
    } on Object {
      _acquire = null;
      _frameRelease = null;
    }
    try {
      _frameWait = _lib.lookupFunction<
          ffi.Int64 Function(ffi.Pointer, ffi.Uint64, ffi.Uint32),
          int Function(ffi.Pointer, int, int)>('onionmedia_frame_wait');
    } on Object {
      _frameWait = null;
    }
    try {
      _framePullEx = _lib.lookupFunction<
          ffi.Int64 Function(
              ffi.Pointer,
              ffi.Pointer<ffi.Uint8>,
              ffi.IntPtr,
              ffi.Pointer<ffi.Uint32>,
              ffi.Pointer<ffi.Uint32>,
              ffi.Pointer<ffi.Uint64>),
          int Function(
              ffi.Pointer,
              ffi.Pointer<ffi.Uint8>,
              int,
              ffi.Pointer<ffi.Uint32>,
              ffi.Pointer<ffi.Uint32>,
              ffi.Pointer<ffi.Uint64>)>('onionmedia_frame_pull_ex');
    } on Object {
      _framePullEx = null;
    }
    try {
      _playerErr = _lib.lookupFunction<
          ffi.Int32 Function(ffi.Pointer, ffi.Pointer<ffi.Uint8>, ffi.IntPtr),
          int Function(ffi.Pointer, ffi.Pointer<ffi.Uint8>, int)>(
          'onionmedia_player_error');
    } on Object {
      _playerErr = null;
    }
    try {
      _videoRotation = _lib.lookupFunction<ffi.Int32 Function(ffi.Pointer),
          int Function(ffi.Pointer)>('onionmedia_video_rotation');
    } on Object {
      _videoRotation = null;
    }
    try {
      _bufferedMs = _lib.lookupFunction<ffi.Int64 Function(ffi.Pointer),
          int Function(ffi.Pointer)>('onionmedia_buffered_ms');
    } on Object {
      _bufferedMs = null;
    }
    try {
      _bufferUnderrun = _lib.lookupFunction<ffi.Int32 Function(ffi.Pointer),
          int Function(ffi.Pointer)>('onionmedia_buffer_underrun');
    } on Object {
      _bufferUnderrun = null;
    }
    try {
      _setLoop = _lib.lookupFunction<
          ffi.Int32 Function(ffi.Pointer, ffi.Int64),
          int Function(ffi.Pointer, int)>('onionmedia_set_loop');
    } on Object {
      _setLoop = null;
    }
    // Scalaires pré-ABI 5 — lookups gardés HORS la porte :
    // ils servent précisément quand le bloc 5 est sauté.
    try {
      _frameSeq = _lib.lookupFunction<ffi.Uint64 Function(ffi.Pointer),
          int Function(ffi.Pointer)>('onionmedia_frame_seq');
    } on Object {
      _frameSeq = null;
    }
    try {
      _droppedFrames = _lib.lookupFunction<ffi.Uint64 Function(ffi.Pointer),
          int Function(ffi.Pointer)>('onionmedia_dropped_frames');
    } on Object {
      _droppedFrames = null;
    }
    // Famille pistes/sous-titres — pré-ABI 5, hors la porte.
    try {
      _streamCount = _lib.lookupFunction<
          ffi.Int32 Function(ffi.Pointer), int Function(ffi.Pointer)>(
          'onionmedia_stream_count');
      _streamKind = _lib.lookupFunction<
          ffi.Int32 Function(ffi.Pointer, ffi.Int32),
          int Function(ffi.Pointer, int)>('onionmedia_stream_kind');
      _streamDispos = _lib.lookupFunction<
          ffi.Int32 Function(ffi.Pointer, ffi.Int32),
          int Function(ffi.Pointer, int)>(
          'onionmedia_stream_disposition');
      _streamCodec = _lib.lookupFunction<
          ffi.Int32 Function(ffi.Pointer, ffi.Int32,
              ffi.Pointer<ffi.Char>, ffi.IntPtr),
          int Function(ffi.Pointer, int, ffi.Pointer<ffi.Char>,
              int)>('onionmedia_stream_codec');
      _streamLang = _lib.lookupFunction<
          ffi.Int32 Function(ffi.Pointer, ffi.Int32,
              ffi.Pointer<ffi.Char>, ffi.IntPtr),
          int Function(ffi.Pointer, int, ffi.Pointer<ffi.Char>,
              int)>('onionmedia_stream_language');
      _streamTitle = _lib.lookupFunction<
          ffi.Int32 Function(ffi.Pointer, ffi.Int32,
              ffi.Pointer<ffi.Char>, ffi.IntPtr),
          int Function(ffi.Pointer, int, ffi.Pointer<ffi.Char>,
              int)>('onionmedia_stream_title');
      _audioStreamCur = _lib.lookupFunction<
          ffi.Int32 Function(ffi.Pointer), int Function(ffi.Pointer)>(
          'onionmedia_audio_stream');
      _subStreamCur = _lib.lookupFunction<
          ffi.Int32 Function(ffi.Pointer), int Function(ffi.Pointer)>(
          'onionmedia_subtitle_stream');
      _setAudioStream = _lib.lookupFunction<
          ffi.Int32 Function(ffi.Pointer, ffi.Int32),
          int Function(ffi.Pointer, int)>(
          'onionmedia_set_audio_stream');
      _setSubStream = _lib.lookupFunction<
          ffi.Int32 Function(ffi.Pointer, ffi.Int32),
          int Function(ffi.Pointer, int)>(
          'onionmedia_set_subtitle_stream');
      _subText = _lib.lookupFunction<
          ffi.Int32 Function(
              ffi.Pointer, ffi.Pointer<ffi.Char>, ffi.IntPtr),
          int Function(ffi.Pointer, ffi.Pointer<ffi.Char>, int)>(
          'onionmedia_subtitle_text');
      _chapterCount = _lib.lookupFunction<
          ffi.Int32 Function(ffi.Pointer), int Function(ffi.Pointer)>(
          'onionmedia_chapter_count');
      _chapterStartMs = _lib.lookupFunction<
          ffi.Int64 Function(ffi.Pointer, ffi.Int32),
          int Function(ffi.Pointer, int)>(
          'onionmedia_chapter_start_ms');
      _chapterEndMs = _lib.lookupFunction<
          ffi.Int64 Function(ffi.Pointer, ffi.Int32),
          int Function(ffi.Pointer, int)>(
          'onionmedia_chapter_end_ms');
      _chapterTitle = _lib.lookupFunction<
          ffi.Int32 Function(ffi.Pointer, ffi.Int32,
              ffi.Pointer<ffi.Char>, ffi.IntPtr),
          int Function(ffi.Pointer, int, ffi.Pointer<ffi.Char>,
              int)>('onionmedia_chapter_title');
      _currentChapter = _lib.lookupFunction<
          ffi.Int32 Function(ffi.Pointer), int Function(ffi.Pointer)>(
          'onionmedia_current_chapter');
      _seekChapter = _lib.lookupFunction<
          ffi.Int32 Function(ffi.Pointer, ffi.Int32),
          int Function(ffi.Pointer, int)>(
          'onionmedia_seek_chapter');
      _coverArtLen = _lib.lookupFunction<
          ffi.Int64 Function(ffi.Pointer), int Function(ffi.Pointer)>(
          'onionmedia_cover_art_len');
      _coverArtData = _lib.lookupFunction<
          ffi.Int64 Function(ffi.Pointer, ffi.Pointer<ffi.Char>,
              ffi.IntPtr),
          int Function(ffi.Pointer, ffi.Pointer<ffi.Char>, int)>(
          'onionmedia_cover_art_data');
      _loopGet = _lib.lookupFunction<
          ffi.Int64 Function(ffi.Pointer), int Function(ffi.Pointer)>(
          'onionmedia_loop');
      _setPauseEof = _lib.lookupFunction<
          ffi.Int32 Function(ffi.Pointer, ffi.Int32),
          int Function(ffi.Pointer, int)>(
          'onionmedia_set_pause_at_eof');
    } on Object {
      // Lookup en paquet : un symbole manquant de la
      // famille laisse le reste null (menu masqué).
      _streamCount = _streamKind = _streamDispos =
          _streamCodec = _streamLang = _streamTitle =
              _audioStreamCur = _subStreamCur =
                  _setAudioStream = _setSubStream = _subText =
                      _chapterCount = _chapterStartMs =
                          _chapterEndMs = _chapterTitle =
                              _currentChapter = _seekChapter =
                                  _coverArtLen = _coverArtData =
                                      _loopGet = _setPauseEof = null;
    }
    // Porte ABI : `abi_version` elle-même se découvre —
    // absente = socle ≤ 4, le bloc 5 est sauté entier.
    try {
      _abiVersion = _lib.lookupFunction<ffi.Uint32 Function(),
          int Function()>('onionmedia_abi_version');
    } on Object {
      _abiVersion = null;
    }
    if ((_abiVersion?.call() ?? 0) >= 5) {
      try {
        _fastSeek = _lib.lookupFunction<
            ffi.Int32 Function(ffi.Pointer, ffi.Int64),
            int Function(ffi.Pointer, int)>('onionmedia_fast_seek_ms');
      } on Object {
        _fastSeek = null;
      }
      try {
        _bufferedRanges = _lib.lookupFunction<
            ffi.Int32 Function(
                ffi.Pointer, ffi.Pointer<ffi.Char>, ffi.IntPtr),
            int Function(ffi.Pointer, ffi.Pointer<ffi.Char>, int)>(
            'onionmedia_buffered_ranges');
      } on Object {
        _bufferedRanges = null;
      }
      try {
        _frameStats = _lib.lookupFunction<
            ffi.Int32 Function(
                ffi.Pointer, ffi.Pointer<_OnionFrameStats>),
            int Function(ffi.Pointer, ffi.Pointer<_OnionFrameStats>)>(
            'onionmedia_frame_stats');
      } on Object {
        _frameStats = null;
      }
      try {
        _hwCodecs = _lib.lookupFunction<
            ffi.Int32 Function(ffi.Pointer<ffi.Char>, ffi.IntPtr),
            int Function(ffi.Pointer<ffi.Char>, int)>(
            'onionmedia_hw_codecs');
      } on Object {
        _hwCodecs = null;
      }
      try {
        _setMaxPixels = _lib.lookupFunction<
            ffi.Int32 Function(ffi.Pointer, ffi.Uint64),
            int Function(ffi.Pointer, int)>('onionmedia_set_max_pixels');
      } on Object {
        _setMaxPixels = null;
      }
      try {
        _framedropAuto = _lib.lookupFunction<
            ffi.Int32 Function(ffi.Pointer, ffi.Uint32, ffi.Uint32,
                ffi.Uint32, ffi.Uint32, ffi.Uint32),
            int Function(ffi.Pointer, int, int, int, int, int)>(
            'onionmedia_set_framedrop_auto');
      } on Object {
        _framedropAuto = null;
      }
      try {
        _setPreload = _lib.lookupFunction<
            ffi.Int32 Function(ffi.Pointer, ffi.Int32),
            int Function(ffi.Pointer, int)>('onionmedia_set_preload');
      } on Object {
        _setPreload = null;
      }
    }
  }

  String _lastError() {
    final buf = calloc<ffi.Uint8>(512);
    try {
      final n = _qtavErr(buf, 512);
      return n > 0 ? String.fromCharCodes(buf.asTypedList(n)) : '';
    } finally {
      calloc.free(buf);
    }
  }

  /// Contrat `copy_out_str` du dépôt (`buffered_ranges`,
  /// `hw_codecs`…) : `buf NULL`/`0` → taille requise hors
  /// NUL, sinon octets écrits hors NUL.
  static String _copyOutStr(
    int Function(ffi.Pointer<ffi.Char>, int) fn,
  ) {
    final need = fn(ffi.nullptr, 0);
    if (need <= 0) return '';
    final buf = calloc<ffi.Char>(need + 1);
    try {
      final n = fn(buf, need + 1);
      return n > 0
          ? String.fromCharCodes(buf.cast<ffi.Uint8>().asTypedList(n))
          : '';
    } finally {
      calloc.free(buf);
    }
  }

  /// `buffered_ranges` (P-70) — CSV `s-e,s-e` ms → paires.
  /// Chaîne vide/export absent = `[]` (repli moteur
  /// `[(0, buffered_ms)]`).
  List<(int, int)> _readBufferedRanges() {
    final fn = _bufferedRanges;
    if (fn == null) return const [];
    final wire = _copyOutStr((b, c) => fn(_h, b, c));
    if (wire.isEmpty) return const [];
    final out = <(int, int)>[];
    for (final seg in wire.split(',')) {
      final dash = seg.indexOf('-');
      if (dash <= 0) continue;
      final s = int.tryParse(seg.substring(0, dash));
      final e = int.tryParse(seg.substring(dash + 1));
      if (s != null && e != null && e > s) out.add((s, e));
    }
    return out;
  }

  /// `frame_stats` (P-67) — `null` sans l'export ou hors
  /// lecture (le contrat distingue « absent » de « zéros »).
  /// Repli pré-ABI 5 : `frame_seq` + `dropped_frames`
  /// synthétisent présentées/droppées mesurées — `corrupted`
  /// et delay non mesurables → 0 (plancher honnête, jamais
  /// fabriqué).
  MediaFrameStats? _readFrameStats() {
    final fn = _frameStats;
    if (fn == null) {
      return frameStatsFromScalars(
        _frameSeq?.call(_h),
        _droppedFrames?.call(_h),
      );
    }
    final fs = calloc<_OnionFrameStats>(1);
    try {
      if (fn(_h, fs) != 0) return null;
      return MediaFrameStats(
        presented: fs.ref.presented,
        dropped: fs.ref.dropped,
        corrupted: fs.ref.corrupted,
        totalDelaySecs: fs.ref.totalFrameDelaySecs,
      );
    } finally {
      calloc.free(fs);
    }
  }

  /// `hw_codecs` — `avc1@4096x2304;…` issu de l'interrogation
  /// D3D11 réelle. Tag nu sans `@WxH` = géométrie non
  /// Synthèse scalaire → `MediaFrameStats` : les deux
  /// compteurs absents → `null` (« absent » du contrat,
  /// repli `frame_stats` à zéros implicites interdit).
  @visibleForTesting
  static MediaFrameStats? frameStatsFromScalars(
    int? presented,
    int? dropped,
  ) =>
      presented == null && dropped == null
          ? null
          : MediaFrameStats(
              presented: presented ?? 0,
              dropped: dropped ?? 0,
              corrupted: 0,
              totalDelaySecs: 0,
            );

  /// `copy_out_str` avec paramètre index (`stream_codec`/
  /// `language`/`title`) — `fn` null ou `need ≤ 0` → `''`.
  String _pullIdxStr(
    int Function(ffi.Pointer, int, ffi.Pointer<ffi.Char>, int)? fn,
    int i,
  ) {
    if (fn == null) return '';
    final need = fn(_h, i, ffi.nullptr, 0);
    if (need <= 0) return '';
    final buf = calloc<ffi.Char>(need + 1);
    try {
      final n = fn(_h, i, buf, need + 1);
      return n > 0
          ? String.fromCharCodes(buf.cast<ffi.Uint8>().asTypedList(n))
          : '';
    } finally {
      calloc.free(buf);
    }
  }

  /// `subtitle_text` — cue à la position courante (`''` =
  /// rien à afficher ; `-1` handle → `''` aussi).
  String _readSubtitleText() {
    final fn = _subText;
    if (fn == null) return '';
    final need = fn(_h, ffi.nullptr, 0);
    if (need <= 0) return '';
    final buf = calloc<ffi.Char>(need + 1);
    try {
      final n = fn(_h, buf, need + 1);
      return n > 0
          ? String.fromCharCodes(buf.cast<ffi.Uint8>().asTypedList(n))
          : '';
    } finally {
      calloc.free(buf);
    }
  }

  /// Pistes courantes — relecture tant que la table est
  /// vide (MSE/HLS la peuplent **après** `open_*`, dès le
  /// premier segment démuxé ; `stream_count` reste un i32
  /// bon marché par snapshot).
  List<MediaTrack> get _tracksLive {
    if (_tracks.isEmpty) _refreshTracks();
    return _tracks;
  }

  /// Table des pistes du conteneur (`stream_*`) — lue une
  /// fois à l'open (statique pour un progressif ; une variante
  /// HLS qui changerait de pistes reste figée — limite connue).
  void _refreshTracks() {
    final countFn = _streamCount;
    if (countFn == null) {
      _tracks = const [];
      return;
    }
    final n = countFn(_h);
    if (n <= 0) {
      _tracks = const [];
      return;
    }
    _tracks = [
      for (var i = 0; i < n; i++)
        MediaTrack(
          index: i,
          kind: _streamKind?.call(_h, i) ?? -1,
          codec: _pullIdxStr(_streamCodec, i),
          language: _pullIdxStr(_streamLang, i),
          title: _pullIdxStr(_streamTitle, i),
          disposition: _streamDispos?.call(_h, i) ?? 0,
        ),
    ];
  }

  /// Chapitres courants — même relecture paresseuse que
  /// [_tracksLive] (table peuplée après `open_*` sur MSE).
  List<MediaChapter> get _chaptersLive {
    if (_chapters.isEmpty) _refreshChapters();
    return _chapters;
  }

  /// Table des chapitres (`chapter_*`) — `end_ms` `-1` =
  /// non déclarée par le conteneur, titre `''` = anonyme.
  void _refreshChapters() {
    final fn = _chapterCount;
    if (fn == null) {
      _chapters = const [];
      return;
    }
    final n = fn(_h);
    if (n <= 0) {
      _chapters = const [];
      return;
    }
    _chapters = [
      for (var i = 0; i < n; i++)
        MediaChapter(
          index: i,
          startMs: _chapterStartMs?.call(_h, i) ?? -1,
          endMs: _chapterEndMs?.call(_h, i) ?? -1,
          title: _pullIdxStr(_chapterTitle, i),
        ),
    ];
  }

  /// `cover_art_len`/`data` → `ui.Image` — octets **encodés**
  /// (JPEG/PNG), le décodage est côté appelant (doc dépôt).
  /// Lu une fois à l'open : `0`/absent → `null` (pas de
  /// pochette — pas de poster fabriqué).
  Future<void> _loadCoverArt() async {
    final lenFn = _coverArtLen;
    final dataFn = _coverArtData;
    if (lenFn == null || dataFn == null) return;
    final len = lenFn(_h);
    if (len <= 0) return;
    final buf = calloc<ffi.Char>(len);
    try {
      final n = dataFn(_h, buf, len);
      if (n <= 0) return;
      final bytes =
          Uint8List.fromList(buf.cast<ffi.Uint8>().asTypedList(n));
      final codec = await ui.instantiateImageCodec(bytes);
      final frame = await codec.getNextFrame();
      _coverArt = frame.image;
    } on Object {
      _coverArt = null;
    } finally {
      calloc.free(buf);
    }
  }

  @override
  ui.Image? get coverArt => _coverArt;

  @override
  Future<void> seekChapter(int index) async {
    _seekChapter?.call(_h, index);
  }

  @override
  Future<void> selectAudioStream(int index) async {
    _setAudioStream?.call(_h, index);
  }

  @override
  Future<void> selectSubtitleStream(int index) async {
    _setSubStream?.call(_h, index);
  }

  /// mesurable (doc dépôt) → ignoré du max. 0 = inconnu →
  /// pas de cap (`set_max_pixels` laissé désarmé).
  int _hwGeoMax() {
    final fn = _hwCodecs;
    return fn == null ? 0 : hwGeoFromWire(_copyOutStr(fn));
  }

  /// dpr de la vue principale — `PlatformDispatcher`
  /// `implicitView` (aucune dépendance `BuildContext`, la
  /// seam vit en couche data). 1.0 headless/test.
  static double get _dpr =>
      ui.PlatformDispatcher.instance.implicitView?.devicePixelRatio ??
      1.0;

  /// Besoin d'affichage en **px device** : rect px CSS × dpr².
  /// 0 = rect vide (`audio` sans controls, 0×0) → pas de
  /// borne d'affichage utile. Publique : la factory l'appelle
  /// en prod ; testée unitairement.
  /// Sonde `onionmedia_can_play_type` (Étape 198) — export
  /// **global** (pas de handle, interrogeable avant
  /// `onionmedia_new`) : `mime` RFC 6381 → verdict
  /// `""`/`maybe`/`probable` du registry décodeurs réel.
  /// `null` = sonde indisponible (DLL ou export absent,
  /// mime refusé) — le résolveur replie alors sur le
  /// statique, jamais de capacité retirée sans mesure.
  static String? probeMimeVerdict(String mime) {
    try {
      final lib = _load();
      final fn = lib.lookupFunction<
          ffi.Int32 Function(ffi.Pointer<ffi.Char>, ffi.IntPtr,
              ffi.Pointer<ffi.Char>, ffi.IntPtr),
          int Function(
              ffi.Pointer<ffi.Char>, int, ffi.Pointer<ffi.Char>, int)>(
          'onionmedia_can_play_type');
      final bytes = mime.codeUnits;
      final mp = calloc<ffi.Char>(bytes.length);
      try {
        for (var i = 0; i < bytes.length; i++) {
          mp[i] = bytes[i];
        }
        final need = fn(mp, bytes.length, ffi.nullptr, 0);
        if (need < 0) return null;
        final buf = calloc<ffi.Char>(need + 1);
        try {
          final n = fn(mp, bytes.length, buf, need + 1);
          return n >= 0
              ? String.fromCharCodes(
                  buf.cast<ffi.Uint8>().asTypedList(n))
              : null;
        } finally {
          calloc.free(buf);
        }
      } finally {
        calloc.free(mp);
      }
    } on Object {
      return null;
    }
  }

  /// Oracle `can_play_type` (Étape 198) — la liste déclarée
  /// au moteur = entrées conteneur inchangées + préfixes
  /// codec que `probe` certifie `probable`. Repli
  /// [declared] intact quand `oracle` est off, la table de
  /// sondes est vide, ou **aucun** préfixe n'est certifié
  /// (sonde indisponible — le repli garde le comportement
  /// historique). `probe` injectable pour les tests.
  static List<String> resolveMediaCodecs(
    List<String> declared,
    Map<String, String> probes, {
    bool oracle = true,
    String? Function(String mime)? probe,
  }) {
    if (!oracle || probes.isEmpty) return declared;
    final pfn = probe ?? probeMimeVerdict;
    final certified = <String>[
      for (final e in probes.entries)
        if (pfn(e.value) == 'probable') e.key,
    ];
    if (certified.isEmpty) return declared;
    return [
      ...declared.where((c) => c.contains('/')),
      ...certified,
    ];
  }

  static int displayPixelsFor(Rect r) {
    if (r.isEmpty) return 0;
    final d = _dpr;
    return (r.width * r.height * d * d).round();
  }

  /// `min(hw_geo, besoin)` — politique du cap
  /// `set_max_pixels` : `besoin == 0` → géométrie hw seule
  /// (rien à affiner) ; `hwGeo == 0` (device absent/muet)
  /// → besoin seul (borne même le décodage sw — pourquoi
  /// décoder du 4K pour un overlay 480p) ; les deux nuls →
  /// pas de cap.
  @visibleForTesting
  static int maxPixelsFor(int hwGeo, int besoin) {
    if (hwGeo <= 0) return besoin;
    if (besoin <= 0) return hwGeo;
    return hwGeo < besoin ? hwGeo : besoin;
  }

  /// Parse filaire `tag[@WxH];…` → max `w*h` (0 = rien de
  /// mesurable). `@visibleForTesting` : la tranche géométrie
  /// du binding, sans la DLL.
  @visibleForTesting
  static int hwGeoFromWire(String wire) {
    var best = 0;
    for (final tag in wire.split(';')) {
      final at = tag.indexOf('@');
      if (at <= 0) continue;
      final dims = tag.substring(at + 1).split('x');
      if (dims.length != 2) continue;
      final w = int.tryParse(dims[0]);
      final h = int.tryParse(dims[1]);
      if (w != null && h != null && w * h > best) best = w * h;
    }
    return best;
  }

  /// `set_framedrop_auto` (P-63) — escalade auto de saut de
  /// trames quand le décodage < temps réel (« mur 4K » ;
  /// `set_max_pixels` borne la géométrie, lui le débit).
  /// `null` = pas de politique (comportement antérieur).
  void _applyFramedrop(MediaFramedropConfig? cfg) {
    final fn = _framedropAuto;
    if (fn == null || cfg == null) return;
    fn(_h, cfg.window.inMilliseconds, cfg.dwell.inMilliseconds,
        cfg.pin.inMilliseconds, cfg.dropsPerWindow, cfg.maxLevel);
  }

  /// Dernière erreur du handle (`player_error` par handle si
  /// la DLL l'expose, sinon TLS) — `null` quand vide.
  String? _playerError() {
    final fn = _playerErr;
    if (fn == null) {
      final e = _lastError();
      return e.isEmpty ? null : e;
    }
    final buf = calloc<ffi.Uint8>(512);
    try {
      final n = fn(_h, buf, 512);
      return n > 0 ? String.fromCharCodes(buf.asTypedList(n)) : null;
    } finally {
      calloc.free(buf);
    }
  }

  /// Charge `onionmedia_ffi` — `ONIONMEDIA_FFI_LIB` > nom standard à côté
  /// de l'exécutable / `PATH` (config §4.13, même patron que
  /// `explorer_ffi`).
  static ffi.DynamicLibrary _load() {
    final p = Platform.environment['ONIONMEDIA_FFI_LIB'];
    return p != null && p.isNotEmpty
        ? ffi.DynamicLibrary.open(p)
        : ffi.DynamicLibrary.open('onionmedia_ffi.dll');
  }

  static void _setHwAccel(ffi.DynamicLibrary lib, ffi.Pointer h, bool on) {
    // `onionmedia_set_hwaccel` peut ne pas exister encore —
    // lookup gardé, ignoré silencieusement si absent.
    for (final sym in ['onionmedia_set_hw_accel', 'onionmedia_set_hwaccel']) {
      try {
        final setHw = lib.lookupFunction<
            ffi.Int32 Function(ffi.Pointer, ffi.Int32),
            int Function(ffi.Pointer, int)>(sym);
        setHw(h, on ? 1 : 0);
        break;
      } on Object {
        // export absent — HW non piloté, comportement antérieur.
      }
    }
  }

  /// `onionmedia_set_http_headers` (P-66, ABI 5) — en-têtes
  /// appliqués aux `open_*` suivants (manifeste + segments
  /// HLS/DASH + Range ; **pas** `open_mse*` qui pull via le
  /// REST local). Format filaire : `Nom: valeur\n`. Lookup
  /// gardé — DLL < 5 = sans effet, jamais de crash.
  static void _setHttpHeaders(
    ffi.DynamicLibrary lib,
    ffi.Pointer h,
    String text,
  ) {
    if (text.isEmpty) return;
    try {
      final fn = lib.lookupFunction<
          ffi.Int32 Function(ffi.Pointer, ffi.Pointer<ffi.Char>, ffi.IntPtr),
          int Function(ffi.Pointer, ffi.Pointer<ffi.Char>, int)>(
          'onionmedia_set_http_headers');
      final s = text.toNativeUtf8();
      try {
        fn(h, s.cast<ffi.Char>(), s.length);
      } finally {
        calloc.free(s);
      }
    } on Object {
      // export absent — en-têtes non pilotés.
    }
  }

  /// Ouvre `url` (http(s) proxyfiée ou chemin `file`/brut).
  /// `referer`/`userAgent` partent via `set_http_headers`
  /// **avant** l'open — googlevideo et consorts les exigent.
  /// `StateError` si la DLL ou l'ouverture échoue — la factory
  /// échoue, l'erreur remonte au claim.
  static Future<MediaPlayer> create(
    String url, {
    bool hwAccel = false,
    bool loop = false,
    String referer = '',
    String userAgent = '',
    String preload = 'auto',
    int displayPixels = 0,
    MediaFramedropConfig? framedrop,
    bool pauseAtEof = false,
  }) async {
    final lib = _load();
    /// `file:///D:/x.mkv` -> `D:/x.mkv` ; chemin nu inchangé.
    String localPath(String url) {
      if (!url.startsWith('file')) {
        return url;
      }
      try {
        return Uri.parse(url).toFilePath(windows: true);
      } on Object {
        return url;
      }
    }

    final h = lib.lookupFunction<ffi.Pointer Function(), ffi.Pointer Function()>(
        'onionmedia_new')();
    if (h == ffi.nullptr) {
      throw StateError('onionmedia_new a échoué');
    }
    final p = OnionMediaPlayer._(lib, h);
    _setHwAccel(lib, h, hwAccel);
    // `set_max_pixels` : cap consommé à `open_*` (sélecteur
    // de rep HLS/DASH direct — no-op MP4/WebM simple et hors
    // `open_mse*`). `min(geo_hw, besoin×dpr)` : pourquoi
    // choisir une rep 8K pour un overlay 480p. Figé à
    // l'open — un `media_moved` ultérieur reste cappé
    // (accepté, documenté étape 194).
    final geo = maxPixelsFor(p._hwGeoMax(), displayPixels);
    if (geo > 0) {
      p._setMaxPixels?.call(h, geo);
    }
    p._applyFramedrop(framedrop);
    // `set_preload` (P-71) — attribut énuméré `auto`|
    // `metadata`|`none` → 0/1/2, consommé aux `open_*`
    // suivants. Progressif seulement : hors `open_mse*`
    // (la page pilote les appends, la politique est moot).
    p._setPreload?.call(h, switch (preload) {
      'metadata' => 1,
      'none' => 2,
      _ => 0,
    });
    try {
      final isHttp = url.startsWith('http');
      if (isHttp) {
        _setHttpHeaders(lib, h, [
          if (referer.isNotEmpty) 'Referer: $referer',
          if (userAgent.isNotEmpty) 'User-Agent: $userAgent',
        ].join('\n'));
      }
      // `file:///D:/x.mkv` -> chemin filesystem pour
      // `open_path` (le moteur attend un path, pas une URL).
      final target = isHttp ? url : localPath(url);
      final bytes = target.codeUnits;
      final c = calloc<ffi.Char>(bytes.length);
      for (var i = 0; i < bytes.length; i++) {
        c[i] = bytes[i];
      }
      final rc = isHttp
          ? p._qtavOpenUrl(h, c, bytes.length)
          : p._qtavOpenPath(h, c, bytes.length);
      p._refreshTracks();
      p._refreshChapters();
      calloc.free(c);
      if (pauseAtEof) {
        p._setPauseEof?.call(h, 1);
      }
      await p._loadCoverArt();
      if (rc != 0) {
        throw StateError('onionmedia_open: ${p._playerError() ?? p._lastError()}');
      }
    } catch (_) {
      p._qtavFree(h);
      rethrow;
    }
    p._applyLoop(loop);
    p._startPump();
    return p;
  }

  /// `<video loop>` → `onionmedia_set_loop(-1)` (P-29,
  /// `loop-file` mpv) ; no-op sans l'export ou l'attribut.
  /// MSE : live non seekable, réarmement ignoré côté DLL.
  void _applyLoop(bool loop) {
    if (loop) {
      _setLoop?.call(_h, -1);
    }
  }

  /// Chemin MSE : offre `mse:true` → le lecteur pull ses
  /// octets via `…/segments` au lieu de fetcher `url`.
  /// `mseBuffers.length > 1` = multi-rail A+V (cas
  /// YouTube/Twitch) → `onionmedia_open_mse_multi` (P-53) ;
  /// sinon `onionmedia_open_mse` mono-rail (lookup gardé :
  /// symbole absent → erreur de claim honnête, pas de crash
  /// FFI).
  static Future<MediaPlayer> createMse(
    MediaOffer offer, {
    String tabId = '',
    String apiBase = '',
    bool hwAccel = false,
    MediaFramedropConfig? framedrop,
    bool pauseAtEof = false,
  }) async {
    final lib = _load();
    // Le symbole peut ne pas exister encore — erreur propre.
    try {
      lib.lookup<ffi.NativeFunction<ffi.Void Function()>>(
          'onionmedia_open_mse');
    } on Object {
      throw StateError(
        'onionmedia_open_mse absent — DLL sans le chemin MSE',
      );
    }
    // `open_mse_multi` est additif (ABI 2→3) : absent →
    // mono-rail dégradé, jamais de claim cassant.
    final openMulti = () {
      try {
        return lib.lookupFunction<
            ffi.Int32 Function(ffi.Pointer, ffi.Pointer<_MseDesc>, ffi.IntPtr),
            int Function(ffi.Pointer, ffi.Pointer<_MseDesc>, int)>(
            'onionmedia_open_mse_multi');
      } on Object {
        return null;
      }
    }();
    final h = lib.lookupFunction<ffi.Pointer Function(), ffi.Pointer Function()>(
        'onionmedia_new')();
    if (h == ffi.nullptr) {
      throw StateError('onionmedia_new a échoué');
    }
    final p = OnionMediaPlayer._(lib, h);
    try {
      _setHwAccel(lib, h, hwAccel);
      // `set_max_pixels` volontairement absent ici :
      // `open_mse*` ne prend pas d'`IoConfig` — le pull passe
      // par le REST local, le cap n'atteint aucune rep.
      p._applyFramedrop(framedrop);
      final tab = int.tryParse(tabId) ?? 0;
      if (offer.mseBuffers.length > 1 && openMulti != null) {
        _openMseMulti(
            openMulti, p, h, apiBase, tab, offer.id, offer.mseBuffers,
            offer.mseOffsets);
      } else {
        _openMseSingle(lib, p, h, apiBase, tab, offer);
      }
    } catch (_) {
      p._qtavFree(h);
      rethrow;
    }
    p._applyLoop(offer.loops);
    if (pauseAtEof) {
      p._setPauseEof?.call(h, 1);
    }
    await p._loadCoverArt();
    p._startPump();
    return p;
  }

  /// Mono-rail `onionmedia_open_mse` — signature réelle
  /// (onionmedia-ffi/lib.rs P-51) : `open_mse(h, base_ptr,
  /// base_len, tab u64, media u64, buffer u32, ext_ptr,
  /// ext_len)` — paires ptr+len UTF-8, PAS de C-string NUL
  /// (vol MSE-DLL : la signature à 6 params supposée envoyait
  /// ext dans le slot buffer et `ext_ptr` recevait un
  /// registre poubelle → « extension mse invalide (utf-8) »).
  /// `ext` = MIME du rail (`video/mp4`, `;codecs=` toléré —
  /// normalisé `container_hint` côté DLL depuis P-59).
  static void _openMseSingle(ffi.DynamicLibrary lib, OnionMediaPlayer p,
      ffi.Pointer h, String apiBase, int tab, MediaOffer offer) {
    final openMse = lib.lookupFunction<
        ffi.Int32 Function(
            ffi.Pointer,
            ffi.Pointer<ffi.Char>,
            ffi.IntPtr,
            ffi.Uint64,
            ffi.Uint64,
            ffi.Uint32,
            ffi.Pointer<ffi.Char>,
            ffi.IntPtr),
        int Function(
            ffi.Pointer,
            ffi.Pointer<ffi.Char>,
            int,
            int,
            int,
            int,
            ffi.Pointer<ffi.Char>,
            int)>('onionmedia_open_mse');
    final ab = apiBase.codeUnits;
    final ext = (offer.mseBuffers.isNotEmpty
            ? offer.mseBuffers.first
            : offer.mime ?? 'mp4')
        .codeUnits;
    final cAb = calloc<ffi.Char>(ab.length);
    final cExt = calloc<ffi.Char>(ext.length);
    for (var i = 0; i < ab.length; i++) {
      cAb[i] = ab[i];
    }
    for (var i = 0; i < ext.length; i++) {
      cExt[i] = ext[i];
    }
    final rc = openMse(h, cAb, ab.length, tab, offer.id, 0, cExt, ext.length);
    calloc.free(cAb);
    calloc.free(cExt);
    if (rc != 0) {
      throw StateError(
          'onionmedia_open_mse: ${p._playerError() ?? p._lastError()}');
    }
  }

  /// Multi-rail `onionmedia_open_mse_multi` (P-53) — A+V :
  /// un `MseDesc` par `SourceBuffer` (`buffer` = index,
  /// `ext` = MIME du rail). Un seul `base` partagé — même
  /// `apiBase` pour tous les rails.
  static void _openMseMulti(
    int Function(ffi.Pointer, ffi.Pointer<_MseDesc>, int) openMulti,
    OnionMediaPlayer p,
    ffi.Pointer h,
    String apiBase,
    int tab,
    int media,
    List<String> rails,
    List<double> offsets,
  ) {
    final ab = apiBase.codeUnits;
    final cAb = calloc<ffi.Char>(ab.length);
    for (var i = 0; i < ab.length; i++) {
      cAb[i] = ab[i];
    }
    final exts = <ffi.Pointer<ffi.Char>>[];
    final descs = calloc<_MseDesc>(rails.length);
    try {
      for (var i = 0; i < rails.length; i++) {
        final e = rails[i].codeUnits;
        final c = calloc<ffi.Char>(e.length);
        for (var j = 0; j < e.length; j++) {
          c[j] = e[j];
        }
        exts.add(c);
        descs[i]
          ..base = cAb
          ..baseLen = ab.length
          ..tab = tab
          ..media = media
          ..buffer = i
          ..ext = c
          ..extLen = e.length
          ..ptsOffsetSecs = i < offsets.length ? offsets[i] : 0.0;
      }
      final rc = openMulti(h, descs, rails.length);
      if (rc != 0) {
        throw StateError(
            'onionmedia_open_mse_multi: ${p._playerError() ?? 'rc=$rc'}');
      }
    } finally {
      for (final c in exts) {
        calloc.free(c);
      }
      calloc.free(cAb);
      calloc.free(descs);
    }
  }

  // -- pompe à trames ---------------------------------------------------------

  void _startPump() {
    if (_frameWait != null) {
      _startWaiter();
      return;
    }
    _pump =
        Timer.periodic(const Duration(milliseconds: 33), (_) => _pull());
  }

  /// Isolate dédié `frame_wait` (P-59) : bloque côté natif
  /// jusqu'à nouvelle trame — le thread UI n'attend jamais,
  /// la latence frame→pull tombe à ~0 et le polling 33 ms
  /// disparaît (CPU nul en pause). `onExit` porte la mort de
  /// l'isolate : `dispose` l'attend avant `free` — le handle
  /// ne doit jamais être libéré sous un `frame_wait` actif
  /// (use-after-free sinon).
  void _startWaiter() {
    final port = ReceivePort();
    final exit = ReceivePort();
    _waiterExit = exit;
    _waiterSub = port.listen((_) => _pull());
    Isolate.spawn(
      _waiterMain,
      (port: port.sendPort, h: _h.address),
      onExit: exit.sendPort,
    ).then((iso) {
      if (_disposed) {
        iso.kill();
      } else {
        _waiter = iso;
      }
    }).catchError((Object e) {
      // spawn impossible → repli polling périodique.
      _pump ??= Timer.periodic(
          const Duration(milliseconds: 33), (_) => _pull());
    });
  }

  /// Corps de l'isolate waiter — `frame_wait` bloquant dans
  /// sa propre copie de la DLL (chaque isolate charge la
  /// sienne). Rend dès `seq > last`, re-sonde au timeout 5 s
  /// (borne le délai de `kill` au dispose), meurt sur
  /// retour `-1` (handle libéré).
  static void _waiterMain(({SendPort port, int h}) msg) {
    final lib = _load();
    final wait = lib.lookupFunction<
        ffi.Int64 Function(ffi.Pointer, ffi.Uint64, ffi.Uint32),
        int Function(ffi.Pointer, int, int)>('onionmedia_frame_wait');
    final h = ffi.Pointer<ffi.Void>.fromAddress(msg.h);
    var last = 0;
    while (true) {
      final seq = wait(h, last, 5000);
      if (seq < 0) {
        return;
      }
      if (seq > last) {
        last = seq;
        msg.port.send(seq);
      }
    }
  }

  /// Tire la trame courante : acquire zéro-copie (ABI ≥ 3)
  /// si la DLL l'expose, sinon chemin copie. Réentrance
  /// écartée (`_pulling`) — un tick/événement peut chevaucher
  /// le `await` du décodage précédent.
  Future<void> _pull() async {
    if (_disposed || _pulling) return;
    _pulling = true;
    try {
      final acquire = _acquire;
      final release = _frameRelease;
      if (acquire != null && release != null) {
        final fv = calloc<_OnionMediaFrame>(1);
        try {
          if (acquire(_h, fv) != 1) return;
          try {
            if (fv.ref.seq == _lastSeq ||
                fv.ref.width == 0 ||
                fv.ref.height == 0) {
              return;
            }
            _lastSeq = fv.ref.seq;
            // `data` valide jusqu'au release : le `await`
            // est sûr — `decodeImageFromPixels` copie les
            // octets dans l'ImmutableBuffer avant de rendre.
            final img = await _decode(
              fv.ref.data.asTypedList(fv.ref.len),
              fv.ref.width,
              fv.ref.height,
            );
            if (img != null) {
              _frame?.dispose();
              _frame = img;
              _notify();
            }
          } finally {
            release(fv.ref.token);
          }
        } finally {
          calloc.free(fv);
        }
        return;
      }
      await _pullCopy();
    } finally {
      _pulling = false;
    }
  }

  /// Repli copie (DLL < ABI 3) : `pull_ex` atomique si
  /// exposé — sonde `buf NULL` → `-2` + dims/seq, puis pull
  /// sur tampon calibré (les métadonnées rapportées sont
  /// celles de la trame copiée) ; sinon paire legacy
  /// `frame_size`/`frame_pull` (fenêtre de course
  /// documentée).
  Future<void> _pullCopy() async {
    if (_qtavSeq(_h) == _lastSeq) return;
    final w = calloc<ffi.Uint32>(1);
    final ht = calloc<ffi.Uint32>(1);
    try {
      final pullEx = _framePullEx;
      if (pullEx != null) {
        final sq = calloc<ffi.Uint64>(1);
        try {
          if (pullEx(_h, ffi.nullptr, 0, w, ht, sq) != -2 ||
              w.value == 0 ||
              ht.value == 0) {
            return;
          }
          final buf = calloc<ffi.Uint8>(w.value * ht.value * 4);
          try {
            final n = pullEx(_h, buf, w.value * ht.value * 4, w, ht, sq);
            if (n <= 0) return;
            _lastSeq = sq.value;
            final img = await _decode(
                buf.asTypedList(n), w.value, ht.value);
            if (img != null) {
              _frame?.dispose();
              _frame = img;
              _notify();
            }
          } finally {
            calloc.free(buf);
          }
        } finally {
          calloc.free(sq);
        }
        return;
      }
      // Legacy : dims puis pull (fenêtre de course connue).
      _qtavFSize(_h, w, ht);
      final width = w.value;
      final height = ht.value;
      if (width == 0 || height == 0) return;
      _lastSeq = _qtavSeq(_h);
      final buf = calloc<ffi.Uint8>(width * height * 4);
      try {
        final n = _qtavFPull(_h, buf, width * height * 4);
        if (n <= 0) return;
        final img = await _decode(buf.asTypedList(n), width, height);
        if (img != null) {
          _frame?.dispose();
          _frame = img;
          _notify();
        }
      } finally {
        calloc.free(buf);
      }
    } finally {
      calloc.free(w);
      calloc.free(ht);
    }
  }

  Future<ui.Image?> _decode(List<int> rgba, int w, int h) async {
    final c = Completer<ui.Image?>();
    ui.decodeImageFromPixels(
      rgba is Uint8List ? rgba : Uint8List.fromList(rgba),
      w,
      h,
      ui.PixelFormat.rgba8888,
      (img) => c.complete(img),
    );
    return c.future;
  }

  void _notify() {
    for (final l in _listeners) {
      l();
    }
  }

  // -- MediaPlayer ------------------------------------------------------------

  @override
  Widget get view {
    final img = _frame;
    if (img == null) {
      return const SizedBox.expand(
          child: ColoredBox(color: Color(0xFF000000)));
    }
    // Rotation conteneur (`tkhd`) lue à la demande — le
    // pivotement physique des pixels reste côté rendu, les
    // trames sortent non tournées (contrat FFI).
    final rot = _videoRotation?.call(_h) ?? 0;
    final quarter = rot > 0 ? (rot ~/ 90) % 4 : 0;
    return SizedBox.expand(
      child: quarter == 0
          ? RawImage(image: img, fit: BoxFit.contain)
          : RotatedBox(
              quarterTurns: quarter,
              child: RawImage(image: img, fit: BoxFit.contain)),
    );
  }

  @override
  bool get ready => _frame != null;

  @override
  MediaSnapshot get snapshot {
    final st = _qtavState(_h);
    final pos = _qtavPos(_h).clamp(0, 1 << 62);
    final dur = _qtavDur(_h);
    final state = switch (st) {
      // `underrun` = file décode affamée en pleine lecture
      // → `buffering` (contrat étendu 139k, `waiting` HTML5).
      2 when _bufferUnderrun?.call(_h) == 1 => 'buffering',
      2 => 'playing',
      3 => 'paused',
      _ when _qtavEof(_h) == 1 => 'ended',
      _ => 'paused',
    };
    final buffered = _bufferedMs?.call(_h) ?? -1;
    return MediaSnapshot(
      positionMs: pos,
      bufferedMs:
          buffered >= 0 ? buffered : pos, // P-60, sinon approximation v1
      durationMs: dur > 0 ? dur : null,
      state: state,
      error: _playerError(),
      bufferedRanges: _readBufferedRanges(),
      frameStats: _readFrameStats(),
      tracks: _tracksLive,
      audioTrack: _audioStreamCur?.call(_h) ?? -1,
      subtitleTrack: _subStreamCur?.call(_h) ?? -1,
      subtitle: _readSubtitleText(),
      chapters: _chaptersLive,
      chapter: _currentChapter?.call(_h) ?? -1,
      loopState: _loopGet == null ? null : _loopGet!(_h),
    );
  }

  @override
  void addListener(void Function() l) => _listeners.add(l);
  @override
  void removeListener(void Function() l) => _listeners.remove(l);

  @override
  Future<void> play() async => _qtavPlay(_h);
  @override
  Future<void> pause() async => _qtavPause(_h);
  @override
  Future<void> seekTo(Duration pos) async => _qtavSeek(_h, pos.inMilliseconds);

  /// `fastSeek` → `onionmedia_fast_seek_ms` (keyframe ≤
  /// cible, P-68) ; repli seek précis sur DLL < ABI 5.
  @override
  Future<void> fastSeek(Duration pos) async {
    final fn = _fastSeek;
    if (fn != null) {
      fn(_h, pos.inMilliseconds);
    } else {
      _qtavSeek(_h, pos.inMilliseconds);
    }
  }
  @override
  double get volume => _volume;
  @override
  bool get muted => _muted;

  @override
  Future<void> setVolume(double v) async {
    _volume = v.clamp(0.0, 1.0);
    _qtavSetVol(_h, _volume);
    _notify();
  }

  @override
  Future<void> setMuted(bool on) async {
    _muted = on;
    _qtavSetMute(_h, on ? 1 : 0);
    _notify();
  }
  @override
  Future<void> setRate(double r) async =>
      _qtavSetSpeed(_h, r.clamp(0.25, 4.0));

  @override
  Future<void> setLoop(bool on) async => _setLoop?.call(_h, on ? -1 : 0);

  @override
  Future<void> dispose() async {
    if (_disposed) return;
    _disposed = true;
    _pump?.cancel();
    await _waiterSub?.cancel();
    final iso = _waiter;
    final exit = _waiterExit;
    if (iso != null) {
      iso.kill(priority: Isolate.immediate);
      // Mort réelle avant `free` : l'isolate peut être
      // bloqué dans `frame_wait` sur le handle (borne 5 s
      // + marge — `wait` rend au timeout si rien n'arrive).
      if (exit != null) {
        await exit.first
            .timeout(const Duration(seconds: 10), onTimeout: () => null);
      }
      exit?.close();
    }
    _qtavStop(_h);
    _qtavFree(_h);
    _frame?.dispose();
    _frame = null;
  }
}

// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:async';
import 'dart:ffi' as ffi;
import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:ffi/ffi.dart';
import 'package:flutter/widgets.dart';

import 'media_player.dart';

/// Lecteur `onionmedia_ffi` (OnionMedia/OnionCodec — pur Rust, MIT/Apache)
/// derrière la seam `MediaPlayer` (ADR-0024) — backend
/// via FFI (`onionmedia_ffi.dll`). Les frames RGBA
/// sont tirées par polling (`onionmedia_frame_seq`/`pull`, patron
/// `dirty_rects`) puis décodées en `ui.Image` — l'audio joue en
/// natif dans la DLL (cpal). Aucun codec n'est embarqué côté
/// Dart : un échec d'ouverture remonte en erreur de claim.
class OnionMediaPlayer implements MediaPlayer {
  OnionMediaPlayer._(this._lib, this._h);

  final ffi.DynamicLibrary _lib;
  final ffi.Pointer _h;

  final List<void Function()> _listeners = [];
  Timer? _pump;
  ui.Image? _frame;
  int _lastSeq = -1;
  bool _disposed = false;

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

  String _lastError() {
    final buf = calloc<ffi.Uint8>(512);
    try {
      final n = _qtavErr(buf, 512);
      return n > 0 ? String.fromCharCodes(buf.asTypedList(n)) : '';
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

  /// Ouvre `url` (http(s) proxyfiée ou chemin `file`/brut).
  /// `StateError` si la DLL ou l'ouverture échoue — la factory
  /// échoue, l'erreur remonte au claim.
  static Future<MediaPlayer> create(
    String url, {
    bool hwAccel = false,
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
    // Décodage matériel opt-in : l'export
    // `onionmedia_set_hwaccel` peut ne pas exister encore —
    // lookup gardé, ignoré silencieusement si absent.
    for (final sym in ['onionmedia_set_hw_accel', 'onionmedia_set_hwaccel']) {
      try {
        final setHw = lib.lookupFunction<
            ffi.Int32 Function(ffi.Pointer, ffi.Int32),
            int Function(ffi.Pointer, int)>(sym);
        setHw(h, hwAccel ? 1 : 0);
        break;
      } on Object {
        // export absent — HW non piloté, comportement antérieur.
      }
    }
    try {
      final isHttp = url.startsWith('http');
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
      calloc.free(c);
      if (rc != 0) {
        throw StateError('onionmedia_open: ${p._lastError()}');
      }
    } catch (_) {
      p._qtavFree(h);
      rethrow;
    }
    p._startPump();
    return p;
  }

  void _startPump() {
    _pump = Timer.periodic(const Duration(milliseconds: 33), (_) async {
      if (_disposed) return;
      final seq = _qtavSeq(_h);
      if (seq == _lastSeq) return;
      _lastSeq = seq;
      final w = calloc<ffi.Uint32>(1);
      final ht = calloc<ffi.Uint32>(1);
      try {
        _qtavFSize(_h, w, ht);
        final width = w.value;
        final height = ht.value;
        if (width == 0 || height == 0) return;
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
    });
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
  Widget get view => _frame == null
      ? const SizedBox.expand(child: ColoredBox(color: Color(0xFF000000)))
      : SizedBox.expand(
          child: RawImage(image: _frame, fit: BoxFit.contain));

  @override
  bool get ready => _frame != null;

  @override
  MediaSnapshot get snapshot {
    final st = _qtavState(_h);
    final pos = _qtavPos(_h).clamp(0, 1 << 62);
    final dur = _qtavDur(_h);
    final state = switch (st) {
      2 => 'playing',
      3 => 'paused',
      _ when _qtavEof(_h) == 1 => 'ended',
      _ => 'paused',
    };
    return MediaSnapshot(
      positionMs: pos,
      bufferedMs: pos, // tampon interne borné — approximation v1
      durationMs: dur > 0 ? dur : null,
      state: state,
      error: null,
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
  @override
  Future<void> setVolume(double v) async =>
      _qtavSetVol(_h, v.clamp(0.0, 1.0));
  @override
  Future<void> setMuted(bool on) async => _qtavSetMute(_h, on ? 1 : 0);
  @override
  Future<void> setRate(double r) async =>
      _qtavSetSpeed(_h, r.clamp(0.25, 4.0));

  @override
  Future<void> dispose() async {
    if (_disposed) return;
    _disposed = true;
    _pump?.cancel();
    _qtavStop(_h);
    _qtavFree(_h);
    _frame?.dispose();
    _frame = null;
  }
}

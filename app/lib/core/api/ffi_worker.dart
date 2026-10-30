// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Worker isolate FFI (étape 107 — vol humain 22/10 constat
/// « FFI inutilisable sous charge ») : chaque appel natif est
/// bloquant (`block_on` côté Rust — `navigate` peut tenir
/// des dizaines de secondes sous `request_timeout`) et
/// tournait dans l'isolate UI → freezes francs. La bibliothèque
/// et le handle sont **possédés par cet isolate** : tous les
/// appels natifs y restent séquentiels (l'ABI n'est pas
/// thread-safe) et l'UI ne gèle plus.
///
/// Protocole : `[seq, op, arg]` sur le port commande →
/// `{seq, ok, data|err}` sur le port de réponse partagé.
/// `err` = `explorer_last_error` lu **sur ce thread** (TLS
/// valide — c'est le seul endroit où il l'est).
library;

import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:isolate';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

// ---------------------------------------------------------------------------
// Signatures natives — `crates/explorer-ffi` (ADR-0018).
// Miroir de celles que portait `ffi_engine.dart` avant 107.
// ---------------------------------------------------------------------------

typedef _NativeHandle = ffi.Pointer<ffi.Void>;

typedef _CreateN = _NativeHandle Function();
typedef _CreateD = _NativeHandle Function();
typedef _DestroyN = ffi.Void Function(_NativeHandle);
typedef _DestroyD = void Function(_NativeHandle);
typedef _OpenTabN = ffi.Uint64 Function(_NativeHandle);
typedef _OpenTabD = int Function(_NativeHandle);
typedef _TabCodeN = ffi.Int32 Function(_NativeHandle, ffi.Uint64);
typedef _TabCodeD = int Function(_NativeHandle, int);
typedef _NavigateN = ffi.Int32 Function(
  _NativeHandle,
  ffi.Uint64,
  ffi.Pointer<ffi.Uint8>,
  ffi.UintPtr,
);
typedef _NavigateD = int Function(
  _NativeHandle,
  int,
  ffi.Pointer<ffi.Uint8>,
  int,
);
typedef _ViewportN = ffi.Int32 Function(
  _NativeHandle,
  ffi.Float,
  ffi.Float,
  ffi.Float,
);
typedef _ViewportD = int Function(_NativeHandle, double, double, double);
typedef _ColorSchemeN = ffi.Int32 Function(_NativeHandle, ffi.Int32);
typedef _ColorSchemeD = int Function(_NativeHandle, int);
typedef _SetLangN = ffi.Int32 Function(
  _NativeHandle,
  ffi.Pointer<ffi.Uint8>,
  ffi.UintPtr,
);
typedef _SetLangD = int Function(_NativeHandle, ffi.Pointer<ffi.Uint8>, int);
typedef _InputN = ffi.Int32 Function(
  _NativeHandle,
  ffi.Uint64,
  ffi.Uint8,
  ffi.Float,
  ffi.Float,
  ffi.Pointer<ffi.Uint8>,
  ffi.UintPtr,
  ffi.Uint8,
);
typedef _InputD = int Function(
  _NativeHandle,
  int,
  int,
  double,
  double,
  ffi.Pointer<ffi.Uint8>,
  int,
  int,
);
typedef _FrameSizeN = ffi.Int32 Function(
  _NativeHandle,
  ffi.Uint64,
  ffi.Pointer<ffi.Uint32>,
  ffi.Pointer<ffi.Uint32>,
);
typedef _FrameSizeD = int Function(
  _NativeHandle,
  int,
  ffi.Pointer<ffi.Uint32>,
  ffi.Pointer<ffi.Uint32>,
);
typedef _FrameCopyN = ffi.IntPtr Function(
  _NativeHandle,
  ffi.Uint64,
  ffi.Pointer<ffi.Uint8>,
  ffi.UintPtr,
);
typedef _FrameCopyD = int Function(
  _NativeHandle,
  int,
  ffi.Pointer<ffi.Uint8>,
  int,
);
typedef _BufReadN = ffi.IntPtr Function(ffi.Pointer<ffi.Uint8>, ffi.UintPtr);
typedef _BufReadD = int Function(ffi.Pointer<ffi.Uint8>, int);
typedef _TabUrlN = ffi.IntPtr Function(
  _NativeHandle,
  ffi.Uint64,
  ffi.Pointer<ffi.Uint8>,
  ffi.UintPtr,
);
typedef _TabUrlD = int Function(
  _NativeHandle,
  int,
  ffi.Pointer<ffi.Uint8>,
  int,
);
typedef _TabScrollN = ffi.Double Function(_NativeHandle, ffi.Uint64);
typedef _TabScrollD = double Function(_NativeHandle, int);
typedef _FrameCopyRegionN = ffi.IntPtr Function(
  _NativeHandle,
  ffi.Uint64,
  ffi.Uint32,
  ffi.Uint32,
  ffi.Uint32,
  ffi.Uint32,
  ffi.Pointer<ffi.Uint8>,
  ffi.UintPtr,
);
typedef _FrameCopyRegionD = int Function(
  _NativeHandle,
  int,
  int,
  int,
  int,
  int,
  ffi.Pointer<ffi.Uint8>,
  int,
);
typedef _DirtyRectsN = ffi.IntPtr Function(
  _NativeHandle,
  ffi.Uint64,
  ffi.Pointer<ffi.Float>,
  ffi.UintPtr,
);
typedef _DirtyRectsD = int Function(
  _NativeHandle,
  int,
  ffi.Pointer<ffi.Float>,
  int,
);
// Seam MediaSurface (étape 71e) — drain NDJSON + fiches +
// claim/release/state, mêmes formes que les accesseurs
// chaîne existants.
typedef _MediaEventsN = ffi.IntPtr Function(
  _NativeHandle,
  ffi.Pointer<ffi.Uint8>,
  ffi.UintPtr,
);
typedef _MediaEventsD = int Function(
  _NativeHandle,
  ffi.Pointer<ffi.Uint8>,
  int,
);
typedef _MediaReleaseN = ffi.Int32 Function(
  _NativeHandle,
  ffi.Uint64,
  ffi.Uint64,
);
typedef _MediaReleaseD = int Function(_NativeHandle, int, int);
typedef _MediaClaimN = ffi.Int32 Function(
  _NativeHandle,
  ffi.Uint64,
  ffi.Uint64,
  ffi.Pointer<ffi.Uint8>,
  ffi.UintPtr,
);
typedef _MediaClaimD = int Function(
  _NativeHandle,
  int,
  int,
  ffi.Pointer<ffi.Uint8>,
  int,
);

// Pont favoris (PR-10 suite) — exports additifs : dll
// antérieure → stubs côté engine, listes vides.
typedef _BmListN = ffi.IntPtr Function(
    _NativeHandle, ffi.Pointer<ffi.Uint8>, ffi.UintPtr);
typedef _BmListD = int Function(
    _NativeHandle, ffi.Pointer<ffi.Uint8>, int);
typedef _BmAddN = ffi.Int32 Function(
    _NativeHandle, ffi.Pointer<ffi.Uint8>, ffi.UintPtr,
    ffi.Pointer<ffi.Uint8>, ffi.UintPtr);
typedef _BmAddD = int Function(
    _NativeHandle, ffi.Pointer<ffi.Uint8>, int,
    ffi.Pointer<ffi.Uint8>, int);

// ---------------------------------------------------------------------------
// Ops RPC — les entiers sont le contrat wire, ne pas réordonner.
// ---------------------------------------------------------------------------

const opOpenTab = 0;
const opCloseTab = 1;
const opNavigate = 2;
const opReload = 3;
const opGoBack = 4;
const opGoForward = 5;
const opSetViewport = 6;
const opInput = 7;
const opTick = 8;
const opFrame = 9;
const opSetColorScheme = 10;
const opShutdown = 11;
const opFrameRegion = 12;
const opMediaEvents = 13;
const opMediaList = 14;
const opMediaClaim = 15;
const opMediaRelease = 16;
const opMediaState = 17;
const opSetAcceptLanguage = 18;
const opBookmarks = 19;
const opBookmarkAdd = 20;
const opBookmarkDel = 21;

/// Point d'entrée de l'isolate — `args` = `{'reply': SendPort,
/// 'path': String}`. Envoie `{'port': SendPort}` puis
/// `{'ready': bool, 'err'?}` ; si `ready` est faux le worker
/// meurt (DLL ouverte côté UI mais session non créable).
void ffiWorkerMain(Map<String, Object?> args) {
  final reply = args['reply']! as SendPort;
  final cmd = ReceivePort();
  reply.send({'port': cmd.sendPort});
  final _WorkerImpl w;
  try {
    w = _WorkerImpl.open(args['path']! as String);
  } on Object catch (e) {
    reply.send({'ready': false, 'err': '$e'});
    cmd.close();
    return;
  }
  reply.send(const {'ready': true});
  cmd.listen((msg) {
    final m = msg! as List<Object?>;
    final seq = m[0]! as int;
    final op = m[1]! as int;
    Object? data;
    Object? err;
    try {
      data = w.exec(op, m[2]);
    } on Object catch (e) {
      err = '$e';
    }
    reply.send({'seq': seq, 'ok': err == null, 'data': data, 'err': err});
    if (op == opShutdown) {
      cmd.close(); // le port fermé laisse l'isolate mourir
    }
  });
}

/// L'ensemble des liaisons natives — un appelant unique,
/// sur un seul thread, dans un seul isolate.
class _WorkerImpl {
  _WorkerImpl._(
    this._lib,
    this._handle,
    this._destroy,
    this._openTab,
    this._closeTab,
    this._navigate,
    this._reload,
    this._goBack,
    this._goForward,
    this._setViewport,
    this._input,
    this._tick,
    this._frameSize,
    this._frameCopy,
    this._frameCopyRegion,
    this._dirtyRects,
    this._tabUrl,
    this._tabTitle,
    this._tabScroll,
    this._setColorScheme,
    this._lastError,
    this._mediaEvents,
    this._mediaList,
    this._mediaClaim,
    this._mediaRelease,
    this._mediaState,
    this._setAcceptLanguage,
  );

  final ffi.DynamicLibrary _lib;
  final _NativeHandle _handle;
  final _DestroyD _destroy;
  final _OpenTabD _openTab;
  final _TabCodeD _closeTab;
  final _NavigateD _navigate;
  final _TabCodeD _reload;
  final _TabCodeD _goBack;
  final _TabCodeD _goForward;
  final _ViewportD _setViewport;
  final _InputD _input;
  final _TabCodeD _tick;
  final _FrameSizeD _frameSize;
  final _FrameCopyD _frameCopy;
  final _FrameCopyRegionD? _frameCopyRegion;
  final _DirtyRectsD? _dirtyRects;
  final _TabUrlD _tabUrl;
  final _TabUrlD? _tabTitle;
  final _TabScrollD? _tabScroll;
  final _ColorSchemeD? _setColorScheme;
  final _BufReadD _lastError;
  // Seam MediaSurface (étape 71e) — `null` sur une dll
  // antérieure → stubs honnêtes côté appelant.
  // Pont favoris — exports additifs (dll antérieure → null).
  static _BmListD? _bookmarks;
  static _BmAddD? _bookmarkAdd;
  static _SetLangD? _bookmarkDel;
  final _MediaEventsD? _mediaEvents;
  final _TabUrlD? _mediaList;
  final _MediaClaimD? _mediaClaim;
  final _MediaReleaseD? _mediaRelease;
  final _MediaClaimD? _mediaState;
  // Export additif étape 118 — `null` sur dll antérieure.
  final _SetLangD? _setAcceptLanguage;

  static _WorkerImpl open(String path) {
    final lib = ffi.DynamicLibrary.open(path);
    try {
      final create = lib.lookupFunction<_CreateN, _CreateD>('explorer_create');
      final lastError = lib.lookupFunction<_BufReadN, _BufReadD>(
        'explorer_last_error',
      );
      // Exports optionnels (ABI antérieure, étape 61/97) :
      // lookup échoue proprement → `null`, jamais d'exception.
      _TabUrlD? tabTitle;
      try {
        tabTitle = lib.lookupFunction<_TabUrlN, _TabUrlD>('explorer_tab_title');
      } on Object {
        /* export absent */
      }
      _TabScrollD? tabScroll;
      try {
        tabScroll = lib.lookupFunction<_TabScrollN, _TabScrollD>(
          'explorer_tab_scroll',
        );
      } on Object {
        /* export absent */
      }
      _ColorSchemeD? setColorScheme;
      try {
        setColorScheme = lib.lookupFunction<_ColorSchemeN, _ColorSchemeD>(
          'explorer_set_color_scheme',
        );
      } on Object {
        /* export absent */
      }
      // Exports pull partiel (étape 55, câblés UI en 109) —
      // absents d'une dll antérieure → pull plein en repli.
      _FrameCopyRegionD? frameCopyRegion;
      try {
        frameCopyRegion = lib.lookupFunction<_FrameCopyRegionN,
            _FrameCopyRegionD>('explorer_frame_copy_region');
      } on Object {
        /* export absent */
      }
      _DirtyRectsD? dirtyRects;
      try {
        dirtyRects = lib.lookupFunction<_DirtyRectsN, _DirtyRectsD>(
          'explorer_dirty_rects',
        );
      } on Object {
        /* export absent */
      }
      // Exports `MediaSurface` (étape 71e) — dll antérieure
      // → `null` → l'hôte retombe sur les stubs honnêtes.
      _MediaEventsD? mediaEvents;
      try {
        mediaEvents = lib.lookupFunction<_MediaEventsN, _MediaEventsD>(
          'explorer_media_events',
        );
      } on Object {
        /* export absent */
      }
      _TabUrlD? mediaList;
      try {
        mediaList = lib.lookupFunction<_TabUrlN, _TabUrlD>(
          'explorer_media_list',
        );
      } on Object {
        /* export absent */
      }
      _MediaClaimD? mediaClaim;
      try {
        mediaClaim = lib.lookupFunction<_MediaClaimN, _MediaClaimD>(
          'explorer_media_claim',
        );
      } on Object {
        /* export absent */
      }
      _MediaReleaseD? mediaRelease;
      try {
        mediaRelease = lib.lookupFunction<_MediaReleaseN, _MediaReleaseD>(
          'explorer_media_release',
        );
      } on Object {
        /* export absent */
      }
      try {
        _bookmarks = lib.lookupFunction<_BmListN, _BmListD>(
          'explorer_bookmarks',
        );
      } on Object {
        /* export absent */
      }
      try {
        _bookmarkAdd = lib.lookupFunction<_BmAddN, _BmAddD>(
          'explorer_bookmark_add',
        );
      } on Object {
        /* export absent */
      }
      try {
        _bookmarkDel = lib.lookupFunction<_SetLangN, _SetLangD>(
          'explorer_bookmark_del',
        );
      } on Object {
        /* export absent */
      }
      _MediaClaimD? mediaState;
      try {
        mediaState = lib.lookupFunction<_MediaClaimN, _MediaClaimD>(
          'explorer_media_state',
        );
      } on Object {
        /* export absent */
      }
      // Export `Accept-Language` (étape 118) — dll
      // antérieure → `null` → en-tête jamais posé.
      _SetLangD? setAcceptLanguage;
      try {
        setAcceptLanguage = lib.lookupFunction<_SetLangN, _SetLangD>(
          'explorer_set_accept_language',
        );
      } on Object {
        /* export absent */
      }
      final handle = create();
      if (handle == ffi.nullptr) {
        throw _LastError.read(lastError);
      }
      return _WorkerImpl._(
        lib,
        handle,
        lib.lookupFunction<_DestroyN, _DestroyD>('explorer_destroy'),
        lib.lookupFunction<_OpenTabN, _OpenTabD>('explorer_open_tab'),
        lib.lookupFunction<_TabCodeN, _TabCodeD>('explorer_close_tab'),
        lib.lookupFunction<_NavigateN, _NavigateD>('explorer_navigate'),
        lib.lookupFunction<_TabCodeN, _TabCodeD>('explorer_reload'),
        lib.lookupFunction<_TabCodeN, _TabCodeD>('explorer_go_back'),
        lib.lookupFunction<_TabCodeN, _TabCodeD>('explorer_go_forward'),
        lib.lookupFunction<_ViewportN, _ViewportD>('explorer_set_viewport'),
        lib.lookupFunction<_InputN, _InputD>('explorer_input'),
        lib.lookupFunction<_TabCodeN, _TabCodeD>('explorer_tick'),
        lib.lookupFunction<_FrameSizeN, _FrameSizeD>('explorer_frame_size'),
        lib.lookupFunction<_FrameCopyN, _FrameCopyD>('explorer_frame_copy'),
        frameCopyRegion,
        dirtyRects,
        lib.lookupFunction<_TabUrlN, _TabUrlD>('explorer_tab_url'),
        tabTitle,
        tabScroll,
        setColorScheme,
        lastError,
        mediaEvents,
        mediaList,
        mediaClaim,
        mediaRelease,
        mediaState,
        setAcceptLanguage,
      );
    } on Object {
      lib.close();
      rethrow;
    }
  }

  /// `explorer_last_error` — lisible uniquement ici (TLS du
  /// thread de l'isolate).
  String lastError() => _LastError.read(_lastError).message;

  Object? exec(int op, Object? a) {
    final m = (a is Map) ? a : const <String, Object?>{};
    switch (op) {
      case opOpenTab:
        return {'id': _openTab(_handle)};
      case opCloseTab:
        _closeTab(_handle, _tab(m));
        return null;
      case opNavigate:
        final bytes = (m['url'] as String? ?? '').toNativeUtf8();
        try {
          _must(
            _navigate(_handle, _tab(m), bytes.cast<ffi.Uint8>(), bytes.length),
          );
        } finally {
          calloc.free(bytes);
        }
        return _refreshed(m);
      case opReload:
      case opGoBack:
      case opGoForward:
        final f = op == opReload
            ? _reload
            : op == opGoBack
            ? _goBack
            : _goForward;
        _must(f(_handle, _tab(m)));
        return _refreshed(m);
      case opSetViewport:
        _must(
          _setViewport(_handle, _num(m, 'w'), _num(m, 'h'), _num(m, 'scale')),
        );
        return null;
      case opInput:
        return _inputOp(m);
      case opTick:
        // Ét.109 — le tick drain aussi `scroll_y` + dirty
        // rects dans le même RPC : l'UI consomme les régions
        // (pull partiel) et la position pour la traduction
        // locale. Exports absents → `null` → pull plein.
        // Ét.71e — et les events `MediaSurface` accumulés
        // (NDJSON → même dispatch que le SSE côté UI).
        final tab = _tab(m);
        return {
          'invalidated': _tick(_handle, tab) == 1,
          'scroll_y': _tabScroll?.call(_handle, tab),
          'rects': _drainDirty(tab),
          'media': _drainMediaEvents(),
        };
      case opMediaEvents:
        return {'events': _drainMediaEvents()};
      case opMediaList:
        return {'media': _readMediaList(_tab(m))};
      case opMediaClaim:
        return {'code': _mediaCall(m, _mediaClaim)};
      case opMediaRelease:
        final f = _mediaRelease;
        if (f == null) return const {'code': -1};
        return {
          'code': f(_handle, _tab(m), (m['media'] as num?)?.toInt() ?? 0),
        };
      case opMediaState:
        return {'code': _mediaCall(m, _mediaState)};
      case opBookmarks:
        return {'bookmarks': _readBookmarks()};
      case opBookmarkAdd:
        final f = _bookmarkAdd;
        if (f == null) return const {'code': -1};
        final u = (m['url'] as String? ?? '').toNativeUtf8();
        final t = (m['title'] as String? ?? '').toNativeUtf8();
        try {
          return {
            'code': f(
              _handle,
              u.cast<ffi.Uint8>(), u.length,
              t.cast<ffi.Uint8>(), t.length,
            ),
          };
        } finally {
          calloc.free(u);
          calloc.free(t);
        }
      case opBookmarkDel:
        final f = _bookmarkDel;
        if (f == null) return const {'code': -1};
        final u = (m['url'] as String? ?? '').toNativeUtf8();
        try {
          return {'code': f(_handle, u.cast<ffi.Uint8>(), u.length)};
        } finally {
          calloc.free(u);
        }
      case opFrame:
        return _frameOp(m);
      case opFrameRegion:
        return _frameRegionOp(m);
      case opSetColorScheme:
        // Export additif étape 97 — dll antérieure : no-op.
        _setColorScheme?.call(_handle, m['dark'] == true ? 1 : 0);
        return null;
      case opSetAcceptLanguage:
        // Export additif étape 118 — dll antérieure : no-op
        // (l'en-tête n'est jamais posé, comportement inchangé).
        final f = _setAcceptLanguage;
        if (f != null) {
          final bytes = (m['lang'] as String? ?? '').toNativeUtf8();
          try {
            f(_handle, bytes.cast<ffi.Uint8>(), bytes.length);
          } finally {
            calloc.free(bytes);
          }
        }
        return null;
      case opShutdown:
        _destroy(_handle);
        _lib.close();
        return null;
      default:
        throw ArgumentError('op FFI inconnu : $op');
    }
  }

  int _tab(Map m) => (m['tab'] as num?)?.toInt() ?? 0;
  double _num(Map m, String k) => (m[k] as num?)?.toDouble() ?? 0;

  /// `tab_url` + `tab_title` relus après l'action — un seul
  /// RPC retourne l'onglet rafraîchi (URL résolue après
  /// redirections, titre étape 61).
  Map<String, Object?> _refreshed(Map m) => {
    'url': _readUrl(_tab(m)) ?? 'about:blank',
    'title': _readTitle(_tab(m)) ?? '',
  };

  String? _readUrl(int id) {
    const cap = 8192;
    final buf = calloc<ffi.Uint8>(cap);
    try {
      final n = _tabUrl(_handle, id, buf, cap);
      if (n <= 0) return null;
      return String.fromCharCodes(buf.asTypedList(n));
    } finally {
      calloc.free(buf);
    }
  }

  String? _readTitle(int id) {
    final f = _tabTitle;
    if (f == null) return null;
    const cap = 2048;
    final buf = calloc<ffi.Uint8>(cap);
    try {
      final n = f(_handle, id, buf, cap);
      if (n <= 0) return '';
      return String.fromCharCodes(buf.asTypedList(n));
    } finally {
      calloc.free(buf);
    }
  }

  /// `input` — même logique qu'avant 107, portée ici : la
  /// map `ev` (identique au contrat REST `POST …/input`) est
  /// réduite en kind/x/y/key/mods ; `scroll_y` joint au
  /// retour `scrolled` quand l'export existe.
  Map<String, Object?> _inputOp(Map m) {
    final tab = _tab(m);
    final ev = (m['ev'] as Map?) ?? const {};
    final kind = switch (ev['type']) {
      'click' => 0,
      'mousemove' => 1,
      'scroll' => 2,
      'key' => 3,
      'keyup' => 4,
      _ => -1,
    };
    if (kind < 0) return const {'result': 'ignored'};
    final x = (ev['x'] as num?)?.toDouble() ?? 0;
    final yOrDy = ev['type'] == 'scroll' ? ev['dy'] : ev['y'];
    final y = (yOrDy as num?)?.toDouble() ?? 0;
    final mods =
        (ev['ctrl'] == true ? 1 : 0) |
        (ev['shift'] == true ? 2 : 0) |
        (ev['alt'] == true ? 4 : 0) |
        (ev['meta'] == true ? 8 : 0);
    ffi.Pointer<ffi.Uint8> key = ffi.nullptr;
    ffi.Pointer<Utf8> keyBuf = ffi.nullptr;
    try {
      if (kind == 3 || kind == 4) {
        keyBuf = ('${ev['key'] ?? ''}').toNativeUtf8();
        key = keyBuf.cast<ffi.Uint8>();
      }
      final code = _input(
        _handle,
        tab,
        kind,
        x,
        y,
        key,
        key == ffi.nullptr ? 0 : keyBuf.length,
        mods,
      );
      if (code < 0) throw _LastError.read(_lastError);
      return {
        'result': const [
          'ignored',
          'scrolled',
          'navigated',
          'hover',
          'focused',
          'edited',
          'unfocused',
        ][code],
        // `tab_scroll` joint (étape 61) — même forme que la
        // réponse REST `scrolled`.
        if (code == 1 && _tabScroll != null)
          'scroll_y': _tabScroll(_handle, tab),
      };
    } finally {
      calloc.free(keyBuf);
    }
  }

  /// `frame_size` + `frame_copy` → RGBA brut transférable
  /// (zéro-copie de port — `decodeImageFromPixels` reste
  /// côté UI, seul le transport déménage).
  Map<String, Object?> _frameOp(Map m) {
    final tab = _tab(m);
    final w = calloc<ffi.Uint32>(1);
    final h = calloc<ffi.Uint32>(1);
    try {
      if (_frameSize(_handle, tab, w, h) != 0) {
        return const {'none': true};
      }
      final len = w.value * h.value * 4;
      if (len <= 0) return const {'none': true};
      final px = calloc<ffi.Uint8>(len);
      try {
        final n = _frameCopy(_handle, tab, px, len);
        if (n <= 0) return const {'none': true};
        final list = Uint8List(n)..setAll(0, px.asTypedList(n));
        return {
          'w': w.value,
          'h': h.value,
          'px': TransferableTypedData.fromList([list]),
        };
      } finally {
        calloc.free(px);
      }
    } finally {
      calloc
        ..free(w)
        ..free(h);
    }
  }

  /// `frame_copy_region` (étape 55, câblé en 109) — la taille
  /// effective est bornée côté appelant (`x+w ≤ fw`) : le
  /// retour natif ne porte que le nombre d'octets écrits.
  /// `{'none':true}` si région hors frame ou export absent.
  Map<String, Object?> _frameRegionOp(Map m) {
    final f = _frameCopyRegion;
    if (f == null) return const {'none': true};
    final tab = _tab(m);
    final w = calloc<ffi.Uint32>(1);
    final h = calloc<ffi.Uint32>(1);
    try {
      if (_frameSize(_handle, tab, w, h) != 0) {
        return const {'none': true};
      }
      final x = ((m['x'] as num?)?.toInt() ?? 0).clamp(0, w.value).toInt();
      final y = ((m['y'] as num?)?.toInt() ?? 0).clamp(0, h.value).toInt();
      final rw = ((m['w'] as num?)?.toInt() ?? 0)
          .clamp(0, w.value - x)
          .toInt();
      final rh = ((m['h'] as num?)?.toInt() ?? 0)
          .clamp(0, h.value - y)
          .toInt();
      final len = rw * rh * 4;
      if (len <= 0) return const {'none': true};
      final px = calloc<ffi.Uint8>(len);
      try {
        final n = f(_handle, tab, x, y, rw, rh, px, len);
        if (n <= 0) return const {'none': true};
        final list = Uint8List(n)..setAll(0, px.asTypedList(n));
        return {
          'w': rw,
          'h': rh,
          'px': TransferableTypedData.fromList([list]),
        };
      } finally {
        calloc.free(px);
      }
    } finally {
      calloc
        ..free(w)
        ..free(h);
    }
  }

  /// `dirty_rects` (étape 55) — vide l'accumulateur natif et
  /// rend les quadruplets `(x,y,w,h)` px physiques. Le buffer
  /// est large (un scroll y peint la fenêtre entière — le
  /// drain suffit rarement) ; natif en échec → `null`, l'UI
  /// bascule en pull plein (l'export draine quand même, rien
  /// n'est perdu : le plein couvre toujours).
  /// `media_claim`/`media_state` — corps JSON commun
  /// (`{"codecs":…}` ou le rapport tel quel) → code retour
  /// natif (`0`/`-400`/`-409`/`-410`). Export absent → `-1`
  /// (l'hôte retombe sur la dégradation non claimée).
  int _mediaCall(Map m, _MediaClaimD? f) {
    if (f == null) return -1;
    final body = m['codecs'] is List
        ? jsonEncode({'codecs': m['codecs']})
        : jsonEncode(m['report'] ?? const {});
    final bytes = body.toNativeUtf8();
    try {
      return f(
        _handle,
        _tab(m),
        (m['media'] as num?)?.toInt() ?? 0,
        bytes.cast<ffi.Uint8>(),
        bytes.length,
      );
    } finally {
      calloc.free(bytes);
    }
  }

  /// `media_list` → fiches `{"media":[…]}` décodées — `[]`
  /// si l'export manque ou si aucune surface ne vit.
  List<Object?> _readMediaList(int tab) {
    final f = _mediaList;
    if (f == null) return const [];
    const cap = 64 * 1024;
    final buf = calloc<ffi.Uint8>(cap);
    try {
      final n = f(_handle, tab, buf, cap);
      if (n <= 0) return const [];
      final v = jsonDecode(utf8.decode(buf.asTypedList(n)));
      return v is Map && v['media'] is List ? v['media'] as List : const [];
    } on Object {
      return const [];
    } finally {
      calloc.free(buf);
    }
  }

  /// `explorer_bookmarks` → `[{url,title}]` ; export absent
  /// → `[]` (dll antérieure — stubs côté engine).
  List<Object?> _readBookmarks() {
    final f = _bookmarks;
    if (f == null) return const [];
    const cap = 64 * 1024;
    final buf = calloc<ffi.Uint8>(cap);
    try {
      final n = f(_handle, buf, cap);
      if (n <= 0) return const [];
      final v = jsonDecode(utf8.decode(buf.asTypedList(n)));
      return v is Map && v['bookmarks'] is List
          ? v['bookmarks'] as List
          : const [];
    } on Object {
      return const [];
    } finally {
      calloc.free(buf);
    }
  }

  /// `media_events` → lignes NDJSON décodées en
  /// `[{event, data}]` — le dispatch UI est le même que le
  /// SSE (`ServerEvent(name, data)`). Export absent → `[]`.
  static const _maxMediaEvents = 64 * 1024;

  List<Object?> _drainMediaEvents() {
    final f = _mediaEvents;
    if (f == null) return const [];
    final buf = calloc<ffi.Uint8>(_maxMediaEvents);
    try {
      final n = f(_handle, buf, _maxMediaEvents);
      if (n <= 0) return const [];
      return [
        for (final line in utf8.decode(buf.asTypedList(n)).split('\n'))
          if (line.isNotEmpty)
            jsonDecode(line) as Object?,
      ];
    } on Object {
      // Une ligne invalide ne doit pas perdre le lot —
      // le drain a déjà consommé côté natif.
      return const [];
    } finally {
      calloc.free(buf);
    }
  }

  static const _maxDirty = 512;

  List<List<double>>? _drainDirty(int tab) {
    final f = _dirtyRects;
    if (f == null) return null;
    final buf = calloc<ffi.Float>(_maxDirty * 4);
    try {
      final n = f(_handle, tab, buf, _maxDirty * 4);
      if (n <= 0) return n == 0 ? const [] : null;
      final flat = buf.asTypedList(n * 4);
      return [
        for (var i = 0; i < n; i++)
          [flat[i * 4], flat[i * 4 + 1], flat[i * 4 + 2], flat[i * 4 + 3]],
      ];
    } finally {
      calloc.free(buf);
    }
  }

  void _must(int code) {
    if (code != 0) throw _LastError.read(_lastError);
  }
}

/// `explorer_last_error` formaté en exception — levée sur
/// le thread worker, sérialisée en chaîne dans la réponse.
class _LastError implements Exception {
  const _LastError(this.message);
  final String message;
  static _LastError read(_BufReadD read) {
    final buf = calloc<ffi.Uint8>(512);
    try {
      final n = read(buf, 512);
      return _LastError(
        n <= 0 ? 'erreur inconnue' : String.fromCharCodes(buf.asTypedList(n)),
      );
    } finally {
      calloc.free(buf);
    }
  }

  @override
  String toString() => message;
}

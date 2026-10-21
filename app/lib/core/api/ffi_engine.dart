// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:async';
import 'dart:ffi' as ffi;
import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:ffi/ffi.dart';

import '../../features/browser/data/browser_repository.dart';
import '../../features/browser/domain/bookmark.dart';
import '../../features/browser/domain/browser_tab.dart';
import '../../features/browser/domain/find_result.dart';
import '../../features/browser/domain/media_offer.dart';
import '../../features/browser/domain/history_entry.dart';
import '../../features/browser/domain/reload_result.dart';
import '../config/app_config.dart';

// ---------------------------------------------------------------------------
// Signatures natives — `crates/explorer-ffi` (ADR-0018).
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
typedef _InputN = ffi.Int32 Function(
  _NativeHandle,
  ffi.Uint64,
  ffi.Uint8,
  ffi.Float,
  ffi.Float,
  ffi.Pointer<ffi.Uint8>,
  ffi.UintPtr,
);
typedef _InputD = int Function(
  _NativeHandle,
  int,
  int,
  double,
  double,
  ffi.Pointer<ffi.Uint8>,
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

/// Erreur remontée par `explorer-ffi` — détail via
/// `explorer_last_error`.
class FfiException implements Exception {
  const FfiException(this.message);
  final String message;
  @override
  String toString() => 'FfiException: $message';
}

/// Backend moteur **in-process** : charge `explorer_ffi.dll`
/// via `dart:ffi` et expose la même surface que
/// [BrowserRepository] (ADR-0018 — l'UI reste découplée du
/// moteur, seul le transport change : appels natifs au lieu
/// de REST/SSE loopback).
///
/// Différences assumées (déviations documentées dans l'ADR) :
/// - pas de SSE → le contrôleur sonde `tick` périodiquement ;
/// - titre d'onglet et offset de scroll : exports ABI de
///   l'étape 61 — une dll antérieure garde le repli URL ;
/// - `fetchFrame` produit un PNG en ré-encodant l'`ui.Image`
///   (compat interface) ; le chemin rapide est
///   [fetchFrameImage] — `ui.Image` sans aucun codec.
class FfiEngine implements BrowserRepository {
  FfiEngine._(ffi.DynamicLibrary lib)
    : _create = lib.lookupFunction<_CreateN, _CreateD>('explorer_create'),
      _destroy = lib.lookupFunction<_DestroyN, _DestroyD>('explorer_destroy'),
      _openTab = lib.lookupFunction<_OpenTabN, _OpenTabD>('explorer_open_tab'),
      _closeTab = lib.lookupFunction<_TabCodeN, _TabCodeD>(
        'explorer_close_tab',
      ),
      _navigate = lib.lookupFunction<_NavigateN, _NavigateD>(
        'explorer_navigate',
      ),
      _reload = lib.lookupFunction<_TabCodeN, _TabCodeD>('explorer_reload'),
      _goBack = lib.lookupFunction<_TabCodeN, _TabCodeD>('explorer_go_back'),
      _goForward = lib.lookupFunction<_TabCodeN, _TabCodeD>(
        'explorer_go_forward',
      ),
      _setViewport = lib.lookupFunction<_ViewportN, _ViewportD>(
        'explorer_set_viewport',
      ),
      _input = lib.lookupFunction<_InputN, _InputD>('explorer_input'),
      _tick = lib.lookupFunction<_TabCodeN, _TabCodeD>('explorer_tick'),
      _frameSize = lib.lookupFunction<_FrameSizeN, _FrameSizeD>(
        'explorer_frame_size',
      ),
      _frameCopy = lib.lookupFunction<_FrameCopyN, _FrameCopyD>(
        'explorer_frame_copy',
      ),
      _tabUrl = lib.lookupFunction<_TabUrlN, _TabUrlD>('explorer_tab_url'),
      // Étape 61 — ABI additive : une dll antérieure sans
      // ces exports garde les comportements dégradés
      // (URL en guise de titre, pas d'offset de scroll).
      _tabTitle = lib.providesSymbol('explorer_tab_title')
          ? lib.lookupFunction<_TabUrlN, _TabUrlD>('explorer_tab_title')
          : null,
      _tabScroll = lib.providesSymbol('explorer_tab_scroll')
          ? lib.lookupFunction<_TabScrollN, _TabScrollD>('explorer_tab_scroll')
          : null,
      // Étape 97 — ABI additive : une dll antérieure garde
      // `light` (comportement inchangé).
      _setColorScheme = lib.providesSymbol('explorer_set_color_scheme')
          ? lib.lookupFunction<_ColorSchemeN, _ColorSchemeD>(
              'explorer_set_color_scheme',
            )
          : null,
      _lastError = lib.lookupFunction<_BufReadN, _BufReadD>(
        'explorer_last_error',
      ) {
    _handle = _create();
    if (_handle == ffi.nullptr) {
      throw FfiException(lastError());
    }
  }

  /// Tente de charger la bibliothèque dynamique — `null` si
  /// absente ou si la session n'a pas pu être créée (mode
  /// dégradé → REST, jamais d'exception hors du `tryLoad`).
  static FfiEngine? tryLoad(AppConfig config) {
    if (config.engineMode == 'rest') return null;
    for (final candidate in _candidates(config)) {
      try {
        return FfiEngine._(ffi.DynamicLibrary.open(candidate));
      } on Object {
        // Candidat absent ou ABI incompatible → suivant.
      }
    }
    return null;
  }

  static List<String> _candidates(AppConfig config) {
    if (config.engineLibPath.isNotEmpty) return [config.engineLibPath];
    final exeDir = File(Platform.resolvedExecutable).parent.path;
    final names = Platform.isWindows
        ? ['explorer_ffi.dll']
        : Platform.isMacOS
        ? ['libexplorer_ffi.dylib']
        : ['libexplorer_ffi.so'];
    return [
      for (final n in names) ...[
        // À côté de l'exécutable (déploiement).
        '$exeDir${Platform.pathSeparator}$n',
        // Recherche standard (PATH / ldconfig).
        n,
        // Build de dev du workspace (`cargo build -p explorer-ffi`).
        '..${Platform.pathSeparator}target${Platform.pathSeparator}debug'
            '${Platform.pathSeparator}$n',
      ],
    ];
  }

  final _CreateD _create;
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
  final _TabUrlD _tabUrl;
  final _TabUrlD? _tabTitle;
  final _ColorSchemeD? _setColorScheme;
  final _TabScrollD? _tabScroll;
  final _BufReadD _lastError;

  late _NativeHandle _handle;

  /// Onglets connus — le FFI n'a pas de `GET /tabs`, la liste
  /// vit ici (méthode-surface identique au repo REST).
  final List<BrowserTab> _tabs = [];

  /// Libère la session native. Idempotent.
  void dispose() {
    if (_handle != ffi.nullptr) {
      _destroy(_handle);
      _handle = ffi.nullptr;
    }
  }

  /// Détail de la dernière erreur native du thread courant.
  String lastError() {
    final buf = calloc<ffi.Uint8>(512);
    try {
      final n = _lastError(buf, 512);
      if (n <= 0) return 'erreur inconnue';
      return String.fromCharCodes(buf.asTypedList(n));
    } finally {
      calloc.free(buf);
    }
  }

  Never _fail() => throw FfiException(lastError());

  int _idOf(String tabId) => int.tryParse(tabId) ?? -1;

  // ------------------------------------------------------------------
  // Surface BrowserRepository
  // ------------------------------------------------------------------

  @override
  Future<BrowserTab> createTab() async {
    final id = _openTab(_handle);
    if (id == 0) _fail();
    final tab = BrowserTab(id: '$id');
    _tabs.add(tab);
    return tab;
  }

  @override
  Future<List<BrowserTab>> listTabs() async => List.of(_tabs);

  @override
  Future<BrowserTab?> navigate(String tabId, String url) async {
    final id = _idOf(tabId);
    final bytes = url.toNativeUtf8();
    try {
      if (_navigate(_handle, id, bytes.cast<ffi.Uint8>(), bytes.length) != 0) {
        _fail();
      }
    } finally {
      calloc.free(bytes);
    }
    return _refreshTab(id, url: url);
  }

  /// Re-lit l'URL réelle (redirections) et le `<title>`
  /// (étape 61) puis met à jour la liste des onglets.
  BrowserTab _refreshTab(int id, {String? url}) {
    var resolved = url;
    if (resolved == null || resolved.isEmpty) {
      resolved = _readUrl(id) ?? 'about:blank';
    }
    final title = _readTitle(id) ?? '';
    final idx = _tabs.indexWhere((t) => t.id == '$id');
    if (idx >= 0) {
      _tabs[idx] = _tabs[idx].copyWith(url: resolved, title: title);
      return _tabs[idx];
    }
    return BrowserTab(id: '$id', url: resolved, title: title);
  }

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

  /// Titre via `explorer_tab_title` — `null` si l'export est
  /// absent (dll antérieure, étape 61).
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

  /// Chemin in-process : `Session::reload` rejoue la requête
  /// d'origine directement (le verrou `resubmit_confirm` vit
  /// dans l'API REST — étape 93) ; `confirm` est sans effet.
  @override
  Future<ReloadResult> reload(String tabId, {bool confirm = false}) async {
    final tab = await _action(tabId, _reload);
    return ReloadResult.tab(tab);
  }

  @override
  Future<BrowserTab?> back(String tabId) => _action(tabId, _goBack);

  @override
  Future<BrowserTab?> forward(String tabId) => _action(tabId, _goForward);

  Future<BrowserTab?> _action(String tabId, _TabCodeD f) async {
    final id = _idOf(tabId);
    if (f(_handle, id) != 0) _fail();
    return _refreshTab(id);
  }

  @override
  Future<void> closeTab(String tabId) async {
    final id = _idOf(tabId);
    _closeTab(_handle, id);
    _tabs.removeWhere((t) => t.id == tabId);
  }

  @override
  Future<List<int>?> fetchFrame(String tabId) async {
    final img = await fetchFrameImage(tabId);
    if (img == null) return null;
    try {
      final data = await img.toByteData(format: ui.ImageByteFormat.png);
      return data?.buffer.asUint8List();
    } finally {
      img.dispose();
    }
  }

  /// Frame brute → `ui.Image` RGBA8888 — **le** gain de
  /// l'étape 47 : aucun encode/décode PNG, aucune socket.
  /// `null` si l'onglet n'a jamais rendu.
  Future<ui.Image?> fetchFrameImage(String tabId) async {
    final id = _idOf(tabId);
    final w = calloc<ffi.Uint32>(1);
    final h = calloc<ffi.Uint32>(1);
    try {
      if (_frameSize(_handle, id, w, h) != 0) return null;
      final len = w.value * h.value * 4;
      if (len <= 0) return null;
      final px = calloc<ffi.Uint8>(len);
      try {
        final n = _frameCopy(_handle, id, px, len);
        if (n <= 0) return null;
        // Une seule copie Dart ; `decodeImageFromPixels` la
        // transfère au GPU sans codec intermédiaire.
        final list = Uint8List(n)..setAll(0, px.asTypedList(n));
        final done = Completer<ui.Image>();
        ui.decodeImageFromPixels(
          list,
          w.value,
          h.value,
          ui.PixelFormat.rgba8888,
          done.complete,
        );
        return await done.future;
      } finally {
        calloc.free(px);
      }
    } finally {
      calloc
        ..free(w)
        ..free(h);
    }
  }

  @override
  Future<Map<String, dynamic>> sendInput(
    String tabId,
    Map<String, dynamic> ev,
  ) async {
    final id = _idOf(tabId);
    final kind = switch (ev['type']) {
      'click' => 0,
      'mousemove' => 1,
      'scroll' => 2,
      'key' => 3,
      _ => -1,
    };
    if (kind < 0) return const {'result': 'ignored'};
    final x = (ev['x'] as num?)?.toDouble() ?? 0;
    final yOrDy = ev['type'] == 'scroll' ? ev['dy'] : ev['y'];
    final y = (yOrDy as num?)?.toDouble() ?? 0;
    ffi.Pointer<ffi.Uint8> key = ffi.nullptr;
    ffi.Pointer<Utf8> keyBuf = ffi.nullptr;
    try {
      if (kind == 3) {
        keyBuf = ('${ev['key'] ?? ''}').toNativeUtf8();
        key = keyBuf.cast<ffi.Uint8>();
      }
      final code = _input(
        _handle,
        id,
        kind,
        x,
        y,
        key,
        key == ffi.nullptr ? 0 : keyBuf.length,
      );
      if (code < 0) _fail();
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
        // Étape 61 : l'offset est re-lu via `tab_scroll`
        // (même forme que la réponse REST `scrolled`).
        if (code == 1 && _tabScroll != null)
          'scroll_y': _tabScroll(_handle, id),
        // L'URL de survol n'est pas exportée par le FFI (MVP,
        // ADR-0018) — le client garde sa barre de statut vide.
      };
    } finally {
      if (keyBuf != ffi.nullptr) calloc.free(keyBuf);
    }
  }

  @override
  Future<void> setViewport(
    String tabId,
    double w,
    double h,
    double scale,
  ) async {
    if (_setViewport(_handle, w, h, scale) != 0) _fail();
  }

  @override
  Future<void> setColorScheme(String tabId, String scheme) async {
    // Export additif étape 97 — dll antérieure : no-op
    // (l'env reste `light`, comportement inchangé).
    _setColorScheme?.call(_handle, scheme == 'dark' ? 1 : 0);
  }

  // ------------------------------------------------------------------
  // Bibliothèque — dégradation documentée (étape 57) : l'ABI
  // n'exporte ni signets ni historique (le store du profil est
  // câblé côté daemon uniquement). Listes vides / no-op → l'UI
  // affiche le panneau vide plutôt que crasher ; à étendre via
  // de nouveaux exports quand le profil FFI existera.
  // ------------------------------------------------------------------

  @override
  Future<List<Bookmark>> bookmarks() async => const [];

  @override
  Future<bool> addBookmark(String url, {String title = ''}) async => false;

  @override
  Future<bool> removeBookmark(String url) async => false;

  @override
  Future<List<HistoryEntry>> history() async => const [];

  /// Find-in-page — même dégradation que la bibliothèque :
  /// l'ABI n'exporte pas la recherche moteur (étape 57bis).
  /// Retour vide → la barre affiche « 0 résultat » plutôt
  /// que crasher ; à étendre via un export `explorer_find`.
  @override
  Future<FindResult> find(String tabId, String query, {int dir = 0}) async =>
      FindResult.empty;

  /// Téléchargements (étape 70) : pas de canal d'acceptation
  /// en mode in-process — le pending resterait bloqué ; la
  /// dégradation honnête est le rejet (rejeu document) —
  /// à étendre via un export `explorer_download_*`.
  @override
  Future<String> acceptDownload(String tabId, int dlId, {String? path}) =>
      throw UnsupportedError('downloads : non exposé en FFI (étape 70)');

  /// Voir [acceptDownload].
  @override
  Future<void> declineDownload(String tabId, int dlId) =>
      throw UnsupportedError('downloads : non exposé en FFI (étape 70)');

  /// Seam MediaSurface (étape 71) : l'ABI n'exporte pas le
  /// cycle claim/state — variante FFI à spécifier au premier
  /// hôte in-process (journal d'interface). Dégradation honnête :
  /// aucune offre listée, les appels tombent en UnsupportedError.
  @override
  Future<List<MediaOffer>> listMedia(String tabId) async => const [];

  /// Voir [listMedia].
  @override
  Future<bool> claimMedia(String tabId, int mediaId, List<String> codecs) =>
      throw UnsupportedError('media : non exposé en FFI (étape 71)');

  /// Voir [listMedia].
  @override
  Future<void> releaseMedia(String tabId, int mediaId) =>
      throw UnsupportedError('media : non exposé en FFI (étape 71)');

  /// Voir [listMedia].
  @override
  Future<void> mediaState(
    String tabId,
    int mediaId,
    Map<String, dynamic> report,
  ) => throw UnsupportedError('media : non exposé en FFI (étape 71)');

  /// Avance les animations moteur — `true` si la frame a été
  /// invalidée (le contrôleur la retire). Remplace le SSE
  /// `frame_invalidated` absent du canal FFI.
  bool tick(String tabId) => _tick(_handle, _idOf(tabId)) == 1;

  /// Marqueur de test : `true` quand le backend est chargé —
  /// sert aussi d'identifiant de mode pour le contrôleur.
  bool get isLoaded => _handle != ffi.nullptr;
}

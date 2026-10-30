// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:async';
import 'dart:ffi' as ffi;
import 'dart:io';
import 'dart:isolate';
import 'dart:typed_data';
import 'dart:ui' as ui;

import '../../features/browser/data/browser_repository.dart';
import '../../features/browser/domain/bookmark.dart';
import '../../features/browser/domain/browser_tab.dart';
import '../../features/browser/domain/find_result.dart';
import '../../features/browser/domain/media_offer.dart';
import '../../features/browser/domain/history_entry.dart';
import '../../features/browser/domain/reload_result.dart';
import '../config/app_config.dart';
import 'ffi_worker.dart';

/// Erreur remontée par `explorer-ffi` — le détail est lu par
/// le worker (`explorer_last_error`, TLS de son thread) et
/// sérialisé dans la réponse RPC.
class FfiException implements Exception {
  const FfiException(this.message);
  final String message;
  @override
  String toString() => 'FfiException: $message';
}

/// Rapport d'invalidation FFI (étape 109) — l'équivalent du
/// SSE `frame_invalidated` : `rects` en px physiques (delta
/// drainé par `explorer_dirty_rects`), `scrollY` en px CSS.
/// `rects == null` → dll antérieure sans l'export (pull
/// plein de repli, comme un `rects` vide contrat REST).
class FfiInvalidation {
  const FfiInvalidation({
    required this.invalidated,
    this.scrollY,
    this.rects,
    this.mediaEvents = const [],
  });

  final bool invalidated;
  final double? scrollY;
  final List<ui.Rect>? rects;

  /// Events `MediaSurface` drainés dans le même RPC (étape
  /// 71e) — `{'event': <nom_sse>, 'data': <charge>}` ;
  /// le dispatch UI est identique au SSE.
  final List<Map<String, Object?>> mediaEvents;
}

/// Backend moteur **in-process** : charge `explorer_ffi.dll`
/// dans un **isolate worker dédié** (étape 107 — vol 22/10 :
/// les appels natifs synchrones gelaient l'UI, `navigate`
/// pouvant tenir des dizaines de secondes sous timeout) et
/// expose la même surface que [BrowserRepository] (ADR-0018 —
/// l'UI reste découplée du moteur, seul le transport change :
/// RPC inter-isolates au lieu de REST/SSE loopback).
///
/// Différences assumées (déviations documentées dans l'ADR) :
/// - pas de SSE → le contrôleur sonde `tick` périodiquement ;
/// - titre d'onglet et offset de scroll : exports ABI de
///   l'étape 61 — une dll antérieure garde le repli URL ;
/// - `fetchFrame` produit un PNG en ré-encodant l'`ui.Image`
///   (compat interface) ; le chemin rapide est
///   [fetchFrameImage] — `ui.Image` sans aucun codec ;
/// - un appel natif réellement bloqué ne peut pas être tué
///   (`Isolate.kill` diffère à la sortie du code natif) —
///   le contrat de l'étape reste : l'UI continue de répondre.
class FfiEngine implements BrowserRepository {
  FfiEngine._(this._worker, this.libPath);

  /// Chemin de la bibliothèque chargée — affiché dans la barre
  /// de statut (étape 119 : le piège `engineMode=auto` était
  /// de charger silencieusement une dll périmée adjacente).
  final String libPath;

  @override
  String get engineLabel => 'FFI $libPath';

  /// Aucune base REST en mode in-process — `…/segments`
  /// n'existe que via le daemon (ADR-0003) : `''` honnête,
  /// un claim MSE échoue proprement plutôt que de pull à vide.
  @override
  String get apiBase => '';

  /// Tente de charger la bibliothèque dynamique — `null` si
  /// absente (mode dégradé → REST, jamais d'exception hors
  /// du `tryLoad`). La sonde (`open` + lookup
  /// `explorer_create`) est volontairement **synchrone** et
  /// légère : un `dlopen`+symbole tient en millisecondes et
  /// garde la signature sans `await` ; le worker reprend le
  /// chemin gagnant et crée la session hors de l'UI.
  static FfiEngine? tryLoad(AppConfig config) {
    if (config.engineMode == 'rest') return null;
    for (final candidate in _candidates(config)) {
      try {
        final lib = ffi.DynamicLibrary.open(candidate);
        // Vérif ABI minimale : le symbole existe → le worker
        // partira sur ce chemin (sa propre `open`+`create`).
        lib.lookup('explorer_create');
        return FfiEngine._(_FfiWorker.start(candidate), candidate);
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

  final _FfiWorker _worker;

  /// Onglets connus — le FFI n'a pas de `GET /tabs`, la liste
  /// vit ici (méthode-surface identique au repo REST).
  final List<BrowserTab> _tabs = [];

  /// Libère la session native. Idempotent.
  void dispose() => _worker.dispose();

  int _idOf(String tabId) => int.tryParse(tabId) ?? -1;

  // ------------------------------------------------------------------
  // Surface BrowserRepository
  // ------------------------------------------------------------------

  @override
  Future<BrowserTab> createTab() async {
    final r = await _worker.call(opOpenTab);
    final id = (r?['id'] as num?)?.toInt() ?? 0;
    if (id == 0) {
      throw const FfiException('explorer_open_tab a échoué');
    }
    final tab = BrowserTab(id: '$id');
    _tabs.add(tab);
    return tab;
  }

  @override
  Future<List<BrowserTab>> listTabs() async => List.of(_tabs);

  @override
  Future<BrowserTab?> navigate(String tabId, String url) async {
    final id = _idOf(tabId);
    final r = await _worker.call(opNavigate, {'tab': id, 'url': url});
    return _refreshTab(
      id,
      url: r?['url'] as String?,
      title: r?['title'] as String?,
    );
  }

  /// Met à jour la liste des onglets avec l'URL résolue et le
  /// `<title>` retournés par le worker (relus après l'action
  /// dans le même RPC — redirections comprises).
  BrowserTab _refreshTab(int id, {String? url, String? title}) {
    final idx = _tabs.indexWhere((t) => t.id == '$id');
    final resolved = (url == null || url.isEmpty) ? 'about:blank' : url;
    final t = (idx >= 0 ? _tabs[idx] : BrowserTab(id: '$id')).copyWith(
      url: resolved,
      title: title ?? '',
    );
    if (idx >= 0) _tabs[idx] = t;
    return t;
  }

  /// Chemin in-process : `Session::reload` rejoue la requête
  /// d'origine directement (le verrou `resubmit_confirm` vit
  /// dans l'API REST — étape 93) ; `confirm` est sans effet.
  @override
  Future<ReloadResult> reload(String tabId, {bool confirm = false}) async {
    final tab = await _action(tabId, opReload);
    return ReloadResult.tab(tab);
  }

  @override
  Future<BrowserTab?> back(String tabId) => _action(tabId, opGoBack);

  @override
  Future<BrowserTab?> forward(String tabId) => _action(tabId, opGoForward);

  Future<BrowserTab?> _action(String tabId, int op) async {
    final id = _idOf(tabId);
    final r = await _worker.call(op, {'tab': id});
    return _refreshTab(
      id,
      url: r?['url'] as String?,
      title: r?['title'] as String?,
    );
  }

  @override
  Future<void> closeTab(String tabId) async {
    final id = _idOf(tabId);
    await _worker.call(opCloseTab, {'tab': id});
    _tabs.removeWhere((t) => t.id == tabId);
  }

  @override
  Future<Uint8List?> fetchFrame(String tabId) async {
    final img = await fetchFrameImage(tabId);
    if (img == null) return null;
    try {
      final data = await img.toByteData(format: ui.ImageByteFormat.png);
      return data?.buffer.asUint8List();
    } finally {
      img.dispose();
    }
  }

  /// Forme PNG du pull partiel (compat interface REST —
  /// ré-encode l'`ui.Image` de région). Le chemin rapide est
  /// [fetchFrameRegionImage], utilisé par le contrôleur.
  @override
  Future<Uint8List?> fetchFrameRegion(
    String tabId,
    int x,
    int y,
    int w,
    int h,
  ) async {
    final img = await fetchFrameRegionImage(tabId, x, y, w, h);
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
  /// Les pixels arrivent par `TransferableTypedData` (zéro-
  /// copie de port) ; `decodeImageFromPixels` reste sur
  /// l'isolate UI (seul endroit où `ui.*` vit).
  /// `null` si l'onglet n'a jamais rendu.
  Future<ui.Image?> fetchFrameImage(String tabId) async {
    final r = await _worker.call(opFrame, {'tab': _idOf(tabId)});
    if (r == null || r['none'] == true) return null;
    final w = (r['w'] as num).toInt();
    final h = (r['h'] as num).toInt();
    final px = (r['px']! as TransferableTypedData).materialize().asUint8List();
    final done = Completer<ui.Image>();
    ui.decodeImageFromPixels(px, w, h, ui.PixelFormat.rgba8888, done.complete);
    return done.future;
  }

  /// Région de frame `(x,y,w,h)` en px physiques → `ui.Image`
  /// (étape 55, câblée UI en 109 — pull partiel piloté par les
  /// `rects` de l'invalidation). `null` si région hors frame
  /// ou dll antérieure sans `explorer_frame_copy_region` —
  /// l'appelant retombe alors en pull plein.
  Future<ui.Image?> fetchFrameRegionImage(
    String tabId,
    int x,
    int y,
    int w,
    int h,
  ) async {
    final r = await _worker.call(opFrameRegion, {
      'tab': _idOf(tabId),
      'x': x,
      'y': y,
      'w': w,
      'h': h,
    });
    if (r == null || r['none'] == true) return null;
    final rw = (r['w'] as num).toInt();
    final rh = (r['h'] as num).toInt();
    final px = (r['px']! as TransferableTypedData).materialize().asUint8List();
    final done = Completer<ui.Image>();
    ui.decodeImageFromPixels(px, rw, rh, ui.PixelFormat.rgba8888, done.complete);
    return done.future;
  }

  @override
  Future<Map<String, dynamic>> sendInput(
    String tabId,
    Map<String, dynamic> ev,
  ) async {
    final r = await _worker.call(opInput, {'tab': _idOf(tabId), 'ev': ev});
    return r == null
        ? const {'result': 'ignored'}
        : Map<String, dynamic>.from(r);
  }

  @override
  Future<void> setViewport(
    String tabId,
    double w,
    double h,
    double scale,
  ) async {
    await _worker.call(opSetViewport, {'w': w, 'h': h, 'scale': scale});
  }

  @override
  Future<void> setColorScheme(String tabId, String scheme) async {
    // Export additif étape 97 — dll antérieure : no-op
    // (l'env reste `light`, comportement inchangé).
    await _worker.call(opSetColorScheme, {'dark': scheme == 'dark'});
  }

  @override
  Future<void> setAcceptLanguage(String lang) async {
    // Export additif étape 118 — dll antérieure : no-op
    // (l'en-tête n'est pas posé, comportement inchangé).
    await _worker.call(opSetAcceptLanguage, {'lang': lang});
  }

  // ------------------------------------------------------------------
  // Bibliothèque — favoris pontés FFI (`explorer_bookmarks`,
  // `_add`, `_del` — exports additifs : dll antérieure → liste
  // vide / code -1). L'historique reste sans export ABI
  // (panneau vide dégradé, étape 57).
  // ------------------------------------------------------------------

  @override
  Future<List<Bookmark>> bookmarks() async {
    final r = await _worker.call(opBookmarks);
    final list = r?['bookmarks'] as List? ?? const [];
    return [
      for (final b in list)
        if (b is Map<String, dynamic>) Bookmark.fromJson(b),
    ];
  }

  @override
  Future<bool> addBookmark(String url, {String title = ''}) async {
    final r =
        await _worker.call(opBookmarkAdd, {'url': url, 'title': title});
    return (r?['code'] as num?)?.toInt() == 1;
  }

  @override
  Future<bool> removeBookmark(String url) async {
    final r = await _worker.call(opBookmarkDel, {'url': url});
    return (r?['code'] as num?)?.toInt() == 1;
  }

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

  /// Seam MediaSurface (étape 71e) : l'ABI `explorer_media_*`
  /// porte le cycle complet — fiches, claim/release/state
  /// (codes `-409`/`-410` = le `409`/`410` REST) et drain
  /// NDJSON des events (dans le tick). Une dll antérieure
  /// retourne `null` → dégradation honnête (offres vides,
  /// claims refusés — les stubs ADR-0023 continuent).
  @override
  Future<List<MediaOffer>> listMedia(String tabId) async {
    final r = await _worker.call(opMediaList, {'tab': _idOf(tabId)});
    final list = r?['media'] as List?;
    return [
      for (final m in list ?? const [])
        if (m is Map) MediaOffer.fromJson(Map<String, dynamic>.from(m)),
    ];
  }

  /// Claim `{codecs}` → `true` si acquis (`code == 0`) ;
  /// `-409` déjà claimée / `-410` morte → `false` (le
  /// contrôleur laisse alors l'offre non claimée).
  @override
  Future<bool> claimMedia(
    String tabId,
    int mediaId,
    List<String> codecs,
  ) async {
    final r = await _worker.call(opMediaClaim, {
      'tab': _idOf(tabId),
      'media': mediaId,
      'codecs': codecs,
    });
    return r?['code'] == 0;
  }

  @override
  Future<void> releaseMedia(String tabId, int mediaId) async {
    await _worker.call(opMediaRelease, {
      'tab': _idOf(tabId),
      'media': mediaId,
    });
  }

  @override
  Future<void> mediaState(
    String tabId,
    int mediaId,
    Map<String, dynamic> report,
  ) async {
    await _worker.call(opMediaState, {
      'tab': _idOf(tabId),
      'media': mediaId,
      'report': report,
    });
  }

  /// Avance les animations moteur et rapporte l'invalidation
  /// enrichie (étape 109) : `invalidated`, `scroll_y` (px CSS)
  /// et `rects` dirty (px physiques, drainées dans le même
  /// RPC — `null` sur dll antérieure → pull plein). Remplace
  /// le SSE `frame_invalidated` absent du canal FFI.
  /// Asynchrone depuis l'étape 107 (le `explorer_tick` tourne
  /// sur le worker — la boucle UI ne bloque plus dessus).
  Future<FfiInvalidation> tick(String tabId) async {
    final r = await _worker.call(opTick, {'tab': _idOf(tabId)});
    return FfiInvalidation(
      invalidated: r?['invalidated'] == true,
      scrollY: (r?['scroll_y'] as num?)?.toDouble(),
      mediaEvents: [
        for (final e in (r?['media'] as List?) ?? const [])
          if (e is Map) Map<String, Object?>.from(e),
      ],
      rects: (r?['rects'] as List?)
          ?.map(
            (q) => ui.Rect.fromLTWH(
              (q[0] as num).toDouble(),
              (q[1] as num).toDouble(),
              (q[2] as num).toDouble(),
              (q[3] as num).toDouble(),
            ),
          )
          .toList(),
    );
  }

  /// Marqueur de test : `true` quand le backend est chargé —
  /// sert aussi d'identifiant de mode pour le contrôleur.
  bool get isLoaded => _worker.alive;
}

// ---------------------------------------------------------------------------
// Client RPC du worker — un port de réponse, appels séquencés.
// ---------------------------------------------------------------------------

class _FfiWorker {
  _FfiWorker._();

  static _FfiWorker start(String libPath) {
    final w = _FfiWorker._();
    w._rx.listen(w._onMessage);
    unawaited(
      Isolate.spawn(ffiWorkerMain, {
        'reply': w._rx.sendPort,
        'path': libPath,
      }).then<void>((iso) => w._iso = iso).catchError((Object e) {
        // Spawn impossible (OOM, plateforme) → tout appel
        // échoue proprement en FfiException.
        w._failReady('spawn isolate : $e');
      }),
    );
    return w;
  }

  final ReceivePort _rx = ReceivePort();
  final Map<int, Completer<Object?>> _pending = {};
  final Completer<void> _ready = Completer<void>();
  late SendPort _cmd;
  Isolate? _iso;
  int _seq = 0;
  bool _dead = false;

  bool get alive => !_dead && _ready.isCompleted;

  void _onMessage(Object? raw) {
    final m = raw! as Map<Object?, Object?>;
    final port = m['port'];
    if (port is SendPort) {
      _cmd = port;
      return;
    }
    if (m.containsKey('ready')) {
      if (m['ready'] == true) {
        if (!_ready.isCompleted) _ready.complete();
      } else {
        _failReady('${m['err']}');
      }
      return;
    }
    final c = _pending.remove(m['seq']);
    if (c == null) return;
    if (m['ok'] == true) {
      c.complete(m['data']);
    } else {
      c.completeError(FfiException('${m['err']}'));
    }
  }

  void _failReady(String err) {
    if (!_ready.isCompleted) _ready.completeError(FfiException(err));
    _dead = true;
    _rx.close();
  }

  /// Un appel → `{'ok','data'}` ou jette [FfiException]. La
  /// réponse attend le worker — jamais l'isolate UI.
  Future<Map<String, Object?>?> call(int op, [Object? arg]) async {
    await _ready.future;
    if (_dead) throw const FfiException('worker FFI arrêté');
    final seq = ++_seq;
    final c = Completer<Object?>();
    _pending[seq] = c;
    _cmd.send([seq, op, arg]);
    final data = await c.future;
    return data == null ? null : Map<String, Object?>.from(data as Map);
  }

  void dispose() {
    if (_dead) return;
    _dead = true;
    try {
      if (_ready.isCompleted) {
        _cmd.send([++_seq, opShutdown, null]); // le worker détruit le handle
      }
    } on Object {
      // Worker mort avant le shutdown — rien à détruire.
    }
    for (final c in _pending.values) {
      c.completeError(const FfiException('moteur FFI disposé'));
    }
    _pending.clear();
    _rx.close();
    // `beforeNextEvent` file le kill **après** le message de
    // shutdown (le handle natif est détruit avant la mort) —
    // un `immediate` pourrait courir devant et fuiter la
    // session Rust. Un worker coincé en natif n'est de toute
    // façon pas interruptible — le contrat de l'étape.
    _iso?.kill(priority: Isolate.beforeNextEvent);
  }
}

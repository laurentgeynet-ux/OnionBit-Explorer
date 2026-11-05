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

/// Pull de frame FFI **atomique** (étape 185, M1) — même
/// sémantique que `FramePull` REST, la charge étant une
/// `ui.Image` au lieu d'octets PNG (aucun codec sur le
/// chemin). `paintedScrollY`/`media` viennent du JSON
/// `explorer_frame_pull` — `null`/vide sur dll antérieure.
class FfiFramePull {
  const FfiFramePull({
    required this.image,
    this.paintedScrollY,
    this.media,
    this.epoch,
    this.top,
  });

  /// Frame RGBA décodée.
  final ui.Image image;

  /// Scroll réellement peint dans la frame (px CSS).
  final double? paintedScrollY;

  /// Surfaces géométriques à l'instant de la peinture —
  /// `null` hors contrat 185 (même sémantique que
  /// `FramePull.media` : ensemble autoritaire quand présent).
  final List<MediaPaintMeta>? media;

  /// Génération de la frame tirée (`e`, étape 188) — à
  /// renvoyer à [fetchFrameTop] pour appairer le slice.
  /// `null` hors contrat 188.
  final int? epoch;

  /// Rect **viewport** px CSS du top-slice (`top`, étape
  /// 188) — `null` = aucun plan supérieur pour cette frame.
  final ui.Rect? top;
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
  FfiEngine._(
    this._worker,
    this.libPath,
    List<String> mediaCodecs,
    this.abiVersion,
  ) {
    // Prélecture du port loopback média (étape 177/b2) —
    // async car le worker isolate répond en RPC ; le claim
    // MSE survient toujours après, la course est gagnée.
    unawaited(_prefetchApiPort());
    // Capacités média déclarées au spawn (étape 178) —
    // `isTypeSupported` consulte la table dès le premier
    // script de chaque page ; sans elle, un player MSE
    // réel n'instancie jamais la `MediaSource` dont le
    // claim apprendrait les codecs (cercle vicieux).
    unawaited(_declareMediaCodecs(mediaCodecs));
  }

  /// Chemin de la bibliothèque chargée — affiché dans la barre
  /// de statut (étape 119 : le piège `engineMode=auto` était
  /// de charger silencieusement une dll périmée adjacente).
  final String libPath;

  /// Niveau d'ABI déclaré par la dll chargée (étape 187 —
  /// `explorer_abi_version`) : `0` = copie pré-versionnement,
  /// fonctionnelle via les replis par-symbole mais plus
  /// ancienne que le contrat courant. Diagnostiqué dans la
  /// barre de statut, jamais bloquant.
  final int abiVersion;

  @override
  String get engineLabel =>
      'FFI $libPath (abi ${abiVersion == 0 ? '?' : abiVersion})';

  /// Loopback média embarqué (étape 177/b2, ADR-0029) —
  /// `http://127.0.0.1:<port>` sert `GET /api/…/media/*`
  /// comme le daemon ; `''` si le bind a échoué ou si la
  /// prélecture n'a pas encore répondu (honnête, jamais
  /// d'URL inventée).
  @override
  String get apiBase => _apiPort == 0 ? '' : 'http://127.0.0.1:$_apiPort';

  int _apiPort = 0;

  Future<void> _prefetchApiPort() async {
    try {
      final r = await _worker.call(opApiPort);
      _apiPort = (r?['port'] as num?)?.toInt() ?? 0;
    } on Object {
      // Export absent / worker pas prêt — `apiBase` reste ''.
    }
  }

  /// Déclare les capacités média de l'hôte au moteur
  /// (étape 178 — b-e2e) : corps `{codecs:[…]}` identique
  /// au claim REST — `mediaCodecs` de l'app.
  Future<void> _declareMediaCodecs(List<String> codecs) async {
    if (codecs.isEmpty) return;
    try {
      await _worker.call(opMediaCodecs, {'codecs': codecs});
    } on Object {
      // Export absent / worker pas prêt — le claim garde
      // le comportement historique.
    }
  }

  /// Tente de charger la bibliothèque dynamique — `null` si
  /// absente (mode dégradé → REST, jamais d'exception hors
  /// du `tryLoad`). La sonde (`open` + lookup
  /// `explorer_create`) est volontairement **synchrone** et
  /// légère : un `dlopen`+symbole tient en millisecondes et
  /// garde la signature sans `await` ; le worker reprend le
  /// chemin gagnant et crée la session hors de l'UI.
  static FfiEngine? tryLoad(AppConfig config,
      {List<String>? mediaCodecs}) {
    if (config.engineMode == 'rest') return null;
    for (final candidate in _candidates(config)) {
      try {
        final lib = ffi.DynamicLibrary.open(candidate);
        // Vérif ABI minimale : le symbole existe → le worker
        // partira sur ce chemin (sa propre `open`+`create`).
        lib.lookup('explorer_create');
        return FfiEngine._(
          _FfiWorker.start(candidate),
          candidate,
          mediaCodecs ?? config.mediaCodecs,
          _probeAbi(lib),
        );
      } on Object {
        // Candidat absent ou ABI incompatible → suivant.
      }
    }
    return null;
  }

  /// Sonde `explorer_abi_version` (étape 187) — la dll
  /// s'auto-identifie : `0` = copie pré-versionnement (les
  /// replis par-symbole restent la règle), `≥1` = niveau
  /// déclaré par `deploy_engine.ps1` dans le manifest
  /// sidecar de chaque destination.
  static int _probeAbi(ffi.DynamicLibrary lib) {
    try {
      final fn = lib.lookupFunction<ffi.Uint32 Function(), int Function()>(
        'explorer_abi_version',
      );
      return fn();
    } on Object {
      return 0;
    }
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
  Future<FramePull?> fetchFrame(String tabId) async {
    final pull = await fetchFramePull(tabId);
    if (pull == null) return null;
    try {
      final data = await pull.image.toByteData(format: ui.ImageByteFormat.png);
      final bytes = data?.buffer.asUint8List();
      if (bytes == null) return null;
      return FramePull(
        bytes: bytes,
        paintedScrollY: pull.paintedScrollY,
        media: pull.media,
        epoch: pull.epoch,
        top: pull.top,
      );
    } finally {
      pull.image.dispose();
    }
  }

  /// Forme PNG du pull partiel (compat interface REST —
  /// ré-encode l'`ui.Image` de région). Le chemin rapide est
  /// [fetchFrameRegionImage], utilisé par le contrôleur.
  /// La méta de l'étape 185 ne voyage pas par les régions
  /// FFI (export dédié absent — l'app la tire des pulls
  /// pleins ; comportement documenté contrat).
  @override
  Future<FramePull?> fetchFrameRegion(
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
      final bytes = data?.buffer.asUint8List();
      return bytes == null ? null : FramePull(bytes: bytes);
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
  Future<ui.Image?> fetchFrameImage(String tabId) async =>
      (await fetchFramePull(tabId))?.image;

  /// Pull atomique `explorer_frame_pull` (étape 185, M1) —
  /// pixels RGBA + `{scroll_y, media}` **de la même frame**
  /// (même contrat que `x-explorer-scroll-y`/`x-explorer-
  /// media` REST) : `paintedScrollY` remplace le `scroll_y`
  /// supposé du tick, `media` porte rect viewport/fixed/
  /// clip/visible/order. Dll antérieure → champs `null`/
  /// vide → repli pré-185.
  Future<FfiFramePull?> fetchFramePull(String tabId) async {
    final r = await _worker.call(opFrame, {'tab': _idOf(tabId)});
    if (r == null || r['none'] == true) return null;
    final w = (r['w'] as num).toInt();
    final h = (r['h'] as num).toInt();
    final px = (r['px']! as TransferableTypedData).materialize().asUint8List();
    final done = Completer<ui.Image>();
    ui.decodeImageFromPixels(px, w, h, ui.PixelFormat.rgba8888, done.complete);
    final img = await done.future;
    final rawMedia = r['media'] as List?;
    return FfiFramePull(
      image: img,
      paintedScrollY: (r['scroll_y'] as num?)?.toDouble(),
      media: rawMedia == null
          ? null
          : [
              for (final e in rawMedia)
                if (e is Map)
                  MediaPaintMeta.fromJson(Map<String, Object?>.from(e)),
            ],
      epoch: (r['epoch'] as num?)?.toInt(),
      top: MediaPaintMeta.rectFromWire(r['top']),
    );
  }

  /// Pull du top-slice de la frame courante (étape 188 —
  /// M2 composition deux plans) : `explorer_frame_top`,
  /// RGBA+α borné au rect `top` servi par [fetchFramePull].
  /// `epoch` appaire à la base (`null` = génération
  /// courante — rafraîchissement du chemin régions, qui ne
  /// transporte pas de méta) ; `w`/`h` = dims CSS du rect
  /// (dimensionnent le buffer natif — `0` = borne frame
  /// entière, repli wildcard). `stale` = la génération
  /// demandée n'est plus courante → l'appelant efface le
  /// slice retenu. `image` `null` + `stale` `false` = pas
  /// de slice ou export absent (repli plan unique).
  Future<({ui.Image? image, bool stale})> fetchFrameTop(
    String tabId, {
    int? epoch,
    double w = 0,
    double h = 0,
  }) async {
    final r = await _worker.call(opFrameTop, {
      'tab': _idOf(tabId),
      'epoch': epoch ?? -1,
      'w': w,
      'h': h,
    });
    if (r == null || r['none'] == true) {
      return (image: null, stale: false);
    }
    if (r['stale'] == true) return (image: null, stale: true);
    final tw = (r['w'] as num?)?.toInt() ?? 0;
    final th = (r['h'] as num?)?.toInt() ?? 0;
    final px = (r['px']! as TransferableTypedData).materialize().asUint8List();
    final done = Completer<ui.Image>();
    ui.decodeImageFromPixels(
      px,
      tw,
      th,
      ui.PixelFormat.rgba8888,
      done.complete,
    );
    return (image: await done.future, stale: false);
  }

  /// Forme PNG du pull top-slice (compat interface REST) —
  /// ré-encode l'`ui.Image` de [fetchFrameTop] ; `stale`
  /// remonte tel quel. Le chemin UI réel reste
  /// [fetchFrameTop] (aucun codec).
  @override
  Future<({Uint8List? bytes, bool stale})> fetchFrameTopSlice(
    String tabId,
    int epoch,
  ) async {
    final r = await fetchFrameTop(tabId, epoch: epoch, w: 0, h: 0);
    final img = r.image;
    if (img == null) return (bytes: null, stale: r.stale);
    try {
      final data = await img.toByteData(format: ui.ImageByteFormat.png);
      return (bytes: data?.buffer.asUint8List(), stale: false);
    } finally {
      img.dispose();
    }
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
  // Bibliothèque — favoris et historique pontés FFI
  // (`explorer_bookmarks`/`_add`/`_del`, `explorer_history`
  // — exports additifs : dll antérieure → liste vide /
  // code -1).
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
  Future<List<HistoryEntry>> history() async {
    final r = await _worker.call(opHistory);
    final list = r?['history'] as List? ?? const [];
    return [
      for (final e in list)
        if (e is Map<String, dynamic>) HistoryEntry.fromJson(e),
    ];
  }

  /// Find-in-page — `explorer_find` ponté (miroir du
  /// `POST /find` REST ; export additif → count/index 0
  /// sur dll antérieure).
  @override
  Future<FindResult> find(String tabId, String query, {int dir = 0}) async {
    final r = await _worker.call(opFind, {
      'tab': _idOf(tabId),
      'query': query,
      'dir': dir,
    });
    final m = r ?? const <String, Object?>{};
    return FindResult(
      count: (m['count'] as num?)?.toInt() ?? 0,
      index: (m['index'] as num?)?.toInt() ?? 0,
      query: query,
    );
  }

  /// Téléchargements (étape 70) : pas de canal d'acceptation
  /// accept/decline pontés FFI
  /// (`explorer_download_accept`/`_decline` — exports
  /// additifs : dll antérieure → `code -1`/`FfiException`).
  @override
  Future<String> acceptDownload(String tabId, int dlId, {String? path}) async {
    final r = await _worker.call(opDownloadAccept, {
      'tab': _idOf(tabId),
      'dl': dlId,
      'path': ?path,
    });
    final code = (r?['code'] as num?)?.toInt() ?? -1;
    if (code == 0) return r?['path'] as String? ?? '';
    throw FfiException('accept_download : code $code');
  }

  /// Téléchargement direct **non bloquant** (étape 176) —
  /// `explorer_download_start` rend l'`id` immédiatement ;
  /// le cycle de vie (`download_started`/`progress`/`done`/
  /// `failed`) arrive dans le drain NDJSON du tick. Dll
  /// antérieure → `code -1` → `id` `0` (la surface affiche
  /// une erreur douce, jamais de throw).
  @override
  Future<int> downloadStart(String tabId, String url,
      {String? filename}) async {
    final r = await _worker.call(opDownloadStart, {
      'tab': _idOf(tabId),
      'url': url,
      'filename': ?filename,
    });
    return (r?['code'] as num?)?.toInt() == 0
        ? (r?['id'] as num?)?.toInt() ?? 0
        : 0;
  }

  /// Voir [downloadStart] — `explorer_download_cancel`.
  @override
  Future<void> cancelDownload(String tabId, int dlId) async {
    await _worker
        .call(opDownloadCancel, {'tab': _idOf(tabId), 'dl': dlId});
  }

  /// Sonde hit-test (étape 186 — `explorer_hit_targets`,
  /// premier ABI hit FFI : le menu contextuel gagne
  /// `link`/`image` sous FFI aussi). `mediaKnown` faux sur
  /// dll antérieure (`none`) → l'arbitrage pointer retombe
  /// sur la double détente M0.
  @override
  Future<({String? link, String? image, int? media, bool mediaKnown})>
      hitTarget(String tabId, double x, double y) async {
    final res = await _worker
        .call(opHit, {'tab': _idOf(tabId), 'x': x, 'y': y});
    final m = res ?? const <String, Object?>{};
    return (
      link: m['link'] as String?,
      image: m['image'] as String?,
      media: (m['media'] as num?)?.toInt(),
      mediaKnown: m.containsKey('media'),
    );
  }

  /// Pas d'ABI de lecture du journal profil — `[]`.
  @override
  Future<List<Map<String, dynamic>>> listDownloads() async =>
      const [];

  /// Voir [acceptDownload].
  @override
  Future<void> declineDownload(String tabId, int dlId) async {
    await _worker
        .call(opDownloadDecline, {'tab': _idOf(tabId), 'dl': dlId});
  }

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

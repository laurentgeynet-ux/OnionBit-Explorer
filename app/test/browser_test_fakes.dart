// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Dépôt/événements/lecteur simulés partagés entre les tests
/// contrôleur (`browser_controller_test`) et les tests widget
/// (`widget_test`) — étape 116 : la couverture omnibox exige
/// les deux niveaux sur le même harnais.
library;

import 'dart:async';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:explorer_app/core/api/api_client.dart';
import 'package:explorer_app/core/api/events_client.dart';
import 'package:explorer_app/core/config/app_config.dart';
import 'package:explorer_app/core/di/providers.dart';
import 'package:explorer_app/features/browser/data/browser_repository.dart';
import 'package:explorer_app/features/browser/domain/bookmark.dart';
import 'package:explorer_app/features/browser/domain/browser_tab.dart';
import 'package:explorer_app/features/browser/data/media_player.dart';
import 'package:explorer_app/features/browser/domain/find_result.dart';
import 'package:explorer_app/features/browser/domain/history_entry.dart';
import 'package:explorer_app/features/browser/domain/media_offer.dart';
import 'package:explorer_app/features/browser/domain/reload_result.dart';
import 'package:explorer_app/features/browser/presentation/browser_controller.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

/// Dépôt simulé — réponses scriptées, appels enregistrés.
/// `online = false` simule un daemon injoignable.
class FakeRepo implements BrowserRepository {
  int nextId = 0;
  bool online = true;
  final List<String> navigations = [];
  final List<String> actions = [];
  final List<Bookmark> marks = [];
  final List<HistoryEntry> visits = [
    const HistoryEntry(url: 'http://ex.com/', title: 'Ex'),
  ];

  @override
  Future<BrowserTab> createTab() async {
    if (!online) throw const ApiException('injoignable');
    return BrowserTab(id: '${++nextId}');
  }

  @override
  Future<List<BrowserTab>> listTabs() async {
    if (!online) throw const ApiException('injoignable');
    return [];
  }

  @override
  Future<BrowserTab?> navigate(String tabId, String url) async {
    navigations.add('$tabId:$url');
    return BrowserTab(id: tabId, url: url, title: 'T', canBack: true);
  }

  /// Résultat injectable du prochain `reload` — `needsConfirm`
  /// simule une page POST (étape 93).
  ReloadResult? nextReload;

  /// Onglets rechargés avec `confirm:true`.
  final List<String> confirmedReloads = [];

  @override
  Future<ReloadResult> reload(String tabId, {bool confirm = false}) async {
    if (confirm) confirmedReloads.add(tabId);
    return nextReload ??
        ReloadResult.tab(BrowserTab(id: tabId, url: 'http://h/'));
  }

  @override
  Future<BrowserTab?> back(String tabId) async {
    actions.add('back:$tabId');
    return BrowserTab(id: tabId, url: 'http://prev/');
  }

  @override
  Future<BrowserTab?> forward(String tabId) async {
    actions.add('forward:$tabId');
    return BrowserTab(id: tabId, url: 'http://next/');
  }

  @override
  Future<void> closeTab(String tabId) async => actions.add('close:$tabId');

  /// Gate optionnelle sur `fetchFrame` (étape 89) : tant que
  /// `frameGate` n'est pas complété, le pull attend — permet
  /// de mesurer la coalescence des rafales `frame_invalidated`.
  Completer<void>? frameGate;
  int frameFetches = 0;

  /// `realPng = true` → un vrai PNG 8×8 sert le pull (étape
  /// 109 : `paintedScrollY` + plan région exigent une frame
  /// installée). `false` (défaut) = octets invalides — le
  /// codec jette vite, comme avant : les tests historiques
  /// mesurent des compteurs, pas des pixels.
  bool realPng = false;
  Uint8List? _png;

  Future<Uint8List> _framePng() async =>
      _png ??= await encodeTestPng(8, 8, 255, 0, 0);

  @override
  Future<Uint8List?> fetchFrame(String tabId) async {
    frameFetches++;
    final gate = frameGate;
    if (gate != null) await gate.future;
    return realPng ? _framePng() : Uint8List.fromList([1, 2, 3]);
  }

  /// Régions demandées par le pull partiel (étape 109) —
  /// `'x,y,WxH'` enregistrées pour les assertions.
  final List<String> regionFetches = [];

  @override
  Future<Uint8List?> fetchFrameRegion(
    String tabId,
    int x,
    int y,
    int w,
    int h,
  ) async {
    regionFetches.add('$x,$y,${w}x$h');
    return realPng
        ? encodeTestPng(w.clamp(1, 4096), h.clamp(1, 4096), 0, 255, 0)
        : Uint8List.fromList([1, 2, 3]);
  }

  /// Résultat scripté pour les événements `key` (étape 86 —
  /// « unfocused » doit resynchroniser `fieldFocused`).
  String keyResult = 'ignored';

  /// `scroll_y` renvoyé par `input scroll` (étape 109 — la
  /// traduction locale part de cette valeur autoritaire).
  double scrollResult = 0;

  @override
  Future<Map<String, dynamic>> sendInput(
    String tabId,
    Map<String, dynamic> ev,
  ) async {
    actions.add('input:${ev['type']}');
    return {
      'result': switch (ev['type']) {
        'click' => 'focused',
        'key' => keyResult,
        'scroll' => 'scrolled',
        _ => 'ignored',
      },
      if (ev['type'] == 'scroll') 'scroll_y': scrollResult,
    };
  }

  @override
  Future<void> setViewport(
    String tabId,
    double w,
    double h,
    double scale,
  ) async =>
      actions.add('viewport:${w.toInt()}x${h.toInt()}');

  @override
  Future<void> setColorScheme(String tabId, String scheme) async =>
      actions.add('scheme:$tabId:$scheme');

  /// `Accept-Language` poussés (étape 118).
  final List<String> langPushes = [];

  @override
  Future<void> setAcceptLanguage(String lang) async => langPushes.add(lang);

  @override
  String get engineLabel => 'TEST fake';

  /// Base REST factice pour le claim MSE (139i) — le fake ne
  /// sert jamais de `…/segments` : chaîne vide honnête.
  @override
  String get apiBase => '';

  @override
  Future<List<Bookmark>> bookmarks() async => List.of(marks);

  @override
  Future<bool> addBookmark(String url, {String title = ''}) async {
    if (marks.any((b) => b.url == url)) return false;
    marks.add(Bookmark(url: url, title: title));
    return true;
  }

  @override
  Future<bool> removeBookmark(String url) async {
    final before = marks.length;
    marks.removeWhere((b) => b.url == url);
    return marks.length != before;
  }

  @override
  Future<List<HistoryEntry>> history() async => List.of(visits);

  /// Recherches find-in-page simulées (étape 57bis) —
  /// `findCount` scripté, appels enregistrés.
  int findCount = 0;
  final List<String> findCalls = [];

  @override
  Future<FindResult> find(String tabId, String query, {int dir = 0}) async {
    findCalls.add('$tabId:$query:$dir');
    if (dir == 0) {
      return FindResult(count: findCount, index: 0, query: query);
    }
    return FindResult(
      count: findCount,
      index: findCount == 0 ? 0 : 1,
      query: query,
    );
  }

  /// Acceptations enregistrées (`$tabId:$dlId`).
  final List<String> acceptedDownloads = [];

  /// Rejets enregistrés.
  final List<String> declinedDownloads = [];

  @override
  Future<String> acceptDownload(String tabId, int dlId, {String? path}) async {
    acceptedDownloads.add('$tabId:$dlId');
    return path ?? '/dl/file.bin';
  }

  @override
  Future<void> declineDownload(String tabId, int dlId) async {
    declinedDownloads.add('$tabId:$dlId');
  }

  /// Appels média enregistrés (`claim:`/`release:`/`state:`) +
  /// offres servies par `listMedia` (réconciliation bootstrap).
  final List<String> mediaCalls = [];
  List<MediaOffer> mediaList = [];

  @override
  Future<List<MediaOffer>> listMedia(String tabId) async => List.of(mediaList);

  @override
  Future<bool> claimMedia(
    String tabId,
    int mediaId,
    List<String> codecs,
  ) async {
    mediaCalls.add('claim:$tabId:$mediaId');
    return true;
  }

  @override
  Future<void> releaseMedia(String tabId, int mediaId) async {
    mediaCalls.add('release:$tabId:$mediaId');
  }

  @override
  Future<void> mediaState(
    String tabId,
    int mediaId,
    Map<String, dynamic> report,
  ) async {
    mediaCalls.add('state:$tabId:$mediaId:${report['state']}');
  }
}

/// Lecteur natif simulé — compte les ordres `MediaCommand` et
/// notifie comme un `VideoPlayerController` pour les rapports.
class FakePlayer implements MediaPlayer {
  FakePlayer(this.url);

  /// Toutes les instances créées via la fabrique injectée —
  /// la dernière = la surface sous test.
  static final List<FakePlayer> instances = [];
  final String url;

  int plays = 0, pauses = 0, disposes = 0;
  Duration? seeked;
  double? volume, rate;
  bool? muted;
  String state = 'paused';
  int positionMs = 0;
  final List<void Function()> _listeners = [];

  void notify() {
    for (final l in List.of(_listeners)) {
      l();
    }
  }

  @override
  Widget get view => const SizedBox();
  @override
  bool get ready => true;
  @override
  MediaSnapshot get snapshot => MediaSnapshot(
    positionMs: positionMs,
    bufferedMs: 0,
    durationMs: 1000,
    state: state,
  );
  @override
  void addListener(void Function() l) => _listeners.add(l);
  @override
  void removeListener(void Function() l) => _listeners.remove(l);
  @override
  Future<void> play() async {
    plays++;
    state = 'playing';
    notify();
  }

  @override
  Future<void> pause() async {
    pauses++;
    state = 'paused';
    notify();
  }

  @override
  Future<void> seekTo(Duration pos) async => seeked = pos;
  @override
  Future<void> setVolume(double v) async => volume = v;
  @override
  Future<void> setMuted(bool on) async => muted = on;
  @override
  Future<void> setRate(double r) async => rate = r;
  @override
  Future<void> dispose() async => disposes++;
}

/// Fabrique enregistrant ses instances — les tests lisent le
/// dernier `FakePlayer` pour vérifier commandes et dispose.
Future<MediaPlayer> fakeMediaPlayerFactory(
  MediaOffer offer, {
  String tabId = '',
  String apiBase = '',
}) async {
  final p = FakePlayer(offer.url);
  FakePlayer.instances.add(p);
  return p;
}

/// Encode un PNG `w×h` monochrome — les pulls partiels ont
/// besoin de vraies images pour mesurer le plan (étape 109) ;
/// `toByteData(png)` est le chemin de prod, déjà vérifié en
/// test (rasterizer logiciel du binding).
Future<Uint8List> encodeTestPng(int w, int h, int r, int g, int b) async {
  final px = Uint8List(w * h * 4);
  for (var i = 0; i < px.length; i += 4) {
    px[i] = r;
    px[i + 1] = g;
    px[i + 2] = b;
    px[i + 3] = 255;
  }
  final c = Completer<ui.Image>();
  ui.decodeImageFromPixels(px, w, h, ui.PixelFormat.rgba8888, c.complete);
  final img = await c.future;
  try {
    final data = await img.toByteData(format: ui.ImageByteFormat.png);
    return data!.buffer.asUint8List();
  } finally {
    img.dispose();
  }
}

/// Client SSE simulé — événements injectés à la main.
class FakeEvents implements EventsClient {
  final ctrl = StreamController<ServerEvent>();
  bool disposed = false;

  @override
  final AppConfig config = const AppConfig();

  @override
  Stream<ServerEvent> events() => ctrl.stream;

  @override
  Future<void> dispose() async {
    disposed = true;
    await ctrl.close();
  }
}

ProviderContainer makeContainer(
  FakeRepo repo,
  FakeEvents events, {
  AppConfig config = const AppConfig(),
  MediaPlayerFactory? playerFactory,
}) {
  return ProviderContainer(
    overrides: [
      configProvider.overrideWithValue(config),
      apiClientProvider.overrideWithValue(ApiClient(const AppConfig())),
      browserRepositoryProvider.overrideWithValue(repo),
      eventsClientProvider.overrideWithValue(events),
      mediaPlayerFactoryProvider.overrideWithValue(
        playerFactory ?? fakeMediaPlayerFactory,
      ),
    ],
  );
}

/// Force le build du provider puis laisse le bootstrap
/// terminer ses microtasks.
Future<void> settle(ProviderContainer c) async {
  c.read(browserProvider);
  await Future<void>.delayed(Duration.zero);
  await Future<void>.delayed(Duration.zero);
  await Future<void>.delayed(Duration.zero);
}

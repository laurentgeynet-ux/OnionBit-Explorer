// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:async';
import 'dart:typed_data';

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
import 'package:flutter_test/flutter_test.dart';

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
  Future<BrowserTab?> back(String tabId) async =>
      BrowserTab(id: tabId, url: 'http://prev/');

  @override
  Future<BrowserTab?> forward(String tabId) async =>
      BrowserTab(id: tabId, url: 'http://next/');

  @override
  Future<void> closeTab(String tabId) async => actions.add('close:$tabId');

  /// Gate optionnelle sur `fetchFrame` (étape 89) : tant que
  /// `frameGate` n'est pas complété, le pull attend — permet
  /// de mesurer la coalescence des rafales `frame_invalidated`.
  Completer<void>? frameGate;
  int frameFetches = 0;

  @override
  Future<Uint8List?> fetchFrame(String tabId) async {
    frameFetches++;
    final gate = frameGate;
    if (gate != null) await gate.future;
    return Uint8List.fromList([1, 2, 3]);
  }

  /// Résultat scripté pour les événements `key` (étape 86 —
  /// « unfocused » doit resynchroniser `fieldFocused`).
  String keyResult = 'ignored';

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
        _ => 'ignored',
      },
    };
  }

  @override
  Future<void> setViewport(
    String tabId,
    double w,
    double h,
    double scale,
  ) async => actions.add('viewport:${w.toInt()}x${h.toInt()}');

  @override
  Future<void> setColorScheme(String tabId, String scheme) async =>
      actions.add('scheme:$tabId:$scheme');

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
Future<MediaPlayer> fakeFactory(String url) async {
  final p = FakePlayer(url);
  FakePlayer.instances.add(p);
  return p;
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
        playerFactory ?? fakeFactory,
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

void main() {
  test('bootstrap crée un onglet et le rend actif', () async {
    final repo = FakeRepo();
    final events = FakeEvents();
    final c = makeContainer(repo, events);
    addTearDown(c.dispose);

    // build() lance _bootstrap en microtask — on attend qu'il pose.
    await settle(c);
    final st = c.read(browserProvider);
    expect(st.daemonOnline, isTrue);
    expect(st.tabs, hasLength(1));
    expect(st.activeId, '1');
  });

  test('navigate complète le schéma et pose loading', () async {
    final repo = FakeRepo();
    final events = FakeEvents();
    final c = makeContainer(repo, events);
    addTearDown(c.dispose);
    await settle(c);

    await c.read(browserProvider.notifier).navigate('example.com');
    expect(repo.navigations, ['1:http://example.com']);
    final tab = c.read(browserProvider).active!;
    expect(tab.url, 'http://example.com');
    expect(tab.canBack, isTrue);
    expect(tab.loading, isFalse);
  });

  test('événements SSE mettent à jour le state', () async {
    final repo = FakeRepo();
    final events = FakeEvents();
    final c = makeContainer(repo, events);
    addTearDown(c.dispose);
    await settle(c);

    events.ctrl.add(
      ServerEvent('navigated', {
        'tab': 1,
        'url': 'http://h/page',
        'title': 'Page',
      }),
    );
    await Future<void>.delayed(Duration.zero);
    var tab = c.read(browserProvider).active!;
    expect(tab.url, 'http://h/page');
    expect(tab.title, 'Page');

    events.ctrl.add(
      ServerEvent('title_changed', {'tab': 1, 'title': 'Nouveau'}),
    );
    await Future<void>.delayed(Duration.zero);
    tab = c.read(browserProvider).active!;
    expect(tab.title, 'Nouveau');

    events.ctrl.add(ServerEvent('frame_invalidated', {'tab': 1}));
    await Future<void>.delayed(Duration.zero);
    expect(c.read(browserProvider).frameTick, 1);

    events.ctrl.add(
      ServerEvent('pipeline_error', {'tab': 1, 'message': 'boom'}),
    );
    await Future<void>.delayed(Duration.zero);
    expect(c.read(browserProvider).lastError, 'boom');
  });

  test('pipeline_error pose la page d’erreur, navigated l’efface', () async {
    final repo = FakeRepo();
    final events = FakeEvents();
    final c = makeContainer(repo, events);
    addTearDown(c.dispose);
    await settle(c);

    events.ctrl.add(
      ServerEvent('pipeline_error', {'tab': 1, 'message': 'HTTP 404'}),
    );
    await Future<void>.delayed(Duration.zero);
    expect(c.read(browserProvider).pageErrors['1'], 'HTTP 404');

    events.ctrl.add(
      ServerEvent('navigated', {'tab': 1, 'url': 'http://h/ok', 'title': 'OK'}),
    );
    await Future<void>.delayed(Duration.zero);
    expect(c.read(browserProvider).pageErrors.containsKey('1'), isFalse);
  });

  test('download_offered expose l’offre, accept/decline décident', () async {
    final repo = FakeRepo();
    final events = FakeEvents();
    final c = makeContainer(repo, events);
    addTearDown(c.dispose);
    await settle(c);

    events.ctrl.add(
      ServerEvent('download_offered', {
        'tab': 1,
        'id': 7,
        'url': 'http://h/f.bin',
        'filename': 'f.bin',
        'mime': 'application/octet-stream',
        'size': 42,
      }),
    );
    await Future<void>.delayed(Duration.zero);
    final offer = c.read(browserProvider).downloadOffer!;
    expect(offer.id, 7);
    expect(offer.filename, 'f.bin');
    expect(offer.size, 42);

    // Accepter → appel repo + offre consumée ; DownloadDone
    // SSE rapporte le chemin en lastError (snackbar).
    await c.read(browserProvider.notifier).acceptDownload(offer);
    expect(repo.acceptedDownloads, ['1:7']);
    expect(c.read(browserProvider).downloadOffer, isNull);

    events.ctrl.add(
      ServerEvent('download_done', {'tab': 1, 'id': 7, 'path': '/dl/f.bin'}),
    );
    await Future<void>.delayed(Duration.zero);
    expect(c.read(browserProvider).lastError, contains('/dl/f.bin'));

    // Rejet d'une seconde offre → declineDownload.
    events.ctrl.add(
      ServerEvent('download_offered', {
        'tab': 1,
        'id': 8,
        'url': 'http://h/g.bin',
        'filename': 'g.bin',
        'mime': 'application/octet-stream',
      }),
    );
    await Future<void>.delayed(Duration.zero);
    final offer2 = c.read(browserProvider).downloadOffer!;
    await c.read(browserProvider.notifier).declineDownload(offer2);
    expect(repo.declinedDownloads, ['1:8']);
    expect(c.read(browserProvider).downloadOffer, isNull);
  });

  test('daemon absent → daemonOnline false, retry récupère', () async {
    final events = FakeEvents();
    final repo = FakeRepo()..online = false;
    final c = makeContainer(repo, events);
    addTearDown(c.dispose);
    await settle(c);
    expect(c.read(browserProvider).daemonOnline, isFalse);

    repo.online = true;
    await c.read(browserProvider.notifier).retry();
    expect(c.read(browserProvider).daemonOnline, isTrue);
    expect(c.read(browserProvider).tabs, hasLength(1));
  });

  test('daemon absent → re-bootstrap automatique (étape 81)', () async {
    final events = FakeEvents();
    final repo = FakeRepo()..online = false;
    final c = makeContainer(
      repo,
      events,
      config: const AppConfig(daemonRetryDelay: Duration(milliseconds: 10)),
    );
    addTearDown(c.dispose);
    await settle(c);
    expect(c.read(browserProvider).daemonOnline, isFalse);

    // Le daemon revient sans toucher le bouton « Réessayer » :
    // le timer de re-bootstrap doit le détecter seul.
    repo.online = true;
    await Future<void>.delayed(const Duration(milliseconds: 50));
    expect(c.read(browserProvider).daemonOnline, isTrue);
    expect(c.read(browserProvider).tabs, hasLength(1));
  });

  test('reconnexion purge le lastError persistant (étape 88)', () async {
    final events = FakeEvents();
    final repo = FakeRepo();
    final c = makeContainer(
      repo,
      events,
      // Timer long : le retry est piloté à la main ici.
      config: const AppConfig(daemonRetryDelay: Duration(seconds: 30)),
    );
    addTearDown(c.dispose);
    await settle(c);

    events.ctrl.add(
      ServerEvent('pipeline_error', {'tab': 1, 'message': 'boom'}),
    );
    await Future<void>.delayed(Duration.zero);
    expect(c.read(browserProvider).lastError, 'boom');

    // Daemon parti puis revenu : le bootstrap réussi purge
    // l'erreur — avant elle survivait à la reconnexion.
    repo.online = false;
    await c.read(browserProvider.notifier).retry();
    expect(c.read(browserProvider).daemonOnline, isFalse);
    expect(c.read(browserProvider).lastError, 'boom');

    repo.online = true;
    await c.read(browserProvider.notifier).retry();
    expect(c.read(browserProvider).daemonOnline, isTrue);
    expect(c.read(browserProvider).lastError, isNull);
  });

  test('about:blank passe sans complétion http:// (étape 88)', () async {
    final repo = FakeRepo();
    final c = makeContainer(repo, FakeEvents());
    addTearDown(c.dispose);
    await settle(c);

    await c.read(browserProvider.notifier).navigate('about:blank');
    expect(repo.navigations, ['1:about:blank']);

    // Une entrée sans schéma est toujours préfixée.
    await c.read(browserProvider.notifier).navigate('example.com');
    expect(repo.navigations, ['1:about:blank', '1:http://example.com']);
  });

  test('rafale frame_invalidated → pulls coalescés (étape 89)', () async {
    final events = FakeEvents();
    final repo = FakeRepo()..frameGate = Completer<void>();
    final c = makeContainer(repo, events);
    addTearDown(c.dispose);
    await settle(c);

    // 1re invalidation → pull en vol (bloqué par la gate) ;
    // les suivantes se replient en un seul « à refaire ».
    events.ctrl.add(ServerEvent('frame_invalidated', {'tab': 1}));
    await Future<void>.delayed(Duration.zero);
    events.ctrl.add(ServerEvent('frame_invalidated', {'tab': 1}));
    events.ctrl.add(ServerEvent('frame_invalidated', {'tab': 1}));
    await Future<void>.delayed(Duration.zero);
    expect(repo.frameFetches, 1);

    // Libération : un seul second pull rejoue la dernière
    // frame — « dernière gagne », pas une file par event.
    repo.frameGate!.complete();
    for (var i = 0; i < 5; i++) {
      await Future<void>.delayed(Duration.zero);
    }
    expect(repo.frameFetches, 2);
  });

  test('frame_invalidated porte scroll_y par onglet (étape 71d)', () async {
    final repo = FakeRepo();
    final events = FakeEvents();
    final c = makeContainer(repo, events);
    addTearDown(c.dispose);
    await settle(c);

    // Champ additif : un event ancien (sans scroll_y) ne casse
    // rien ; un event récent pose la valeur pour l'onglet.
    events.ctrl.add(ServerEvent('frame_invalidated', {'tab': 1}));
    await Future<void>.delayed(Duration.zero);
    expect(c.read(browserProvider).scrollY['1'], isNull);

    events.ctrl.add(
      ServerEvent('frame_invalidated', {'tab': 1, 'scroll_y': 240.5}),
    );
    await Future<void>.delayed(Duration.zero);
    expect(c.read(browserProvider).scrollY['1'], 240.5);

    // Autre onglet isolé — le scroll est par-onglet.
    events.ctrl.add(
      ServerEvent('frame_invalidated', {'tab': 9, 'scroll_y': 10}),
    );
    await Future<void>.delayed(Duration.zero);
    expect(c.read(browserProvider).scrollY['9'], 10);
    expect(c.read(browserProvider).scrollY['1'], 240.5);
  });

  test('media_offered → claim + lecteur natif (étape 71d)', () async {
    final repo = FakeRepo();
    final events = FakeEvents();
    FakePlayer.instances.clear();
    final c = makeContainer(repo, events);
    addTearDown(c.dispose);
    await settle(c);

    events.ctrl.add(
      ServerEvent('media_offered', {
        'tab': 1,
        'media': 42,
        'kind': 'video',
        'rect': {'x': 10.0, 'y': 500.0, 'w': 320.0, 'h': 180.0},
        'url': 'http://h/v.mp4',
        'mime': 'video/mp4',
        'controls': true,
        'autoplay': false,
      }),
    );
    // claim REST → fabrique async → listener : quelques ticks.
    for (var i = 0; i < 5; i++) {
      await Future<void>.delayed(Duration.zero);
    }
    final m = c.read(browserProvider).media['1']?[42];
    expect(m, isNotNull);
    expect(m!.claimed, isTrue);
    expect(m.rect.width, 320);
    expect(repo.mediaCalls, contains('claim:1:42'));
    expect(FakePlayer.instances, hasLength(1));
    expect(FakePlayer.instances.single.url, 'http://h/v.mp4');
    // Premier rapport d'état forcé (paused initial).
    expect(repo.mediaCalls, contains('state:1:42:paused'));
  });

  test('media_command route vers le lecteur, media_gone dispose', () async {
    final repo = FakeRepo();
    final events = FakeEvents();
    FakePlayer.instances.clear();
    final c = makeContainer(repo, events);
    addTearDown(c.dispose);
    await settle(c);

    events.ctrl.add(
      ServerEvent('media_offered', {
        'tab': 1,
        'media': 7,
        'kind': 'video',
        'rect': {'x': 0, 'y': 0, 'w': 100, 'h': 50},
        'url': 'http://h/v.mp4',
      }),
    );
    for (var i = 0; i < 5; i++) {
      await Future<void>.delayed(Duration.zero);
    }
    final p = FakePlayer.instances.single;

    // Commandes JS → méthodes natives.
    for (final cmd in [
      {'type': 'play'},
      {'type': 'pause'},
      {'type': 'seek', 'ms': 1500},
      {'type': 'volume', 'v': 0.4},
      {'type': 'muted', 'on': true},
      {'type': 'rate', 'r': 1.5},
    ]) {
      events.ctrl.add(
        ServerEvent('media_command', {'tab': 1, 'media': 7, 'cmd': cmd}),
      );
    }
    await Future<void>.delayed(Duration.zero);
    await Future<void>.delayed(Duration.zero);
    expect(p.plays, 1);
    expect(p.pauses, 1);
    expect(p.seeked, const Duration(milliseconds: 1500));
    expect(p.volume, 0.4);
    expect(p.muted, isTrue);
    expect(p.rate, 1.5);

    // media_moved met à jour le rect document.
    events.ctrl.add(
      ServerEvent('media_moved', {
        'tab': 1,
        'media': 7,
        'rect': {'x': 5.0, 'y': 999.0, 'w': 100.0, 'h': 60.0},
      }),
    );
    await Future<void>.delayed(Duration.zero);
    expect(c.read(browserProvider).media['1']?[7]?.rect.top, 999);

    // media_gone → dispose + retrait de l'état.
    events.ctrl.add(ServerEvent('media_gone', {'tab': 1, 'media': 7}));
    await Future<void>.delayed(Duration.zero);
    expect(c.read(browserProvider).media['1'], isNot(contains(7)));
    expect(p.disposes, 1);

    // Commande tardive sur surface morte → ignorée proprement.
    events.ctrl.add(
      ServerEvent('media_command', {
        'tab': 1,
        'media': 7,
        'cmd': {'type': 'play'},
      }),
    );
    await Future<void>.delayed(Duration.zero);
    expect(p.plays, 1);
  });

  test('rapport state borné à 4 Hz sauf changement d’état', () async {
    final repo = FakeRepo();
    final events = FakeEvents();
    FakePlayer.instances.clear();
    final c = makeContainer(
      repo,
      events,
      config: const AppConfig(mediaReportInterval: Duration(milliseconds: 250)),
    );
    addTearDown(c.dispose);
    await settle(c);

    events.ctrl.add(
      ServerEvent('media_offered', {
        'tab': 1,
        'media': 5,
        'kind': 'audio',
        'rect': {'x': 0, 'y': 0, 'w': 0, 'h': 0},
        'url': 'http://h/a.mp3',
      }),
    );
    for (var i = 0; i < 5; i++) {
      await Future<void>.delayed(Duration.zero);
    }
    final p = FakePlayer.instances.single;
    // Rapport initial forcé (paused).
    expect(repo.mediaCalls.where((s) => s.startsWith('state:')), hasLength(1));

    // Rafale de notifications même-état dans la fenêtre → 0 POST.
    for (var i = 0; i < 5; i++) {
      p.positionMs = i * 16;
      p.notify();
    }
    await Future<void>.delayed(Duration.zero);
    await Future<void>.delayed(Duration.zero);
    expect(repo.mediaCalls.where((s) => s.startsWith('state:')), hasLength(1));

    // Changement d'état dans la fenêtre → POST immédiat.
    p.state = 'playing';
    p.notify();
    await Future<void>.delayed(Duration.zero);
    await Future<void>.delayed(Duration.zero);
    expect(repo.mediaCalls, contains('state:1:5:playing'));
  });

  test('tab_closed dispose les surfaces de l’onglet', () async {
    final repo = FakeRepo();
    final events = FakeEvents();
    FakePlayer.instances.clear();
    final c = makeContainer(repo, events);
    addTearDown(c.dispose);
    await settle(c);

    events.ctrl.add(
      ServerEvent('media_offered', {
        'tab': 1,
        'media': 3,
        'kind': 'video',
        'rect': {'x': 0, 'y': 0, 'w': 10, 'h': 10},
        'url': 'http://h/v.mp4',
      }),
    );
    for (var i = 0; i < 5; i++) {
      await Future<void>.delayed(Duration.zero);
    }
    final p = FakePlayer.instances.single;

    events.ctrl.add(
      ServerEvent('frame_invalidated', {'tab': 1, 'scroll_y': 33}),
    );
    await Future<void>.delayed(Duration.zero);
    events.ctrl.add(ServerEvent('tab_closed', {'tab': 1}));
    await Future<void>.delayed(Duration.zero);

    expect(p.disposes, 1);
    expect(c.read(browserProvider).media.containsKey('1'), isFalse);
    expect(c.read(browserProvider).scrollY.containsKey('1'), isFalse);
  });

  test('signet : toggle ajoute puis retire, état rechargé', () async {
    final repo = FakeRepo();
    final events = FakeEvents();
    final c = makeContainer(repo, events);
    addTearDown(c.dispose);
    await settle(c);

    final marks = c.read(bookmarksProvider.notifier);
    expect(await marks.toggle('http://ex.com/', title: 'Ex'), isTrue);
    expect(c.read(bookmarksProvider).value, [
      isA<Bookmark>().having((b) => b.url, 'url', 'http://ex.com/'),
    ]);

    // Re-toggle → retrait ; état de nouveau vide.
    expect(await marks.toggle('http://ex.com/'), isFalse);
    expect(c.read(bookmarksProvider).value, isEmpty);

    // URL non signetable → no-op.
    expect(await marks.toggle('about:blank'), isFalse);
  });

  test('historique : le provider relaie le store moteur', () async {
    final repo = FakeRepo();
    final events = FakeEvents();
    final c = makeContainer(repo, events);
    addTearDown(c.dispose);
    await settle(c);

    final entries = await c.read(historyProvider.future);
    expect(entries, hasLength(1));
    expect(entries.first.url, 'http://ex.com/');
  });

  test('zoom : borné et réinitialisable', () async {
    final c = ProviderContainer();
    addTearDown(c.dispose);
    final z = c.read(zoomProvider.notifier);

    for (var i = 0; i < 20; i++) {
      z.zoomIn();
    }
    expect(c.read(zoomProvider), ZoomController.max);
    z.reset();
    expect(c.read(zoomProvider), 1.0);
    for (var i = 0; i < 20; i++) {
      z.zoomOut();
    }
    expect(c.read(zoomProvider), ZoomController.min);
  });

  test(
    'find : submit envoie la requête, step navigue, close réinitialise',
    () async {
      final repo = FakeRepo()..findCount = 4;
      final events = FakeEvents();
      final c = makeContainer(repo, events);
      addTearDown(c.dispose);
      await c.read(browserProvider.notifier).retry();
      final find = c.read(findProvider.notifier);

      find.open();
      expect(c.read(findProvider).visible, isTrue);

      await find.submit('lorem');
      var st = c.read(findProvider);
      expect(st.count, 4);
      expect(st.index, 0);
      expect(repo.findCalls.last, endsWith(':lorem:0'));

      await find.step(1);
      st = c.read(findProvider);
      expect(st.index, 1);
      expect(repo.findCalls.last, endsWith(':lorem:1'));

      await find.close();
      st = c.read(findProvider);
      expect(st.visible, isFalse);
      // Fermeture → requête vide envoyée au moteur (efface le
      // surlignage).
      expect(repo.findCalls.last, endsWith('::0'));
    },
  );

  test(
    'reload POST : resubmit_confirm pose le pending, confirm rejoue (étape 93)',
    () async {
      final repo = FakeRepo();
      final c = makeContainer(repo, FakeEvents());
      addTearDown(c.dispose);
      await c.read(browserProvider.notifier).retry();
      final ctrl = c.read(browserProvider.notifier);
      final id = c.read(browserProvider).activeId!;

      // Page servie par POST → resubmit_confirm en attente.
      repo.nextReload = const ReloadResult.needsConfirm(
        method: 'POST',
        url: 'http://h/doit',
      );
      await ctrl.reload();
      var st = c.read(browserProvider);
      expect(st.resubmit?.tabId, id);
      expect(st.resubmit?.method, 'POST');
      expect(st.tabs.first.loading, isFalse);

      // Accord → rejoue avec confirm:true, pending effacé.
      repo.nextReload = ReloadResult.tab(
        BrowserTab(id: id, url: 'http://h/doit'),
      );
      await ctrl.reloadConfirmed();
      st = c.read(browserProvider);
      expect(st.resubmit, isNull);
      expect(repo.confirmedReloads, [id]);

      // Refus → pending effacé, rien d'autre.
      repo.nextReload = const ReloadResult.needsConfirm(
        method: 'POST',
        url: 'http://h/doit',
      );
      await ctrl.reload();
      ctrl.dismissResubmit();
      expect(c.read(browserProvider).resubmit, isNull);
      expect(repo.confirmedReloads, hasLength(1));
    },
  );

  test('find : pas d’onglet actif → no-op sans exception', () async {
    final repo = FakeRepo();
    final c = ProviderContainer(
      overrides: [
        browserRepositoryProvider.overrideWithValue(repo),
        browserProvider.overrideWith(FindlessBrowser.new),
      ],
    );
    addTearDown(c.dispose);
    await c.read(findProvider.notifier).submit('x');
    expect(repo.findCalls, isEmpty);
  });

  test('cycleTab/selectTabAt : cyclage et index (étape 85)', () async {
    final repo = FakeRepo();
    final events = FakeEvents();
    final c = makeContainer(repo, events);
    addTearDown(c.dispose);
    await settle(c);
    final ctrl = c.read(browserProvider.notifier);
    await ctrl.newTab();
    await ctrl.newTab();
    expect(c.read(browserProvider).tabs, hasLength(3));
    expect(c.read(browserProvider).activeId, '3');

    // Cyclage circulaire dans les deux sens.
    ctrl.cycleTab(1);
    expect(c.read(browserProvider).activeId, '1');
    ctrl.cycleTab(1);
    expect(c.read(browserProvider).activeId, '2');
    ctrl.cycleTab(-1);
    expect(c.read(browserProvider).activeId, '1');
    ctrl.cycleTab(-1);
    expect(c.read(browserProvider).activeId, '3');

    // Index borné ; hors liste = no-op.
    ctrl.selectTabAt(1);
    expect(c.read(browserProvider).activeId, '2');
    ctrl.selectTabAt(9);
    expect(c.read(browserProvider).activeId, '2');
    ctrl.selectTabAt(-1);
    expect(c.read(browserProvider).activeId, '2');
  });

  test(
    'openInNewTab : nouvel onglet chargé en arrière-plan (étape 85)',
    () async {
      final repo = FakeRepo();
      final events = FakeEvents();
      final c = makeContainer(repo, events);
      addTearDown(c.dispose);
      await settle(c);
      final ctrl = c.read(browserProvider.notifier);

      // Clic-molette : l'onglet neuf charge l'URL mais l'onglet
      // d'origine garde le premier plan (sémantique Chrome).
      await ctrl.openInNewTab('http://lien.test/');
      expect(c.read(browserProvider).tabs, hasLength(2));
      expect(repo.navigations, ['2:http://lien.test/']);
      expect(c.read(browserProvider).activeId, '1');
    },
  );

  test(
    'openInNewTab : échec de création → aucune navigation (étape 85)',
    () async {
      final repo = FakeRepo()..online = false;
      final events = FakeEvents();
      final c = makeContainer(repo, events);
      addTearDown(c.dispose);
      await settle(c);
      repo.online = true;
      await c.read(browserProvider.notifier).retry();

      // Le daemon retombe entre-temps : createTab jette → pas
      // de navigation vers l'onglet courant, activeId inchangé.
      repo.online = false;
      await c.read(browserProvider.notifier).openInNewTab('http://x.test/');
      expect(repo.navigations, isEmpty);
      expect(c.read(browserProvider).activeId, '1');
    },
  );

  test('key unfocused resynchronise fieldFocused (étape 86)', () async {
    final repo = FakeRepo()..keyResult = 'unfocused';
    final events = FakeEvents();
    final c = makeContainer(repo, events);
    addTearDown(c.dispose);
    await settle(c);
    final ctrl = c.read(browserProvider.notifier);

    // Clic sur un champ → focused → flag posé.
    await ctrl.click(10, 10);
    expect(c.read(browserProvider).fieldFocused, isTrue);

    // Le moteur a perdu le focus (Enter/Escape, nœud mort) :
    // le résultat `unfocused` resynchronise le flag — avant,
    // chaque frappe suivante partait dans le vide.
    await ctrl.key('a');
    expect(c.read(browserProvider).fieldFocused, isFalse);
  });

  test('selectTab remet fieldFocused à zéro (étape 86)', () async {
    final repo = FakeRepo();
    final events = FakeEvents();
    final c = makeContainer(repo, events);
    addTearDown(c.dispose);
    await settle(c);
    final ctrl = c.read(browserProvider.notifier);

    await ctrl.newTab();
    await ctrl.click(10, 10);
    expect(c.read(browserProvider).fieldFocused, isTrue);

    // Le flag est global mais décrit l'onglet actif — changer
    // d'onglet le réinitialise (le focus réel de l'onglet
    // entrant sera ré-appris au prochain clic).
    ctrl.selectTab('1');
    expect(c.read(browserProvider).fieldFocused, isFalse);
  });
}

/// Contrôleur minimal sans onglet — `activeId` reste null.
class FindlessBrowser extends BrowserController {
  @override
  BrowserState build() => const BrowserState();
}

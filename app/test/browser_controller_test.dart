// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:async';

import 'package:explorer_app/core/api/api_client.dart';
import 'package:explorer_app/core/api/events_client.dart';
import 'package:explorer_app/core/config/app_config.dart';
import 'package:explorer_app/core/di/providers.dart';
import 'package:explorer_app/features/browser/data/browser_repository.dart';
import 'package:explorer_app/features/browser/domain/browser_tab.dart';
import 'package:explorer_app/features/browser/presentation/browser_controller.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

/// Dépôt simulé — réponses scriptées, appels enregistrés.
/// `online = false` simule un daemon injoignable.
class FakeRepo implements BrowserRepository {
  int nextId = 0;
  bool online = true;
  final List<String> navigations = [];
  final List<String> actions = [];

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

  @override
  Future<BrowserTab?> reload(String tabId) async =>
      BrowserTab(id: tabId, url: 'http://h/');

  @override
  Future<BrowserTab?> back(String tabId) async =>
      BrowserTab(id: tabId, url: 'http://prev/');

  @override
  Future<BrowserTab?> forward(String tabId) async =>
      BrowserTab(id: tabId, url: 'http://next/');

  @override
  Future<void> closeTab(String tabId) async => actions.add('close:$tabId');

  @override
  Future<List<int>?> fetchFrame(String tabId) async => [1, 2, 3];

  @override
  Future<void> sendInput(String tabId, Map<String, dynamic> ev) async {
    actions.add('input:${ev['type']}');
  }

  @override
  Future<void> setViewport(
          String tabId, double w, double h, double scale) async =>
      actions.add('viewport:${w.toInt()}x${h.toInt()}');
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

ProviderContainer makeContainer(FakeRepo repo, FakeEvents events) {
  return ProviderContainer(overrides: [
    configProvider.overrideWithValue(const AppConfig()),
    apiClientProvider
        .overrideWithValue(ApiClient(const AppConfig())),
    browserRepositoryProvider.overrideWithValue(repo),
    eventsClientProvider.overrideWithValue(events),
  ]);
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

    events.ctrl.add(ServerEvent('navigated', {
      'tab': 1,
      'url': 'http://h/page',
      'title': 'Page',
    }));
    await Future<void>.delayed(Duration.zero);
    var tab = c.read(browserProvider).active!;
    expect(tab.url, 'http://h/page');
    expect(tab.title, 'Page');

    events.ctrl.add(ServerEvent('title_changed', {
      'tab': 1,
      'title': 'Nouveau',
    }));
    await Future<void>.delayed(Duration.zero);
    tab = c.read(browserProvider).active!;
    expect(tab.title, 'Nouveau');

    events.ctrl.add(ServerEvent('frame_invalidated', {'tab': 1}));
    await Future<void>.delayed(Duration.zero);
    expect(c.read(browserProvider).frameTick, 1);

    events.ctrl.add(ServerEvent('pipeline_error', {
      'tab': 1,
      'message': 'boom',
    }));
    await Future<void>.delayed(Duration.zero);
    expect(c.read(browserProvider).lastError, 'boom');
  });

  test('daemon absent → daemonOnline false, retry récupère',
      () async {
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
}

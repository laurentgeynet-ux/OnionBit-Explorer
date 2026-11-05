// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Tests de la barre de contrôles média native (étape 183) :
/// lecture/seek/volume/mute, auto-masquage, buffering, ended
/// → replay, barre `<audio controls>`, surface hors
/// `controls` sans UI.
library;

import 'package:explorer_app/core/api/events_client.dart';
import 'package:explorer_app/core/config/app_config.dart';
import 'package:explorer_app/core/design/design_tokens.dart';
import 'package:explorer_app/core/l10n/app_localizations.dart';
import 'package:explorer_app/features/browser/data/media_player.dart';
import 'package:explorer_app/features/browser/domain/media_offer.dart';
import 'package:explorer_app/features/browser/presentation/browser_controller.dart';
import 'package:explorer_app/features/browser/presentation/media_controls.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'browser_test_fakes.dart';

/// Pousse `media_offered` → claim `FakePlayer`, puis monte la
/// surface 640×360 dans un `MaterialApp` localisé.
Future<({ProviderContainer c, FakePlayer p})> pumpSurface(
  WidgetTester tester, {
  MediaOffer? offer,
  AppConfig config = const AppConfig(),
}) async {
  final repo = FakeRepo();
  final events = FakeEvents();
  FakePlayer.instances.clear();
  final c = makeContainer(repo, events, config: config);
  addTearDown(c.dispose);
  // `settle` est inutilisable ici : sous `testWidgets` les
  // `Future.delayed` vivent dans la fake-clock — `runAsync`
  // cède à l'event loop réel pour bootstrap + claim.
  c.read(browserProvider);
  await tester.runAsync(() async {
    for (var i = 0; i < 6; i++) {
      await Future<void>.delayed(Duration.zero);
    }
  });

  events.ctrl.add(
    ServerEvent('media_offered', {
      'tab': 1,
      'media': 7,
      'kind': offer?.kind ?? 'video',
      'rect': {'x': 0, 'y': 0, 'w': 640, 'h': 360},
      'url': 'http://h/v.mp4',
      'controls': true,
    }),
  );
  await tester.runAsync(() async {
    for (var i = 0; i < 8; i++) {
      await Future<void>.delayed(Duration.zero);
    }
  });
  final ctrl = c.read(browserProvider.notifier);
  final o =
      offer ??
      const MediaOffer(
        id: 7,
        kind: 'video',
        rect: Rect.fromLTWH(0, 0, 640, 360),
        url: 'http://h/v.mp4',
        controls: true,
      );
  await tester.pumpWidget(
    UncontrolledProviderScope(
      container: c,
      child: MaterialApp(
        locale: const Locale('fr'),
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Scaffold(
          body: SizedBox(
            width: 640,
            height: 360,
            child: MediaSurfaceView(
              ctrl: ctrl,
              tabId: '1',
              offer: o,
              tokens: DesignTokens.fallback,
              cssPerLogical: 1,
              scrollY: 0,
              onActive: (_) {},
            ),
          ),
        ),
      ),
    ),
  );
  await tester.pump();
  return (c: c, p: FakePlayer.instances.single);
}

void main() {
  testWidgets('183 — barre controls : play/pause, icône d’état', (
    tester,
  ) async {
    final (:c, :p) = await pumpSurface(tester);
    // Paused (état initial FakePlayer) → bouton ▶.
    expect(find.byIcon(Icons.play_arrow), findsOneWidget);
    await tester.tap(find.byIcon(Icons.play_arrow));
    await tester.pump();
    expect(p.plays, 1);
    expect(find.byIcon(Icons.pause), findsOneWidget);
    await tester.tap(find.byIcon(Icons.pause));
    await tester.pump();
    expect(p.pauses, 1);
    expect(find.byIcon(Icons.play_arrow), findsOneWidget);
  });

  testWidgets('183 — drag de seek → seekTo en fin de geste', (
    tester,
  ) async {
    final (:c, :p) = await pumpSurface(tester);
    p.durationMs = 60000;
    // Premier Slider = progression ; drag vers ~50 %.
    final slider = find.byType(Slider).first;
    await tester.drag(slider, const Offset(300, 0));
    await tester.pump();
    expect(p.seeked, isNotNull);
    expect(p.seeked!.inMilliseconds, greaterThan(0));
    expect(p.seeked!.inMilliseconds, lessThan(60000));
  });

  testWidgets('183 — volume slider + bouton mute', (tester) async {
    final (:c, :p) = await pumpSurface(tester);
    // Second Slider = volume (1.0 initial) — drag à gauche.
    final vol = find.byType(Slider).last;
    await tester.drag(vol, const Offset(-40, 0));
    await tester.pump();
    expect(p.volume, lessThan(1.0));
    // Le bouton mute par son tooltip (l'icône dépend du
    // niveau atteint par le drag — volume_off à 0).
    await tester.tap(find.byTooltip('Couper le son'));
    await tester.pump();
    expect(p.muted, isTrue);
    expect(find.byIcon(Icons.volume_off), findsOneWidget);
  });

  testWidgets('183 — buffering → spinner, ended → bouton replay', (
    tester,
  ) async {
    final (:c, :p) = await pumpSurface(tester);
    p.state = 'buffering';
    p.notify();
    await tester.pump();
    expect(find.byType(CircularProgressIndicator), findsOneWidget);
    p.state = 'ended';
    p.notify();
    await tester.pump();
    // `replay` apparaît deux fois : bouton central +
    // bouton lecture de la barre (même icône en `ended`).
    expect(find.byIcon(Icons.replay), findsNWidgets(2));
    await tester.tap(find.byIcon(Icons.replay).first);
    await tester.pump();
    await tester.pump();
    expect(p.seeked, Duration.zero);
    expect(p.plays, 1);
  });

  testWidgets('183 — auto-masquage en lecture, ré-apparition à pause', (
    tester,
  ) async {
    const config = AppConfig(
      mediaControlsHideDelay: Duration(milliseconds: 300),
      mediaControlsFade: Duration(milliseconds: 50),
    );
    final (:c, :p) = await pumpSurface(tester, config: config);
    await p.play();
    await tester.pump();
    // Visible au départ, puis fondue après `hideDelay`.
    expect(
      tester.widget<AnimatedOpacity>(
        find.byType(AnimatedOpacity),
      ).opacity,
      1.0,
    );
    await tester.pump(const Duration(milliseconds: 400));
    expect(
      tester.widget<AnimatedOpacity>(
        find.byType(AnimatedOpacity),
      ).opacity,
      0.0,
    );
    // Pause → la barre ré-apparaît et reste.
    await p.pause();
    await tester.pump();
    expect(
      tester.widget<AnimatedOpacity>(
        find.byType(AnimatedOpacity),
      ).opacity,
      1.0,
    );
    await tester.pump(const Duration(milliseconds: 400));
    expect(
      tester.widget<AnimatedOpacity>(
        find.byType(AnimatedOpacity),
      ).opacity,
      1.0,
    );
  });

  testWidgets('183 — <audio controls> : barre fixe, jamais masquée', (
    tester,
  ) async {
    final (:c, :p) = await pumpSurface(
      tester,
      config: const AppConfig(
        mediaControlsHideDelay: Duration(milliseconds: 300),
      ),
      offer: const MediaOffer(
        id: 7,
        kind: 'audio',
        rect: Rect.fromLTWH(0, 0, 640, 48),
        url: 'http://h/a.mp3',
        controls: true,
      ),
    );
    expect(find.byIcon(Icons.play_arrow), findsOneWidget);
    await p.play();
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 500));
    // Audio : toujours visible en lecture (pas de vidéo à
    // découvrir) — la barre EST la surface.
    expect(
      tester.widget<AnimatedOpacity>(
        find.byType(AnimatedOpacity),
      ).opacity,
      1.0,
    );
  });

  testWidgets('183 — hors controls : aucune UI native', (tester) async {
    await pumpSurface(
      tester,
      offer: const MediaOffer(
        id: 7,
        kind: 'video',
        rect: Rect.fromLTWH(0, 0, 640, 360),
        url: 'http://h/v.mp4',
      ),
    );
    expect(find.byType(Slider), findsNothing);
    expect(find.byIcon(Icons.play_arrow), findsNothing);
    expect(find.byType(AnimatedOpacity), findsNothing);
  });

  /// Étape 186 — arbitrage pointer M2 : la sonde `/hit`
  /// décide si le tap tombe sur la vidéo nue (`media` = id
  /// de la surface → forward + toggle) ou sur un contrôle
  /// DOM du site peint par-dessus (`media` null → forward
  /// **seul**, pas de double détente). Sonde en échec ou
  /// sans la clé → repli M0 (toggle conservé).
  testWidgets(
      '186 — tap vidéo : media→toggle, couvert→forward seul, échec→repli',
      (tester) async {
    final (:c, :p) = await pumpSurface(tester);
    final repo = c.read(browserRepositoryProvider) as FakeRepo;
    final actions = repo.actions;
    final surface = find.byType(MediaSurfaceView);

    // Vidéo nue — la sonde rend notre surface : toggle + clic
    // (paused → play : `plays` monte).
    repo.nextHit =
        (link: null, image: null, media: 7, mediaKnown: true);
    await tester.tap(surface);
    await tester.pump();
    await tester.pump();
    expect(p.plays, 1);
    expect(actions, contains('input:click'));

    // Un contrôle DOM du site recouvre le point (nœud
    // topmost hors de l'élément) → forward **seul** : le
    // site reçoit son clic, aucun toggle hôte.
    repo.nextHit =
        (link: null, image: null, media: null, mediaKnown: true);
    actions.clear();
    await tester.tap(surface);
    await tester.pump();
    await tester.pump();
    expect(p.plays, 1);
    expect(p.pauses, 0);
    expect(actions, contains('input:click'));

    // Daemon antérieur : la clé `media` n'est pas émise →
    // `mediaKnown` faux → repli double détente M0 (playing →
    // pause : `pauses` monte).
    repo.nextHit =
        (link: null, image: null, media: null, mediaKnown: false);
    actions.clear();
    await tester.tap(surface);
    await tester.pump();
    await tester.pump();
    expect(p.pauses, 1);
    expect(actions, contains('input:click'));

    // Sonde en échec (`null` → le repo lève) → repli M0
    // (paused → play de nouveau).
    repo.nextHit = null;
    actions.clear();
    await tester.tap(surface);
    await tester.pump();
    await tester.pump();
    expect(p.plays, 2);
    expect(actions, contains('input:click'));
  });

  testWidgets('196 — cue sous-titre affichée + menu pistes', (
    tester,
  ) async {
    final (:c, :p) = await pumpSurface(tester);
    p
      ..tracks = const [
        MediaTrack(
          index: 0,
          kind: MediaTrack.kindAudio,
          title: 'Voix FR',
        ),
        MediaTrack(index: 1, kind: MediaTrack.kindAudio, language: 'eng'),
        MediaTrack(
          index: 2,
          kind: MediaTrack.kindSubtitle,
          language: 'fre',
        ),
      ]
      ..subtitle = 'Bonjour le monde'
      ..audioTrack = 0
      ..notify();
    await tester.pump();
    // Cue posée sur la surface, indépendante de la barre.
    expect(find.text('Bonjour le monde'), findsOneWidget);
    await tester.tap(find.byIcon(Icons.subtitles));
    await tester.pump(const Duration(milliseconds: 400));
    expect(find.text('Voix FR'), findsWidgets);
    expect(find.text('Off'), findsOneWidget);
    await tester.pump(const Duration(milliseconds: 400));
    await tester.tap(
      find.ancestor(
        of: find.text('eng'),
        matching: find.byType(InkWell),
      ),
    );
    await tester.pump(const Duration(milliseconds: 400));
    expect(p.selectedAudio, 1);
    expect(p.audioTrack, 1);
  });

  testWidgets('197 — menu chapitres + prev/next bornés', (
    tester,
  ) async {
    final (:c, :p) = await pumpSurface(tester);
    p
      ..chapters = const [
        MediaChapter(index: 0, startMs: 0, endMs: 60000, title: 'Intro'),
        MediaChapter(index: 1, startMs: 60000, title: 'Combat'),
      ]
      ..chapterIdx = 0
      ..notify();
    await tester.pump();
    // Prev borné à 0, next avance.
    await tester.tap(find.byIcon(Icons.skip_previous));
    await tester.pump();
    expect(p.seekedChapter, 0);
    await tester.tap(find.byIcon(Icons.skip_next));
    await tester.pump();
    expect(p.seekedChapter, 1);
    // Liste titrée + coche courante.
    await tester.tap(find.byIcon(Icons.bookmark_border));
    await tester.pump(const Duration(milliseconds: 400));
    await tester.pump(const Duration(milliseconds: 400));
    expect(find.text('0:00 — Intro'), findsOneWidget);
    expect(find.text('1:00 — Combat'), findsOneWidget);
  });
}

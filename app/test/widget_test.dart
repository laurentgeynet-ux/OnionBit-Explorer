// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'package:explorer_app/core/api/events_client.dart';
import 'package:explorer_app/core/design/app_theme.dart';
import 'package:explorer_app/core/design/design_tokens.dart';
import 'package:explorer_app/core/di/providers.dart';
import 'package:explorer_app/features/browser/presentation/browser_page.dart';
import 'package:explorer_app/main.dart';
import 'package:flutter/material.dart';
import 'package:explorer_app/core/l10n/app_localizations.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'browser_test_fakes.dart';

void main() {
  testWidgets('app démarre et affiche la coquille navigateur', (
    WidgetTester tester,
  ) async {
    await tester.pumpWidget(const ProviderScope(child: ExplorerApp()));
    await tester.pump();
    // Le champ omnibox est présent (tokens de repli utilisés hors bundle).
    expect(find.byType(TextField), findsOneWidget);
    expect(find.byIcon(Icons.search), findsOneWidget);
    // Étape 57 : étoile signet + bibliothèque + réglages.
    expect(find.byIcon(Icons.star_border), findsOneWidget);
    expect(find.byIcon(Icons.history), findsOneWidget);
    expect(find.byIcon(Icons.bookmarks_outlined), findsOneWidget);
    expect(find.byIcon(Icons.settings_outlined), findsOneWidget);
  });

  testWidgets('navbar fenêtre étroite : trailing sans overflow', (
    WidgetTester tester,
  ) async {
    // Repro étape 80 (run M22, fenêtre plus étroite que la
    // session manuelle) : à ~380 px les IconButton fixes
    // poussent le trailing hors de la frame.
    tester.view.physicalSize = const Size(380, 600);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    // Tokens synchrones en override — le test porte sur le
    // layout navbar, pas sur le chargement des assets (vrai
    // async, flaky sous charge de suite).
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          designTokensProvider.overrideWith(
            (ref) => Future.value(DesignTokens.fallback),
          ),
          lightTokensProvider.overrideWith(
            (ref) => Future.value(DesignTokens.fallbackLight),
          ),
        ],
        child: const ExplorerApp(),
      ),
    );
    await tester.pump();
    await tester.pump();
    // RenderFlex overflow → FlutterError pendant le layout.
    expect(tester.takeException(), isNull);
    // Le trailing est replié dans le menu « ⋮ » : chaque action
    // reste joignable en une tape.
    expect(find.byIcon(Icons.more_vert), findsOneWidget);
    await tester.tap(find.byIcon(Icons.more_vert));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 300));
    for (final label in const [
      'Historique',
      'Signets',
      'Rechercher dans la page',
      'Zoom avant',
      'Réglages',
    ]) {
      expect(find.text(label), findsOneWidget, reason: '$label absent du menu');
    }
  });

  testWidgets('omnibox suit l’URL après navigation par lien', (
    WidgetTester tester,
  ) async {
    // Terrain 23/10 : `list_tabs` rapportait fr.wikipedia.org
    // pendant que la barre gardait www.wikipedia.com — le
    // listener ne sélectionnait que `activeId`, inchangé quand
    // l'onglet courant navigue en interne.
    final repo = FakeRepo();
    final events = FakeEvents();
    final c = makeContainer(repo, events);
    addTearDown(c.dispose);
    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: c,
        child: MaterialApp(
          localizationsDelegates:
              AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          locale: const Locale('fr'),
          home: BrowserPage(tokens: DesignTokens.fallback),
        ),
      ),
    );
    await tester.pump();
    await tester.pump();
    // Navigation interne : l'événement `navigated` met à jour
    // `active.url` sans toucher `activeId`.
    events.ctrl.add(
      ServerEvent('navigated', {
        'tab': '1',
        'url': 'https://fr.wikipedia.org/wiki/Accueil',
        'title': 'Accueil',
      }),
    );
    await tester.pump();
    await tester.pump();
    final field = tester.widget<TextField>(find.byType(TextField));
    expect(field.controller?.text, 'https://fr.wikipedia.org/wiki/Accueil');
  });

  testWidgets('barre de statut affiche le moteur actif', (
    WidgetTester tester,
  ) async {
    // Étape 119 — piège `engineMode=auto` : le choix FFI/REST
    // doit rester visible en permanence (une dll périmée
    // adjacente détournait les tests en silence).
    final c = makeContainer(FakeRepo(), FakeEvents());
    addTearDown(c.dispose);
    await tester.pumpWidget(
      UncontrolledProviderScope(
        container: c,
        child: MaterialApp(
          localizationsDelegates:
              AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          locale: const Locale('fr'),
          home: BrowserPage(tokens: DesignTokens.fallback),
        ),
      ),
    );
    await tester.pump();
    await tester.pump();
    expect(find.text('TEST fake'), findsOneWidget);
  });

  test('tokens de repli exposent les sections clés', () {
    final t = DesignTokens.fallback;
    expect(t.color('surface', 'base'), isA<Color>());
    expect(t.dim('spacing', 'md'), greaterThan(0));
    expect(t.dim('radius', 'md'), greaterThan(0));
    expect(t.str('typography', 'fontFamily'), isNotEmpty);
  });

  test('variante claire : scheme + thème dérivé en mode light', () {
    expect(DesignTokens.fallback.scheme, 'dark');
    expect(DesignTokens.fallbackLight.scheme, 'light');
    expect(buildTheme(DesignTokens.fallback).brightness, Brightness.dark);
    expect(buildTheme(DesignTokens.fallbackLight).brightness, Brightness.light);
  });
}

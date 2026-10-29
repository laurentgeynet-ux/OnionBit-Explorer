// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'core/l10n/app_localizations.dart';

import 'core/design/app_theme.dart';
import 'core/design/design_tokens.dart';
import 'core/di/providers.dart';
import 'core/settings/app_settings.dart';
import 'features/browser/presentation/browser_page.dart';

void main() {
  runApp(const ProviderScope(child: ExplorerApp()));
}

/// Racine de l'app — attend le chargement des tokens (source de
/// vérité ADR-0005) puis construit les thèmes dérivés. Splash
/// minimal pendant la lecture. `AppSettings.theme` choisit la
/// variante claire/sombre/système (étape 57) : `MaterialApp`
/// reçoit `theme` (clair) + `darkTheme` (sombre) + `themeMode`.
class ExplorerApp extends ConsumerWidget {
  const ExplorerApp({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final dark = ref.watch(designTokensProvider);
    final light = ref.watch(lightTokensProvider);
    final themeMode = switch (ref.watch(settingsProvider).theme) {
      'light' => ThemeMode.light,
      'dark' => ThemeMode.dark,
      _ => ThemeMode.system,
    };

    // Les deux jeux de tokens doivent être prêts avant de
    // construire le thème actif ; chaque état dégrade vers son
    // repli embarqué (jamais d'écran noir sur asset absent).
    final darkTokens = dark.value ?? DesignTokens.fallback;
    final lightTokens = light.value ?? DesignTokens.fallbackLight;
    final loading = dark.isLoading || light.isLoading;
    // `system` suit la luminosité OS — les tokens passés aux
    // widgets doivent refléter le thème réellement actif.
    final lightActive =
        themeMode == ThemeMode.light ||
        (themeMode == ThemeMode.system &&
            MediaQuery.platformBrightnessOf(context) == Brightness.light);
    final active = lightActive ? lightTokens : darkTokens;

    return MaterialApp(
      debugShowCheckedModeBanner: false,
      // i18n fr/en — locale choisie une fois au premier
      // lancement (`uiLanguage`, pas un réglage quotidien) ;
      // `''` → français (défaut produit).
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales,
      locale: Locale(
        ref.watch(settingsProvider).uiLanguage.isEmpty
            ? 'fr'
            : ref.watch(settingsProvider).uiLanguage,
      ),
      theme: buildTheme(lightTokens),
      darkTheme: buildTheme(darkTokens),
      themeMode: themeMode,
      home: loading
          ? const Scaffold(body: Center(child: CircularProgressIndicator()))
          : BrowserPage(tokens: active),
    );
  }
}

// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/api_client.dart';
import '../api/events_client.dart';
import '../config/app_config.dart';
import '../design/design_tokens.dart';
import '../settings/app_settings.dart';

/// Configuration runtime — surchargeable en test via `override`.
final configProvider = Provider<AppConfig>((ref) => const AppConfig());

/// Tokens de design — chargés au démarrage (`DesignTokens.load`),
/// repli embarqué si l'asset est absent. C'est la variante
/// **sombre** (`meta.scheme = "dark"`).
final designTokensProvider = FutureProvider<DesignTokens>(
  (ref) => DesignTokens.load(),
);

/// Variante **claire** des tokens (`tokens.light.json`) — chargée
/// une fois, utilisée quand `AppSettings.theme` l'exige
/// (étape 57).
final lightTokensProvider = FutureProvider<DesignTokens>(
  (ref) => DesignTokens.loadVariant('light'),
);

/// Mode moteur effectif — `AppSettings.engine` l'emporte ;
/// `'auto'` défère à `AppConfig.engineMode` (env `EXPLORER_ENGINE`).
final effectiveEngineModeProvider = Provider<String>((ref) {
  // `.select` — seule la clé `engine` déclenche la re-création ;
  // sans elle, tout réglage (thème…) reconstruisait le repo et
  // ré-exécutait `BrowserController.build()` sur le même
  // notifier (late final réassigné → LateInitializationError).
  final s = ref.watch(settingsProvider.select((s) => s.engine));
  return s == 'auto' ? ref.watch(configProvider).engineMode : s;
});

/// Client REST du daemon (dispose au shutdown du provider scope).
final apiClientProvider = Provider<ApiClient>((ref) {
  final c = ApiClient(ref.watch(configProvider));
  ref.onDispose(c.dispose);
  return c;
});

/// Canal SSE `GET /api/events`.
final eventsClientProvider = Provider<EventsClient>((ref) {
  final c = EventsClient(ref.watch(configProvider));
  ref.onDispose(c.dispose);
  return c;
});

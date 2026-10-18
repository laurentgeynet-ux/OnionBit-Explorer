// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/api_client.dart';
import '../api/events_client.dart';
import '../config/app_config.dart';
import '../design/design_tokens.dart';

/// Configuration runtime — surchargeable en test via `override`.
final configProvider = Provider<AppConfig>((ref) => const AppConfig());

/// Tokens de design — chargés au démarrage (`DesignTokens.load`),
/// repli embarqué si l'asset est absent.
final designTokensProvider = FutureProvider<DesignTokens>(
  (ref) => DesignTokens.load(),
);

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

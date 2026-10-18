// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'core/design/app_theme.dart';
import 'core/design/design_tokens.dart';
import 'core/di/providers.dart';
import 'features/browser/presentation/browser_page.dart';

void main() {
  runApp(const ProviderScope(child: ExplorerApp()));
}

/// Racine de l'app — attend le chargement des tokens (source de
/// vérité ADR-0005) puis construit le thème dérivé. Splash minimal
/// pendant la lecture (la connexion daemon viendra s'y greffer).
class ExplorerApp extends ConsumerWidget {
  const ExplorerApp({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final tokens = ref.watch(designTokensProvider);
    return tokens.when(
      loading: () => MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: buildTheme(DesignTokens.fallback),
        home: const Scaffold(body: Center(child: CircularProgressIndicator())),
      ),
      error: (_, _) => MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: buildTheme(DesignTokens.fallback),
        home: BrowserPage(tokens: DesignTokens.fallback),
      ),
      data: (t) => MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: buildTheme(t),
        home: BrowserPage(tokens: t),
      ),
    );
  }
}

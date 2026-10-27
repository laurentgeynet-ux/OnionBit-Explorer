// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'package:flutter/material.dart';

import 'features/browser/presentation/delegated_webview.dart';

/// PoC 136a — embed minimal d'une WebView2 dans l'app Flutter,
/// aucun wiring navigate. Lanceur séparé :
/// `flutter run -d windows -t lib/poc_delegated.dart`.
///
/// URL cible par `--dart-define=POC_URL=…` (défaut : la page
/// de test DRM publique de Bitmovin — Widevine/PlayReady
/// mesurables **sans compte** : la pastille de statut CDM et
/// la lecture de test se vérifient en vrai). Exemples :
///   POC_URL=https://bitmovin.com/demos/drm-test/
///   POC_URL=https://www.youtube.com/   (rendu + MSE natifs)
void main() {
  runApp(const _DelegatedPocApp());
}

class _DelegatedPocApp extends StatelessWidget {
  const _DelegatedPocApp();

  static const _url = String.fromEnvironment(
    'POC_URL',
    defaultValue: 'https://bitmovin.com/demos/drm-test/',
  );

  @override
  Widget build(BuildContext context) {
    return const MaterialApp(
      home: Scaffold(body: DelegatedWebView(url: _url)),
    );
  }
}

// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'package:flutter/material.dart';

import 'features/browser/presentation/delegated_webview.dart';

/// PoC 136a — embed minimal d'une WebView2 dans l'app Flutter,
/// aucun wiring navigate. Lanceur séparé :
/// `flutter run -d windows -t lib/poc_delegated.dart`.
///
/// Charge une page DRM-capable pour vérifier que le HWND natif
/// s'affiche ; la lecture DRM réelle (login requis) se mesure
/// au terrain, pas ici.
void main() {
  runApp(const _DelegatedPocApp());
}

class _DelegatedPocApp extends StatelessWidget {
  const _DelegatedPocApp();

  @override
  Widget build(BuildContext context) {
    return const MaterialApp(
      home: Scaffold(
        body: DelegatedWebView(url: 'https://www.netflix.com/'),
      ),
    );
  }
}

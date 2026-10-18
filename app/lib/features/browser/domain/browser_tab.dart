// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Entité métier `BrowserTab` — miroir d'un onglet côté daemon
/// (le moteur possède le contenu ; l'app ne garde que l'état
/// d'affichage et l'id).
class BrowserTab {
  const BrowserTab({
    required this.id,
    this.title = '',
    this.url = 'about:blank',
    this.loading = false,
  });

  final String id;
  final String title;
  final String url;
  final bool loading;

  BrowserTab copyWith({String? title, String? url, bool? loading}) =>
      BrowserTab(
        id: id,
        title: title ?? this.title,
        url: url ?? this.url,
        loading: loading ?? this.loading,
      );
}

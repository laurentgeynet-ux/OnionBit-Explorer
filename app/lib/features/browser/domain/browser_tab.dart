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
    this.canBack = false,
    this.canForward = false,
  });

  final String id;
  final String title;
  final String url;
  final bool loading;
  final bool canBack;
  final bool canForward;

  /// Reconstruit depuis le DTO JSON de l'API
  /// (`{id, url, title, can_back, can_forward}`).
  factory BrowserTab.fromJson(Map<String, dynamic> j) => BrowserTab(
        id: '${j['id']}',
        title: (j['title'] as String?) ?? '',
        url: (j['url'] as String?) ?? 'about:blank',
        canBack: j['can_back'] == true,
        canForward: j['can_forward'] == true,
      );

  BrowserTab copyWith({
    String? title,
    String? url,
    bool? loading,
    bool? canBack,
    bool? canForward,
  }) =>
      BrowserTab(
        id: id,
        title: title ?? this.title,
        url: url ?? this.url,
        loading: loading ?? this.loading,
        canBack: canBack ?? this.canBack,
        canForward: canForward ?? this.canForward,
      );
}

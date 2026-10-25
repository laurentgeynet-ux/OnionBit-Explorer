// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Nature de l'onglet — étape 136 (ADR-0027) :
/// `engine` = contenu rendu par le moteur (daemon/FFI) ;
/// `delegated` = surface WebView2 native pour domaine DRM
/// allowlisté — le moteur n'est jamais contacté pour ces
/// domaines. Le kind se décide **par URL à chaque
/// navigation** : un même onglet peut passer engine →
/// delegated (navigation vers Netflix) puis delegated →
/// engine (lien sortant) — l'onglet daemon dormant derrière
/// un onglet délégué reprend la main au retour.
enum TabKind { engine, delegated }

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
    this.kind = TabKind.engine,
  });

  final String id;
  final String title;
  final String url;
  final bool loading;
  final bool canBack;
  final bool canForward;

  /// `engine` (défaut — tout onglet venant du daemon) ou
  /// `delegated` (posé par l'interception allowlist côté
  /// contrôleur, jamais par le DTO).
  final TabKind kind;

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
    TabKind? kind,
  }) =>
      BrowserTab(
        id: id,
        title: title ?? this.title,
        url: url ?? this.url,
        loading: loading ?? this.loading,
        canBack: canBack ?? this.canBack,
        canForward: canForward ?? this.canForward,
        kind: kind ?? this.kind,
      );
}

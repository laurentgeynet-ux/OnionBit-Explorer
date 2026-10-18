// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Configuration de l'app — toutes les valeurs « en dur » vivent ici
/// (AGENTS.md §4.13 : aucune valeur en dur ailleurs).
class AppConfig {
  const AppConfig({
    this.daemonBaseUrl = 'http://127.0.0.1:8321',
    this.apiKey = '',
    this.connectTimeout = const Duration(seconds: 5),
    this.requestTimeout = const Duration(seconds: 10),
    this.sseReconnectDelay = const Duration(seconds: 2),
  });

  /// URL de base du daemon `explorer-daemon` (loopback — ADR-0003).
  final String daemonBaseUrl;

  /// Clé d'API optionnelle (`x-api-key` / `?key=` pour le SSE).
  final String apiKey;

  /// Timeout de connexion et de requête REST.
  final Duration connectTimeout;
  final Duration requestTimeout;

  /// Délai avant reconnexion du canal SSE `GET /api/events`.
  final Duration sseReconnectDelay;
}

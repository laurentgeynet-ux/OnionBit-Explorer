// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Configuration de l'app — toutes les valeurs « en dur » vivent ici
/// (AGENTS.md §4.13 : aucune valeur en dur ailleurs).
class AppConfig {
  const AppConfig({
    this.daemonBaseUrl = 'http://127.0.0.1:9377',
    this.apiKey = '',
    this.connectTimeout = const Duration(seconds: 5),
    this.requestTimeout = const Duration(seconds: 10),
    this.sseReconnectDelay = const Duration(seconds: 2),
    this.daemonRetryDelay = const Duration(seconds: 3),
    this.engineMode = const String.fromEnvironment('EXPLORER_ENGINE',
        defaultValue: 'auto'),
    this.engineLibPath = const String.fromEnvironment('EXPLORER_ENGINE_LIB',
        defaultValue: ''),
  });

  /// URL de base du daemon `explorer-daemon` (loopback — ADR-0003).
  final String daemonBaseUrl;

  /// Backend moteur : `auto` (FFI si la DLL est présente, REST
  /// sinon), `rest` (daemon seul), `ffi` (in-process seul —
  /// échoue si la bibliothèque ne charge pas). ADR-0018.
  final String engineMode;

  /// Chemin explicite vers `explorer_ffi.dll` — vide = noms
  /// standards dans le répertoire de l'exécutable / `PATH`.
  final String engineLibPath;

  /// Clé d'API optionnelle (`x-api-key` / `?key=` pour le SSE).
  final String apiKey;

  /// Timeout de connexion et de requête REST.
  final Duration connectTimeout;
  final Duration requestTimeout;

  /// Délai avant reconnexion du canal SSE `GET /api/events`.
  final Duration sseReconnectDelay;

  /// Cadence de re-bootstrap tant que le daemon est injoignable —
  /// `daemonOnline` repasse à `true` dès le premier `listTabs`
  /// réussi (étape 81 : récupération automatique, pas seulement
  /// au bouton « Réessayer »).
  final Duration daemonRetryDelay;
}

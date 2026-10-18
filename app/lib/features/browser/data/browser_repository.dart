// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import '../../../core/api/api_client.dart';
import '../domain/browser_tab.dart';

/// Accès aux onglets via le contrat REST du daemon (ADR-0003) :
/// `POST /api/tabs`, `POST …/navigate`, `DELETE /api/tabs/{id}`.
/// Les DTO JSON ne fuient pas au-delà de cette couche.
class BrowserRepository {
  const BrowserRepository(this._api);

  final ApiClient _api;

  /// Crée un onglet côté daemon et le traduit en entité métier.
  Future<BrowserTab> createTab() async {
    final res = await _api.postJson('/api/tabs');
    if (res is Map<String, dynamic> && res['id'] is String) {
      return BrowserTab(id: res['id'] as String);
    }
    throw const ApiException('POST /api/tabs : réponse inattendue');
  }

  /// Demande la navigation d'un onglet vers `url`.
  Future<void> navigate(String tabId, String url) =>
      _api.postJson('/api/tabs/$tabId/navigate', body: {'url': url});

  /// Ferme l'onglet côté daemon.
  Future<void> closeTab(String tabId) => _api.deleteJson('/api/tabs/$tabId');

  /// Frame PNG courante de l'onglet (`GET …/frame`).
  Future<List<int>> fetchFrame(String tabId) =>
      _api.getBytes('/api/tabs/$tabId/frame');
}

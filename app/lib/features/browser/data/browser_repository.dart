// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import '../../../core/api/api_client.dart';
import '../domain/browser_tab.dart';

/// Accès aux onglets via le contrat REST du daemon (ADR-0003) :
/// `GET/POST /api/tabs`, `…/navigate`, `…/reload`, `…/back`,
/// `…/forward`, `DELETE /api/tabs/{id}` et `GET …/frame`.
/// Les DTO JSON ne fuient pas au-delà de cette couche.
class BrowserRepository {
  const BrowserRepository(this._api);

  final ApiClient _api;

  /// Crée un onglet côté daemon et le traduit en entité métier.
  Future<BrowserTab> createTab() async {
    final res = await _api.postJson('/api/tabs');
    if (res is Map<String, dynamic> && res['id'] != null) {
      return BrowserTab(id: '${res['id']}');
    }
    throw const ApiException('POST /api/tabs : réponse inattendue');
  }

  /// Liste les onglets (`GET /api/tabs`).
  Future<List<BrowserTab>> listTabs() async {
    final res = await _api.getJson('/api/tabs');
    if (res is Map<String, dynamic> && res['tabs'] is List) {
      return (res['tabs'] as List)
          .whereType<Map<String, dynamic>>()
          .map(BrowserTab.fromJson)
          .toList();
    }
    throw const ApiException('GET /api/tabs : réponse inattendue');
  }

  /// Demande la navigation d'un onglet vers `url` — renvoie
  /// l'état résultant de l'onglet.
  Future<BrowserTab?> navigate(String tabId, String url) async {
    final res =
        await _api.postJson('/api/tabs/$tabId/navigate', body: {'url': url});
    return res is Map<String, dynamic> ? BrowserTab.fromJson(res) : null;
  }

  /// `POST …/reload|back|forward` → état résultant.
  Future<BrowserTab?> _action(String tabId, String route) async {
    final res = await _api.postJson('/api/tabs/$tabId/$route');
    return res is Map<String, dynamic> ? BrowserTab.fromJson(res) : null;
  }

  /// Recharge la page courante.
  Future<BrowserTab?> reload(String tabId) => _action(tabId, 'reload');

  /// Page précédente de l'historique local.
  Future<BrowserTab?> back(String tabId) => _action(tabId, 'back');

  /// Page suivante de l'historique local.
  Future<BrowserTab?> forward(String tabId) => _action(tabId, 'forward');

  /// Ferme l'onglet côté daemon.
  Future<void> closeTab(String tabId) => _api.deleteJson('/api/tabs/$tabId');

  /// Frame PNG courante de l'onglet (`GET …/frame`) — `null`
  /// si l'onglet n'a jamais rendu (204).
  Future<List<int>?> fetchFrame(String tabId) async {
    try {
      final bytes = await _api.getBytes('/api/tabs/$tabId/frame');
      return bytes.isEmpty ? null : bytes;
    } on ApiException {
      return null;
    }
  }

  /// Événement d'entrée (`POST …/input`) — `click`,
  /// `mousemove`, `scroll`, `key` (étape 26).
  Future<void> sendInput(String tabId, Map<String, dynamic> ev) =>
      _api.postJson('/api/tabs/$tabId/input', body: ev);

  /// Viewport moteur (`POST …/viewport`) — envoyé au resize.
  Future<void> setViewport(String tabId, double w, double h, double scale) =>
      _api.postJson('/api/tabs/$tabId/viewport',
          body: {'w': w, 'h': h, 'scale': scale});
}

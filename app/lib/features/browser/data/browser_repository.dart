// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import '../../../core/api/api_client.dart';
import '../domain/bookmark.dart';
import '../domain/browser_tab.dart';
import '../domain/find_result.dart';
import '../domain/history_entry.dart';

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
    final res = await _api.postJson(
      '/api/tabs/$tabId/navigate',
      body: {'url': url},
    );
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
  /// `mousemove`, `scroll`, `key` (étape 26). Retourne le
  /// résultat (`{"result":"hover","url":…,"editable":…}`…).
  Future<Map<String, dynamic>> sendInput(
    String tabId,
    Map<String, dynamic> ev,
  ) async {
    final res = await _api.postJson('/api/tabs/$tabId/input', body: ev);
    return res is Map<String, dynamic> ? res : const {};
  }

  /// Viewport moteur (`POST …/viewport`) — envoyé au resize.
  /// `scale` = facteur DPI : l'app compose zoom × dpr (étape 57).
  Future<void> setViewport(String tabId, double w, double h, double scale) =>
      _api.postJson(
        '/api/tabs/$tabId/viewport',
        body: {'w': w, 'h': h, 'scale': scale},
      );

  // ------------------------------------------------------------------
  // Bibliothèque — signets + historique du store moteur (étape 57).
  // ------------------------------------------------------------------

  /// Signets persistés (`GET /api/bookmarks` → `{bookmarks:[…]}`).
  Future<List<Bookmark>> bookmarks() async {
    final res = await _api.getJson('/api/bookmarks');
    if (res is Map<String, dynamic> && res['bookmarks'] is List) {
      return (res['bookmarks'] as List)
          .whereType<Map<String, dynamic>>()
          .map(Bookmark.fromJson)
          .where((b) => b.url.isNotEmpty)
          .toList();
    }
    throw const ApiException('GET /api/bookmarks : réponse inattendue');
  }

  /// Ajoute un signet (`POST /api/bookmarks`) — `false` si déjà
  /// présent (409 dédupliqué côté moteur).
  Future<bool> addBookmark(String url, {String title = ''}) async {
    try {
      await _api.postJson('/api/bookmarks', body: {'url': url, 'title': title});
      return true;
    } on ApiException catch (e) {
      if (e.statusCode == 409) return false;
      rethrow;
    }
  }

  /// Retire un signet (`DELETE /api/bookmarks` corps `{url}`) —
  /// `false` si absent (404).
  Future<bool> removeBookmark(String url) async {
    try {
      await _api.deleteJson('/api/bookmarks', body: {'url': url});
      return true;
    } on ApiException catch (e) {
      if (e.statusCode == 404) return false;
      rethrow;
    }
  }

  /// Historique persisté (`GET /api/history` → `{history:[…]}`,
  /// le plus récent d'abord).
  Future<List<HistoryEntry>> history() async {
    final res = await _api.getJson('/api/history');
    if (res is Map<String, dynamic> && res['history'] is List) {
      return (res['history'] as List)
          .whereType<Map<String, dynamic>>()
          .map(HistoryEntry.fromJson)
          .where((e) => e.url.isNotEmpty)
          .toList();
    }
    throw const ApiException('GET /api/history : réponse inattendue');
  }

  /// Recherche dans la page (`POST /api/tabs/{id}/find`,
  /// étape 57bis). `dir = 0` lance une nouvelle recherche ;
  /// ±1 navigue parmi les occurrences. Le surlignage est
  /// peint côté moteur — la frame suivante le montre.
  Future<FindResult> find(String tabId, String query, {int dir = 0}) async {
    final res = await _api.postJson(
      '/api/tabs/$tabId/find',
      body: {'query': query, 'dir': dir},
    );
    if (res is Map<String, dynamic>) {
      return FindResult.fromJson(res, query: query);
    }
    throw const ApiException('POST /find : réponse inattendue');
  }
}

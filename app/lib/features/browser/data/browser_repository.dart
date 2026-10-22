// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:typed_data';

import '../../../core/api/api_client.dart';
import '../domain/bookmark.dart';
import '../domain/browser_tab.dart';
import '../domain/find_result.dart';
import '../domain/history_entry.dart';
import '../domain/media_offer.dart';
import '../domain/reload_result.dart';

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

  /// `POST …/back|forward` → état résultant.
  Future<BrowserTab?> _action(String tabId, String route) async {
    final res = await _api.postJson('/api/tabs/$tabId/$route');
    return res is Map<String, dynamic> ? BrowserTab.fromJson(res) : null;
  }

  /// Recharge la page courante (étape 93) : `confirm` lève le
  /// verrou de resoumission d'une méthode non sûre — sans lui
  /// le daemon répond `resubmit_confirm` au lieu de rejouer.
  Future<ReloadResult> reload(String tabId, {bool confirm = false}) async {
    final res = await _api.postJson(
      '/api/tabs/$tabId/reload',
      body: {'confirm': confirm},
    );
    if (res is Map<String, dynamic>) {
      return ReloadResult.fromJson(res);
    }
    throw const ApiException('POST /reload : réponse inattendue');
  }

  /// Page précédente de l'historique local.
  Future<BrowserTab?> back(String tabId) => _action(tabId, 'back');

  /// Page suivante de l'historique local.
  Future<BrowserTab?> forward(String tabId) => _action(tabId, 'forward');

  /// Ferme l'onglet côté daemon.
  Future<void> closeTab(String tabId) => _api.deleteJson('/api/tabs/$tabId');

  /// Frame PNG courante de l'onglet (`GET …/frame`) — `null`
  /// si l'onglet n'a jamais rendu (204).
  Future<Uint8List?> fetchFrame(String tabId) async {
    try {
      final bytes = await _api.getBytes('/api/tabs/$tabId/frame');
      return bytes.isEmpty ? null : bytes;
    } on ApiException {
      return null;
    }
  }

  /// PNG d'une région de frame (`GET …/frame?x&y&w&h`, étape
  /// 55 — pixels **physiques**, mêmes unités que les `rects`
  /// de `frame_invalidated`). `null` si région vide/hors
  /// frame (204) ou daemon antérieur sans la route (il sert
  /// alors la frame entière — l'appelant détecte à la taille
  /// décodée et retombe en pull plein).
  Future<Uint8List?> fetchFrameRegion(
    String tabId,
    int x,
    int y,
    int w,
    int h,
  ) async {
    try {
      final bytes = await _api.getBytes(
        '/api/tabs/$tabId/frame',
        query: {'x': '$x', 'y': '$y', 'w': '$w', 'h': '$h'},
      );
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

  /// Schéma de couleurs préféré (`POST …/color-scheme`,
  /// étape 97) — `'dark'`/`'light'` propagé à `StyleEnv`
  /// (`prefers-color-scheme` + défauts UA). Réglage
  /// session-level : le serveur repeint l'onglet nommé.
  Future<void> setColorScheme(String tabId, String scheme) =>
      _api.postJson(
        '/api/tabs/$tabId/color-scheme',
        body: {'scheme': scheme},
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

  /// Accepte un téléchargement en attente (étape 70) :
  /// écrit le corps bufferisé sous le répertoire de
  /// téléchargement du daemon ( explicite possible).
  /// Retourne le chemin écrit.
  Future<String> acceptDownload(String tabId, int dlId, {String? path}) async {
    final res = await _api.postJson(
      '/api/tabs/$tabId/downloads/$dlId/accept',
      body: {'path': ?path},
    );
    if (res is Map<String, dynamic>) {
      return res['path'] as String? ?? '';
    }
    throw const ApiException('POST /downloads/accept : réponse inattendue');
  }

  /// Rejette un téléchargement : le corps est rejoué comme
  /// document (dégradation « dump texte » historique).
  Future<void> declineDownload(String tabId, int dlId) =>
      _api.postJson('/api/tabs/$tabId/downloads/$dlId/decline');

  // ------------------------------------------------------------------
  // Seam MediaSurface (étape 71, ADR-0024) — offres `media_offered`
  // SSE, claim/release/state REST. `rect` reste en px CSS document.
  // ------------------------------------------------------------------

  /// Surfaces média live d'un onglet (`GET …/media`).
  Future<List<MediaOffer>> listMedia(String tabId) async {
    final res = await _api.getJson('/api/tabs/$tabId/media');
    if (res is Map<String, dynamic> && res['media'] is List) {
      return (res['media'] as List)
          .whereType<Map<String, dynamic>>()
          .map(MediaOffer.fromJson)
          .toList();
    }
    throw const ApiException('GET /media : réponse inattendue');
  }

  /// Pose le claim sur une surface (`POST …/claim`) — `codecs`
  /// déclare ce que l'hôte sait jouer ; `false` si 409/410
  /// (déjà claimée ou morte — l'offre reste honnêtement
  /// non claimée, les stubs ADR-0023 continuent côté JS).
  Future<bool> claimMedia(
    String tabId,
    int mediaId,
    List<String> codecs,
  ) async {
    try {
      await _api.postJson(
        '/api/tabs/$tabId/media/$mediaId/claim',
        body: {'codecs': codecs},
      );
      return true;
    } on ApiException catch (e) {
      if (e.statusCode == 409 || e.statusCode == 410) return false;
      rethrow;
    }
  }

  /// Rend la surface au moteur (`POST …/release`) — ordre de
  /// nettoyage local : dispose d'abord, release ensuite.
  Future<void> releaseMedia(String tabId, int mediaId) =>
      _api.postJson('/api/tabs/$tabId/media/$mediaId/release');

  /// Rapporte l'état hôte (`POST …/state`) — le moteur le
  /// pousse dans le nœud média claimé (props + events DOM).
  /// Les appels doivent rester bornés (≤ 4 Hz, contrat).
  Future<void> mediaState(
    String tabId,
    int mediaId,
    Map<String, dynamic> report,
  ) => _api.postJson('/api/tabs/$tabId/media/$mediaId/state', body: report);
}

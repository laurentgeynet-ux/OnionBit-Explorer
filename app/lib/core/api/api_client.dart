// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import '../config/app_config.dart';

/// Erreur de l'API — soit transport (daemon injoignable), soit
/// erreur métier `{"error":{…}}` renvoyée par le daemon (ADR-0003).
class ApiException implements Exception {
  const ApiException(this.message, {this.statusCode});

  final String message;
  final int? statusCode;

  @override
  String toString() => 'ApiException($statusCode): $message';
}

/// Client REST minimal vers `explorer-daemon` — HTTP loopback via
/// `dart:io` (zéro dépendance). Chaque requête borne son temps par
/// `AppConfig.requestTimeout` ; les erreurs remontent typées.
class ApiClient {
  ApiClient(this.config, {HttpClient? http}) : _http = http ?? HttpClient();

  final AppConfig config;
  final HttpClient _http;

  Uri _uri(String path, [Map<String, String>? query]) {
    final base = Uri.parse(config.daemonBaseUrl);
    return base.replace(
      path: '${base.path}$path',
      queryParameters: query,
    );
  }

  void _sign(HttpClientRequest req) {
    if (config.apiKey.isNotEmpty) {
      req.headers.set('x-api-key', config.apiKey);
    }
  }

  /// `GET` → JSON décodé. Corps non-JSON → [ApiException].
  Future<dynamic> getJson(String path, {Map<String, String>? query}) =>
      _send('GET', path, query: query);

  /// `POST` avec corps JSON optionnel → JSON décodé.
  Future<dynamic> postJson(
    String path, {
    Map<String, dynamic>? body,
    Map<String, String>? query,
  }) =>
      _send('POST', path, body: body, query: query);

  /// `DELETE` → JSON décodé.
  Future<dynamic> deleteJson(String path) => _send('DELETE', path);

  /// `GET` → octets bruts (frame PNG — `GET /api/tabs/{id}/frame`).
  Future<List<int>> getBytes(String path) async {
    try {
      final req = await _http
          .getUrl(_uri(path))
          .timeout(config.requestTimeout);
      _sign(req);
      final res = await req.close().timeout(config.requestTimeout);
      final bytes = await res.expand((c) => c).toList();
      if (res.statusCode >= 400) {
        throw ApiException(_extractError(bytes), statusCode: res.statusCode);
      }
      return bytes;
    } on ApiException {
      rethrow;
    } on Object catch (e) {
      throw ApiException('daemon injoignable : $e');
    }
  }

  Future<dynamic> _send(
    String method,
    String path, {
    Map<String, dynamic>? body,
    Map<String, String>? query,
  }) async {
    try {
      final req = await _http
          .openUrl(method, _uri(path, query))
          .timeout(config.requestTimeout);
      _sign(req);
      if (body != null) {
        req.headers.contentType = ContentType.json;
        req.write(jsonEncode(body));
      }
      final res = await req.close().timeout(config.requestTimeout);
      final raw = await res.transform(utf8.decoder).join();
      if (res.statusCode >= 400) {
        throw ApiException(
          _extractError(utf8.encode(raw)),
          statusCode: res.statusCode,
        );
      }
      if (raw.isEmpty) return null;
      return jsonDecode(raw);
    } on ApiException {
      rethrow;
    } on Object catch (e) {
      throw ApiException('daemon injoignable : $e');
    }
  }

  /// Extrait `error.message` d'un corps d'erreur daemon, sinon le brut.
  static String _extractError(List<int> bytes) {
    try {
      final decoded = jsonDecode(utf8.decode(bytes, allowMalformed: true));
      if (decoded is Map && decoded['error'] is Map) {
        final msg = (decoded['error'] as Map)['message'];
        if (msg is String) return msg;
      }
    } on Object {
      // Corps non-JSON → tomber sur le brut tronqué.
    }
    final s = utf8.decode(bytes, allowMalformed: true);
    return s.length > 200 ? '${s.substring(0, 200)}…' : s;
  }

  void dispose() => _http.close();
}

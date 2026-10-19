// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import '../config/app_config.dart';

/// Un événement SSE `GET /api/events` — `event:` + `data:` (JSON).
class ServerEvent {
  const ServerEvent(this.name, this.data);

  final String name;
  final Map<String, dynamic> data;
}

/// Canal SSE vers le daemon — push des `frame_invalidated`,
/// `tab_state`, `title_changed`… (ADR-0003). Reconnecte avec délai
/// `AppConfig.sseReconnectDelay` tant que le stream n'est pas annulé.
class EventsClient {
  EventsClient(this.config, {HttpClient? http}) : _http = http ?? HttpClient();

  final AppConfig config;
  final HttpClient _http;
  StreamSubscription<Object>? _sub;
  bool _closed = false;
  Timer? _retryTimer;
  Completer<void>? _retryWake;

  /// Stream des événements parsés. Démarre la connexion SSE ;
  /// `EventSource` ne posant pas d'en-têtes, la clé part en `?key=`.
  Stream<ServerEvent> events() {
    final ctrl = StreamController<ServerEvent>();
    _connect(ctrl);
    return ctrl.stream;
  }

  Future<void> _connect(StreamController<ServerEvent> ctrl) async {
    while (!_closed && !ctrl.isClosed) {
      try {
        final base = Uri.parse(config.daemonBaseUrl);
        final uri = base.replace(
          path: '${base.path}/api/events',
          queryParameters:
              config.apiKey.isEmpty ? null : {'key': config.apiKey},
        );
        final req =
            await _http.getUrl(uri).timeout(config.connectTimeout);
        final res = await req.close().timeout(config.connectTimeout);
        if (res.statusCode != 200) {
          throw HttpException('SSE ${res.statusCode}', uri: uri);
        }
        var event = 'message';
        var data = StringBuffer();
        await for (final line
            in res.transform(utf8.decoder).transform(const LineSplitter())) {
          if (_closed || ctrl.isClosed) return;
          if (line.isEmpty) {
            if (data.isNotEmpty) {
              final decoded = jsonDecode(data.toString());
              ctrl.add(ServerEvent(
                event,
                decoded is Map<String, dynamic> ? decoded : {},
              ));
            }
            event = 'message';
            data = StringBuffer();
          } else if (line.startsWith('event:')) {
            event = line.substring(6).trim();
          } else if (line.startsWith('data:')) {
            if (data.isNotEmpty) data.write('\n');
            data.write(line.substring(5).trimLeft());
          }
        }
      } on Object {
        // Daemon absent/coupé → délai puis retry (tolérance).
      }
      if (!_closed && !ctrl.isClosed) {
        // Délai de reconnexion annulable : `dispose` réveille
        // la boucle immédiatement au lieu d'attendre le timer.
        _retryWake = Completer<void>();
        _retryTimer = Timer(config.sseReconnectDelay, _retryWake!.complete);
        await _retryWake!.future;
      }
    }
  }

  Future<void> dispose() async {
    _closed = true;
    _retryTimer?.cancel();
    if (_retryWake?.isCompleted == false) _retryWake?.complete();
    await _sub?.cancel();
    _http.close();
  }
}

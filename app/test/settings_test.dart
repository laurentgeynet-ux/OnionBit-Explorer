// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:io';

import 'package:explorer_app/core/settings/app_settings.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('AppSettings : roundtrip JSON + tolérance aux entrées', () {
    const s = AppSettings(
      theme: 'light',
      engine: 'ffi',
      profileDir: '/tmp/prof',
    );
    final back = AppSettings.fromJson(s.toJson());
    expect(back.theme, 'light');
    expect(back.engine, 'ffi');
    expect(back.profileDir, '/tmp/prof');

    // Valeurs inconnues → défauts (un réglage d'une version
    // future ne casse pas le décodage).
    final bad = AppSettings.fromJson({
      'theme': 'neon',
      'engine': 'warp',
      'profileDir': 42,
    });
    expect(bad.theme, 'system');
    expect(bad.engine, 'ffi'); // défaut 'ffi' depuis 21acf409
    expect(bad.profileDir, '');

    // Corps non-JSON → défauts.
    expect(AppSettings.fromJson('oops').theme, 'system');
  });

  test('SettingsStore : write/read roundtrip, fichier absent → '
      'défauts', () async {
    final dir = await Directory.systemTemp.createTemp('ui_cfg');
    addTearDown(() => dir.delete(recursive: true));
    final store = SettingsStore(
      File('${dir.path}${Platform.pathSeparator}ui.json'),
    );

    // Absent → défauts, jamais d'exception.
    expect((await store.read()).theme, 'system');

    await store.write(const AppSettings(theme: 'dark'));
    expect((await store.read()).theme, 'dark');
  });
}

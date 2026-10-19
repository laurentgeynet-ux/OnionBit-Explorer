// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:io';

import 'package:explorer_app/core/api/ffi_engine.dart';
import 'package:explorer_app/core/config/app_config.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('tryLoad : mode rest ou DLL absente → null (repli REST)',
      () {
    expect(
      FfiEngine.tryLoad(const AppConfig(engineMode: 'rest')),
      isNull,
    );
    expect(
      FfiEngine.tryLoad(
        const AppConfig(engineLibPath: 'explorer_ffi_absent.dll'),
      ),
      isNull,
    );
  });

  test('pipeline ABI réel si explorer_ffi.dll est buildée', () {
    // `cargo build -p explorer-ffi` produit la DLL — le test
    // se saute silencieusement si l'artefact n'existe pas
    // (CI Flutter seule, sans toolchain Rust).
    const rel = '../target/debug/explorer_ffi.dll';
    if (!File(rel).existsSync()) return;
    final e = FfiEngine.tryLoad(const AppConfig(engineLibPath: rel));
    expect(e, isNotNull);
    addTearDown(e!.dispose);
    expect(e.isLoaded, isTrue);

    // Onglet → liste → frame absente → tick inerte → close.
    e.createTab().then((tab) async {
      expect(int.tryParse(tab.id), isNotNull);
      expect(await e.listTabs(), isNotEmpty);
      expect(await e.fetchFrameImage(tab.id), isNull);
      expect(e.tick(tab.id), isFalse);
      await e.closeTab(tab.id);
      expect(await e.listTabs(), isEmpty);
    });
  });
}

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

  test('pipeline ABI réel si explorer_ffi.dll est buildée', () async {
    // `cargo build -p explorer-ffi` produit la DLL — le test
    // se saute silencieusement si l'artefact n'existe pas
    // (CI Flutter seule, sans toolchain Rust).
    const rel = '../target/debug/explorer_ffi.dll';
    if (!File(rel).existsSync()) return;
    final e = FfiEngine.tryLoad(const AppConfig(engineLibPath: rel));
    expect(e, isNotNull);
    addTearDown(e!.dispose);
    // Le `ready` du worker arrive en async — un premier appel
    // réel attend `_ready` (avant, `isLoaded` était sondé
    // synchrone : course toujours perdue). `listTabs` est
    // local — `createTab` passe par le worker.
    final tab = await e.createTab();
    expect(e.isLoaded, isTrue);
    expect(int.tryParse(tab.id), isNotNull);
    expect(await e.listTabs(), isNotEmpty);
    expect(await e.fetchFrameImage(tab.id), isNull);
    // `tick` est async (étape 71e) : `invalidated` = false —
    // rien n'a repeint ; awaité sinon le dispose du tearDown
    // casse un appel encore en vol.
    expect((await e.tick(tab.id)).invalidated, isFalse);
    await e.closeTab(tab.id);
    expect(await e.listTabs(), isEmpty);
  });
}

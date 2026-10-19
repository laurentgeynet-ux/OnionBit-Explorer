// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:ffi' as ffi;
import 'dart:io';

import 'package:ffi/ffi.dart';

/// Écrit une variable d'environnement **du processus courant** —
/// `Platform.environment` est en lecture seule ; le moteur
/// in-process lit `EXPLORER_PROFILE` au chargement de la
/// bibliothèque (`explorer_create`), il faut donc passer par le
/// syscall (`setenv` POSIX, `SetEnvironmentVariableW` Windows).
///
/// `false` si la plateforme refuse — le réglage reste persisté
/// et s'appliquera au prochain démarrage du moteur/daemon.
bool setProcessEnv(String name, String value) {
  try {
    if (Platform.isWindows) {
      final kernel32 = ffi.DynamicLibrary.open('kernel32.dll');
      final setVar = kernel32
          .lookupFunction<
            ffi.Int32 Function(ffi.Pointer<Utf16>, ffi.Pointer<Utf16>),
            int Function(ffi.Pointer<Utf16>, ffi.Pointer<Utf16>)
          >('SetEnvironmentVariableW');
      final n = name.toNativeUtf16();
      final v = value.toNativeUtf16();
      try {
        return setVar(n, v) != 0;
      } finally {
        calloc
          ..free(n)
          ..free(v);
      }
    }
    final libc = ffi.DynamicLibrary.process();
    final setenv = libc
        .lookupFunction<
          ffi.Int32 Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, ffi.Int32),
          int Function(ffi.Pointer<Utf8>, ffi.Pointer<Utf8>, int)
        >('setenv');
    final n = name.toNativeUtf8();
    final v = value.toNativeUtf8();
    try {
      return setenv(n, v, 1) == 0;
    } finally {
      calloc
        ..free(n)
        ..free(v);
    }
  } on Object {
    return false;
  }
}

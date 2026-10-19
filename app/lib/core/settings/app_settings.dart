// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_riverpod/flutter_riverpod.dart';

/// Réglages applicatifs persistés (étape 57) — fichier JSON dans
/// le répertoire de config utilisateur, zéro dépendance
/// (`dart:io`). Trois axes :
/// - [theme] : `'system'` (suit l'OS), `'dark'` ou `'light'` ;
/// - [engine] : `'auto'` (FFI si présent, REST sinon — défère à
///   `AppConfig.engineMode`), `'rest'` ou `'ffi'` forcé ;
/// - [profileDir] : répertoire de profil moteur exporté vers
///   `EXPLORER_PROFILE` avant le chargement FFI — en mode REST
///   il appartient au daemon (même variable à son lancement).
class AppSettings {
  const AppSettings({
    this.theme = 'system',
    this.engine = 'auto',
    this.profileDir = '',
  });

  final String theme;
  final String engine;
  final String profileDir;

  AppSettings copyWith({String? theme, String? engine, String? profileDir}) =>
      AppSettings(
        theme: theme ?? this.theme,
        engine: engine ?? this.engine,
        profileDir: profileDir ?? this.profileDir,
      );

  Map<String, dynamic> toJson() => {
    'theme': theme,
    'engine': engine,
    'profileDir': profileDir,
  };

  /// Tolérant : entrée absente/invalide → valeur par défaut
  /// (un fichier corrompu ne casse jamais le démarrage).
  static AppSettings fromJson(Object? json) {
    if (json is! Map) return const AppSettings();
    String pick(String k, String d, Set<String> ok) {
      final v = json[k];
      return v is String && ok.contains(v) ? v : d;
    }

    return AppSettings(
      theme: pick('theme', 'system', const {'system', 'dark', 'light'}),
      engine: pick('engine', 'auto', const {'auto', 'rest', 'ffi'}),
      profileDir: json['profileDir'] is String
          ? json['profileDir'] as String
          : '',
    );
  }
}

/// Persistance JSON — résolution du chemin : `EXPLORER_UI_CONFIG`
/// > répertoire config OS (`%APPDATA%` Windows,
/// `~/Library/Application Support` macOS, `$XDG_CONFIG_HOME` ou
/// `~/.config` ailleurs). Absent/corrompu → [AppSettings] par
/// défaut ; écriture best-effort (un répertoire non accessible
/// ne doit pas empêcher de régler).
class SettingsStore {
  const SettingsStore(this.file);

  final File file;

  /// Chemin par défaut de l'app — surchargeable par env var,
  /// utile aussi aux tests.
  static File defaultFile() {
    final forced = Platform.environment['EXPLORER_UI_CONFIG'];
    if (forced != null && forced.isNotEmpty) return File(forced);
    final sep = Platform.pathSeparator;
    String base;
    if (Platform.isWindows) {
      base = Platform.environment['APPDATA'] ?? Directory.current.path;
      return File('$base${sep}Explorer${sep}ui_settings.json');
    }
    if (Platform.isMacOS) {
      base = Platform.environment['HOME'] ?? Directory.current.path;
      return File(
        '$base${sep}Library${sep}Application Support${sep}Explorer${sep}ui_settings.json',
      );
    }
    base =
        Platform.environment['XDG_CONFIG_HOME'] ??
        '${Platform.environment['HOME'] ?? Directory.current.path}$sep.config';
    return File('$base${sep}explorer${sep}ui_settings.json');
  }

  Future<AppSettings> read() async {
    try {
      final raw = await file.readAsString();
      return AppSettings.fromJson(jsonDecode(raw));
    } on Object {
      return const AppSettings();
    }
  }

  Future<void> write(AppSettings s) async {
    try {
      await file.parent.create(recursive: true);
      await file.writeAsString(jsonEncode(s.toJson()));
    } on Object {
      // Répertoire non inscriptible → réglage gardé en mémoire.
    }
  }
}

/// Store injectable — les tests passent un fichier temporaire.
final settingsStoreProvider = Provider<SettingsStore>(
  (_) => SettingsStore(SettingsStore.defaultFile()),
);

/// `Notifier` des réglages : `build` renvoie les défauts puis
/// recharge le fichier en tâche de fond ; chaque setter persiste.
class SettingsController extends Notifier<AppSettings> {
  @override
  AppSettings build() {
    unawaited(_load());
    return const AppSettings();
  }

  Future<void> _load() async {
    final s = await ref.read(settingsStoreProvider).read();
    if (ref.mounted) state = s;
  }

  Future<void> _save(AppSettings s) async {
    state = s;
    await ref.read(settingsStoreProvider).write(s);
  }

  /// `'system'` | `'dark'` | `'light'`.
  Future<void> setTheme(String theme) => _save(state.copyWith(theme: theme));

  /// `'auto'` | `'rest'` | `'ffi'` — recrée le dépôt via
  /// `browserRepositoryProvider` (il observe ce provider).
  Future<void> setEngine(String engine) =>
      _save(state.copyWith(engine: engine));

  /// Répertoire de profil moteur (`EXPLORER_PROFILE`) — effet au
  /// prochain chargement du moteur in-process / démarrage daemon.
  Future<void> setProfileDir(String dir) =>
      _save(state.copyWith(profileDir: dir.trim()));
}

/// Provider des réglages — `main.dart` en dérive `themeMode`,
/// `browserRepositoryProvider` en dérive le mode moteur.
final settingsProvider = NotifierProvider<SettingsController, AppSettings>(
  SettingsController.new,
);

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
///   il appartient au daemon (même variable à son lancement) ;
/// - [language] : valeur `Accept-Language` poussée au moteur
///   (étape 118) — `''` = locale OS, sinon étiquette de langue
///   (`fr-FR`) ou liste qualifiée complète (`fr-FR,fr;q=0.9`).
class AppSettings {
  const AppSettings({
    this.theme = 'system',
    this.engine = 'ffi',
    this.profileDir = '',
    this.language = '',
    this.uiLanguage = '',
    this.hwAccel = true,
  });

  final String theme;
  final String engine;
  final String profileDir;
  final String language;

  /// Langue de l'interface (`'fr'` | `'en'`) — choisie **une
  /// fois** au premier lancement (dialogue one-shot), jamais
  /// réaffichée ensuite : ce n'est pas un réglage d'usage
  /// quotidien. `''` = pas encore choisi.
  final String uiLanguage;

  /// Décodage matériel (D3D11VA) — opt-in OnionMedia : le
  /// lecteur pose `spec.hw_accel` quand le réglage est on.
  /// Sans effet hors Windows / DLL sans le chemin HW.
  final bool hwAccel;

  AppSettings copyWith({
    String? theme,
    String? engine,
    String? profileDir,
    String? language,
    String? uiLanguage,
    bool? hwAccel,
  }) => AppSettings(
    theme: theme ?? this.theme,
    engine: engine ?? this.engine,
    profileDir: profileDir ?? this.profileDir,
    language: language ?? this.language,
    uiLanguage: uiLanguage ?? this.uiLanguage,
    hwAccel: hwAccel ?? this.hwAccel,
  );

  Map<String, dynamic> toJson() => {
    'theme': theme,
    'engine': engine,
    'profileDir': profileDir,
    'language': language,
    'uiLanguage': uiLanguage,
    'hwAccel': hwAccel,
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
      engine: pick('engine', 'ffi', const {'auto', 'rest', 'ffi'}),
      profileDir: json['profileDir'] is String
          ? json['profileDir'] as String
          : '',
      language: json['language'] is String ? json['language'] as String : '',
      uiLanguage: pick(
        'uiLanguage',
        '',
        const {'', 'fr', 'en', 'es', 'pt', 'zh', 'hi', 'id', 'de', 'ru', 'ja'},
      ),
      hwAccel: json['hwAccel'] is bool
          ? json['hwAccel'] as bool
          : const AppSettings().hwAccel,
    );
  }
}

/// Marqueur de bundle portable à la racine — miroir de
/// `explorer-core::paths::PORTABLE_MARKER` : sa présence fait
/// vivre `state/` à côté de l'exe (étape 167).
const _portableMarker = 'Explorer.portable';

/// `<racine>/state` si l'exe vit dans un bundle marqué
/// `Explorer.portable` (payload `windows/` un niveau sous la
/// racine) ; `null` hors bundle — même remontée que
/// `find_portable_root` côté moteur.
Directory? _portableStateDir() {
  final sep = Platform.pathSeparator;
  var dir = File(Platform.resolvedExecutable).parent;
  while (true) {
    if (File('${dir.path}$sep$_portableMarker').existsSync()) {
      return Directory('${dir.path}${sep}state');
    }
    final parent = dir.parent;
    if (parent.path == dir.path) return null;
    dir = parent;
  }
}

/// Persistance JSON — résolution du chemin : `EXPLORER_UI_CONFIG`
/// > bundle portable (`state/ui_settings.json` à côté du
/// marqueur) > répertoire config OS (`%APPDATA%` Windows,
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
    final portable = _portableStateDir();
    if (portable != null) {
      return File('${portable.path}${sep}ui_settings.json');
    }
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

  /// `Accept-Language` (`''` = locale OS) — propagé au moteur
  /// en direct par l'écoute `browserProvider` (étape 118).
  Future<void> setLanguage(String lang) =>
      _save(state.copyWith(language: lang.trim()));

  /// Langue UI (`'fr'` | `'en'`, `''` = défaut `fr`) —
  /// réglée dans le dialogue Réglages.
  Future<void> setUiLanguage(String lang) =>
      _save(state.copyWith(uiLanguage: lang.trim()));

  /// Décodage matériel (D3D11VA) — posé au lecteur OnionMedia
  /// à la création de chaque surface vidéo.
  Future<void> setHwAccel(bool v) => _save(state.copyWith(hwAccel: v));
}

/// Provider des réglages — `main.dart` en dérive `themeMode`,
/// `browserRepositoryProvider` en dérive le mode moteur.
final settingsProvider = NotifierProvider<SettingsController, AppSettings>(
  SettingsController.new,
);

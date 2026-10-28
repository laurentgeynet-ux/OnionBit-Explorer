// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../core/design/design_tokens.dart';
import '../../../core/l10n/app_localizations.dart';
import '../../../core/settings/app_settings.dart';

/// Panneau de réglages (étape 57) : thème (clair/sombre/système),
/// moteur (auto/REST/FFI), répertoire de profil
/// (`EXPLORER_PROFILE`). Persisté à chaque changement via
/// [settingsProvider] — le mode moteur recrée le dépôt, le
/// profil prend effet au prochain chargement du moteur.
class SettingsDialog extends ConsumerStatefulWidget {
  const SettingsDialog({super.key, required this.tokens});

  final DesignTokens tokens;

  static Future<void> show(BuildContext context, DesignTokens tokens) =>
      showDialog<void>(
        context: context,
        builder: (_) => SettingsDialog(tokens: tokens),
      );

  @override
  ConsumerState<SettingsDialog> createState() => _SettingsDialogState();
}

class _SettingsDialogState extends ConsumerState<SettingsDialog> {
  late final TextEditingController _profile;
  late final TextEditingController _language;

  @override
  void initState() {
    super.initState();
    final s = ref.read(settingsProvider);
    _profile = TextEditingController(text: s.profileDir);
    _language = TextEditingController(text: s.language);
  }

  @override
  void dispose() {
    _profile.dispose();
    _language.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = widget.tokens;
    final settings = ref.watch(settingsProvider);
    final ctrl = ref.read(settingsProvider.notifier);
    final muted = t.color('onSurface', 'muted');
    final pad = t.dim('spacing', 'md', fallback: 16);

    Widget row(String label, Widget child) => Padding(
      padding: EdgeInsets.symmetric(vertical: t.dim('spacing', 'xs')),
      child: Row(
        children: [
          SizedBox(width: 110, child: Text(label)),
          Expanded(child: child),
        ],
      ),
    );

    return AlertDialog(
      backgroundColor: t.color('surface', 'raised'),
      title: Text(AppLocalizations.of(context).settings),
      content: SizedBox(
        width: 420,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            row(
              AppLocalizations.of(context).theme,
              DropdownButton<String>(
                value: settings.theme,
                isExpanded: true,
                items: [
                  DropdownMenuItem(value: 'system', child: Text(AppLocalizations.of(context).themeSystem)),
                  DropdownMenuItem(value: 'dark', child: Text(AppLocalizations.of(context).themeDark)),
                  DropdownMenuItem(value: 'light', child: Text(AppLocalizations.of(context).themeLight)),
                ],
                onChanged: (v) {
                  if (v != null) ctrl.setTheme(v);
                },
              ),
            ),
            row(
              AppLocalizations.of(context).engineLabel,
              DropdownButton<String>(
                value: settings.engine,
                isExpanded: true,
                items: [
                  DropdownMenuItem(
                    value: 'auto',
                    child: Text(AppLocalizations.of(context).engineAuto),
                  ),
                  DropdownMenuItem(value: 'rest', child: Text(AppLocalizations.of(context).engineRest)),
                  DropdownMenuItem(
                    value: 'ffi',
                    child: Text(AppLocalizations.of(context).engineFfi),
                  ),
                ],
                onChanged: (v) {
                  if (v != null) ctrl.setEngine(v);
                },
              ),
            ),
            row(
              AppLocalizations.of(context).profile,
              TextField(
                controller: _profile,
                decoration: InputDecoration(
                  hintText: AppLocalizations.of(context).profileDirLabel,
                  isDense: true,
                ),
                onSubmitted: ctrl.setProfileDir,
              ),
            ),
            row(
              AppLocalizations.of(context).language,
              TextField(
                controller: _language,
                decoration: InputDecoration(
                  hintText: AppLocalizations.of(context).acceptLanguageLabel,
                  isDense: true,
                ),
                onSubmitted: ctrl.setLanguage,
              ),
            ),
            row(
              AppLocalizations.of(context).interfaceLanguage,
              DropdownButton<String>(
                value: settings.uiLanguage.isEmpty
                    ? 'fr'
                    : settings.uiLanguage,
                isExpanded: true,
                items: const [
                  DropdownMenuItem(value: 'fr', child: Text('Français')),
                  DropdownMenuItem(value: 'en', child: Text('English')),
                  DropdownMenuItem(value: 'es', child: Text('Español')),
                  DropdownMenuItem(value: 'pt', child: Text('Português')),
                  DropdownMenuItem(value: 'zh', child: Text('')),
                  DropdownMenuItem(value: 'hi', child: Text('हिन्दी')),
                  DropdownMenuItem(
                    value: 'id',
                    child: Text('Bahasa Indonesia'),
                  ),
                  DropdownMenuItem(value: 'de', child: Text('Deutsch')),
                  DropdownMenuItem(value: 'ru', child: Text('Русский')),
                  DropdownMenuItem(value: 'ja', child: Text('')),
                ],
                onChanged: (v) {
                  if (v != null) ctrl.setUiLanguage(v);
                },
              ),
            ),
            row(
              'HW decode',
              Align(
                alignment: Alignment.centerLeft,
                child: Switch(
                  value: settings.hwAccel,
                  onChanged: (v) => ctrl.setHwAccel(v),
                ),
              ),
            ),
            Align(
              alignment: Alignment.centerLeft,
              child: Padding(
                padding: EdgeInsets.only(top: t.dim('spacing', 'xs')),
                child: Text(
                  AppLocalizations.of(context).profileNote,
                  style: TextStyle(
                    color: muted,
                    fontSize: t.dim2('typography', 'size', 'sm', fallback: 12),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
      contentPadding: EdgeInsets.all(pad),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(AppLocalizations.of(context).close),
        ),
      ],
    );
  }
}

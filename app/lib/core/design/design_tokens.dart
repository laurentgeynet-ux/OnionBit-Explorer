// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:convert';
import 'dart:ui';

import 'package:flutter/services.dart' show rootBundle;

/// Design tokens chargés depuis `assets/design/tokens.json`
/// (source de vérité — ADR-0005). Toute couleur/espacement/rayon
/// dans l'UI transite par cet objet ; jamais de valeur en dur.
///
/// Une entrée absente ou invalide dégrade vers le repli embarqué
/// ([DesignTokens.fallback]) — tolérance fichier, jamais de crash.
class DesignTokens {
  const DesignTokens._(this._raw);

  final Map<String, dynamic> _raw;

  /// Charge les tokens depuis l'asset bundle ; repli si absent.
  static Future<DesignTokens> load() async {
    try {
      final json = await rootBundle.loadString('assets/design/tokens.json');
      final decoded = jsonDecode(json);
      if (decoded is Map<String, dynamic>) return DesignTokens._(decoded);
    } on Object {
      // Fichier absent/corrompu → repli embarqué (ADR-0005 §conséquences).
    }
    return fallback;
  }

  /// Tokens de repli embarqués — utilisés si l'asset est absent,
  /// et dans les tests de widgets sans bundle.
  static final DesignTokens fallback = DesignTokens._(const {
    'color': {
      'surface': {'base': '#10141A', 'raised': '#1A1F27', 'overlay': '#232A35'},
      'onSurface': {'base': '#E8ECF2', 'muted': '#9AA5B4'},
      'accent': {'base': '#4F8FE8', 'pressed': '#3A72C4'},
      'danger': {'base': '#D95D5D'},
      'border': {'base': '#2C3542'},
    },
    'spacing': {'xs': 4.0, 'sm': 8.0, 'md': 16.0, 'lg': 24.0, 'xl': 40.0},
    'radius': {'sm': 4.0, 'md': 8.0, 'lg': 14.0},
    'typography': {
      'fontFamily': 'Segoe UI',
      'size': {'sm': 12.0, 'md': 14.0, 'lg': 18.0, 'title': 22.0},
      'weight': {'regular': 400, 'medium': 500, 'bold': 700},
    },
    'motion': {'durationMs': {'fast': 120, 'normal': 220}},
    'elevation': {'low': 1.0, 'high': 8.0},
  });

  dynamic _at(List<String> path) {
    dynamic cur = _raw;
    for (final k in path) {
      if (cur is! Map<String, dynamic>) return null;
      cur = cur[k];
    }
    return cur;
  }

  /// Couleur au chemin `section.name` (`color('surface', 'base')`).
  /// Retourne `fallback` si le chemin est absent ou non hexadécimal.
  Color color(String section, String name, {Color fallback = const Color(0xFF101418)}) {
    final v = _at(['color', section, name]);
    if (v is String && v.startsWith('#')) {
      final hex = v.substring(1);
      final n = int.tryParse(hex, radix: 16);
      if (n != null) {
        return Color(hex.length == 6 ? 0xFF000000 | n : n);
      }
    }
    return fallback;
  }

  /// Valeur numérique (`dim('spacing', 'md')` → `16.0`).
  double dim(String section, String name, {double fallback = 0}) {
    final v = _at([section, name]);
    return v is double ? v : (v is int ? v.toDouble() : fallback);
  }

  /// Valeur numérique imbriquée (`dim2('typography','size','md')`).
  double dim2(String section, String group, String name, {double fallback = 0}) {
    final v = _at([section, group, name]);
    return v is double ? v : (v is int ? v.toDouble() : fallback);
  }

  /// Chaîne (`str('typography','fontFamily')` → `'Segoe UI'`).
  String str(String section, String name, {String fallback = ''}) {
    final v = _at([section, name]);
    return v is String ? v : fallback;
  }
}

// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'package:flutter/material.dart';

import 'design_tokens.dart';

/// `ThemeData` **dérivé** des tokens (ADR-0005) — aucune couleur,
/// taille ou rayon ne vient d'une constante Flutter.
ThemeData buildTheme(DesignTokens t) {
  final base = t.color('surface', 'base');
  final raised = t.color('surface', 'raised');
  final onSurface = t.color('onSurface', 'base');
  final muted = t.color('onSurface', 'muted');
  final accent = t.color('accent', 'base');
  final border = t.color('border', 'base');
  final family = t.str('typography', 'fontFamily', fallback: 'Segoe UI');
  final sizeMd = t.dim2('typography', 'size', 'md', fallback: 14);

  final scheme = ColorScheme.dark(
    surface: base,
    primary: accent,
    onSurface: onSurface,
    outline: border,
  );

  return ThemeData(
    useMaterial3: true,
    colorScheme: scheme,
    scaffoldBackgroundColor: base,
    fontFamily: family,
    textTheme: TextTheme(
      bodyMedium: TextStyle(color: onSurface, fontSize: sizeMd),
      bodySmall: TextStyle(color: muted, fontSize: t.dim2('typography', 'size', 'sm', fallback: 12)),
      titleLarge: TextStyle(color: onSurface, fontSize: t.dim2('typography', 'size', 'title', fallback: 22)),
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: raised,
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(t.dim('radius', 'md', fallback: 8)),
        borderSide: BorderSide(color: border),
      ),
      contentPadding: EdgeInsets.symmetric(
        horizontal: t.dim('spacing', 'md', fallback: 16),
        vertical: t.dim('spacing', 'sm', fallback: 8),
      ),
    ),
  );
}

// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'package:flutter/material.dart';

import 'design_tokens.dart';

/// `ThemeData` **dérivé** des tokens (ADR-0005) — aucune couleur,
/// taille ou rayon ne vient d'une constante Flutter : chaque
/// composant puise dans `tokens.json` via [DesignTokens].
ThemeData buildTheme(DesignTokens t) {
  final base = t.color('surface', 'base');
  final raised = t.color('surface', 'raised');
  final overlay = t.color('surface', 'overlay');
  final onSurface = t.color('onSurface', 'base');
  final muted = t.color('onSurface', 'muted');
  final accent = t.color('accent', 'base');
  final accentPressed = t.color('accent', 'pressed');
  final danger = t.color('danger', 'base');
  final border = t.color('border', 'base');

  final family = t.str('typography', 'fontFamily', fallback: 'Segoe UI');
  final sizeSm = t.dim2('typography', 'size', 'sm', fallback: 12);
  final sizeMd = t.dim2('typography', 'size', 'md', fallback: 14);
  final sizeLg = t.dim2('typography', 'size', 'lg', fallback: 18);
  final sizeTitle = t.dim2('typography', 'size', 'title', fallback: 22);
  final wRegular = t.dim2('typography', 'weight', 'regular', fallback: 400);
  final wMedium = t.dim2('typography', 'weight', 'medium', fallback: 500);
  final wBold = t.dim2('typography', 'weight', 'bold', fallback: 700);

  final spSm = t.dim('spacing', 'sm', fallback: 8);
  final spMd = t.dim('spacing', 'md', fallback: 16);
  final rSm = t.dim('radius', 'sm', fallback: 4);
  final rMd = t.dim('radius', 'md', fallback: 8);
  final rLg = t.dim('radius', 'lg', fallback: 14);
  final iconMd = t.dim('metrics', 'iconMd', fallback: 20);
  final motionFast = t.dim2('motion', 'durationMs', 'fast', fallback: 120);
  final elevHigh = t.dim('elevation', 'high', fallback: 8);

  // `meta.scheme` pilote la luminosité du ColorScheme — la
  // palette elle-même vient toujours des tokens (étape 57 :
  // variante claire `tokens.light.json`).
  final light = t.scheme == 'light';
  final scheme = (light ? ColorScheme.light : ColorScheme.dark)(
    surface: base,
    primary: accent,
    onSurface: onSurface,
    outline: border,
    error: danger,
    surfaceContainerHighest: overlay,
  );

  TextStyle style(double size, Color color, double weight) => TextStyle(
    color: color,
    fontSize: size,
    fontWeight: FontWeight.values[((weight / 100).round() - 1).clamp(0, 8)],
  );

  return ThemeData(
    useMaterial3: true,
    colorScheme: scheme,
    scaffoldBackgroundColor: base,
    fontFamily: family,
    splashFactory: InkSparkle.splashFactory,

    textTheme: TextTheme(
      bodyMedium: style(sizeMd, onSurface, wRegular),
      bodySmall: style(sizeSm, muted, wRegular),
      titleLarge: style(sizeTitle, onSurface, wBold),
      titleMedium: style(sizeLg, onSurface, wMedium),
      labelMedium: style(sizeSm, onSurface, wMedium),
    ),

    iconTheme: IconThemeData(color: onSurface, size: iconMd),

    textSelectionTheme: TextSelectionThemeData(
      cursorColor: accent,
      selectionColor: accent.withValues(alpha: 0.35),
      selectionHandleColor: accentPressed,
    ),

    progressIndicatorTheme: ProgressIndicatorThemeData(
      color: accent,
      linearTrackColor: border,
      circularTrackColor: border,
      strokeWidth: t.dim('metrics', 'progressStroke', fallback: 2),
    ),

    dividerTheme: DividerThemeData(color: border, thickness: 1),

    tooltipTheme: TooltipThemeData(
      decoration: BoxDecoration(
        color: overlay,
        borderRadius: BorderRadius.circular(rSm),
        border: Border.all(color: border),
      ),
      textStyle: style(sizeSm, onSurface, wRegular),
      waitDuration: Duration(milliseconds: (motionFast * 4).round()),
    ),

    scrollbarTheme: ScrollbarThemeData(
      thumbColor: WidgetStateProperty.all(border),
      thickness: WidgetStateProperty.all(spSm),
      radius: Radius.circular(rSm),
    ),

    appBarTheme: AppBarTheme(
      backgroundColor: base,
      foregroundColor: onSurface,
      elevation: 0,
      titleTextStyle: style(sizeTitle, onSurface, wBold),
    ),

    snackBarTheme: SnackBarThemeData(
      backgroundColor: overlay,
      contentTextStyle: style(sizeMd, onSurface, wRegular),
      behavior: SnackBarBehavior.floating,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(rMd),
        side: BorderSide(color: border),
      ),
      elevation: elevHigh,
    ),

    iconButtonTheme: IconButtonThemeData(
      style: IconButton.styleFrom(
        foregroundColor: onSurface,
        disabledForegroundColor: muted,
        highlightColor: accent.withValues(alpha: 0.2),
        iconSize: iconMd,
      ),
    ),

    textButtonTheme: TextButtonThemeData(
      style: TextButton.styleFrom(
        foregroundColor: accent,
        textStyle: style(sizeMd, accent, wMedium),
      ),
    ),

    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: raised,
      hintStyle: style(sizeMd, muted, wRegular),
      prefixIconColor: muted,
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(rMd),
        borderSide: BorderSide(color: border),
      ),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(rMd),
        borderSide: BorderSide(color: border),
      ),
      focusedBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(rMd),
        borderSide: BorderSide(color: accent, width: 1.5),
      ),
      contentPadding: EdgeInsets.symmetric(horizontal: spMd, vertical: spSm),
    ),

    cardTheme: CardThemeData(
      color: raised,
      elevation: elevHigh,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(rLg),
        side: BorderSide(color: border),
      ),
    ),
  );
}

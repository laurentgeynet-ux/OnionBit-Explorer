// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:webview_flutter_platform_interface/webview_flutter_platform_interface.dart';
import 'package:webview_win_floating/webview_win_floating.dart';

/// Onglet délégué WebView2 — étape 136, ADR-0027.
///
/// Monté à la place de la frame moteur quand l'URL d'un onglet
/// matche l'allowlist DRM (`AppConfig.drmDelegateHosts`, 136b).
/// Le plugin `webview_win_floating` place un HWND WebView2 natif
/// au-dessus de la fenêtre Flutter — choix acté en 136a : les
/// surfaces vidéo protégées (Widevine L1 / PlayReady matériel)
/// ne peuvent pas être capturées en texture (`webview_windows`
/// rend noir sur les flux DRM) ; seul un HWND réel les affiche.
///
/// Limites assumées du mode HWND overlay (documentées par le
/// plugin) : aucun widget Flutter au-dessus de la zone webview,
/// pas de route poussée par-dessus — le chrome Explorer reste
/// hors de cette zone ; `setVisibility` couvre les cas
/// d'occlusion (panneaux, onglet inactif).
///
/// Hors Windows : repli honnête (ADR-0027 §6) — le widget rend
/// un espace vide ; la navigation reste sur le moteur et le site
/// porte son propre échec DRM.
class DelegatedWebView extends StatefulWidget {
  const DelegatedWebView({
    super.key,
    required this.url,
    this.userDataFolder,
    this.profileName,
    this.onController,
  });

  /// URL initiale du domaine délégué.
  final String url;

  /// Dossier de données WebView2 dédié — séparé du profil
  /// Explorer (ADR-0027 §5). `null` = défaut du plugin.
  final String? userDataFolder;

  /// Nom de profil WebView2 (isolation cookies/cache entre
  /// profils, partage du process navigateur).
  final String? profileName;

  /// Rappel d'accès au contrôleur natif (resync navigation,
  /// 136e : `goBack`/`goForward`/`currentUrl`/`getTitle`…).
  final void Function(WinWebViewController controller)? onController;

  @override
  State<DelegatedWebView> createState() => _DelegatedWebViewState();
}

class _DelegatedWebViewState extends State<DelegatedWebView> {
  WinWebViewController? _controller;

  @override
  void initState() {
    super.initState();
    if (!Platform.isWindows) return;
    final controller = WinWebViewController(
      params: WindowsWebViewControllerCreationParams(
        userDataFolder: widget.userDataFolder,
        profileName: widget.profileName,
      ),
    );
    controller.setJavaScriptMode(JavaScriptMode.unrestricted);
    controller.loadRequest(Uri.parse(widget.url));
    _controller = controller;
    widget.onController?.call(controller);
  }

  @override
  void didUpdateWidget(DelegatedWebView old) {
    super.didUpdateWidget(old);
    // Navigation déléguée → rechargement (interception 136c :
    // `BrowserController.navigate` a posé `tab.url` sans
    // contacter le moteur).
    if (old.url != widget.url) {
      _controller?.loadRequest(Uri.parse(widget.url));
    }
  }

  @override
  void dispose() {
    _controller = null; // GC → finalizer du plugin libère le HWND.
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final controller = _controller;
    if (controller == null) {
      // Hors-Windows : espace neutre — le chemin moteur garde la
      // main, rien n'est délégué (repli honnête ADR-0027 §6).
      return const SizedBox.expand();
    }
    return WinWebViewWidget(controller: controller);
  }
}

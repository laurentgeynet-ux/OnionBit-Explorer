// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'browser_tab.dart';

/// Résultat d'un `POST …/reload` (étape 93) : soit l'état
/// résultant de l'onglet, soit la demande de confirmation
/// `resubmit_confirm` — la page courante a été servie par
/// une méthode non sûre et rejouer la requête répéterait
/// l'effet de bord (commande, paiement). Le client rappelle
/// avec `{"confirm":true}` après accord explicite.
class ReloadResult {
  const ReloadResult.tab(this.tab) : resubmitMethod = null, resubmitUrl = null;

  const ReloadResult.needsConfirm({required String method, required String url})
    : resubmitMethod = method,
      resubmitUrl = url,
      tab = null;

  /// Traduit la réponse JSON du daemon.
  factory ReloadResult.fromJson(Map<String, dynamic> res) {
    if (res['result'] == 'resubmit_confirm') {
      return ReloadResult.needsConfirm(
        method: res['method'] as String? ?? 'POST',
        url: res['url'] as String? ?? '',
      );
    }
    return ReloadResult.tab(BrowserTab.fromJson(res));
  }

  /// État de l'onglet après rechargement (`null` si la
  /// resoumission attend un accord).
  final BrowserTab? tab;

  /// Méthode non sûre à rejouer (`POST`…) — non nul quand
  /// `needsConfirm`.
  final String? resubmitMethod;

  /// URL cible de la requête à rejouer.
  final String? resubmitUrl;

  /// `true` quand le reload demande un accord explicite.
  bool get needsConfirm => resubmitMethod != null;
}

// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for French (`fr`).
class AppLocalizationsFr extends AppLocalizations {
  AppLocalizationsFr([String locale = 'fr']) : super(locale);

  @override
  String get newTab => 'Nouvel onglet';

  @override
  String get close => 'Fermer';

  @override
  String get home => 'Accueil';

  @override
  String get history => 'Historique';

  @override
  String get bookmarks => 'Signets';

  @override
  String get addBookmark => 'Ajouter le signet';

  @override
  String get removeBookmark => 'Retirer le signet';

  @override
  String get downloads => 'Téléchargements';

  @override
  String get findInPage => 'Rechercher dans la page';

  @override
  String get findInPageShortcut => 'Rechercher dans la page (Ctrl+F)';

  @override
  String get omniboxHint => 'Rechercher ou saisir une URL';

  @override
  String get daemonOffline => 'Daemon injoignable — lancez explorer-daemon';

  @override
  String get retry => 'Réessayer';

  @override
  String get settings => 'Réglages';

  @override
  String get zoomIn => 'Zoom avant';

  @override
  String get zoomOut => 'Zoom arrière';

  @override
  String get actualSize => 'Taille réelle';

  @override
  String get moreActions => 'Plus d’actions';

  @override
  String get delegatedTabTooltip => 'Onglet délégué — rendu WebView2 natif';

  @override
  String get noTabs => 'Aucun onglet';

  @override
  String get pageLoadError => 'Impossible de charger la page';

  @override
  String get navigateToUrl => 'Naviguez vers une URL';

  @override
  String get language => 'Langue';

  @override
  String get profile => 'Profil';

  @override
  String get profileDirLabel => 'Répertoire de profil (défaut : OS)';

  @override
  String get save => 'Enregistrer';

  @override
  String get cancel => 'Annuler';

  @override
  String get resend => 'Renvoyer';

  @override
  String get resendFormTitle => 'Renvoyer le formulaire ?';

  @override
  String get emptyBookmarks => 'Aucun signet — l’étoile de la barre en ajoute';

  @override
  String get emptyHistory => 'Aucune page visitée';

  @override
  String get activeFieldTooltip => 'Champ actif — saisie envoyée au moteur';

  @override
  String get acceptLanguageLabel => 'Accept-Language (vide : locale OS)';

  @override
  String get engineRest => 'REST (daemon)';

  @override
  String get engineFfi => 'FFI (moteur in-process)';

  @override
  String get engineLabel => 'Moteur';

  @override
  String get openAsText => 'Ouvrir comme texte';

  @override
  String get theme => 'Thème';

  @override
  String get downloadDialogTitle => 'Téléchargement';

  @override
  String get downloadNotDisplayable =>
      'Ce fichier n’est pas affichable dans le navigateur.';

  @override
  String resendFormBody(Object method, Object url) {
    return 'La page a été obtenue par $method.\n$url\n\nRecharger renverra les données — un effet de bord\n(commande, paiement) peut être répété.';
  }

  @override
  String get themeSystem => 'Système';

  @override
  String get themeDark => 'Sombre';

  @override
  String get themeLight => 'Clair';

  @override
  String get engineAuto => 'Auto (FFI si présent, REST sinon)';

  @override
  String get profileNote =>
      'Exporté vers EXPLORER_PROFILE — effet au prochain démarrage du moteur (FFI) ou du daemon (REST). Valider avec Entrée.';

  @override
  String historyUnavailable(Object error) {
    return 'Historique indisponible — $error';
  }

  @override
  String bookmarksUnavailable(Object error) {
    return 'Signets indisponibles — $error';
  }

  @override
  String get closeEsc => 'Fermer (Échap)';

  @override
  String get previous => 'Précédent';

  @override
  String get next => 'Suivant';

  @override
  String get reload => 'Recharger';

  @override
  String zoomPercent(Object pct) {
    return 'Zoom $pct %';
  }

  @override
  String get interfaceLanguage => 'Interface';

  @override
  String downloadSaved(Object path) {
    return 'Téléchargement enregistré : $path';
  }

  @override
  String downloadError(Object error) {
    return 'Téléchargement : $error';
  }
}

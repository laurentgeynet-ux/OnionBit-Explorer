// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for German (`de`).
class AppLocalizationsDe extends AppLocalizations {
  AppLocalizationsDe([String locale = 'de']) : super(locale);

  @override
  String get newTab => 'Neuer Tab';

  @override
  String get close => 'Schließen';

  @override
  String get home => 'Startseite';

  @override
  String get history => 'Verlauf';

  @override
  String get bookmarks => 'Lesezeichen';

  @override
  String get addBookmark => 'Lesezeichen hinzufügen';

  @override
  String get removeBookmark => 'Lesezeichen entfernen';

  @override
  String get downloads => 'Downloads';

  @override
  String get findInPage => 'Auf der Seite suchen';

  @override
  String get findInPageShortcut => 'Auf der Seite suchen (Strg+F)';

  @override
  String get omniboxHint => 'Suchen oder URL eingeben';

  @override
  String get daemonOffline =>
      'Daemon nicht erreichbar — explorer-daemon starten';

  @override
  String get retry => 'Erneut versuchen';

  @override
  String get settings => 'Einstellungen';

  @override
  String get zoomIn => 'Vergrößern';

  @override
  String get zoomOut => 'Verkleinern';

  @override
  String get actualSize => 'Tatsächliche Größe';

  @override
  String get moreActions => 'Weitere Aktionen';

  @override
  String get delegatedTabTooltip =>
      'Delegierter Tab — natives WebView2-Rendering';

  @override
  String get noTabs => 'Keine Tabs';

  @override
  String get pageLoadError => 'Seite konnte nicht geladen werden';

  @override
  String get navigateToUrl => 'Zu einer URL navigieren';

  @override
  String get language => 'Sprache';

  @override
  String get profile => 'Profil';

  @override
  String get profileDirLabel => 'Profilverzeichnis (Standard: OS)';

  @override
  String get save => 'Speichern';

  @override
  String get cancel => 'Abbrechen';

  @override
  String get resend => 'Erneut senden';

  @override
  String get resendFormTitle => 'Formular erneut senden?';

  @override
  String get emptyBookmarks =>
      'Keine Lesezeichen — der Stern in der Leiste fügt eines hinzu';

  @override
  String get emptyHistory => 'Keine Seiten besucht';

  @override
  String get activeFieldTooltip =>
      'Aktives Feld — Eingabe wird an die Engine gesendet';

  @override
  String get acceptLanguageLabel => 'Accept-Language (leer: OS-Locale)';

  @override
  String get engineRest => 'REST (Daemon)';

  @override
  String get engineFfi => 'FFI (In-Process-Engine)';

  @override
  String get engineLabel => 'Engine';

  @override
  String get openAsText => 'Als Text öffnen';

  @override
  String get theme => 'Design';

  @override
  String get downloadDialogTitle => 'Download';

  @override
  String get downloadNotDisplayable =>
      'Diese Datei kann im Browser nicht angezeigt werden.';

  @override
  String resendFormBody(Object method, Object url) {
    return 'Die Seite wurde per $method geladen.\n$url\n\nBeim Neuladen werden die Daten erneut gesendet —\neine Nebenwirkung (Bestellung, Zahlung) kann sich wiederholen.';
  }

  @override
  String get themeSystem => 'System';

  @override
  String get themeDark => 'Dunkel';

  @override
  String get themeLight => 'Hell';

  @override
  String get engineAuto => 'Auto (FFI falls vorhanden, sonst REST)';

  @override
  String get profileNote =>
      'Nach EXPLORER_PROFILE exportiert — wirkt beim nächsten Start des Motors (FFI) oder Daemons (REST). Mit Eingabetaste bestätigen.';

  @override
  String historyUnavailable(Object error) {
    return 'Verlauf nicht verfügbar — $error';
  }

  @override
  String bookmarksUnavailable(Object error) {
    return 'Lesezeichen nicht verfügbar — $error';
  }

  @override
  String get closeEsc => 'Schließen (Esc)';

  @override
  String get previous => 'Zurück';

  @override
  String get next => 'Weiter';

  @override
  String get reload => 'Neu laden';

  @override
  String zoomPercent(Object pct) {
    return 'Zoom $pct %';
  }

  @override
  String get interfaceLanguage => 'Oberfläche';

  @override
  String downloadSaved(Object path) {
    return 'Download gespeichert: $path';
  }

  @override
  String downloadError(Object error) {
    return 'Download: $error';
  }
}

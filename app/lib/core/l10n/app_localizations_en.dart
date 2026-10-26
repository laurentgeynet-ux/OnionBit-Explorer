// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for English (`en`).
class AppLocalizationsEn extends AppLocalizations {
  AppLocalizationsEn([String locale = 'en']) : super(locale);

  @override
  String get newTab => 'New tab';

  @override
  String get close => 'Close';

  @override
  String get home => 'Home';

  @override
  String get history => 'History';

  @override
  String get bookmarks => 'Bookmarks';

  @override
  String get addBookmark => 'Add bookmark';

  @override
  String get removeBookmark => 'Remove bookmark';

  @override
  String get downloads => 'Downloads';

  @override
  String get findInPage => 'Find in page';

  @override
  String get findInPageShortcut => 'Find in page (Ctrl+F)';

  @override
  String get omniboxHint => 'Search or enter a URL';

  @override
  String get daemonOffline => 'Daemon unreachable — start explorer-daemon';

  @override
  String get retry => 'Retry';

  @override
  String get settings => 'Settings';

  @override
  String get zoomIn => 'Zoom in';

  @override
  String get zoomOut => 'Zoom out';

  @override
  String get actualSize => 'Actual size';

  @override
  String get moreActions => 'More actions';

  @override
  String get delegatedTabTooltip => 'Delegated tab — native WebView2 rendering';

  @override
  String get noTabs => 'No tabs';

  @override
  String get pageLoadError => 'Failed to load the page';

  @override
  String get navigateToUrl => 'Navigate to a URL';

  @override
  String get language => 'Language';

  @override
  String get profile => 'Profile';

  @override
  String get profileDirLabel => 'Profile directory (default: OS)';

  @override
  String get save => 'Save';

  @override
  String get cancel => 'Cancel';

  @override
  String get resend => 'Resend';

  @override
  String get resendFormTitle => 'Resend the form?';

  @override
  String get emptyBookmarks => 'No bookmarks — the star in the bar adds one';

  @override
  String get emptyHistory => 'No pages visited';

  @override
  String get activeFieldTooltip => 'Active field — typing sent to the engine';

  @override
  String get acceptLanguageLabel => 'Accept-Language (empty: OS locale)';

  @override
  String get engineRest => 'REST (daemon)';

  @override
  String get engineFfi => 'FFI (in-process engine)';

  @override
  String get engineLabel => 'Engine';

  @override
  String get openAsText => 'Open as text';

  @override
  String get theme => 'Theme';

  @override
  String get downloadDialogTitle => 'Download';

  @override
  String get downloadNotDisplayable =>
      'This file cannot be displayed in the browser.';

  @override
  String resendFormBody(Object method, Object url) {
    return 'This page was obtained by $method.\n$url\n\nReloading will resend the data — a side effect\n(order, payment) may be repeated.';
  }

  @override
  String get themeSystem => 'System';

  @override
  String get themeDark => 'Dark';

  @override
  String get themeLight => 'Light';

  @override
  String get engineAuto => 'Auto (FFI if present, REST otherwise)';

  @override
  String get profileNote =>
      'Exported to EXPLORER_PROFILE — applies at next engine (FFI) or daemon (REST) start. Confirm with Enter.';

  @override
  String historyUnavailable(Object error) {
    return 'History unavailable — $error';
  }

  @override
  String bookmarksUnavailable(Object error) {
    return 'Bookmarks unavailable — $error';
  }

  @override
  String get closeEsc => 'Close (Esc)';

  @override
  String get previous => 'Previous';

  @override
  String get next => 'Next';

  @override
  String get reload => 'Reload';

  @override
  String zoomPercent(Object pct) {
    return 'Zoom $pct %';
  }

  @override
  String get interfaceLanguage => 'Interface';

  @override
  String downloadSaved(Object path) {
    return 'Download saved: $path';
  }

  @override
  String downloadError(Object error) {
    return 'Download: $error';
  }
}

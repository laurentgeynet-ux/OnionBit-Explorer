import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:intl/intl.dart' as intl;

import 'app_localizations_de.dart';
import 'app_localizations_en.dart';
import 'app_localizations_es.dart';
import 'app_localizations_fr.dart';
import 'app_localizations_hi.dart';
import 'app_localizations_id.dart';
import 'app_localizations_ja.dart';
import 'app_localizations_pt.dart';
import 'app_localizations_ru.dart';
import 'app_localizations_zh.dart';

// ignore_for_file: type=lint

/// Callers can lookup localized strings with an instance of AppLocalizations
/// returned by `AppLocalizations.of(context)`.
///
/// Applications need to include `AppLocalizations.delegate()` in their app's
/// `localizationDelegates` list, and the locales they support in the app's
/// `supportedLocales` list. For example:
///
/// ```dart
/// import 'l10n/app_localizations.dart';
///
/// return MaterialApp(
///   localizationsDelegates: AppLocalizations.localizationsDelegates,
///   supportedLocales: AppLocalizations.supportedLocales,
///   home: MyApplicationHome(),
/// );
/// ```
///
/// ## Update pubspec.yaml
///
/// Please make sure to update your pubspec.yaml to include the following
/// packages:
///
/// ```yaml
/// dependencies:
///   # Internationalization support.
///   flutter_localizations:
///     sdk: flutter
///   intl: any # Use the pinned version from flutter_localizations
///
///   # Rest of dependencies
/// ```
///
/// ## iOS Applications
///
/// iOS applications define key application metadata, including supported
/// locales, in an Info.plist file that is built into the application bundle.
/// To configure the locales supported by your app, you’ll need to edit this
/// file.
///
/// First, open your project’s ios/Runner.xcworkspace Xcode workspace file.
/// Then, in the Project Navigator, open the Info.plist file under the Runner
/// project’s Runner folder.
///
/// Next, select the Information Property List item, select Add Item from the
/// Editor menu, then select Localizations from the pop-up menu.
///
/// Select and expand the newly-created Localizations item then, for each
/// locale your application supports, add a new item and select the locale
/// you wish to add from the pop-up menu in the Value field. This list should
/// be consistent with the languages listed in the AppLocalizations.supportedLocales
/// property.
abstract class AppLocalizations {
  AppLocalizations(String locale)
    : localeName = intl.Intl.canonicalizedLocale(locale.toString());

  final String localeName;

  static AppLocalizations of(BuildContext context) {
    return Localizations.of<AppLocalizations>(context, AppLocalizations)!;
  }

  static const LocalizationsDelegate<AppLocalizations> delegate =
      _AppLocalizationsDelegate();

  /// A list of this localizations delegate along with the default localizations
  /// delegates.
  ///
  /// Returns a list of localizations delegates containing this delegate along with
  /// GlobalMaterialLocalizations.delegate, GlobalCupertinoLocalizations.delegate,
  /// and GlobalWidgetsLocalizations.delegate.
  ///
  /// Additional delegates can be added by appending to this list in
  /// MaterialApp. This list does not have to be used at all if a custom list
  /// of delegates is preferred or required.
  static const List<LocalizationsDelegate<dynamic>> localizationsDelegates =
      <LocalizationsDelegate<dynamic>>[
        delegate,
        GlobalMaterialLocalizations.delegate,
        GlobalCupertinoLocalizations.delegate,
        GlobalWidgetsLocalizations.delegate,
      ];

  /// A list of this localizations delegate's supported locales.
  static const List<Locale> supportedLocales = <Locale>[
    Locale('fr'),
    Locale('de'),
    Locale('en'),
    Locale('es'),
    Locale('hi'),
    Locale('id'),
    Locale('ja'),
    Locale('pt'),
    Locale('ru'),
    Locale('zh'),
  ];

  /// No description provided for @newTab.
  ///
  /// In fr, this message translates to:
  /// **'Nouvel onglet'**
  String get newTab;

  /// No description provided for @close.
  ///
  /// In fr, this message translates to:
  /// **'Fermer'**
  String get close;

  /// No description provided for @home.
  ///
  /// In fr, this message translates to:
  /// **'Accueil'**
  String get home;

  /// No description provided for @history.
  ///
  /// In fr, this message translates to:
  /// **'Historique'**
  String get history;

  /// No description provided for @bookmarks.
  ///
  /// In fr, this message translates to:
  /// **'Signets'**
  String get bookmarks;

  /// No description provided for @addBookmark.
  ///
  /// In fr, this message translates to:
  /// **'Ajouter le signet'**
  String get addBookmark;

  /// No description provided for @removeBookmark.
  ///
  /// In fr, this message translates to:
  /// **'Retirer le signet'**
  String get removeBookmark;

  /// No description provided for @downloads.
  ///
  /// In fr, this message translates to:
  /// **'Téléchargements'**
  String get downloads;

  /// No description provided for @findInPage.
  ///
  /// In fr, this message translates to:
  /// **'Rechercher dans la page'**
  String get findInPage;

  /// No description provided for @findInPageShortcut.
  ///
  /// In fr, this message translates to:
  /// **'Rechercher dans la page (Ctrl+F)'**
  String get findInPageShortcut;

  /// No description provided for @omniboxHint.
  ///
  /// In fr, this message translates to:
  /// **'Rechercher ou saisir une URL'**
  String get omniboxHint;

  /// No description provided for @daemonOffline.
  ///
  /// In fr, this message translates to:
  /// **'Daemon injoignable — lancez explorer-daemon'**
  String get daemonOffline;

  /// No description provided for @retry.
  ///
  /// In fr, this message translates to:
  /// **'Réessayer'**
  String get retry;

  /// No description provided for @settings.
  ///
  /// In fr, this message translates to:
  /// **'Réglages'**
  String get settings;

  /// No description provided for @zoomIn.
  ///
  /// In fr, this message translates to:
  /// **'Zoom avant'**
  String get zoomIn;

  /// No description provided for @zoomOut.
  ///
  /// In fr, this message translates to:
  /// **'Zoom arrière'**
  String get zoomOut;

  /// No description provided for @actualSize.
  ///
  /// In fr, this message translates to:
  /// **'Taille réelle'**
  String get actualSize;

  /// No description provided for @moreActions.
  ///
  /// In fr, this message translates to:
  /// **'Plus d’actions'**
  String get moreActions;

  /// No description provided for @delegatedTabTooltip.
  ///
  /// In fr, this message translates to:
  /// **'Onglet délégué — rendu WebView2 natif'**
  String get delegatedTabTooltip;

  /// No description provided for @noTabs.
  ///
  /// In fr, this message translates to:
  /// **'Aucun onglet'**
  String get noTabs;

  /// No description provided for @pageLoadError.
  ///
  /// In fr, this message translates to:
  /// **'Impossible de charger la page'**
  String get pageLoadError;

  /// No description provided for @navigateToUrl.
  ///
  /// In fr, this message translates to:
  /// **'Naviguez vers une URL'**
  String get navigateToUrl;

  /// No description provided for @language.
  ///
  /// In fr, this message translates to:
  /// **'Langue'**
  String get language;

  /// No description provided for @profile.
  ///
  /// In fr, this message translates to:
  /// **'Profil'**
  String get profile;

  /// No description provided for @profileDirLabel.
  ///
  /// In fr, this message translates to:
  /// **'Répertoire de profil (défaut : OS)'**
  String get profileDirLabel;

  /// No description provided for @save.
  ///
  /// In fr, this message translates to:
  /// **'Enregistrer'**
  String get save;

  /// No description provided for @cancel.
  ///
  /// In fr, this message translates to:
  /// **'Annuler'**
  String get cancel;

  /// No description provided for @resend.
  ///
  /// In fr, this message translates to:
  /// **'Renvoyer'**
  String get resend;

  /// No description provided for @resendFormTitle.
  ///
  /// In fr, this message translates to:
  /// **'Renvoyer le formulaire ?'**
  String get resendFormTitle;

  /// No description provided for @emptyBookmarks.
  ///
  /// In fr, this message translates to:
  /// **'Aucun signet — l’étoile de la barre en ajoute'**
  String get emptyBookmarks;

  /// No description provided for @emptyHistory.
  ///
  /// In fr, this message translates to:
  /// **'Aucune page visitée'**
  String get emptyHistory;

  /// No description provided for @activeFieldTooltip.
  ///
  /// In fr, this message translates to:
  /// **'Champ actif — saisie envoyée au moteur'**
  String get activeFieldTooltip;

  /// No description provided for @acceptLanguageLabel.
  ///
  /// In fr, this message translates to:
  /// **'Accept-Language (vide : locale OS)'**
  String get acceptLanguageLabel;

  /// No description provided for @engineRest.
  ///
  /// In fr, this message translates to:
  /// **'REST (daemon)'**
  String get engineRest;

  /// No description provided for @engineFfi.
  ///
  /// In fr, this message translates to:
  /// **'FFI (moteur in-process)'**
  String get engineFfi;

  /// No description provided for @engineLabel.
  ///
  /// In fr, this message translates to:
  /// **'Moteur'**
  String get engineLabel;

  /// No description provided for @openAsText.
  ///
  /// In fr, this message translates to:
  /// **'Ouvrir comme texte'**
  String get openAsText;

  /// No description provided for @theme.
  ///
  /// In fr, this message translates to:
  /// **'Thème'**
  String get theme;

  /// No description provided for @downloadDialogTitle.
  ///
  /// In fr, this message translates to:
  /// **'Téléchargement'**
  String get downloadDialogTitle;

  /// No description provided for @downloadNotDisplayable.
  ///
  /// In fr, this message translates to:
  /// **'Ce fichier n’est pas affichable dans le navigateur.'**
  String get downloadNotDisplayable;

  /// No description provided for @resendFormBody.
  ///
  /// In fr, this message translates to:
  /// **'La page a été obtenue par {method}.\n{url}\n\nRecharger renverra les données — un effet de bord\n(commande, paiement) peut être répété.'**
  String resendFormBody(Object method, Object url);

  /// No description provided for @themeSystem.
  ///
  /// In fr, this message translates to:
  /// **'Système'**
  String get themeSystem;

  /// No description provided for @themeDark.
  ///
  /// In fr, this message translates to:
  /// **'Sombre'**
  String get themeDark;

  /// No description provided for @themeLight.
  ///
  /// In fr, this message translates to:
  /// **'Clair'**
  String get themeLight;

  /// No description provided for @engineAuto.
  ///
  /// In fr, this message translates to:
  /// **'Auto (FFI si présent, REST sinon)'**
  String get engineAuto;

  /// No description provided for @profileNote.
  ///
  /// In fr, this message translates to:
  /// **'Exporté vers EXPLORER_PROFILE — effet au prochain démarrage du moteur (FFI) ou du daemon (REST). Valider avec Entrée.'**
  String get profileNote;

  /// No description provided for @historyUnavailable.
  ///
  /// In fr, this message translates to:
  /// **'Historique indisponible — {error}'**
  String historyUnavailable(Object error);

  /// No description provided for @bookmarksUnavailable.
  ///
  /// In fr, this message translates to:
  /// **'Signets indisponibles — {error}'**
  String bookmarksUnavailable(Object error);

  /// No description provided for @closeEsc.
  ///
  /// In fr, this message translates to:
  /// **'Fermer (Échap)'**
  String get closeEsc;

  /// No description provided for @previous.
  ///
  /// In fr, this message translates to:
  /// **'Précédent'**
  String get previous;

  /// No description provided for @next.
  ///
  /// In fr, this message translates to:
  /// **'Suivant'**
  String get next;

  /// No description provided for @reload.
  ///
  /// In fr, this message translates to:
  /// **'Recharger'**
  String get reload;

  /// No description provided for @zoomPercent.
  ///
  /// In fr, this message translates to:
  /// **'Zoom {pct} %'**
  String zoomPercent(Object pct);

  /// No description provided for @interfaceLanguage.
  ///
  /// In fr, this message translates to:
  /// **'Interface'**
  String get interfaceLanguage;

  /// No description provided for @downloadSaved.
  ///
  /// In fr, this message translates to:
  /// **'Téléchargement enregistré : {path}'**
  String downloadSaved(Object path);

  /// No description provided for @downloadError.
  ///
  /// In fr, this message translates to:
  /// **'Téléchargement : {error}'**
  String downloadError(Object error);
}

class _AppLocalizationsDelegate
    extends LocalizationsDelegate<AppLocalizations> {
  const _AppLocalizationsDelegate();

  @override
  Future<AppLocalizations> load(Locale locale) {
    return SynchronousFuture<AppLocalizations>(lookupAppLocalizations(locale));
  }

  @override
  bool isSupported(Locale locale) => <String>[
    'de',
    'en',
    'es',
    'fr',
    'hi',
    'id',
    'ja',
    'pt',
    'ru',
    'zh',
  ].contains(locale.languageCode);

  @override
  bool shouldReload(_AppLocalizationsDelegate old) => false;
}

AppLocalizations lookupAppLocalizations(Locale locale) {
  // Lookup logic when only language code is specified.
  switch (locale.languageCode) {
    case 'de':
      return AppLocalizationsDe();
    case 'en':
      return AppLocalizationsEn();
    case 'es':
      return AppLocalizationsEs();
    case 'fr':
      return AppLocalizationsFr();
    case 'hi':
      return AppLocalizationsHi();
    case 'id':
      return AppLocalizationsId();
    case 'ja':
      return AppLocalizationsJa();
    case 'pt':
      return AppLocalizationsPt();
    case 'ru':
      return AppLocalizationsRu();
    case 'zh':
      return AppLocalizationsZh();
  }

  throw FlutterError(
    'AppLocalizations.delegate failed to load unsupported locale "$locale". This is likely '
    'an issue with the localizations generation tool. Please file an issue '
    'on GitHub with a reproducible sample app and the gen-l10n configuration '
    'that was used.',
  );
}

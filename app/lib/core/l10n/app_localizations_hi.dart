// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Hindi (`hi`).
class AppLocalizationsHi extends AppLocalizations {
  AppLocalizationsHi([String locale = 'hi']) : super(locale);

  @override
  String get newTab => 'नया टैब';

  @override
  String get close => 'बंद करें';

  @override
  String get home => 'मुखपृष्ठ';

  @override
  String get history => 'इतिहास';

  @override
  String get bookmarks => 'बुकमार्क';

  @override
  String get addBookmark => 'बुकमार्क जोड़ें';

  @override
  String get removeBookmark => 'बुकमार्क हटाएँ';

  @override
  String get downloads => 'डाउनलोड';

  @override
  String get findInPage => 'पृष्ठ में खोजें';

  @override
  String get findInPageShortcut => 'पृष्ठ में खोजें (Ctrl+F)';

  @override
  String get omniboxHint => 'खोजें या URL दर्ज करें';

  @override
  String get daemonOffline => 'डेमन उपलब्ध नहीं — explorer-daemon चलाएँ';

  @override
  String get retry => 'पुनः प्रयास';

  @override
  String get settings => 'सेटिंग्स';

  @override
  String get zoomIn => 'ज़ूम इन';

  @override
  String get zoomOut => 'ज़ूम आउट';

  @override
  String get actualSize => 'वास्तविक आकार';

  @override
  String get moreActions => 'और क्रियाएँ';

  @override
  String get delegatedTabTooltip => 'डेलीगेटेड टैब — मूल WebView2 रेंडरिंग';

  @override
  String get noTabs => 'कोई टैब नहीं';

  @override
  String get pageLoadError => 'पृष्ठ लोड नहीं हो सका';

  @override
  String get navigateToUrl => 'किसी URL पर जाएँ';

  @override
  String get language => 'भाषा';

  @override
  String get profile => 'प्रोफ़ाइल';

  @override
  String get profileDirLabel => 'प्रोफ़ाइल निर्देशिका (डिफ़ॉल्ट: OS)';

  @override
  String get save => 'सहेजें';

  @override
  String get cancel => 'रद्द करें';

  @override
  String get resend => 'पुनः भेजें';

  @override
  String get resendFormTitle => 'फ़ॉर्म पुनः भेजें?';

  @override
  String get emptyBookmarks => 'कोई बुकमार्क नहीं — बार का सितारा एक जोड़ता है';

  @override
  String get emptyHistory => 'कोई पृष्ठ नहीं देखा गया';

  @override
  String get activeFieldTooltip =>
      'सक्रिय फ़ील्ड — टाइपिंग इंजन को भेजी जाती है';

  @override
  String get acceptLanguageLabel => 'Accept-Language (खाली: OS लोकेल)';

  @override
  String get engineRest => 'REST (डेमन)';

  @override
  String get engineFfi => 'FFI (इन-प्रोसेस इंजन)';

  @override
  String get engineLabel => 'इंजन';

  @override
  String get openAsText => 'टेक्स्ट के रूप में खोलें';

  @override
  String get theme => 'थीम';

  @override
  String get downloadDialogTitle => 'डाउनलोड';

  @override
  String get downloadNotDisplayable =>
      'यह फ़ाइल ब्राउज़र में प्रदर्शित नहीं हो सकती।';

  @override
  String resendFormBody(Object method, Object url) {
    return 'यह पृष्ठ $method से प्राप्त हुआ था।\n$url\n\nरीलोड करने से डेटा फिर भेजा जाएगा — कोई दुष्प्रभाव\n(ऑर्डर, भुगतान) दोहराया जा सकता है।';
  }

  @override
  String get themeSystem => 'सिस्टम';

  @override
  String get themeDark => 'डार्क';

  @override
  String get themeLight => 'लाइट';

  @override
  String get engineAuto => 'ऑटो (FFI उपलब्ध हो तो, वरना REST)';

  @override
  String get profileNote =>
      'EXPLORER_PROFILE में निर्यात — इंजन (FFI) या डेमन (REST) के अगले स्टार्ट पर प्रभावी। Enter से पुष्टि करें।';

  @override
  String historyUnavailable(Object error) {
    return 'इतिहास अनुपलब्ध — $error';
  }

  @override
  String bookmarksUnavailable(Object error) {
    return 'बुकमार्क अनुपलब्ध — $error';
  }

  @override
  String get closeEsc => 'बंद करें (Esc)';

  @override
  String get previous => 'पिछला';

  @override
  String get next => 'अगला';

  @override
  String get reload => 'पुनः लोड करें';

  @override
  String zoomPercent(Object pct) {
    return 'ज़ूम $pct %';
  }

  @override
  String get interfaceLanguage => 'इंटरफ़ेस';

  @override
  String downloadSaved(Object path) {
    return 'डाउनलोड सहेजा गया: $path';
  }

  @override
  String downloadError(Object error) {
    return 'डाउनलोड: $error';
  }
}

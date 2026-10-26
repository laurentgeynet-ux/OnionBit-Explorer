// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Russian (`ru`).
class AppLocalizationsRu extends AppLocalizations {
  AppLocalizationsRu([String locale = 'ru']) : super(locale);

  @override
  String get newTab => 'Новая вкладка';

  @override
  String get close => 'Закрыть';

  @override
  String get home => 'Главная';

  @override
  String get history => 'История';

  @override
  String get bookmarks => 'Закладки';

  @override
  String get addBookmark => 'Добавить закладку';

  @override
  String get removeBookmark => 'Удалить закладку';

  @override
  String get downloads => 'Загрузки';

  @override
  String get findInPage => 'Найти на странице';

  @override
  String get findInPageShortcut => 'Найти на странице (Ctrl+F)';

  @override
  String get omniboxHint => 'Поиск или ввод URL';

  @override
  String get daemonOffline => 'Демон недоступен — запустите explorer-daemon';

  @override
  String get retry => 'Повторить';

  @override
  String get settings => 'Настройки';

  @override
  String get zoomIn => 'Увеличить';

  @override
  String get zoomOut => 'Уменьшить';

  @override
  String get actualSize => 'Фактический размер';

  @override
  String get moreActions => 'Ещё действия';

  @override
  String get delegatedTabTooltip =>
      'Делегированная вкладка — нативный рендеринг WebView2';

  @override
  String get noTabs => 'Нет вкладок';

  @override
  String get pageLoadError => 'Не удалось загрузить страницу';

  @override
  String get navigateToUrl => 'Перейдите по URL';

  @override
  String get language => 'Язык';

  @override
  String get profile => 'Профиль';

  @override
  String get profileDirLabel => 'Каталог профиля (по умолчанию: ОС)';

  @override
  String get save => 'Сохранить';

  @override
  String get cancel => 'Отмена';

  @override
  String get resend => 'Отправить снова';

  @override
  String get resendFormTitle => 'Отправить форму повторно?';

  @override
  String get emptyBookmarks => 'Нет закладок — звезда в панели добавляет их';

  @override
  String get emptyHistory => 'Нет посещённых страниц';

  @override
  String get activeFieldTooltip => 'Активное поле — ввод отправляется движку';

  @override
  String get acceptLanguageLabel => 'Accept-Language (пусто: локаль ОС)';

  @override
  String get engineRest => 'REST (демон)';

  @override
  String get engineFfi => 'FFI (движок в процессе)';

  @override
  String get engineLabel => 'Движок';

  @override
  String get openAsText => 'Открыть как текст';

  @override
  String get theme => 'Тема';

  @override
  String get downloadDialogTitle => 'Загрузка';

  @override
  String get downloadNotDisplayable =>
      'Этот файл нельзя отобразить в браузере.';

  @override
  String resendFormBody(Object method, Object url) {
    return 'Страница получена методом $method.\n$url\n\nПерезагрузка отправит данные снова — побочный эффект\n(заказ, платёж) может повториться.';
  }

  @override
  String get themeSystem => 'Системная';

  @override
  String get themeDark => 'Тёмная';

  @override
  String get themeLight => 'Светлая';

  @override
  String get engineAuto => 'Авто (FFI при наличии, иначе REST)';

  @override
  String get profileNote =>
      'Экспортируется в EXPLORER_PROFILE — действует при следующем запуске движка (FFI) или демона (REST). Подтвердите Enter.';

  @override
  String historyUnavailable(Object error) {
    return 'История недоступна — $error';
  }

  @override
  String bookmarksUnavailable(Object error) {
    return 'Закладки недоступны — $error';
  }

  @override
  String get closeEsc => 'Закрыть (Esc)';

  @override
  String get previous => 'Назад';

  @override
  String get next => 'Вперёд';

  @override
  String get reload => 'Перезагрузить';

  @override
  String zoomPercent(Object pct) {
    return 'Масштаб $pct %';
  }

  @override
  String get interfaceLanguage => 'Интерфейс';

  @override
  String downloadSaved(Object path) {
    return 'Загрузка сохранена: $path';
  }

  @override
  String downloadError(Object error) {
    return 'Загрузка: $error';
  }
}

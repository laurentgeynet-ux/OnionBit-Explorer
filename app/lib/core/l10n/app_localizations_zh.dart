// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Chinese (`zh`).
class AppLocalizationsZh extends AppLocalizations {
  AppLocalizationsZh([String locale = 'zh']) : super(locale);

  @override
  String get newTab => '新标签页';

  @override
  String get close => '关闭';

  @override
  String get home => '主页';

  @override
  String get history => '历史记录';

  @override
  String get bookmarks => '书签';

  @override
  String get addBookmark => '添加书签';

  @override
  String get removeBookmark => '移除书签';

  @override
  String get downloads => '下载';

  @override
  String get findInPage => '在页面中查找';

  @override
  String get findInPageShortcut => '在页面中查找 (Ctrl+F)';

  @override
  String get omniboxHint => '搜索或输入网址';

  @override
  String get daemonOffline => '无法连接守护进程 — 请启动 explorer-daemon';

  @override
  String get retry => '重试';

  @override
  String get settings => '设置';

  @override
  String get zoomIn => '放大';

  @override
  String get zoomOut => '缩小';

  @override
  String get actualSize => '实际大小';

  @override
  String get moreActions => '更多操作';

  @override
  String get delegatedTabTooltip => '委托标签页 — 原生 WebView2 渲染';

  @override
  String get noTabs => '没有标签页';

  @override
  String get pageLoadError => '页面加载失败';

  @override
  String get navigateToUrl => '导航到一个网址';

  @override
  String get language => '语言';

  @override
  String get profile => '配置文件';

  @override
  String get profileDirLabel => '配置文件目录（默认：操作系统）';

  @override
  String get save => '保存';

  @override
  String get cancel => '取消';

  @override
  String get resend => '重新发送';

  @override
  String get resendFormTitle => '重新发送表单？';

  @override
  String get emptyBookmarks => '没有书签 — 点击栏上的星标添加';

  @override
  String get emptyHistory => '尚未访问任何页面';

  @override
  String get activeFieldTooltip => '活动字段 — 输入内容发送给引擎';

  @override
  String get acceptLanguageLabel => 'Accept-Language（留空：使用系统区域）';

  @override
  String get engineRest => 'REST（守护进程）';

  @override
  String get engineFfi => 'FFI（进程内引擎）';

  @override
  String get engineLabel => '引擎';

  @override
  String get openAsText => '以文本打开';

  @override
  String get theme => '主题';

  @override
  String get downloadDialogTitle => '下载';

  @override
  String get downloadNotDisplayable => '此文件无法在浏览器中显示。';

  @override
  String resendFormBody(Object method, Object url) {
    return '该页面是通过 $method 获取的。\n$url\n\n重新加载将重新发送数据 — 副作用\n（订单、付款）可能会重复。';
  }

  @override
  String get themeSystem => '系统';

  @override
  String get themeDark => '深色';

  @override
  String get themeLight => '浅色';

  @override
  String get engineAuto => '自动（有 FFI 则用 FFI，否则用 REST）';

  @override
  String get profileNote =>
      '导出到 EXPLORER_PROFILE — 在引擎（FFI）或守护进程（REST）下次启动时生效。按回车确认。';

  @override
  String historyUnavailable(Object error) {
    return '历史记录不可用 — $error';
  }

  @override
  String bookmarksUnavailable(Object error) {
    return '书签不可用 — $error';
  }

  @override
  String get closeEsc => '关闭 (Esc)';

  @override
  String get previous => '后退';

  @override
  String get next => '前进';

  @override
  String get reload => '重新加载';

  @override
  String zoomPercent(Object pct) {
    return '缩放 $pct %';
  }

  @override
  String get interfaceLanguage => '界面';

  @override
  String downloadSaved(Object path) {
    return '下载已保存：$path';
  }

  @override
  String downloadError(Object error) {
    return '下载：$error';
  }
}

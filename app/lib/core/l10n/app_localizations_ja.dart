// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Japanese (`ja`).
class AppLocalizationsJa extends AppLocalizations {
  AppLocalizationsJa([String locale = 'ja']) : super(locale);

  @override
  String get newTab => '新しいタブ';

  @override
  String get close => '閉じる';

  @override
  String get home => 'ホーム';

  @override
  String get history => '履歴';

  @override
  String get bookmarks => 'ブックマーク';

  @override
  String get addBookmark => 'ブックマークに追加';

  @override
  String get removeBookmark => 'ブックマークを削除';

  @override
  String get downloads => 'ダウンロード';

  @override
  String get findInPage => 'ページ内を検索';

  @override
  String get findInPageShortcut => 'ページ内を検索 (Ctrl+F)';

  @override
  String get omniboxHint => '検索またはURLを入力';

  @override
  String get daemonOffline => 'デーモンに接続できません — explorer-daemon を起動してください';

  @override
  String get retry => '再試行';

  @override
  String get settings => '設定';

  @override
  String get zoomIn => '拡大';

  @override
  String get zoomOut => '縮小';

  @override
  String get actualSize => '実際のサイズ';

  @override
  String get moreActions => 'その他の操作';

  @override
  String get delegatedTabTooltip => '委譲タブ — ネイティブ WebView2 レンダリング';

  @override
  String get noTabs => 'タブがありません';

  @override
  String get pageLoadError => 'ページを読み込めませんでした';

  @override
  String get navigateToUrl => 'URLに移動';

  @override
  String get language => '言語';

  @override
  String get profile => 'プロファイル';

  @override
  String get profileDirLabel => 'プロファイルディレクトリ（デフォルト：OS）';

  @override
  String get save => '保存';

  @override
  String get cancel => 'キャンセル';

  @override
  String get resend => '再送信';

  @override
  String get resendFormTitle => 'フォームを再送信しますか？';

  @override
  String get emptyBookmarks => 'ブックマークがありません — バーのスターで追加できます';

  @override
  String get emptyHistory => '訪問したページがありません';

  @override
  String get activeFieldTooltip => 'アクティブなフィールド — 入力はエンジンに送信されます';

  @override
  String get acceptLanguageLabel => 'Accept-Language（空：OS ロケール）';

  @override
  String get engineRest => 'REST（デーモン）';

  @override
  String get engineFfi => 'FFI（インプロセスエンジン）';

  @override
  String get engineLabel => 'エンジン';

  @override
  String get openAsText => 'テキストとして開く';

  @override
  String get theme => 'テーマ';

  @override
  String get downloadDialogTitle => 'ダウンロード';

  @override
  String get downloadNotDisplayable => 'このファイルはブラウザで表示できません。';

  @override
  String resendFormBody(Object method, Object url) {
    return 'このページは $method で取得されました。\n$url\n\n再読み込みするとデータが再送信されます — 副作用\n（注文、支払い）が繰り返される可能性があります。';
  }

  @override
  String get themeSystem => 'システム';

  @override
  String get themeDark => 'ダーク';

  @override
  String get themeLight => 'ライト';

  @override
  String get engineAuto => '自動（FFI があれば FFI、なければ REST）';

  @override
  String get profileNote =>
      'EXPLORER_PROFILE にエクスポート — エンジン（FFI）またはデーモン（REST）の次回起動時に有効。Enter で確定。';

  @override
  String historyUnavailable(Object error) {
    return '履歴を利用できません — $error';
  }

  @override
  String bookmarksUnavailable(Object error) {
    return 'ブックマークを利用できません — $error';
  }

  @override
  String get closeEsc => '閉じる (Esc)';

  @override
  String get previous => '戻る';

  @override
  String get next => '進む';

  @override
  String get reload => '再読み込み';

  @override
  String zoomPercent(Object pct) {
    return 'ズーム $pct %';
  }

  @override
  String get interfaceLanguage => 'インターフェース';

  @override
  String downloadSaved(Object path) {
    return 'ダウンロードを保存しました：$path';
  }

  @override
  String downloadError(Object error) {
    return 'ダウンロード：$error';
  }
}

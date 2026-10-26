// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Indonesian (`id`).
class AppLocalizationsId extends AppLocalizations {
  AppLocalizationsId([String locale = 'id']) : super(locale);

  @override
  String get newTab => 'Tab baru';

  @override
  String get close => 'Tutup';

  @override
  String get home => 'Beranda';

  @override
  String get history => 'Riwayat';

  @override
  String get bookmarks => 'Bookmark';

  @override
  String get addBookmark => 'Tambah bookmark';

  @override
  String get removeBookmark => 'Hapus bookmark';

  @override
  String get downloads => 'Unduhan';

  @override
  String get findInPage => 'Cari di halaman';

  @override
  String get findInPageShortcut => 'Cari di halaman (Ctrl+F)';

  @override
  String get omniboxHint => 'Cari atau masukkan URL';

  @override
  String get daemonOffline =>
      'Daemon tidak terjangkau — jalankan explorer-daemon';

  @override
  String get retry => 'Coba lagi';

  @override
  String get settings => 'Pengaturan';

  @override
  String get zoomIn => 'Perbesar';

  @override
  String get zoomOut => 'Perkecil';

  @override
  String get actualSize => 'Ukuran sebenarnya';

  @override
  String get moreActions => 'Tindakan lainnya';

  @override
  String get delegatedTabTooltip =>
      'Tab didelegasikan — rendering WebView2 native';

  @override
  String get noTabs => 'Tidak ada tab';

  @override
  String get pageLoadError => 'Halaman gagal dimuat';

  @override
  String get navigateToUrl => 'Navigasi ke URL';

  @override
  String get language => 'Bahasa';

  @override
  String get profile => 'Profil';

  @override
  String get profileDirLabel => 'Direktori profil (default: OS)';

  @override
  String get save => 'Simpan';

  @override
  String get cancel => 'Batal';

  @override
  String get resend => 'Kirim ulang';

  @override
  String get resendFormTitle => 'Kirim ulang formulir?';

  @override
  String get emptyBookmarks =>
      'Belum ada bookmark — bintang di bar menambahkannya';

  @override
  String get emptyHistory => 'Belum ada halaman dikunjungi';

  @override
  String get activeFieldTooltip => 'Kolom aktif — ketikan dikirim ke engine';

  @override
  String get acceptLanguageLabel => 'Accept-Language (kosong: locale OS)';

  @override
  String get engineRest => 'REST (daemon)';

  @override
  String get engineFfi => 'FFI (engine in-process)';

  @override
  String get engineLabel => 'Engine';

  @override
  String get openAsText => 'Buka sebagai teks';

  @override
  String get theme => 'Tema';

  @override
  String get downloadDialogTitle => 'Unduhan';

  @override
  String get downloadNotDisplayable =>
      'File ini tidak dapat ditampilkan di browser.';

  @override
  String resendFormBody(Object method, Object url) {
    return 'Halaman diperoleh melalui $method.\n$url\n\nMemuat ulang akan mengirim ulang data — efek samping\n(pesanan, pembayaran) dapat terulang.';
  }

  @override
  String get themeSystem => 'Sistem';

  @override
  String get themeDark => 'Gelap';

  @override
  String get themeLight => 'Terang';

  @override
  String get engineAuto => 'Otomatis (FFI jika ada, REST jika tidak)';

  @override
  String get profileNote =>
      'Diekspor ke EXPLORER_PROFILE — berlaku saat engine (FFI) atau daemon (REST) berikutnya dimulai. Konfirmasi dengan Enter.';

  @override
  String historyUnavailable(Object error) {
    return 'Riwayat tidak tersedia — $error';
  }

  @override
  String bookmarksUnavailable(Object error) {
    return 'Bookmark tidak tersedia — $error';
  }

  @override
  String get closeEsc => 'Tutup (Esc)';

  @override
  String get previous => 'Sebelumnya';

  @override
  String get next => 'Berikutnya';

  @override
  String get reload => 'Muat ulang';

  @override
  String zoomPercent(Object pct) {
    return 'Zoom $pct %';
  }

  @override
  String get interfaceLanguage => 'Antarmuka';

  @override
  String downloadSaved(Object path) {
    return 'Unduhan disimpan: $path';
  }

  @override
  String downloadError(Object error) {
    return 'Unduhan: $error';
  }
}

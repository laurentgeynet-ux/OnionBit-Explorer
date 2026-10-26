// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Portuguese (`pt`).
class AppLocalizationsPt extends AppLocalizations {
  AppLocalizationsPt([String locale = 'pt']) : super(locale);

  @override
  String get newTab => 'Nova aba';

  @override
  String get close => 'Fechar';

  @override
  String get home => 'Início';

  @override
  String get history => 'Histórico';

  @override
  String get bookmarks => 'Favoritos';

  @override
  String get addBookmark => 'Adicionar favorito';

  @override
  String get removeBookmark => 'Remover favorito';

  @override
  String get downloads => 'Transferências';

  @override
  String get findInPage => 'Pesquisar na página';

  @override
  String get findInPageShortcut => 'Pesquisar na página (Ctrl+F)';

  @override
  String get omniboxHint => 'Pesquisar ou introduzir um URL';

  @override
  String get daemonOffline => 'Daemon inacessível — inicie explorer-daemon';

  @override
  String get retry => 'Tentar novamente';

  @override
  String get settings => 'Definições';

  @override
  String get zoomIn => 'Ampliar';

  @override
  String get zoomOut => 'Reduzir';

  @override
  String get actualSize => 'Tamanho real';

  @override
  String get moreActions => 'Mais ações';

  @override
  String get delegatedTabTooltip =>
      'Aba delegada — renderização WebView2 nativa';

  @override
  String get noTabs => 'Sem abas';

  @override
  String get pageLoadError => 'Não foi possível carregar a página';

  @override
  String get navigateToUrl => 'Navegue para um URL';

  @override
  String get language => 'Idioma';

  @override
  String get profile => 'Perfil';

  @override
  String get profileDirLabel => 'Diretório de perfil (padrão: SO)';

  @override
  String get save => 'Guardar';

  @override
  String get cancel => 'Cancelar';

  @override
  String get resend => 'Reenviar';

  @override
  String get resendFormTitle => 'Reenviar o formulário?';

  @override
  String get emptyBookmarks => 'Sem favoritos — a estrela da barra adiciona um';

  @override
  String get emptyHistory => 'Nenhuma página visitada';

  @override
  String get activeFieldTooltip => 'Campo ativo — escrita enviada ao motor';

  @override
  String get acceptLanguageLabel => 'Accept-Language (vazio: locale do SO)';

  @override
  String get engineRest => 'REST (daemon)';

  @override
  String get engineFfi => 'FFI (motor em processo)';

  @override
  String get engineLabel => 'Motor';

  @override
  String get openAsText => 'Abrir como texto';

  @override
  String get theme => 'Tema';

  @override
  String get downloadDialogTitle => 'Transferência';

  @override
  String get downloadNotDisplayable =>
      'Este ficheiro não pode ser apresentado no navegador.';

  @override
  String resendFormBody(Object method, Object url) {
    return 'A página foi obtida por $method.\n$url\n\nRecarregar reenviará os dados — um efeito secundário\n(encomenda, pagamento) pode ser repetido.';
  }

  @override
  String get themeSystem => 'Sistema';

  @override
  String get themeDark => 'Escuro';

  @override
  String get themeLight => 'Claro';

  @override
  String get engineAuto => 'Auto (FFI se presente, REST caso contrário)';

  @override
  String get profileNote =>
      'Exportado para EXPLORER_PROFILE — efeito no próximo arranque do motor (FFI) ou daemon (REST). Validar com Enter.';

  @override
  String historyUnavailable(Object error) {
    return 'Histórico indisponível — $error';
  }

  @override
  String bookmarksUnavailable(Object error) {
    return 'Favoritos indisponíveis — $error';
  }

  @override
  String get closeEsc => 'Fechar (Esc)';

  @override
  String get previous => 'Anterior';

  @override
  String get next => 'Seguinte';

  @override
  String get reload => 'Recarregar';

  @override
  String zoomPercent(Object pct) {
    return 'Zoom $pct %';
  }

  @override
  String get interfaceLanguage => 'Interface';

  @override
  String downloadSaved(Object path) {
    return 'Transferência guardada: $path';
  }

  @override
  String downloadError(Object error) {
    return 'Transferência: $error';
  }
}

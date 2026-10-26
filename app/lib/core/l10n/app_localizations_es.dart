// ignore: unused_import
import 'package:intl/intl.dart' as intl;

import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for Spanish Castilian (`es`).
class AppLocalizationsEs extends AppLocalizations {
  AppLocalizationsEs([String locale = 'es']) : super(locale);

  @override
  String get newTab => 'Nueva pestaña';

  @override
  String get close => 'Cerrar';

  @override
  String get home => 'Inicio';

  @override
  String get history => 'Historial';

  @override
  String get bookmarks => 'Marcadores';

  @override
  String get addBookmark => 'Añadir marcador';

  @override
  String get removeBookmark => 'Quitar marcador';

  @override
  String get downloads => 'Descargas';

  @override
  String get findInPage => 'Buscar en la página';

  @override
  String get findInPageShortcut => 'Buscar en la página (Ctrl+F)';

  @override
  String get omniboxHint => 'Buscar o introducir una URL';

  @override
  String get daemonOffline => 'Daemon inaccesible — inicia explorer-daemon';

  @override
  String get retry => 'Reintentar';

  @override
  String get settings => 'Configuración';

  @override
  String get zoomIn => 'Acercar';

  @override
  String get zoomOut => 'Alejar';

  @override
  String get actualSize => 'Tamaño real';

  @override
  String get moreActions => 'Más acciones';

  @override
  String get delegatedTabTooltip =>
      'Pestaña delegada — renderizado WebView2 nativo';

  @override
  String get noTabs => 'Sin pestañas';

  @override
  String get pageLoadError => 'No se pudo cargar la página';

  @override
  String get navigateToUrl => 'Navega a una URL';

  @override
  String get language => 'Idioma';

  @override
  String get profile => 'Perfil';

  @override
  String get profileDirLabel => 'Directorio de perfil (predeterminado: SO)';

  @override
  String get save => 'Guardar';

  @override
  String get cancel => 'Cancelar';

  @override
  String get resend => 'Reenviar';

  @override
  String get resendFormTitle => '¿Reenviar el formulario?';

  @override
  String get emptyBookmarks =>
      'Sin marcadores — la estrella de la barra añade uno';

  @override
  String get emptyHistory => 'Ninguna página visitada';

  @override
  String get activeFieldTooltip =>
      'Campo activo — la escritura se envía al motor';

  @override
  String get acceptLanguageLabel => 'Accept-Language (vacío: locale del SO)';

  @override
  String get engineRest => 'REST (daemon)';

  @override
  String get engineFfi => 'FFI (motor en proceso)';

  @override
  String get engineLabel => 'Motor';

  @override
  String get openAsText => 'Abrir como texto';

  @override
  String get theme => 'Tema';

  @override
  String get downloadDialogTitle => 'Descarga';

  @override
  String get downloadNotDisplayable =>
      'Este archivo no se puede mostrar en el navegador.';

  @override
  String resendFormBody(Object method, Object url) {
    return 'La página se obtuvo mediante $method.\n$url\n\nRecargar reenviará los datos — puede repetirse un efecto secundario\n(pedido, pago).';
  }

  @override
  String get themeSystem => 'Sistema';

  @override
  String get themeDark => 'Oscuro';

  @override
  String get themeLight => 'Claro';

  @override
  String get engineAuto => 'Auto (FFI si está presente, REST si no)';

  @override
  String get profileNote =>
      'Exportado a EXPLORER_PROFILE — efecto en el próximo inicio del motor (FFI) o daemon (REST). Confirmar con Entrar.';

  @override
  String historyUnavailable(Object error) {
    return 'Historial no disponible — $error';
  }

  @override
  String bookmarksUnavailable(Object error) {
    return 'Marcadores no disponibles — $error';
  }

  @override
  String get closeEsc => 'Cerrar (Esc)';

  @override
  String get previous => 'Anterior';

  @override
  String get next => 'Siguiente';

  @override
  String get reload => 'Recargar';

  @override
  String zoomPercent(Object pct) {
    return 'Zoom $pct %';
  }

  @override
  String get interfaceLanguage => 'Interfaz';

  @override
  String downloadSaved(Object path) {
    return 'Descarga guardada: $path';
  }

  @override
  String downloadError(Object error) {
    return 'Descarga: $error';
  }
}

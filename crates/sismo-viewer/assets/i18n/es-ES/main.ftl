## Común
common-open = Abrir
common-cancel = Cancelar
common-close = Cerrar
common-export = Exportar
common-state-needs-code = PIDE CÓDIGO
common-space = Espacio

## Grupos y pestañas
group-app = App
group-framework = Framework
group-other = Otros
tab-timeline = Timeline
tab-flamegraph = Flamegraph
tab-functions = Funciones
tab-logs = Logs
tab-metrics = Métricas

## Avisos
toast-open-failed = No se pudo abrir
toast-connect-failed = No se pudo conectar
toast-compare-failed = No se pudo comparar
toast-recording-saved = Grabación guardada
toast-recording-saved-body = { $duration } · { $size } · { $markers } marcadores
toast-incompatible-format = Formato incompatible
toast-incompatible-format-body = Actualiza el visor o usa la misma versión de sismo en la app.

## Sesiones
sessions-title = Sesiones
sessions-open-file = Abrir archivo…
sessions-seen-now = ahora
sessions-seen-ago = visto hace { $secs } s
sessions-state-incompatible = INCOMPATIBLE
sessions-state-connected = CONECTADA
sessions-state-available = DISPONIBLE
sessions-action-details = Detalles
sessions-action-pair = Emparejar…
sessions-action-connect = Conectar
sessions-incompatible-detail = { $platform } · usa formato sismo v{ $theirs }, este visor lee v{ $ours }
sessions-step-live-title = La app tiene sismo en modo Live
sessions-step-live-body = SISMO=live o Config::live(..); en release viene apagado.
sessions-step-wifi-title = Mismo Wi-Fi que esta PC
sessions-step-wifi-body = Redes de invitados y algunos routers bloquean broadcast entre equipos.
sessions-step-firewall-title = El firewall deja pasar UDP 7701
sessions-step-firewall-body = Es el puerto de descubrimiento; los datos usan 7720.
sessions-step-android-title = Android: permiso INTERNET en el manifest
sessions-step-android-body = Sin él la app no puede anunciarse.
sessions-searching = Buscando apps…
sessions-none-found = Ninguna app con sismo en esta red
sessions-scan-failed = El escaneo falló: { $error }
sessions-no-network-hint = ¿Sin red con el cliente? Pídele «Exportar diagnóstico» y abre el archivo a la derecha.
sessions-live-title = En vivo en la red
sessions-live-broadcast = broadcast zudp :7701 · { $count } apps
sessions-live-footer = Las apps aparecen solas cuando activan sismo en modo Live · Enter abre la seleccionada
sessions-open-sessions = Sesiones abiertas
sessions-files = Archivos
sessions-drop-title = Suelta un .sismo aquí
sessions-drop-body = o Ctrl O para elegirlo · también los que exporta un cliente
sessions-recent = Recientes
sessions-tag-black-box = caja negra
sessions-tag-recorded = grabado
sessions-headless = Sin interfaz
sessions-cli-file = archivo

## Ventana de sesión
shell-file-badge = archivo · { $duration }
shell-black-box-badge = CAJA NEGRA · panic · { $duration }
shell-damaged-badge = { $label } · cortado
shell-connecting = CONECTANDO
shell-lost = SIN CONEXIÓN
shell-paused = PAUSADO
shell-live = EN VIVO
shell-rec-badge = REC { $clock } · { $size } · { $markers } marcadores
shell-marker = Marcador  M
shell-resume = Reanudar
shell-pause = Pausar
shell-stop-recording = Detener  Ctrl R
shell-record = Grabar…
shell-export = Exportar…
shell-time-share = % tiempo
shell-threads = Hilos
shell-status-live = { $rate } eventos/s   ·   { $dropped } descartados   ·   recibido { $received }   ·   zudp { $addr }   ·   rtt { $rtt }
shell-status-file = archivo · { $frames } frames · { $events } eventos · { $logs } logs · { $dropped } descartados · { $platform } · { $device } · { $version }
shell-app-filter = filtro en app: { $filter }
shell-waiting-connecting = Conectando…
shell-waiting-for = Esperando a { $addr }
shell-waiting-pairing = Falta el código de emparejamiento
shell-waiting-pairing-body = La app muestra un código de 6 dígitos en su log (target sismo).
shell-waiting-lost = Sin conexión
shell-retry = Reintentar
shell-enter-code = Ingresar código…
shell-lost-banner = Se perdió la conexión ({ $reason }) · lo recibido hasta { $until } sigue aquí
shell-retry-now = Reintentar ahora
shell-save-received = Guardar lo recibido…
shell-paused-banner = Vista congelada · la app sigue mandando y el visor guarda en segundo plano · Espacio reanuda

## Barra de consulta
query-group = grupo
query-thread-spans = { $count } spans
query-level-or-worse = ese nivel o más grave
query-level-only = solo ese nivel
query-frame-slow = frames sobre 16.6 ms
query-placeholder = Filtrar · crate:app dur>2ms level:warn+ "texto"
query-values = Valores
query-filters = Filtros
query-hint = ↑↓ elegir · Tab completar · , agrega otro valor · - delante excluye · ? sintaxis
common-show-in-functions = Ver en Funciones

## Timeline
timeline-no-frames = Sin frames: llama a sismo::frame_mark() o usa el plugin de Freya
timeline-span-location = { $file }:{ $line }   ·   crate { $krate } ({ $group })   ·   hilo { $thread }
timeline-duration = Duración
timeline-self-time = Tiempo propio
timeline-children = Hijos
timeline-start = Inicio
timeline-allocs = Allocs
timeline-open-in-flamegraph = Abrir en Flamegraph
timeline-session-calls = En la sesión · { $count } llamadas · p50 { $p50 } · p95 { $p95 } · max { $max }
timeline-frames = Frames
timeline-frames-summary = { $count } frames   p50 { $p50 }   p95 { $p95 }   > 16.6 ms: { $slow }
timeline-hint-live = arrastrar en frames = elegir rango · clic derecho = volver a vivo · rueda = zoom · arrastrar carriles = mover
timeline-hint-file = arrastrar en frames = elegir rango · clic derecho = ver todo · rueda = zoom · arrastrar carriles = mover
timeline-next-slow = Siguiente lento  N
timeline-follow-live = En vivo  Fin
timeline-show-all = Ver todo

## Flamegraph
flame-empty = Nada que mostrar en este rango con esta consulta
flame-root = (raíz)
flame-total-own = total { $total } · propio { $own }
flame-called-by = Llamado por
flame-calls = Llama a
flame-own = (propio)
flame-where-time-goes = ¿Dónde se va el tiempo?
flame-window-selection = selección
flame-window-session = toda la sesión
flame-icicle = Icicle
flame-aggregate = agregado · { $window } · { $threads } hilos
flame-hint = ancho = parte del tiempo · clic = acercar · clic en la barra de arriba o en vacío = alejar · clic derecho = menú
flame-root = Todo
flame-up = Subir un nivel  Esc

## Métricas
metrics-frame-time = Frame time
metrics-frame-summary = p95 { $p95 } · máx { $max } · { $slow } lentos
metrics-now = ahora { $now }
metrics-now-range = ahora { $now } · rango { $lo }–{ $hi }
metrics-no-counters = La app no registró contadores · usa sismo::counter!("nombre", valor)

## Logs
common-show-in-timeline = Ver en Timeline
common-count-of-total = { $count } de { $total }
logs-detail-location = { $time }  ·  { $target }  ·  { $file }:{ $line }  ·  hilo { $thread }  ·  fuente { $source }
logs-fields = Campos
logs-no-fields = sin campos
logs-inside = Dentro de
logs-outside-span = fuera de cualquier span
logs-following = Siguiendo  End
logs-follow-paused = Pausado el seguimiento

## Funciones
functions-col-function = Función
functions-col-calls = Llamadas
functions-col-total = Total
functions-col-self = Propio
functions-col-p50 = p50
functions-col-p95 = p95
functions-col-max = Máx
functions-col-alloc = Alloc
functions-col-history = Histórico p95
functions-detail-calls = { $count } llamadas · p50 { $p50 } · p95 { $p95 } · máx { $max }
functions-detail-allocs = alloc { $own } propio · { $total } total · { $per_call } por llamada · { $count } allocs
functions-history-caption = Histórico · p95 por tramo
functions-worst-calls = Peores llamadas
functions-worst-hint = Clic en una llamada = abrirla en el Timeline
functions-filter-to = Filtrar a esta función
functions-empty-title = Nada con «{ $query }»
functions-empty-body = Quita el término más estricto o amplía el rango.
functions-clear-query = Limpiar consulta
functions-footer = { $count } de { $total } funciones · clic en la cabecera ordena · Enter abre el detalle
functions-remove-compare = Quitar comparación
functions-compare-new = nueva
functions-compare-gone = ya no está
functions-compare-footer = { $count } funciones · rojo = peor en B · azul = mejor · gris = dentro del ruido (±10%) · ordenado por Δ absoluto

## Diálogos
toast-saved = Guardado
toast-save-failed = No se pudo guardar
toast-cleared = Limpio, se muestra lo que llegue desde ahora
toast-clear-failed = No se pudo limpiar
toast-record-failed = No se pudo grabar
toast-exported = Exportado
toast-export-failed = No se pudo exportar
toast-show-in-folder = Mostrar en la carpeta
dialog-pick-recording = Abrir grabación de sismo
dialog-pick-compare = Comparar con…
dialog-save-received = Guardar lo recibido
dialog-file = Archivo
dialog-file-path = ruta del archivo
dialog-pair-title = Emparejar con { $app }
dialog-pair-body = Escribe el código de 6 dígitos que la app muestra en su log (target sismo) o en su pantalla de diagnóstico.
dialog-pair-remembered = Se recuerda este visor: la próxima vez conecta sin código.
dialog-pair-submit = Emparejar
dialog-record-title = Grabar sesión
dialog-record-body = Guarda en esta PC lo que llega de la app, en el formato que abren el visor y el CLI.
dialog-record-from = Desde
dialog-record-include-buffer = Incluir lo ya recibido
dialog-record-include-buffer-sub = { $size } desde que te conectaste, con el buffer de la app · sirve para grabar algo que ya pasó
dialog-record-from-now = Solo desde ahora
dialog-record-stop = Parar
dialog-stop-manual = A mano
dialog-stop-5min = 5 min
dialog-stop-500mb = 500 MB
dialog-stop-first-error = Al primer error
dialog-record-first-error-hint = «Al primer error» sigue 10 s más para capturar lo que vino después.
dialog-record-on-device = Grabar también en el dispositivo
dialog-record-on-device-sub = La app escribe su propia copia y la manda al terminar; aguanta cortes de Wi-Fi.
dialog-record-start = Empezar a grabar  Ctrl R
dialog-format-perfetto = Perfetto  .pftrace
dialog-format-perfetto-sub = Timeline completo en ui.perfetto.dev o SQL con trace_processor
dialog-format-summary = Resumen JSON  .json
dialog-format-summary-sub = Para agentes y scripts: top funciones, frames lentos, errores
dialog-format-markdown = Reporte Markdown  .md
dialog-format-markdown-sub = Legible para un issue o un PR
dialog-format-chrome = Chrome trace  .json
dialog-format-chrome-sub = chrome://tracing y herramientas viejas
dialog-format-sismo = .sismo recortado
dialog-format-sismo-sub = Solo el rango elegido, para mandarlo a otra persona
dialog-export-format = Formato
dialog-export-scope = Alcance
dialog-export-scope-session = Toda la sesión
dialog-export-scope-selection = Selección del Timeline
dialog-export-scope-query = Lo que pasa la consulta actual
dialog-export-include-logs = Incluir logs
dialog-export-strip-fields = Quitar campos de los logs
dialog-export-strip-fields-sub = Útil si pueden tener datos de usuarios
dialog-filter-default = (por defecto)
dialog-filter-title = Qué manda la app
dialog-filter-body = Se aplica en el dispositivo: bajar el nivel baja el costo, no solo lo que ves.
dialog-filter-placeholder = info,mi_app=trace,zudp=warn
dialog-filter-levels = Niveles por target
dialog-filter-apply = Aplicar en la app
dialog-clear-title = ¿Limpiar esta sesión?
dialog-clear-body = Todo lo recibido de { $app } hasta ahora ({ $size }, { $duration }) sale de este visor. La app sigue conectada y lo nuevo aparece desde ahora.
dialog-clear-save-hint = «Guardar y limpiar» deja una copia en { $folder }.
dialog-clear-discard = Limpiar
dialog-clear-save = Guardar y limpiar
shell-clear = Limpiar
dialog-marker-title = Marcador
dialog-marker-placeholder = qué estás haciendo
dialog-marker-body = Queda en la app y en la grabación; aparece en Timeline, Logs y en el resumen del CLI.
dialog-marker-save = Guardar  Enter

## Paleta de comandos
palette-placeholder = Buscar acción o función…
palette-actions = Acciones
palette-hint = Enter ejecuta la primera · Esc cierra
palette-compare = Comparar con…
palette-record-toggle = Grabar / detener
palette-go-timeline = Ir a Timeline
palette-go-flamegraph = Ir a Flamegraph
palette-go-functions = Ir a Funciones
palette-go-logs = Ir a Logs
palette-go-metrics = Ir a Métricas
palette-app-filter = Qué manda la app…
palette-back-to-sessions = Volver a Sesiones
palette-shortcuts = Atajos de teclado

## Atajos
shortcuts-title = Atajos
shortcuts-general = General
shortcuts-session = Sesión
shortcuts-lists = Listas
shortcuts-palette = Paleta de comandos
shortcuts-open-file = Abrir archivo
shortcuts-close-session = Cerrar sesión
shortcuts-these = Estos atajos
shortcuts-pause-resume = Pausar / reanudar
shortcuts-zoom = Acercar / alejar
shortcuts-pan = Mover
shortcuts-range = Rango
shortcuts-key-drag = arrastrar
shortcuts-next-slow = Siguiente frame lento
shortcuts-back-to-live = Volver a vivo
shortcuts-pick = Elegir
shortcuts-key-click = clic
shortcuts-tab = Pestaña 1–5
shortcuts-clear-selection = Quitar selección

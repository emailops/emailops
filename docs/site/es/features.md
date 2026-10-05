---
title: 'Funciones estándar'
description: 'El cliente de correo en sí: cuentas, bandeja unificada, archivo, posponer, envío programado, firmas, calendario, adjuntos, búsqueda, filtrado de correo basura, notificaciones y atajos de teclado.'
weight: 30
nav:
  unified-inbox: view/inbox
  calendar: view/calendar
  attachments-view: view/attachments
  junk-and-bulk-mail: settings/junk
  privacy-and-security-controls: settings/privacy
  interface: settings/appearance
  undo-send-and-scheduled-send: settings/appearance
  unsubscribe-and-block-sender: settings/junk
  keyboard-shortcuts: settings/appearance
---

<!-- claim:feat-intro-1 -->
Todo lo de esta página funciona con la IA desactivada. La capa de IA se trata aparte en
[Funciones de IA](../ai-features/).

## Cuentas y sincronización

<!-- claim:feat-accounts-sync-1 -->
Conecta tantos buzones como quieras — Gmail, Outlook / Microsoft 365 (API Graph) y cualquier
servidor IMAP/SMTP (iCloud, Yahoo, Fastmail, ProtonMail Bridge, autoalojado). El correo se
sincroniza en una base de datos SQLite local, así que leer y buscar es rápido y funciona sin
conexión.

<!-- claim:feat-accounts-sync-2 -->
El nombre de cada cuenta es el remitente que ven los destinatarios en el correo que envías
desde ella. Cámbialo en el campo **Nombre del remitente** de los ajustes de la cuenta, o déjalo
vacío para enviar solo con la dirección. Las cuentas de Gmail empiezan con el nombre de su
configuración «Enviar como» de Gmail, y las IMAP con el nombre visible que indicaste al
conectarlas. El correo enviado por Outlook lleva el nombre que Microsoft tiene para el buzón.

## Bandeja unificada {#unified-inbox}

<!-- claim:feat-unified-inbox-1 -->
La vista **Todas las cuentas** fusiona cada buzón activo en una sola lista, junto a las vistas
por cuenta. Las carpetas IMAP personalizadas también se sincronizan, y puedes crearlas,
renombrarlas, borrarlas y arrastrar mensajes entre ellas desde la propia app.

## Reenviar

<!-- claim:reading-pane-forward -->
**Reenviar** está junto a **Responder** y **Responder a todos** en el panel de lectura. El
borrador se abre sin destinatarios y lleva el mensaje original bajo una cabecera *Mensaje
reenviado* con su remitente, fecha y destinatarios, junto con los adjuntos originales (hasta
20 MB en total). Sale como un mensaje nuevo, así que no se une a las conversaciones que el
destinatario ya tiene.

## Organizar conversaciones {#organizing-conversations}

<!-- claim:feat-organize-1 -->
**Archivar** saca una conversación de la bandeja de entrada sin borrarla, y **Mover a Recibidos**
la devuelve. Ambas, junto con **Marcar como no leído** y **Destacar**, están en el panel de lectura
y en el menú **Más acciones** (⋮) de cada conversación; la estrella aparece además en cada fila de
la lista. El cambio se hace también en tu proveedor de correo, así que Gmail u Outlook muestran lo
mismo.

<!-- claim:feat-organize-2 -->
**Destacados**, en la barra lateral, lista tus conversaciones destacadas. **Archivo** lista el
correo archivado de las cuentas de Gmail y Outlook y en **Todas las cuentas**; una cuenta IMAP
archiva en su propia carpeta de archivo, que aparece junto a sus demás carpetas. El correo
archivado solo sale de la bandeja de entrada: la búsqueda, los filtros inteligentes y las
funciones de IA lo siguen alcanzando.

<!-- claim:feat-organize-3 -->
Marca la casilla al principio de una fila para seleccionarla. Con una o más seleccionadas, una
barra sobre la lista actúa sobre todas a la vez — archivar, posponer, eliminar, marcar como leídas
o no leídas, destacar — y **Borrar selección** la termina. Las cuentas con carpetas propias
tienen además **Mover a carpeta**.

<!-- claim:feat-organize-4 -->
Archivar o eliminar quita las conversaciones de la lista al momento y muestra un aviso con
**Deshacer** durante 6 segundos. Tu proveedor de correo solo se entera cuando pasan esos segundos
(o antes, si archivas o eliminas otra cosa o abres otra vista), así que Deshacer simplemente las
devuelve.

<!-- claim:feat-organize-5 -->
Cuando la conversación que estás leyendo sale de la lista — archivada, eliminada, pospuesta o
movida a Spam —, se abre la siguiente. **Ajustes → Apariencia → Después de archivar o eliminar**
elige entre la conversación siguiente, la anterior o volver a la lista. Marcar una conversación
como no leída siempre vuelve a la lista.

## Posponer {#snooze}

<!-- claim:feat-snooze-1 -->
**Posponer** oculta una conversación de la bandeja de entrada hasta el momento que elijas: más
tarde hoy, mañana, este fin de semana, la próxima semana, o la fecha y hora que indiques. Se
ofrece en el panel de lectura, en el menú ⋮ de la fila y en la barra de selección.

<!-- claim:feat-snooze-2 -->
Las conversaciones pospuestas aparecen en **Pospuestos**, en la barra lateral, de la más próxima
a la más lejana; **Dejar de posponer** devuelve una antes de tiempo. Llegado el momento, la
conversación vuelve arriba del todo en la bandeja de entrada, marcada como no leída. Un mensaje
nuevo en una conversación pospuesta la devuelve en el acto.

<!-- claim:feat-snooze-3 -->
Posponer se guarda solo en este ordenador: las demás aplicaciones de correo siguen mostrando la
conversación en la bandeja de entrada. Las conversaciones vuelven mientras EmailOps está en
marcha; una cuya hora pasó con la app cerrada vuelve la próxima vez que la abras.

## Deshacer envío y envío programado {#undo-send-and-scheduled-send}

<!-- claim:feat-send-1 -->
Al pulsar **Enviar**, el mensaje espera unos segundos con un aviso de **Deshacer**; Deshacer lo
retira y lo vuelve a abrir para editarlo. La espera se ajusta en **Ajustes → Apariencia →
Deshacer envío**: desactivado, 5, 10, 20 o 30 segundos, 10 por defecto. EmailOps tiene que seguir
abierto hasta que el mensaje haya salido.

<!-- claim:feat-send-2 -->
La flecha junto a **Enviar** abre **Programar envío**: mañana por la mañana, mañana por la tarde,
lunes por la mañana, o la fecha y hora que indiques.

<!-- claim:feat-send-3 -->
Los mensajes pendientes de salir aparecen en **Programados**, en la barra lateral, donde puedes
**Enviar ahora**, **Editar** o **Eliminar** cada uno. Un mensaje programado solo sale mientras
EmailOps está abierto; uno cuya hora pasó con la app cerrada se envía la próxima vez que la
abras. Un mensaje que no se pudo enviar se queda ahí marcado como **No enviado**, con
**Reintentar**: EmailOps nunca lo reenvía por su cuenta.
## Firmas {#signatures}

<!-- claim:feat-signatures-1 -->
Cada cuenta tiene su propia firma, que se configura en **Ajustes → Firmas**. Dos interruptores
deciden dónde va: **Insertar en mensajes nuevos** e **Insertar en respuestas y reenvíos**. En un
mensaje nuevo o una respuesta va debajo de tu texto; en un reenvío, encima del mensaje reenviado.
Forma parte del cuerpo del mensaje, así que puedes cambiarla o borrarla en cualquier mensaje antes
de enviarlo.

<!-- claim:feat-signatures-2 -->
**Añadir imagen** inserta en ella un logotipo o una imagen de tu firma manuscrita. Se aceptan PNG,
JPEG, GIF y WebP; SVG y otros archivos se rechazan indicando el motivo. Una imagen ancha se reduce
a 600 px, cada imagen puede ocupar 200 KB como máximo y la firma entera 512 KB. Las cuentas de
Gmail ofrecen además **Importar de Gmail**, que copia en el editor la firma que Gmail tiene para
esa dirección.

<!-- claim:feat-signatures-3 -->
En la versión de texto plano de un mensaje, una firma que lo cierra va precedida de la línea
estándar `-- `, para que otras aplicaciones de correo la reconozcan. Cuando la cuenta tiene firma
para ese tipo de mensaje, los borradores de la IA no incluyen tu nombre ni tus datos de contacto y
dejan que firme la firma.

## Filtros inteligentes

<!-- claim:feat-smart-filters-1 -->
Acota la lista por dominio, remitente o cualquier etiqueta de clasificación — útil para
despachar un cliente, un proyecto o una avalancha de newsletters de una vez. Con la IA
activada, esas mismas etiquetas alimentan el [Tablero de etiquetas](../ai-features/#tag-board),
que las muestra como una cuadrícula de bloques.

## Calendario {#calendar}

<!-- claim:feat-calendar-1 -->
Vistas de mes, semana y día por cuenta para Google Calendar y Outlook. Recibes recordatorios
antes de cada evento con un botón **Unirse** de un clic para enlaces de Meet, Teams, Webex y
Zoom. La sincronización de calendario está activada por defecto en las cuentas de Gmail y
Outlook y puede desactivarse por cuenta, junto con la antelación del aviso, en
**Ajustes → Calendario**.

<!-- claim:feat-calendar-2 -->
Se sincronizan todos los calendarios de una cuenta, no solo el principal — así que un
calendario que un compañero haya compartido contigo aparece aquí igual que en Google o en
Outlook. Cada uno se tiñe con el color que le da su proveedor, y la leyenda sobre la
cuadrícula oculta o muestra calendarios individuales; los mismos interruptores están en
**Ajustes → Calendario**.

<!-- claim:feat-calendar-3 -->
Las notificaciones del sistema de un recordatorio omiten el título de la reunión salvo que
actives **Mostrar el título de la reunión en las notificaciones** en **Ajustes → Calendario**:
pueden verse en la pantalla de bloqueo y en el centro de notificaciones, fuera del bloqueo de
la app. El aviso dentro de EmailOps siempre muestra la reunión completa.

## Vista de adjuntos {#attachments-view}

<!-- claim:feat-attachments-view-1 -->
Un único sitio con los adjuntos que te importan — facturas, contratos, recibos — con vista
previa y descarga, en lugar de bucear otra vez en los hilos. Ábrela desde **Adjuntos** en la
barra lateral.

<!-- claim:feat-attachments-view-2 -->
La vista recopila adjuntos mediante **reglas**, así que empieza vacía. Pulsa **Gestionar
reglas** (o **Crear una regla** en la vista vacía) y rellena:

- **Nombre de la regla** — cómo aparece en la lista. <!-- claim:feat-attachments-view-3 -->
- **Patrón del remitente** — separados por coma; coincidencia exacta salvo que lleve `*`
  (`*apple.com*` coincide con cualquier remitente que contenga "apple.com"). Déjalo vacío para
  cualquier remitente. <!-- claim:feat-attachments-view-4 -->
- **Patrón del asunto** y **Patrón del nombre de archivo** — `*` es un comodín; solo se
  recopilan los nombres de archivo que coinciden. <!-- claim:feat-attachments-view-5 -->
- **Etiquetas** — elige las que ya usas o escribe para crear una nueva; aparecen como botones de filtro arriba de la vista. <!-- claim:feat-attachments-view-6 -->

<!-- claim:feat-attachments-view-7 -->
Tienen que coincidir todos los patrones que rellenes. Las reglas se aplican al correo nuevo
según se sincroniza; marca **Aplicar a los correos existentes después de crear** para recopilar
también del correo que ya tienes. Las reglas llegan a la bandeja de entrada, Enviados, el
archivo y tus propias carpetas, nunca a Spam ni a la Papelera. Selecciona adjuntos para descargarlos juntos en
tu carpeta de Descargas.

<!-- claim:feat-attachments-view-8 -->
EmailOps también propone reglas por su cuenta. Cuando un mismo remitente te envía documentos una
y otra vez (PDF, archivos de Office o facturas electrónicas) — al menos dos correos en dos meses
distintos, en la bandeja de entrada o en una carpeta donde los archives —, aparece una sección **Reglas sugeridas** en **Gestionar reglas**, y un contador
junto a **Adjuntos** en la barra lateral indica cuántas hay. **Revisar** abre el formulario de la
regla ya relleno (remitente y patrón de nombre de archivo); la regla solo se crea cuando
la guardas. **Descartar** oculta la sugerencia para siempre, aunque el remitente escriba más
adelante desde otra dirección; **Deshacer**, o **Restaurar** en **Sugerencias descartadas**, la
recupera. Nunca se sugiere el correo de tu propia dirección ni el de
compañeros de tu propia empresa.

## Búsqueda

<!-- claim:feat-search-1 -->
Búsqueda de texto completo en asuntos, cuerpos, remitentes y adjuntos. Con la IA activada se
suma la búsqueda semántica, que encuentra por significado en vez de por palabras exactas.

<!-- claim:feat-search-2 -->
Las búsquedas se pueden acotar con operadores, solos o junto a texto libre:

| Operador | Busca por |
|---|---|
| `from:ana` | dirección o nombre del remitente |
| `to:ana` | destinatario |
| `subject:factura` | asunto |
| `before:2026-09-01` / `after:2026-09-01` | fecha de recepción |
| `id:<id del correo>` | un correo concreto |
| `tag:newsletter` / `tag:intent=request` | una etiqueta del clasificador, opcionalmente dentro de una faceta |

## Correo basura y masivo {#junk-and-bulk-mail}

<!-- claim:feat-junk-bulk-1 -->
EmailOps puntúa localmente cada mensaje entrante en busca de spam y correo masivo no deseado.
No interviene ningún modelo ni ninguna llamada de red, y tus correcciones ("es basura" / "no
es basura") entrenan el filtro con el tiempo. Tú decides qué pasa con el correo marcado:

- **Atenuarlo en la lista** — sigue ahí, solo que la vista lo salta con facilidad. <!-- claim:feat-junk-bulk-2 -->
- **Sacarlo de la bandeja** — se quita de la lista, pero sigue accesible por búsqueda y en las
  carpetas de tu proveedor. <!-- claim:feat-junk-bulk-3 -->

<!-- claim:feat-junk-bulk-4 -->
Ninguna de las dos opciones mueve ni borra nada en el servidor; solo lo hace un **Confirmar
basura** o un **Bloquear remitente** explícito. Hay un aviso opcional de suplantación/phishing, desactivado por defecto.

## Cancelar suscripciones y bloquear remitentes {#unsubscribe-and-block-sender}

<!-- claim:feat-unsubscribe-1 -->
Un boletín o un mensaje de una lista de correo que indica cómo darse de baja muestra **Cancelar
suscripción** junto a su remitente. Antes de enviar nada, una confirmación explica exactamente qué
va a pasar: una petición enviada directamente al servidor del remitente (no a través de tu
proveedor de correo), un correo de baja enviado desde tu cuenta, o la propia página del remitente
abierta en tu navegador.

<!-- claim:feat-block-sender-1 -->
**Bloquear remitente**, en el menú ⋮ de una conversación, manda a Spam el correo nuevo de ese
remitente en esta cuenta y lo notifica a tu proveedor de correo como spam. **Mover también sus
mensajes actuales a Spam** archiva lo que ya está ahí, y la conversación indica que el remitente
está bloqueado, con **Desbloquear** a mano.

<!-- claim:feat-block-sender-2 -->
**Ajustes → Basura → Remitentes bloqueados** lista a todos los que has bloqueado, con
**Desbloquear**, que además puede devolver sus mensajes de Spam a la bandeja de entrada.
**Ocultar de los filtros inteligentes**, en el menú ⋮, es otra cosa: solo quita al remitente de
los filtros inteligentes de la barra lateral.

## Notificaciones de correo nuevo {#new-mail-notifications}

<!-- claim:feat-notifications-1 -->
EmailOps muestra una notificación de escritorio cuando llega correo nuevo a tu bandeja de entrada
y cuando vuelve una conversación pospuesta — nunca en la primera sincronización de una cuenta, ni
por correo antiguo, correo que ya leíste, basura o remitentes bloqueados. Más de tres mensajes
nuevos a la vez se agrupan en un único resumen. Al pulsar una notificación, EmailOps pasa a primer
plano; no abre el mensaje.

<!-- claim:feat-notifications-2 -->
**Ajustes → Notificaciones** tiene el interruptor general, uno por cuenta, el **Contenido de la
notificación** (remitente y asunto, u **Ocultar contenido**, que solo muestra la cuenta) y **Solo
cuando EmailOps no está en primer plano**; todo viene activado por defecto, con remitente y asunto
visibles. El texto del mensaje no se muestra nunca, y mientras la app está bloqueada con la
contraseña principal tampoco el remitente ni el asunto.

## Controles de privacidad y seguridad {#privacy-and-security-controls}

<!-- claim:feat-privacy-security-1 -->
Una contraseña principal bloquea la app al arrancar, las imágenes remotas y los píxeles de
seguimiento se bloquean hasta que los permitas, y las credenciales viven en el llavero del
sistema. Todo ello se detalla en [Privacidad y seguridad](../privacy-security/).

## Interfaz {#interface}

<!-- claim:feat-interface-1 -->
Bandeja en vista dividida o a ancho completo, y una interfaz disponible en español, inglés,
francés y alemán. El idioma de salida de la IA se configura aparte, así que puedes leer la
interfaz en un idioma y que los borradores se redacten en otro.

## Atajos de teclado {#keyboard-shortcuts}

<!-- claim:feat-shortcuts-1 -->
Pulsa `?` en cualquier sitio fuera de un campo de texto para ver todos los atajos. Siguen los de
Gmail:

| Teclas | Acción |
|---|---|
| `j` / `k` | conversación siguiente / anterior |
| `Enter` u `o`, `u` | abrir la conversación, volver a la lista |
| `x` | seleccionar o deseleccionar la conversación |
| `e`, `#`, `s`, `b` | archivar, eliminar, destacar, posponer |
| `Shift+U` / `Shift+I` | marcar como no leído / leído |
| `c`, `r`, `a`, `f` | mensaje nuevo, responder, responder a todos, reenviar |
| `g` y luego `i`, `s`, `b`, `a`, `l` | ir a Bandeja de entrada, Destacados, Pospuestos, Archivo, Programados |
| `/` | buscar |

<!-- claim:feat-shortcuts-2 -->
Los atajos se pausan mientras escribes y mientras hay un diálogo o un menú abierto. Los botones a
los que corresponden indican su tecla en la ayuda emergente, como en «Archivar (E)». **Ajustes →
Apariencia → Atajos de teclado** los desactiva, y **Ver la lista** abre el mismo resumen que `?`.

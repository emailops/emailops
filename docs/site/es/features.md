---
title: 'Funciones estándar'
description: 'El cliente de correo en sí: cuentas, bandeja unificada, calendario, adjuntos, búsqueda y filtrado de correo basura.'
weight: 30
nav:
  unified-inbox: view/inbox
  calendar: view/calendar
  attachments-view: view/attachments
  junk-and-bulk-mail: settings/junk
  privacy-and-security-controls: settings/privacy
  interface: settings/appearance
---

Todo lo de esta página funciona con la IA desactivada. La capa de IA se trata aparte en
[Funciones de IA](../ai-features/).

## Cuentas y sincronización

Conecta tantos buzones como quieras — Gmail, Outlook / Microsoft 365 (API Graph) y cualquier
servidor IMAP/SMTP (iCloud, Yahoo, Fastmail, ProtonMail Bridge, autoalojado). El correo se
sincroniza en una base de datos SQLite local, así que leer y buscar es rápido y funciona sin
conexión.

El nombre de cada cuenta es el remitente que ven los destinatarios en el correo que envías
desde ella. Cámbialo en el campo **Nombre del remitente** de los ajustes de la cuenta, o déjalo
vacío para enviar solo con la dirección. Las cuentas de Gmail empiezan con el nombre de su
configuración «Enviar como» de Gmail, y las IMAP con el nombre visible que indicaste al
conectarlas. El correo enviado por Outlook lleva el nombre que Microsoft tiene para el buzón.

## Bandeja unificada {#unified-inbox}

La vista **Todas las cuentas** fusiona cada buzón activo en una sola lista, junto a las vistas
por cuenta. Las carpetas IMAP personalizadas también se sincronizan, y puedes crearlas,
renombrarlas, borrarlas y arrastrar mensajes entre ellas desde la propia app.

## Reenviar

**Reenviar** está junto a **Responder** y **Responder a todos** en el panel de lectura. El
borrador se abre sin destinatarios y lleva el mensaje original bajo una cabecera *Mensaje
reenviado* con su remitente, fecha y destinatarios, junto con los adjuntos originales (hasta
20 MB en total). Sale como un mensaje nuevo, así que no se une a las conversaciones que el
destinatario ya tiene.

## Filtros inteligentes

Acota la lista por dominio, remitente o cualquier etiqueta de clasificación — útil para
despachar un cliente, un proyecto o una avalancha de newsletters de una vez. Con la IA
activada, esas mismas etiquetas alimentan el [Tablero de etiquetas](../ai-features/#tag-board),
que las muestra como una cuadrícula de bloques.

## Calendario {#calendar}

Vistas de mes, semana y día por cuenta para Google Calendar y Outlook. Recibes recordatorios
antes de cada evento con un botón **Unirse** de un clic para enlaces de Meet, Teams, Webex y
Zoom. La sincronización de calendario está activada por defecto en las cuentas de Gmail y
Outlook y puede desactivarse por cuenta, junto con la antelación del aviso, en
**Ajustes → Calendario**.

Se sincronizan todos los calendarios de una cuenta, no solo el principal — así que un
calendario que un compañero haya compartido contigo aparece aquí igual que en Google o en
Outlook. Cada uno se tiñe con el color que le da su proveedor, y la leyenda sobre la
cuadrícula oculta o muestra calendarios individuales; los mismos interruptores están en
**Ajustes → Calendario**.

## Vista de adjuntos {#attachments-view}

Un único sitio con los adjuntos que te importan — facturas, contratos, recibos — con vista
previa y descarga, en lugar de bucear otra vez en los hilos. Ábrela desde **Adjuntos** en la
barra lateral.

La vista recopila adjuntos mediante **reglas**, así que empieza vacía. Pulsa **Gestionar
reglas** (o **Crear una regla** en la vista vacía) y rellena:

- **Nombre de la regla** — cómo aparece en la lista.
- **Patrón del remitente** — separados por coma; coincidencia exacta salvo que lleve `*`
  (`*apple.com*` coincide con cualquier remitente que contenga "apple.com"). Déjalo vacío para
  cualquier remitente.
- **Patrón del asunto** y **Patrón del nombre de archivo** — `*` es un comodín; solo se
  recopilan los nombres de archivo que coinciden.
- **Etiquetas** — separadas por coma; aparecen como botones de filtro arriba de la vista.

Tienen que coincidir todos los patrones que rellenes. Las reglas se aplican al correo nuevo
según se sincroniza; marca **Aplicar a los correos existentes después de crear** para recopilar
también del correo que ya tienes. Selecciona adjuntos para descargarlos juntos en tu carpeta de
Descargas.

## Búsqueda

Búsqueda de texto completo en asuntos, cuerpos, remitentes y adjuntos. Con la IA activada se
suma la búsqueda semántica, que encuentra por significado en vez de por palabras exactas.

## Correo basura y masivo {#junk-and-bulk-mail}

EmailOps puntúa localmente cada mensaje entrante en busca de spam y correo masivo no deseado.
No interviene ningún modelo ni ninguna llamada de red, y tus correcciones ("es basura" / "no
es basura") entrenan el filtro con el tiempo. Tú decides qué pasa con el correo marcado:

- **Atenuarlo en la lista** — sigue ahí, solo que la vista lo salta con facilidad.
- **Sacarlo de la bandeja** — se quita de la lista, pero sigue accesible por búsqueda y en las
  carpetas de tu proveedor.

Ninguna de las dos opciones mueve ni borra nada en el servidor; solo lo hace un **Confirmar
basura** explícito. Hay un aviso opcional de suplantación/phishing, desactivado por defecto.

## Controles de privacidad y seguridad {#privacy-and-security-controls}

Una contraseña principal bloquea la app al arrancar, las imágenes remotas y los píxeles de
seguimiento se bloquean hasta que los permitas, y las credenciales viven en el llavero del
sistema. Todo ello se detalla en [Privacidad y seguridad](../privacy-security/).

## Interfaz {#interface}

Bandeja en vista dividida o a ancho completo, y una interfaz disponible en español, inglés,
francés y alemán. El idioma de salida de la IA se configura aparte, así que puedes leer la
interfaz en un idioma y que los borradores se redacten en otro.

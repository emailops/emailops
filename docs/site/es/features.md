---
title: 'Funciones estándar'
description: 'El cliente de correo en sí: cuentas, bandeja unificada, calendario, adjuntos, búsqueda y filtrado de correo basura.'
weight: 30
---

Todo lo de esta página funciona con la IA desactivada. La capa de IA se trata aparte en
[Funciones de IA](../ai-features/).

## Cuentas y sincronización

Conecta tantos buzones como quieras — Gmail, Outlook / Microsoft 365 (API Graph) y cualquier
servidor IMAP/SMTP (iCloud, Yahoo, Fastmail, ProtonMail Bridge, autoalojado). El correo se
sincroniza en una base de datos SQLite local, así que leer y buscar es rápido y funciona sin
conexión.

## Bandeja unificada

La vista **Todas las cuentas** fusiona cada buzón activo en una sola lista, junto a las vistas
por cuenta. Las carpetas IMAP personalizadas también se sincronizan, y puedes crearlas,
renombrarlas, borrarlas y arrastrar mensajes entre ellas desde la propia app.

## Filtros inteligentes

Acota la lista por dominio, remitente o cualquier etiqueta de clasificación — útil para
despachar un cliente, un proyecto o una avalancha de newsletters de una vez.

## Calendario

Vistas de mes, semana y día por cuenta para Google Calendar y Outlook. Recibes recordatorios
antes de cada evento con un botón **Unirse** de un clic para enlaces de Meet, Teams, Webex y
Zoom. La sincronización de calendario está activada por defecto en las cuentas de Gmail y
Outlook y puede desactivarse por cuenta, junto con la antelación del aviso, en
**Ajustes → Calendario**.

## Vista de adjuntos

Un único sitio con todos los adjuntos de tu correo — facturas, contratos, imágenes — con
vista previa y exportación, en lugar de bucear otra vez en los hilos.

## Búsqueda

Búsqueda de texto completo en asuntos, cuerpos, remitentes y adjuntos. Con la IA activada se
suma la búsqueda semántica, que encuentra por significado en vez de por palabras exactas.

## Correo basura y masivo

EmailOps puntúa localmente cada mensaje entrante en busca de spam y correo masivo no deseado.
No interviene ningún modelo ni ninguna llamada de red, y tus correcciones ("es basura" / "no
es basura") entrenan el filtro con el tiempo. Tú decides qué pasa con el correo marcado:

- **Atenuarlo en la lista** — sigue ahí, solo que la vista lo salta con facilidad.
- **Sacarlo de la bandeja** — se quita de la lista, pero sigue accesible por búsqueda y en las
  carpetas de tu proveedor.

Ninguna de las dos opciones mueve ni borra nada en el servidor; solo lo hace un **Confirmar
basura** explícito. Hay un aviso opcional de suplantación/phishing, desactivado por defecto.

## Controles de privacidad y seguridad

Una contraseña principal bloquea la app al arrancar, las imágenes remotas y los píxeles de
seguimiento se bloquean hasta que los permitas, y las credenciales viven en el almacén de
credenciales del sistema. Todo ello se detalla en [Privacidad y seguridad](../privacy-security/).

## Interfaz

Bandeja en vista dividida o a ancho completo, y una interfaz disponible en español, inglés,
francés y alemán. El idioma de salida de la IA se configura aparte, así que puedes leer la
interfaz en un idioma y que los borradores se redacten en otro.

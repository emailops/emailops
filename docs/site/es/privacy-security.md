---
title: 'Privacidad y seguridad'
description: 'Dónde se guarda tu correo, qué sale de tu máquina y los controles que te protegen del propio correo.'
weight: 45
---

EmailOps se construye sobre una regla: tu correo se queda en tu máquina. Esta página describe
qué significa eso en concreto — dónde se escriben los datos, qué llamadas de red existen y qué
funciones de seguridad puedes activar.

## Dónde se guardan tus datos {#where-your-data-is-stored}

Todo vive en el directorio de datos de aplicación de tu sistema:

| Plataforma | Ubicación |
|---|---|
| macOS | `~/Library/Application Support/com.emailops.app` |
| Windows | `%APPDATA%\com.emailops.app` |
| Linux | `~/.local/share/com.emailops.app` |

Dentro:

- **Una base de datos SQLite** — mensajes, hilos, contactos, eventos de calendario, etiquetas
  de clasificación, embeddings de búsqueda y la memoria de la IA. Es la única copia que
  guarda EmailOps.
- **Una carpeta `models/`** — los modelos de IA que hayas descargado.

Apunta `EMAILOPS_DATA_DIR` a otro sitio antes de arrancar para usar otra ubicación — un
segundo perfil, o un volumen cifrado.

**Las credenciales no están ahí.** Los tokens OAuth y las contraseñas IMAP van al almacén de
credenciales del sistema: el Llavero en macOS, el Administrador de credenciales en Windows o un
llavero Secret Service en Linux. Nunca se escriben en un archivo de configuración y sobreviven
a la desinstalación de la app.

## No hay ningún servidor de EmailOps

No hay cuenta que crear, ni registro, ni backend operado por nosotros — así que no hay ningún
sitio al que se suba tu correo, ni nada que puedan vulnerar. La app habla exactamente con
estos destinos, todos identificables:

| Destino | Cuándo | ¿Contiene tu correo? |
|---|---|---|
| Tu proveedor de correo (Gmail, Microsoft Graph, tu servidor IMAP/SMTP) | En cada sincronización y envío | Sí — es tu buzón |
| Tu proveedor de calendario (Google, Outlook) | Sincronización de calendario, si está activada | Solo datos de calendario |
| Hugging Face | Solo mientras descargas un modelo de IA que hayas elegido | No |
| OpenRouter | Solo si cambias el proveedor de IA a él | **Sí — los prompts incluyen contenido de correo** |

La última fila es la única vía por la que tu correo puede llegar a un tercero, está desactivada
por defecto y requiere un cambio deliberado en **Ajustes → IA: backend y modelos** más tu
propia clave de API.

## Sin telemetría

La app no recopila analíticas de uso, no envía informes de fallos y no tiene ninguna llamada
a casa en las versiones publicadas. No hay opción de exclusión porque no hay nada de lo que
excluirse. (El código fuente contiene una función opcional de trazas con OpenTelemetry para
desarrollo local; queda fuera de todas las compilaciones de publicación.)

## IA local por defecto

El backend predeterminado ejecuta los modelos dentro del propio proceso mediante un runtime
llama.cpp integrado. Sin demonio, sin servidor local, sin socket de red — el modelo lee tu
correo desde el mismo proceso que ya lo tiene. Clasificación, borradores, embeddings, chat y
extracción de tareas y memoria se ejecutan ahí.

Cambiar a Ollama también mantiene la inferencia local, solo que en otro proceso de tu máquina.
Solo OpenRouter envía contenido fuera del dispositivo. Consulta
[elegir un backend](../ai-features/#choosing-a-backend).

## Protección frente al propio correo

El correo es una superficie de ataque. Las defensas del lado del cliente:

- **Bloqueo de contenido remoto** — las imágenes externas, los píxeles de seguimiento y otros
  recursos remotos se bloquean hasta que los permitas. Un aviso por correo te deja cargarlos
  una vez, o puedes confiar en un remitente concreto de forma permanente. Esto es lo que
  impide que el remitente sepa cuándo y cuántas veces abriste un mensaje.
- **Puntuación de basura y correo masivo** — cada mensaje se puntúa localmente para detectar
  spam y correo masivo no deseado. Tus correcciones ("es basura" / "no es basura") lo
  entrenan. El correo marcado se atenúa u oculta, nunca se borra ni se mueve en el servidor
  salvo que lo confirmes explícitamente.
- **Avisos de suplantación** — una comprobación opcional que señala mensajes que aparentan
  venir de quien no vienen. Desactivada por defecto, porque es la única comprobación que acusa
  a un remitente de fraude y es la que menos evidencias tiene.
- **Renderizado saneado** — al HTML de los mensajes se le quitan scripts, manejadores de
  eventos y objetos incrustados antes de mostrarlo, en ambos lados de la app. Los adjuntos
  nunca se abren por su cuenta.

## Bloquear la app

Define una **contraseña principal** en **Ajustes → Privacidad y seguridad** y EmailOps
permanecerá bloqueado al arrancar hasta que la introduzcas. No hay forma de recuperarla — si
la olvidas, reinstalas contra un directorio de datos nuevo y vuelves a sincronizar desde tu
proveedor.

Conviene ser claro sobre lo que hace: bloquea la aplicación, **no** cifra la base de datos.
Cualquiera con acceso a tu sesión de usuario desbloqueada y al directorio de datos puede leer
el archivo SQLite directamente. Si eso entra en tu modelo de amenazas, usa cifrado de disco
completo — FileVault en macOS, BitLocker en Windows, LUKS en Linux — que es la herramienta
adecuada para ello.

## Cómo auditar todo esto

EmailOps es Apache-2.0 y se desarrolla en abierto. Las afirmaciones de esta página son
verificables contra el código en
[github.com/emailops/emailops](https://github.com/emailops/emailops), y también lo es el
comportamiento de red — ejecútalo tras un proxy o con `tcpdump` y compáralo con la tabla de
arriba. Si algo no cuadra,
[abre una incidencia](https://github.com/emailops/emailops/issues).

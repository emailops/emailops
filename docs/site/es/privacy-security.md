---
title: 'Privacidad y seguridad'
description: 'Dónde se guarda tu correo, qué sale de tu máquina y los controles que te protegen del propio correo.'
weight: 45
---

<!-- claim:priv-intro-1 -->
EmailOps se construye sobre una regla: tu correo se queda en tu máquina. Esta página describe
qué significa eso en concreto — dónde se escriben los datos, qué llamadas de red existen y qué
funciones de seguridad puedes activar.

## Dónde se guardan tus datos {#where-your-data-is-stored}

<!-- claim:priv-where-data-1 -->
Todo reside en el directorio de datos de aplicación de tu sistema:

| Plataforma | Ubicación |
|---|---|
| macOS | `~/Library/Application Support/com.emailops.app` |
| Windows | `%APPDATA%\com.emailops.app` |
| Linux | `~/.local/share/com.emailops.app` |

<!-- claim:priv-where-data-2 -->
Dentro:

- **Una base de datos SQLite** — mensajes, hilos, contactos, eventos de calendario, etiquetas
  de clasificación, embeddings de búsqueda y la memoria de la IA. Es la única copia que
  guarda EmailOps. <!-- claim:priv-where-data-3 -->
- **Una carpeta `models/`** — los modelos de IA que hayas descargado. <!-- claim:priv-where-data-4 -->

<!-- claim:priv-where-data-5 -->
Apunta `EMAILOPS_DATA_DIR` a otro sitio antes de arrancar para usar otra ubicación — un
segundo perfil, o un volumen cifrado.

<!-- claim:priv-where-data-6 -->
**Las credenciales no están ahí.** Los tokens OAuth y las contraseñas IMAP van al almacén de
credenciales del sistema: el Llavero en macOS, el Administrador de credenciales en Windows o un
llavero Secret Service en Linux. Nunca se escriben en un archivo de configuración y sobreviven
a la desinstalación de la app.

## No hay ningún servidor de EmailOps

<!-- claim:priv-there-no-1 -->
No hay cuenta que crear, ni registro, ni backend operado por nosotros — así que no hay ningún
sitio al que se suba tu correo, ni nada que puedan vulnerar. La app habla exactamente con
estos destinos, todos identificables:

| Destino | Cuándo | ¿Contiene tu correo? |
|---|---|---|
| Tu proveedor de correo (Gmail, Microsoft Graph, tu servidor IMAP/SMTP) | En cada sincronización y envío | Sí — es tu buzón |
| Tu proveedor de calendario (Google, Outlook) | Sincronización de calendario, si está activada | Solo datos de calendario |
| Hugging Face | Solo mientras descargas un modelo de IA que hayas elegido | No |
| OpenRouter | Solo si cambias el proveedor de IA a él | **Sí — los prompts incluyen contenido de correo** |

<!-- claim:priv-there-no-2 -->
La última fila es la única vía por la que tu correo puede llegar a un tercero, está desactivada
por defecto y requiere un cambio deliberado en **Ajustes → IA: backend y modelos** más tu
propia clave de API.

## Qué cambia EmailOps en tu buzón

<!-- claim:priv-what-emailops-1 -->
Casi todo lo que hace EmailOps es de solo lectura: descarga tu correo y guarda una copia
local. Unas pocas acciones llegan deliberadamente hasta la cuenta, para que lo que haces aquí sea lo
que ves en todas partes:

| Acción | Efecto en la cuenta |
|---|---|
| Marcar un mensaje como leído o no leído | El mismo mensaje se marca como leído en la cuenta (Gmail) |
| Eliminar un mensaje | El mensaje se mueve a la **Papelera** de la cuenta (Gmail), donde se puede recuperar durante 30 días |
| Mover un mensaje a otra carpeta, o **Confirmar basura** | El mensaje también se mueve en la cuenta — Confirmar basura lo archiva en la carpeta de correo no deseado del proveedor |
| Crear, renombrar o eliminar una carpeta | La carpeta también cambia en la cuenta |
| Guardar un borrador | El borrador se guarda en los Borradores de la cuenta |

<!-- claim:priv-what-emailops-2 -->
EmailOps nunca borra un mensaje de forma permanente: las eliminaciones siempre van a la
papelera, nunca a un borrado definitivo. La lectura se aplica primero en local para que la app
funcione sin conexión, y la cuenta se pone al día en segundo plano. Todo lo demás — etiquetas,
filtros, carpetas que no has tocado — se queda exactamente como está.

<!-- claim:priv-what-emailops-3 -->
Cada mensaje que envías desde EmailOps termina con una breve línea «Enviado con EmailOps» que
enlaza a getemailops.com. El enlace lleva `utm_source=email_footer`, que solo indica a la
analítica de la web que la visita llegó desde el pie de un correo; no contiene nada que os
identifique ni a ti ni al destinatario.

## Sin telemetría

<!-- claim:priv-no-telemetry-1 -->
La app no recopila analíticas de uso, no envía informes de fallos y no tiene ninguna llamada
a casa en las versiones publicadas. No hay opción de exclusión porque no hay nada de lo que
excluirse. (El código fuente contiene una función opcional de trazas con OpenTelemetry para
desarrollo local; queda fuera de todas las compilaciones de publicación.)

## IA local por defecto

<!-- claim:priv-local-ai-1 -->
El backend predeterminado ejecuta los modelos dentro del propio proceso mediante un runtime
llama.cpp integrado. Sin demonio, sin servidor local, sin socket de red — el modelo lee tu
correo desde el mismo proceso que ya lo tiene. Clasificación, borradores, embeddings, chat y
extracción de tareas y memoria se ejecutan ahí.

<!-- claim:priv-local-ai-2 -->
Cambiar a Ollama también mantiene la inferencia local, solo que en otro proceso de tu máquina.
Solo OpenRouter envía contenido fuera del dispositivo. Consulta
[elegir un backend](../ai-features/#choosing-a-backend).

## Protección frente al propio correo

<!-- claim:priv-protection-from-1 -->
El correo es una superficie de ataque. Las defensas del lado del cliente:

- **Bloqueo de contenido remoto** — las imágenes externas, los píxeles de seguimiento y otros
  recursos remotos se bloquean hasta que los permitas. Un aviso por correo te deja cargarlos
  una vez, o puedes confiar en un remitente concreto de forma permanente. Esto es lo que
  impide que el remitente sepa cuándo y cuántas veces abriste un mensaje. <!-- claim:priv-protection-from-2 -->
- **Puntuación de basura y correo masivo** — cada mensaje se puntúa localmente para detectar
  spam y correo masivo no deseado. Tus correcciones ("es basura" / "no es basura") lo
  entrenan. El correo marcado se atenúa u oculta, nunca se borra ni se mueve en el servidor
  salvo que lo confirmes explícitamente. <!-- claim:priv-protection-from-3 -->
- **Avisos de suplantación** — una comprobación opcional que señala mensajes que aparentan
  venir de quien no vienen. Desactivada por defecto, porque es la única comprobación que acusa
  a un remitente de fraude y es la que menos evidencias tiene. <!-- claim:priv-protection-from-4 -->
- **Renderizado saneado** — al HTML de los mensajes se le quitan scripts, manejadores de
  eventos y objetos incrustados antes de mostrarlo, en ambos lados de la app. Los adjuntos
  nunca se abren por su cuenta. <!-- claim:priv-protection-from-5 -->

## Bloquear la app

<!-- claim:priv-locking-app-1 -->
Define una **contraseña principal** en **Ajustes → Privacidad y seguridad** y EmailOps
permanecerá bloqueado al arrancar hasta que la introduzcas. No hay forma de recuperarla — si
la olvidas, reinstalas contra un directorio de datos nuevo y vuelves a sincronizar desde tu
proveedor.

<!-- claim:priv-locking-app-2 -->
Conviene ser claro sobre lo que hace: bloquea la aplicación, **no** cifra la base de datos.
Cualquiera con acceso a tu sesión de usuario desbloqueada y al directorio de datos puede leer
el archivo SQLite directamente. Si eso entra en tu modelo de amenazas, usa cifrado de disco
completo — FileVault en macOS, BitLocker en Windows, LUKS en Linux — que es la herramienta
adecuada para ello.

## Cómo auditar todo esto

<!-- claim:priv-auditing-any-1 -->
EmailOps es Apache-2.0 y se desarrolla en abierto. Las afirmaciones de esta página son
verificables contra el código en
[github.com/emailops/emailops](https://github.com/emailops/emailops), y también lo es el
comportamiento de red — ejecútalo tras un proxy o con `tcpdump` y compáralo con la tabla de
arriba. Si algo no cuadra,
[abre una incidencia](https://github.com/emailops/emailops/issues).

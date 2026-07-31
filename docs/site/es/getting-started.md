---
title: 'Primeros pasos'
description: 'El asistente inicial: elige un backend de IA, descarga un modelo y conecta tu primer buzón.'
weight: 20
---

La primera vez que abres EmailOps se ejecuta un asistente de cuatro pasos. Lleva un par de
minutos, la mayor parte de ellos descargando un modelo en segundo plano.

## 1. IA sí o no

EmailOps analiza tu hardware y recomienda si activar la IA local. Elige:

- **IA activada** — chat, borradores, clasificación y búsqueda semántica se ejecutan en esta
  máquina.
- **Cliente de correo sin más** — no se descarga ningún modelo ni se hace ninguna llamada de
  IA. Puedes activar la IA más tarde en **Ajustes → IA: backend y modelos**, y desactivarla
  con la misma facilidad.

## 2. Backend y modelo de IA

Si activaste la IA, elige dónde se ejecuta la inferencia:

| Backend | Qué significa |
|---|---|
| **En la app (local)** | El predeterminado. Un runtime llama.cpp integrado en EmailOps. Sin demonio, sin configuración, sin red. |
| **Ollama (local)** | Usa tu servidor Ollama existente en `http://localhost:11434`. |
| **OpenRouter (remoto)** | Envía los prompts a una API de pago en la nube. Opcional, por función, y desactivado por defecto. |

Con el backend integrado, elige un modelo de chat del catálogo. **Qwen 3.5 4B** es el
predeterminado recomendado: unos 3 GB de descarga, necesita aproximadamente 8 GB de memoria
para ejecutarse y admite las llamadas a herramientas de las que depende el chat. Los modelos
demasiado grandes para la memoria de tu sistema aparecen atenuados. La descarga corre en
segundo plano — puedes seguir con el asistente.

La memoria que cuenta depende de la máquina: **memoria unificada** en un Mac con Apple
Silicon, la **VRAM de tu GPU** en un equipo Windows o Linux con tarjeta dedicada, y la RAM del
sistema si no hay GPU. El [catálogo de modelos](../ai-features/#the-model-catalog) indica la
cifra de cada modelo.

El modelo de embeddings que impulsa la búsqueda semántica (**Nomic Embed Text v1.5**, ~80 MB)
viene incluido dentro de la app en macOS, así que no hay nada que descargar para la búsqueda.

## 3. Diseño de la bandeja

Elige cómo se distribuye el buzón — **dividido** (lista a la izquierda, mensaje a la derecha)
o **ancho completo** (un panel cada vez). Puedes cambiarlo cuando quieras en
**Ajustes → Apariencia**, junto con el idioma de la interfaz (español, inglés, francés,
alemán).

## 4. Conectar una cuenta

El último paso añade tu primer buzón. EmailOps admite:

- **Gmail** — inicia sesión en el navegador y concede el acceso. Los tokens van directos al
  llavero del sistema.
- **Outlook / Microsoft 365** — el mismo flujo por navegador, vía la API Microsoft Graph.
- **IMAP / SMTP** — iCloud, Yahoo, Fastmail, ProtonMail Bridge o cualquier servidor
  personalizado. Introduce los datos del servidor y las credenciales directamente.

Añade más cuentas cuando quieras con **Añadir cuenta** en la barra lateral. Con varias conectadas obtienes
una bandeja unificada "Todas las cuentas" además de las vistas por cuenta.

## Después del asistente

### La primera sincronización tarda

EmailOps descarga tu correo a una base de datos local, y la primera pasada tiene que traerlo
todo desde cero. Cuánto tarda depende del tamaño del buzón — unos minutos en una cuenta
pequeña, bastante más en una con años de historial y adjuntos pesados. Se ejecuta en segundo
plano y puedes leer y buscar lo que ya ha llegado mientras el resto se pone al día.

Es un coste único. Cada sincronización posterior es **incremental**: solo pide a tu proveedor
lo que ha cambiado desde la última vez, así que termina en segundos y se ejecuta discretamente
según su programación. Si la IA está activada, la clasificación y los embeddings también
procesan el atraso en la primera ejecución y después solo tocan el correo nuevo.

Cuando termine la primera sincronización:

1. La **clasificación** empieza a etiquetar el correo nuevo por prioridad, intención y tema —
   consulta [Funciones de IA](../ai-features/#classification).
2. Los **embeddings** se generan en segundo plano para que la búsqueda semántica tenga algo
   sobre lo que buscar. Puedes ver el progreso y reconstruir el índice en
   **Ajustes → Búsqueda con IA**.
3. Plantéate poner una **contraseña principal** en **Ajustes → Privacidad y seguridad** si
   quieres que la app se bloquee al arrancar — consulta
   [Privacidad y seguridad](../privacy-security/).

Tanto la clasificación como los embeddings respetan **Limitar el procesado de IA a
correos recientes**
(**Ajustes → IA: backend y modelos**), así que un archivo de hace una década no se procesa a
menos que lo pidas.

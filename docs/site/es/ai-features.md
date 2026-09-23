---
title: 'Funciones de IA'
description: 'Chatea con tu buzón, genera respuestas, clasifica correo, extrae tareas — todo con un modelo que tú controlas.'
weight: 40
nav:
  choosing-a-backend: settings/ai
  the-model-catalog: settings/ai
  performance-knobs: settings/ai
  chat-with-your-mailbox: view/chat
  ai-drafts: settings/aidrafts
  classification: settings/classification
  tag-board: view/tagboard
  semantic-search: settings/aisearch
  translation: settings/aitranslation
  tasks: settings/tasks
  memory: settings/memory
  lenses: settings/lenses
  turning-it-all-off: settings/ai
---

<!-- claim:ai-intro-1 -->
Todas las funciones de IA de abajo se ejecutan mediante el backend que hayas elegido, y cada
una puede desactivarse por separado. Con el backend integrado predeterminado, ningún prompt ni
ningún correo sale nunca de tu máquina.

## Elegir un backend {#choosing-a-backend}

<!-- claim:ai-choosing-backend-1 -->
**Ajustes → IA: backend y modelos** controla dónde ocurre la inferencia:

- **En la app** — un runtime llama.cpp integrado. Nada que instalar y sin demonio; una vez
  descargado el modelo, responder no genera tráfico de red. Es el predeterminado. Usa tu GPU automáticamente cuando la hay — Metal en
  Apple Silicon, Vulkan en Windows y Linux — y la CPU cuando no. En Mac requiere Apple Silicon
  (M1 o posterior); en un Mac Intel permanece no disponible. <!-- claim:ai-choosing-backend-2 -->
- **Ollama** — un servidor Ollama que ya tengas en `http://localhost:11434`. Útil si
  mantienes una biblioteca de modelos compartida. Ten en cuenta que en un Mac Intel tampoco
  obtiene aceleración por GPU, así que será lento. <!-- claim:ai-choosing-backend-3 -->
- **OpenRouter** — una API de pago en la nube. Requiere una clave de API, admite un
  tope de gasto mensual y envía el contenido del correo a un tercero — así que permanece
  desactivado salvo que lo actives. Su panel muestra lo gastado en el periodo actual frente a
  ese tope, con **Iniciar un periodo nuevo** para poner la cuenta a cero. <!-- claim:ai-choosing-backend-4 -->

### El catálogo de modelos {#the-model-catalog}

<!-- claim:ai-choosing-backend-model-catalog-1 -->
El backend integrado descarga modelos de un catálogo curado, cada uno fijado a un checksum
verificado:

<!-- generated:model-catalog -->
| Modelo | Tamaño de descarga | Memoria que pide EmailOps |
|---|---|---|
| Qwen 3.5 4B | ~3,0 GB | 8 GB |
| Qwen 3.5 4B Q8 | ~4,6 GB | 12 GB |
| Qwen 3.5 9B | ~5,7 GB | 16 GB |
| Gemma 4 12B Instruct | ~6,7 GB | 16 GB |
| Qwen 3.5 27B | ~17,6 GB | 24 GB |
| Qwen 3.6 35B A3B | ~22,4 GB | 32 GB |
| Nomic Embed Text v1.5 *(embeddings, incluido)* | ~84 MB | 1 GB |
<!-- /generated:model-catalog -->

<!-- claim:ai-choosing-backend-model-catalog-2 -->
La columna de la derecha es la memoria que EmailOps pide antes de ofrecer un modelo — un
margen holgado a propósito, no lo que el modelo consume. El pico medido durante la respuesta,
con el contexto que recibe un Mac de 16 GB: unos 3,7 GB para Qwen 3.5 4B, 4,3 GB para su
versión de 8 bits, 5,6 GB para Qwen 3.5 9B y 7,1 GB para Gemma 4 12B. **En qué** memoria debe
caber depende de tu hardware:

- **Apple Silicon** — memoria unificada, compartida entre CPU y GPU, a través de Metal.
  Compara la cifra con la memoria total de tu Mac. <!-- claim:ai-choosing-backend-model-catalog-3 -->
- **Una GPU en Windows o Linux** — la **VRAM** de la tarjeta, no la RAM del sistema, a través
  de Vulkan. Una tarjeta de 8 GB ejecuta la fila de 8 GB y nada por encima, por mucha RAM que
  tenga la máquina. <!-- claim:ai-choosing-backend-model-catalog-4 -->
- **Sin GPU** — la RAM del sistema, en la CPU. Funciona; solo que más lento. <!-- claim:ai-choosing-backend-model-catalog-5 -->

<!-- claim:ai-choosing-backend-model-catalog-6 -->
Un modelo lleva la etiqueta **Recomendado**, elegida para la máquina en la que estás: EmailOps
mira la memoria del sistema y, si tienes una tarjeta gráfica dedicada, también su memoria, y
sugiere el modelo más grande que quepa con holgura. Por eso un portátil y una estación de
trabajo verán sugerencias distintas. Los modelos más grandes responden mejor y van más lentos,
así que la etiqueta es un punto de partida, no una regla. Los requisitos completos están en
[Instalación](../installation/#with-local-ai).

### Ajustes de rendimiento {#performance-knobs}

- **Mantener el modelo cargado** — cuánto tiempo permanece el modelo en memoria entre turnos
  (30 minutos por defecto). Valores más altos evitan la recarga lenta; `0` lo descarga de
  inmediato y libera la memoria para otras apps. <!-- claim:ai-choosing-backend-performance-knobs-1 -->
- **Ventana de contexto** — cuántos tokens puede atender el modelo por turno. Más grande cabe
  más correo recuperado y cuesta más memoria — es lo primero que conviene bajar cuando un
  modelo entra justo. <!-- claim:ai-choosing-backend-performance-knobs-2 -->
- **Modo de razonamiento** — chain-of-thought en los modelos compatibles. Más lento, más
  preciso, y puedes mostrar u ocultar la traza. <!-- claim:ai-choosing-backend-performance-knobs-3 -->
- **Limitar el procesado de IA** — acota lo que cubren los embeddings y la clasificación:
  todos los correos de una cuenta hasta un límite de correos (1000 por defecto) y, en cuentas
  mayores, solo el correo más reciente que un límite de días (365 por defecto). <!-- claim:ai-choosing-backend-performance-knobs-4 -->

## Chatea con tu buzón {#chat-with-your-mailbox}

<!-- claim:ai-chat-mailbox-1 -->
Pregunta en lenguaje natural — *"¿qué dijo el abogado sobre el contrato?"*, *"resume este
hilo"*, *"¿quién me debe todavía una respuesta?"* — y obtén una respuesta con los correos de
origen citados. Las respuestas llegan en streaming según se generan. **Ver en la lista de
correos**, bajo una respuesta, pone en la lista exactamente los correos que cita, para que
puedas abrirlos y trabajarlos.

<!-- claim:ai-chat-mailbox-2 -->
El chat vive en un panel redimensionable acoplado a la derecha de la bandeja, así que puedes
seguir leyendo mientras preguntas; también hay una vista a pantalla completa para sesiones
más largas. Con un correo abierto, el panel ofrece ese hilo como contexto mediante un chip
que puedes quitar: las preguntas sobre ese correo se responden desde el hilo, y una pregunta
sobre el resto del buzón (*"¿qué me ha llegado hoy?"*) sigue buscando en él. Ese contexto se
aplica a una sola pregunta y nunca se guarda en la conversación, así que puedes moverte entre
correos dentro de un mismo chat.

<!-- claim:ai-chat-mailbox-3 -->
El chat busca en una cuenta cada vez, y un selector indica cuál — de modo que una respuesta
nunca sale en silencio del buzón equivocado. Cada cuenta mantiene su propia conversación
mientras la aplicación siga abierta, así que cambiar de cuenta te devuelve donde lo dejaste
y no a un chat en blanco.

<!-- claim:ai-chat-mailbox-10 -->
El chat también responde preguntas sobre el propio EmailOps — *"¿cómo conecto Ollama?"*,
*"¿dónde se guardan mis datos?"*, *"¿qué muestra el Tablero de etiquetas?"* — a partir de estas guías, en
tu idioma y sin buscar en tu buzón. La respuesta enlaza la sección de la guía que ha usado, y
al seguir el enlace se abre el ajuste o la vista correspondiente. Con un correo abierto como
contexto, el chat responde solo desde ese hilo, así que quita el chip para preguntar por la
app. Puedes desactivarlo con **Responder preguntas sobre EmailOps** en
**Ajustes → IA: backend y modelos**; entonces el chat solo conoce tu buzón.

<!-- claim:ai-chat-mailbox-11 -->
Cada respuesta tiene un panel **Mostrar razonamiento** que enumera lo que ha pasado, en
orden: qué ruta ha seguido la pregunta y qué lo ha decidido, el planificador de consultas, la
búsqueda en el buzón, las secciones de las guías usadas, cada llamada al modelo con sus
tiempos y cada llamada a herramientas con sus argumentos y su resultado.

<!-- claim:ai-chat-mailbox-4 -->
Por dentro, el chat combina recuperación (búsqueda semántica sobre tu correo indexado) con
llamadas a herramientas (consultas directas a la base de datos). El modo de enrutado es
configurable:

- **Siempre RAG primero** — el predeterminado; recupera contexto y luego responde. <!-- claim:ai-chat-mailbox-5 -->
- **Auto** — una heurística decide en cada pregunta si recupera contexto antes. <!-- claim:ai-chat-mailbox-6 -->
- **Siempre herramientas primero** — se salta la recuperación y empieza por las consultas
  estructuradas. <!-- claim:ai-chat-mailbox-7 -->

<!-- claim:ai-chat-mailbox-8 -->
En todos los modos las herramientas siguen disponibles; el modo solo decide si se recupera
contexto antes de responder.

<!-- claim:ai-chat-mailbox-9 -->
Los usuarios avanzados pueden editar el prompt del sistema y los prompts de recuperación
(reescritura de consulta, reordenación) en
**Ajustes → IA: backend y modelos → Prompts del chat**.

## Borradores con IA {#ai-drafts}

<!-- claim:ai-ai-drafts-1 -->
Un botón **Borrador con IA** junto a Responder a todos redacta una respuesta basada en el hilo
que estás viendo. Configura una **persona** (una frase sobre quién escribe) y un **estilo de
escritura** — o sustituye toda la plantilla del prompt.
Los borradores aterrizan en el editor para que los revises antes de enviar nada.

## Clasificación {#classification}

<!-- claim:ai-classification-1 -->
Cada correo entrante se etiqueta en tres ejes — **prioridad**, **intención** y **tema** — de
modo que la bandeja se ordena prácticamente sola y los filtros inteligentes tienen algo por lo
que filtrar.

<!-- claim:ai-classification-2 -->
La clasificación funciona en dos capas:

1. Las **Reglas** casan patrones de remitente o asunto (`*@*.beehiiv.com`, `*factura*`) y
   asignan etiquetas al instante, sin llamar al modelo. <!-- claim:ai-classification-3 -->
2. **El modelo** se ocupa de todo lo que las reglas no cubren, con un prompt de instrucciones
   que puedes editar. <!-- claim:ai-classification-4 -->

<!-- claim:ai-classification-5 -->
Tú controlas qué categorías de Gmail se clasifican, puedes reclasificar todo tras cambiar el
prompt y puedes ponerte al día con el correo sin clasificar cuando quieras.

## Tablero de etiquetas {#tag-board}

<!-- claim:tag-board-dimensions -->
El **Tablero de etiquetas** (en **Vistas**, en el menú lateral, junto a la bandeja de
entrada) convierte esas etiquetas en un tablero. Elige una dimensión — **Empresa**,
**Prioridad**, **Intención** o **Tema** — y cada valor de etiqueta se convierte en un bloque
con sus hilos; en **Todas las cuentas** hay un bloque por cuenta y etiqueta. Cada hilo está
en un único bloque, bajo la etiqueta de su mensaje clasificado más reciente.

<!-- claim:ai-tag-board-2 -->
Los bloques se ordenan por la atención que recibe realmente cada etiqueta — cuánto respondes
y lees sus hilos, con más peso para la actividad reciente — y las promociones y
notificaciones quedan al final. Los filtros inteligentes del menú lateral siguen el mismo
orden. Arrastra los bloques para reordenarlos (el orden se recuerda por dimensión), oculta
una etiqueta desde su menú ⋮ — la siguiente etiqueta sube a ocupar su sitio, y el filtro
desaparece también del menú lateral — y recupera las ocultas con el enlace **Mostrar
etiquetas ocultas**.

<!-- claim:tag-board-toolbar -->
La barra superior acota el tablero por periodo (**Hoy**, **Ayer**, **Últimos 7 días** o un
rango de fechas personalizado), por categoría de Gmail, por nombre de etiqueta y con el mismo
interruptor **Ocultar mensajes de basura** que la bandeja; dos iconos fijan el ancho de los
bloques. Al pulsar una tarjeta el hilo se abre en el panel de lectura, su menú ⋮ ofrece las
mismas acciones que una fila de la bandeja, y el icono de chat del panel de lectura inicia
una conversación con ese hilo como contexto.

<!-- claim:ai-tag-board-4 -->
El tablero necesita la clasificación: está vacío hasta que el correo tiene etiquetas y no se
muestra con las funciones de IA desactivadas.

## Búsqueda semántica {#semantic-search}

<!-- claim:ai-semantic-search-1 -->
Los correos se indexan localmente para que la búsqueda case por significado y no solo por
palabras clave — describe lo que recuerdas y EmailOps lo encuentra. Esto también impulsa el paso de recuperación del chat. Elige qué categorías se indexan y
reconstruye el índice desde cero tras cambiar el modelo de embeddings, en
**Ajustes → Búsqueda con IA**.

## Traducción {#translation}

<!-- claim:ai-translation-1 -->
Aparecen botones de traducción en los correos escritos en otro idioma y en la ventana de
redacción. El prompt de traducción es editable como los demás.

## Tareas {#tasks}

<!-- claim:ai-tasks-1 -->
*Experimental.* EmailOps revisa el correo en busca de acciones, compromisos y fechas límite y
los reúne en un panel de Tareas. Como los compromisos reales suelen estar en lo que **tú**
escribiste, existe un modo "aprender solo de los correos que he escrito". Puedes excluir
remitentes y etiquetas (las newsletters se excluyen por defecto), limitar las tareas por
correo, acotar hasta dónde llega la extracción hacia atrás y procesar correo antiguo bajo
demanda.

## Memoria {#memory}

<!-- claim:ai-memory-1 -->
*Experimental.* Los hechos que el asistente aprende sobre tus contactos, dominios y proyectos
se guardan como contexto a largo plazo, para que el chat no empiece de cero cada vez. Los
hechos candidatos se puntúan y se promocionan al superar un umbral; los de baja puntuación
caducan. Todo lo aprendido es inspeccionable, y el subsistema entero tiene un interruptor
general.

## Lentes {#lenses}

<!-- claim:ai-lenses-1 -->
*Experimental.* Vistas tipadas sobre tu buzón — proyecciones estructuradas, guardadas y
extraídas por IA (piensa en "todas las facturas con importe y vencimiento") que creas y
ejecutas desde la barra lateral. Una fila que excluyes desaparece de la vista; **Ver filas
excluidas** las vuelve a mostrar para que puedas incluir alguna de nuevo.

## Apagarlo todo {#turning-it-all-off}

<!-- claim:ai-turning-off-1 -->
**Ajustes → IA: backend y modelos → Funciones de IA** es un interruptor general.
Desactívalo y EmailOps funciona como un cliente de correo normal: sin chat, sin clasificación,
sin embeddings, sin ningún modelo cargado. Tus datos locales de IA se conservan por si vuelves
a activarlo.

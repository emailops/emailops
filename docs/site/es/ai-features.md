---
title: 'Funciones de IA'
description: 'Chatea con tu buzón, genera respuestas, clasifica correo, extrae tareas — todo con un modelo que tú controlas.'
weight: 40
---

Todas las funciones de IA de abajo se ejecutan mediante el backend que hayas elegido, y cada
una puede desactivarse por separado. Con el backend integrado predeterminado, ningún prompt ni
ningún correo sale nunca de tu máquina.

## Elegir un backend {#choosing-a-backend}

**Ajustes → IA: backend y modelos** controla dónde ocurre la inferencia:

- **En la app (local)** — un runtime llama.cpp integrado. Nada que instalar, sin demonio, sin
  tráfico de red. Es el predeterminado. Usa tu GPU automáticamente cuando la hay — Metal en
  Apple Silicon, Vulkan en Windows y Linux — y la CPU cuando no. En Mac requiere Apple Silicon
  (M1 o posterior); en un Mac Intel permanece no disponible.
- **Ollama (local)** — un servidor Ollama que ya tengas en `http://localhost:11434`. Útil si
  mantienes una biblioteca de modelos compartida. Ten en cuenta que en un Mac Intel tampoco
  obtiene aceleración por GPU, así que será lento.
- **OpenRouter (remoto)** — una API de pago en la nube. Requiere una clave de API, admite un
  tope de gasto mensual y envía el contenido del correo a un tercero — así que permanece
  desactivado salvo que lo actives.

### El catálogo de modelos {#the-model-catalog}

El backend integrado descarga modelos de un catálogo curado, cada uno fijado a un checksum
verificado:

| Modelo | Tamaño de descarga | Memoria necesaria para ejecutarlo |
|---|---|---|
| Qwen 3.5 4B | ~3,0 GB | 8 GB |
| Qwen 3.5 4B Q8 | ~4,6 GB | 12 GB |
| Qwen 3.5 9B | ~5,7 GB | 16 GB |
| Gemma 4 12B Instruct | ~6,7 GB | 16 GB |
| Qwen 3.5 27B | ~17,6 GB | 24 GB |
| Qwen 3.6 35B A3B | ~22,4 GB | 32 GB |
| Nomic Embed Text v1.5 *(embeddings, incluido)* | ~84 MB | 1 GB |

La columna de la derecha es la memoria máxima durante la respuesta — pesos más la ventana de
contexto — que siempre es mayor que la descarga. **En qué** memoria debe caber depende de tu
hardware:

- **Apple Silicon** — memoria unificada, compartida entre CPU y GPU, a través de Metal.
  Compara la cifra con la memoria total de tu Mac.
- **Una GPU en Windows o Linux** — la **VRAM** de la tarjeta, no la RAM del sistema, a través
  de Vulkan. Una tarjeta de 8 GB ejecuta la fila de 8 GB y nada por encima, por mucha RAM que
  tenga la máquina.
- **Sin GPU** — la RAM del sistema, en la CPU. Funciona; solo que más lento.

Los modelos demasiado grandes para la memoria de tu sistema aparecen atenuados en el selector.
Un modelo lleva la etiqueta **Recomendado**, elegida para la máquina en la que estás: EmailOps
mira la memoria del sistema y, si tienes una tarjeta gráfica dedicada, también su memoria, y
sugiere el modelo más grande que quepa con holgura. Por eso un portátil y una estación de
trabajo verán sugerencias distintas. Los modelos más grandes responden mejor y van más lentos,
así que la etiqueta es un punto de partida, no una regla. Los requisitos completos están en
[Instalación](../installation/#with-local-ai).

### Ajustes de rendimiento

- **Mantener el modelo cargado** — cuánto tiempo permanece el modelo en memoria entre turnos
  (30 minutos por defecto). Valores más altos evitan la recarga lenta; `0` lo descarga de
  inmediato y libera la memoria para otras apps.
- **Ventana de contexto** — cuántos tokens puede atender el modelo por turno. Más grande cabe
  más correo recuperado y cuesta más memoria — es lo primero que conviene bajar cuando un
  modelo entra justo.
- **Modo de razonamiento** — chain-of-thought en los modelos compatibles. Más lento, más
  preciso, y puedes mostrar u ocultar la traza.
- **Limitar el procesado de IA a correos recientes** — omite embeddings y clasificación
  para el correo con más de N días.

## Chatea con tu buzón

Pregunta en lenguaje natural — *"¿qué dijo el abogado sobre el contrato?"*, *"resume este
hilo"*, *"¿quién me debe todavía una respuesta?"* — y obtén una respuesta con los correos de
origen citados. Las respuestas llegan en streaming según se generan.

El chat vive en un panel redimensionable acoplado a la derecha de la bandeja, así que puedes
seguir leyendo mientras preguntas; también hay una vista a pantalla completa para sesiones
más largas. Con un correo abierto, el panel ofrece ese hilo como contexto mediante un chip
que puedes quitar: las preguntas sobre ese correo se responden desde el hilo, y una pregunta
sobre el resto del buzón (*"¿qué me ha llegado hoy?"*) sigue buscando en él. Ese contexto se
aplica a una sola pregunta y nunca se guarda en la conversación, así que puedes moverte entre
correos dentro de un mismo chat.

El chat busca en una cuenta cada vez, y un selector indica cuál — de modo que una respuesta
nunca sale en silencio del buzón equivocado. Cada cuenta mantiene su propia conversación
mientras la aplicación siga abierta, así que cambiar de cuenta te devuelve donde lo dejaste
y no a un chat en blanco.

Por dentro, el chat combina recuperación (búsqueda semántica sobre tu correo indexado) con
llamadas a herramientas (consultas directas a la base de datos). El modo de enrutado es
configurable:

- **Siempre RAG primero** — el predeterminado; recupera contexto y luego responde.
- **Auto** — una heurística decide en cada pregunta si recupera contexto antes.
- **Siempre herramientas primero** — se salta la recuperación y empieza por las consultas
  estructuradas.

En todos los modos las herramientas siguen disponibles; el modo solo decide si se recupera
contexto antes de responder.

Los usuarios avanzados pueden editar el prompt del sistema y los prompts de recuperación
(reescritura de consulta, reordenación) en
**Ajustes → IA: backend y modelos → Prompts del chat**.

## Borradores con IA

Un botón **Borrador con IA** junto a Responder a todos redacta una respuesta basada en el hilo
que estás viendo. Configura una **persona** (una frase sobre quién escribe), un **estilo de
escritura** y el tono y la longitud por defecto — o sustituye toda la plantilla del prompt.
Los borradores aterrizan en el editor para que los revises antes de enviar nada.

## Clasificación {#classification}

Cada correo entrante se etiqueta en tres ejes — **prioridad**, **intención** y **tema** — de
modo que la bandeja se ordena prácticamente sola y los filtros inteligentes tienen algo por lo
que filtrar.

La clasificación funciona en dos capas:

1. Las **Reglas** casan patrones de remitente o asunto (`*@*.beehiiv.com`, `*factura*`) y
   asignan etiquetas al instante, sin llamar al modelo.
2. **El modelo** se ocupa de todo lo que las reglas no cubren, con un prompt de instrucciones
   que puedes editar.

Tú controlas qué categorías de Gmail se clasifican, puedes reclasificar todo tras cambiar el
prompt y puedes ponerte al día con el correo sin clasificar cuando quieras.

## Tablero de etiquetas {#tag-board}

El **Tablero de etiquetas** (en **Vistas**, en el menú lateral, junto a la bandeja de
entrada) convierte esas etiquetas en un tablero. Elige una dimensión — **Empresa**,
**Prioridad**, **Intención** o **Tema** — y cada valor de etiqueta se convierte en un bloque
con sus hilos; en **Todas las cuentas** hay un bloque por cuenta y etiqueta. Cada hilo está
en un único bloque, bajo la etiqueta de su mensaje clasificado más reciente.

Los bloques se ordenan por la atención que recibe realmente cada etiqueta — cuánto respondes
y lees sus hilos, con más peso para la actividad reciente — y las promociones y
notificaciones quedan al final. Los filtros inteligentes del menú lateral siguen el mismo
orden. Arrastra los bloques para reordenarlos (el orden se recuerda por dimensión), oculta
una etiqueta desde su menú ⋮ — la siguiente etiqueta sube a ocupar su sitio, y el filtro
desaparece también del menú lateral — y recupera las ocultas con el enlace **Mostrar
etiquetas ocultas**.

La barra superior acota el tablero por periodo (**Hoy**, **Ayer**, **Últimos 7 días** o un
rango de fechas personalizado), por categoría de Gmail, por nombre de etiqueta y con el mismo
interruptor **Ocultar mensajes de basura** que la bandeja; dos iconos fijan el ancho de los
bloques. Al pulsar una tarjeta el hilo se abre en el panel de lectura, su menú ⋮ ofrece las
mismas acciones que una fila de la bandeja, y el icono de chat del panel de lectura inicia
una conversación con ese hilo como contexto.

El tablero necesita la clasificación: está vacío hasta que el correo tiene etiquetas y no se
muestra con las funciones de IA desactivadas.

## Búsqueda semántica

Los correos se indexan localmente para que la búsqueda case por significado y no solo por
palabras clave — describe lo que recuerdas y EmailOps lo encuentra. Esto también impulsa
"buscar similares" y el paso de recuperación del chat. Elige qué categorías se indexan y
reconstruye el índice desde cero tras cambiar el modelo de embeddings, en
**Ajustes → Búsqueda con IA**.

## Traducción

Aparecen botones de traducción en los correos escritos en otro idioma y en la ventana de
redacción. El prompt de traducción es editable como los demás.

## Tareas

*Experimental.* EmailOps revisa el correo en busca de acciones, compromisos y fechas límite y
los reúne en un panel de Tareas. Como los compromisos reales suelen estar en lo que **tú**
escribiste, existe un modo "aprender solo de los correos que he escrito". Puedes excluir
remitentes y etiquetas (las newsletters se excluyen por defecto), limitar las tareas por
correo, acotar hasta dónde llega la extracción hacia atrás y procesar correo antiguo bajo
demanda.

## Memoria

*Experimental.* Los hechos que el asistente aprende sobre tus contactos, dominios y proyectos
se guardan como contexto a largo plazo, para que el chat no empiece de cero cada vez. Los
hechos candidatos se puntúan y se promocionan al superar un umbral; los de baja puntuación
caducan. Todo lo aprendido es inspeccionable, y el subsistema entero tiene un interruptor
general.

## Lentes

*Experimental.* Vistas tipadas sobre tu buzón — proyecciones estructuradas, guardadas y
extraídas por IA (piensa en "todas las facturas con importe y vencimiento") que creas y
ejecutas desde la barra lateral.

## Apagarlo todo

**Ajustes → IA: backend y modelos → Funciones de IA** es un interruptor general.
Desactívalo y EmailOps funciona como un cliente de correo normal: sin chat, sin clasificación,
sin embeddings, sin ningún modelo cargado. Tus datos locales de IA se conservan por si vuelves
a activarlo.

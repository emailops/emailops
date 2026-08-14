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
| Qwen 3.5 4B *(recomendado)* | ~3,0 GB | 8 GB |
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
Los modelos más grandes responden mejor y van más lentos — empieza por el recomendado y sube
solo si al hardware le sobra margen. Los requisitos completos están en
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
que puedes quitar, y responde desde el propio hilo en lugar de buscar. Ese contexto se aplica
a una sola pregunta y nunca se guarda en la conversación, así que puedes moverte entre
correos dentro de un mismo chat.

El chat busca en una cuenta cada vez, y un selector indica cuál — de modo que una respuesta
nunca sale en silencio del buzón equivocado. Cada cuenta mantiene su propia conversación
mientras la aplicación siga abierta, así que cambiar de cuenta te devuelve donde lo dejaste
y no a un chat en blanco.

Por dentro, el chat combina recuperación (búsqueda semántica sobre tu correo indexado) con
llamadas a herramientas (consultas directas a la base de datos). El modo de enrutado es
configurable:

- **Siempre RAG primero** — el predeterminado; recupera contexto y luego responde.
- **Auto** — una heurística elige recuperación o herramientas según la pregunta.
- **Siempre herramientas primero** — va directo a las consultas estructuradas.

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

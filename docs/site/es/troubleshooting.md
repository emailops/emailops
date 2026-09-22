---
title: 'Resolución de problemas'
description: 'Soluciones a lo que más se encuentra la gente: IA no disponible, chat lento, búsqueda solo por palabras clave, errores de sincronización.'
weight: 60
---

## Las funciones de IA no están disponibles

<!-- claim:trbl-ai-features-1 -->
Con el backend **integrado**, comprueba que el modelo recomendado terminó de descargarse en
**Ajustes → IA: backend y modelos**. Una descarga interrumpida deja el modelo inservible —
bórralo y descárgalo otra vez.

<!-- claim:trbl-ai-features-2 -->
Si cambiaste a **Ollama**, asegúrate de que el demonio está en marcha y accesible en
`http://localhost:11434`, y de que has descargado un modelo:

```bash
ollama pull llama3.2
ollama pull nomic-embed-text
```

<!-- claim:trbl-ai-features-3 -->
En un **Mac Intel** la IA integrada no puede ejecutarse: necesita un chip Apple Silicon (M1 o
posterior), así que EmailOps la mantiene desactivada. Usa OpenRouter en su lugar. Ollama se
instala, pero en Intel tampoco obtiene aceleración por GPU, así que resultará demasiado lento.

## El chat va lento

<!-- claim:trbl-chat-slow-1 -->
La inferencia local lleva su tiempo — en una máquina modesta, una respuesta de chat puede
tardar decenas de segundos. Lo que ayuda, más o menos por orden de efecto:

1. **Comprueba que el modelo cabe de verdad.** Esto es lo importante. En Windows o Linux, un
   modelo más grande que la **VRAM** de tu GPU se desborda a la CPU y se vuelve varias veces
   más lento — la solución es un modelo más pequeño, no más RAM del sistema. En Apple Silicon
   la comparación es contra la memoria unificada total. Mira el
   [catálogo de modelos](../ai-features/#the-model-catalog) para la cifra de cada modelo. <!-- claim:trbl-chat-slow-2 -->
2. **Usa un modelo más pequeño.** Qwen 3.5 4B es el modelo de chat más pequeño del catálogo. <!-- claim:trbl-chat-slow-3 -->
3. **Sube "mantener el modelo cargado"** en los ajustes de IA para que no se recargue desde
   disco en cada pregunta. <!-- claim:trbl-chat-slow-4 -->
4. **Baja la ventana de contexto** — una ventana menor implica menos que procesar por turno, y
   es lo primero que conviene reducir cuando un modelo entra justo. <!-- claim:trbl-chat-slow-5 -->
5. **Desactiva el modo de razonamiento**, que cambia velocidad por precisión. <!-- claim:trbl-chat-slow-6 -->

## No se está usando la GPU (Windows / Linux)

<!-- claim:trbl-gpu-used-1 -->
El registro de la app indica en qué dispositivo se cargó un modelo. Una carga correcta en GPU
se ve así:

```
llamacpp: chat model offload — Vulkan0 (Vulkan) has 15 GB free — offloading all layers
```

<!-- claim:trbl-gpu-used-2 -->
Si no ves una línea así, el backend Vulkan no encontró ningún dispositivo utilizable y recurrió
en silencio a la CPU — la app sigue funcionando, solo que más lenta. Comprueba, por orden:

1. **Tu controlador gráfico.** Es casi siempre la causa. Instala o actualiza el controlador
   normal de tu tarjeta; no hace falta ningún kit CUDA ni SDK del fabricante. <!-- claim:trbl-gpu-used-3 -->
2. **Que Vulkan vea el dispositivo.** Ejecuta `vulkaninfo --summary` (de `vulkan-tools`). Si
   no detecta ningún dispositivo, el problema está por debajo de EmailOps — arregla antes la
   stack de controladores. <!-- claim:trbl-gpu-used-4 -->
3. **Margen de VRAM.** Si el registro descarga solo *algunas* capas, el modelo es mayor que la
   VRAM libre de la tarjeta. Elige un modelo más pequeño o baja la ventana de contexto. <!-- claim:trbl-gpu-used-5 -->

<!-- claim:trbl-gpu-used-6 -->
Las máquinas virtuales y los escritorios remotos a menudo no exponen ninguna GPU, y eso es lo
esperable.

## La búsqueda solo devuelve resultados por palabras clave

<!-- claim:trbl-search-returns-1 -->
La búsqueda semántica necesita embeddings. Abre **Ajustes → Búsqueda con IA**, comprueba que
están seleccionadas las categorías que te interesan y deja que termine la pasada de
embeddings. Tras cambiar el modelo de embeddings, reconstruye el índice desde esa misma
pantalla.

<!-- claim:trbl-search-returns-2 -->
Revisa también **Limitar el procesado de IA** en los ajustes de IA — el correo más antiguo que
esa ventana se omite a propósito.

## La clasificación no etiqueta nada

- Confirma que **Clasificar nuevos correos automáticamente** está activado en
  **Ajustes → Clasificación con IA**. <!-- claim:trbl-classification-tagging-1 -->
- Comprueba qué categorías de Gmail están seleccionadas; si no hay ninguna, no se clasifica
  nada. <!-- claim:trbl-classification-tagging-2 -->
- Para el correo que llegó antes de activarlo, usa **Clasificar sin clasificar**, o
  **Reclasificar todos** tras cambiar el prompt o las reglas. <!-- claim:trbl-classification-tagging-3 -->

## La sincronización de Gmail se atasca o avisa de límites

<!-- claim:trbl-gmail-sync-1 -->
Gmail impone cuotas por cuenta. Cuando pide a EmailOps que reduzca el ritmo, la sincronización
pausa esa cuenta hasta que se reabre la ventana y se reanuda en la siguiente ejecución
programada — no hay que hacer nada. Si la sincronización sigue rota, elimina y vuelve a añadir
la cuenta para que se emita un token nuevo.

## La app está bloqueada y he olvidado la contraseña principal

<!-- claim:trbl-app-locked-1 -->
La contraseña principal es un bloqueo local sin forma de recuperarla — esa es justamente la
idea. Tu correo sigue en el servidor; puedes reinstalar EmailOps contra un directorio de datos
nuevo y volver a sincronizar.

## Cualquier otra cosa

<!-- claim:trbl-something-else-1 -->
Revisa las [incidencias abiertas](https://github.com/emailops/emailops/issues) y, si tu
problema no está, abre una nueva. Incluye tu sistema operativo y su versión, la versión de
EmailOps, qué backend y modelo de IA usas y qué esperabas que ocurriera.

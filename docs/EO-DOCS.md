# EO Docs: funcionalidades, limitaciones, gaps y siguientes pasos

**Fecha:** 05/10/2026
**Rama analizada:** `feature/shared-docs` (commit `7c78f5a4`)
**Alcance:** backend `src-tauri/src/services/shared_docs/`, `src-tauri/src/db/shared_docs.rs`, migraciones V036–V038; frontend `src/components/Documents/`, `src/lib/sheet*.ts`, `src/lib/officeImport.ts`, `src/stores/sharedDocsStore.ts`, `src/hooks/useSharedYDoc.ts`.

## 1. Resumen

EO Docs permite que varios usuarios de EmailOps editen juntos documentos de texto y hojas de cálculo **sin servidor ni nube**: el correo es el único transporte. Cada instalación guarda un CRDT (Yjs) por documento; los cambios viajan como un adjunto `.eodoc` entre las cuentas de los participantes y se fusionan sin conflictos.

El núcleo (CRDT, transporte, consentimiento, sincronización) está implementado y probado con tests y con dos instancias reales contra un servidor IMAP/SMTP local. Alrededor hay una capa de producto razonable (carpetas, historial, búsqueda, importación de Word/Excel, fórmulas básicas, filtros, deshacer). **Los huecos principales no están en la edición, sino en el transporte con proveedores reales (Gmail y Outlook sin probar), en la experiencia de quien no tiene EmailOps, en la seguridad (sin cifrado ni verificación del remitente) y en el escalado con documentos grandes.**

La funcionalidad lleva la etiqueta experimental y está **activada por defecto** desde el 05/10/2026; se desactiva en Ajustes → EO Docs (preferencia `shared_docs_enabled`).

## 2. Cómo funciona

1. **Crear:** el documento es un `Y.Doc` vacío guardado en `shared_docs.state`, el update v1 completo.
2. **Editar:** el editor (TipTap para texto, una tabla propia para hojas) escribe en el `Y.Doc` del webview. Cada cambio se guarda en el backend en lotes de 400 ms.
3. **Compartir = consentir:** un diálogo nombra a los destinatarios y pide confirmar que EmailOps enviará los cambios automáticamente. Se envía una invitación con el estado completo y una copia HTML legible.
4. **Enviar cambios:** tras 2 minutos sin editar (`FLUSH_DEBOUNCE_SECS = 120`), el bucle del outbox envía un correo por documento a todos los participantes. Lleva el diff contra el participante menos actualizado. También se envía con «Enviar ahora» o al cerrar el documento.
5. **Recibir:** durante la sync, cada mensaje con `.eodoc` pasa por un planner puro (`plan_arrival`). El resultado es aplicar, crear una invitación o ignorar. Las actualizaciones se marcan como leídas y se archivan; las invitaciones se quedan en la bandeja.
6. **Recuperación:** cada mensaje lleva el state vector del remitente. Si a alguien le falta algo, o falta una dependencia aquí, el siguiente envío lo completa. Los duplicados no hacen daño.

Decisiones registradas en `docs/DECISIONS.md`, entradas del 04/10/2026 y del 05/10/2026.

## 3. Funcionalidades

### 3.1 Documentos de texto

| Funcionalidad | Detalle |
|---|---|
| Texto enriquecido | Negrita, cursiva, subrayado, título, listas con viñetas y numeradas, enlaces, tablas, imágenes en línea (`docSchema.ts`) |
| Edición simultánea | Los cambios de otros se fusionan en el editor abierto sin mover el cursor (Yjs + `@tiptap/extension-collaboration`) |
| Deshacer / rehacer | Barra de herramientas, Cmd/Ctrl+Z, Cmd/Ctrl+Shift+Z y el menú Edición. Solo deshace los cambios propios |

### 3.2 Hojas de cálculo

| Funcionalidad | Detalle |
|---|---|
| Rejilla | Añadir y eliminar filas y columnas, insertar una fila encima, navegar con Enter / Shift+Enter |
| Pegar desde Excel | Un bloque copiado (separado por tabuladores) rellena las celdas desde la seleccionada y amplía la rejilla si hace falta |
| Anchos de columna | Se cambian arrastrando y se guardan en el documento (compartidos) |
| Fórmulas | `SUMA`/`SUM`, `PROMEDIO`/`AVERAGE`, `MIN`, `MAX`, `CONTAR`/`COUNT` sobre rangos A1. Errores `#REF!`, `#NAME?`, `#DIV/0!` y `#CYCLE!`. Lee importes como «1.287,81 €» |
| Referencias que siguen a las filas | Insertar o borrar filas o columnas reescribe los rangos, como en Excel |
| Filtros por columna | Lista de valores con casillas; la primera fila hace de cabecera. Son locales: no se comparten |
| Deshacer / rehacer | Como en los documentos de texto (`Y.UndoManager`, solo cambios propios) |

### 3.3 Organización

| Funcionalidad | Detalle |
|---|---|
| Carpetas | Personales y anidables; no se envían por correo. Se pueden crear, renombrar y eliminar (al eliminar una, su contenido sube un nivel) |
| Mover | Arrastrando a una carpeta o a la ruta superior, o con el selector «Mover a» |
| Eliminar | Con confirmación. Solo en este equipo; queda una lápida (V038) para que no reaparezca |
| Búsqueda | FTS5 por título y contenido, por prefijo de cada palabra |
| Historial | Panel lateral de solo lectura. Las ediciones locales del mismo autor se agrupan en ventanas de 5 minutos; cada llegada es una versión. Máximo 200 versiones por documento |

### 3.4 Compartir y correo

| Funcionalidad | Detalle |
|---|---|
| Consentimiento por documento | Compartir o aceptar una invitación activa el envío automático, solo a esos participantes |
| Sugerencias | Al compartir se priorizan los contactos del mismo dominio de empresa, salvo proveedores genéricos como gmail. También hay una pestaña «Mi organización» en Contactos |
| Adjuntar desde el compositor | «Desde el equipo» o «Desde EO Docs». Adjuntar un EO Doc lo comparte con los destinatarios del correo, con un aviso de que solo funciona con usuarios de EmailOps |
| Abandonar | El documento queda en solo lectura y los mensajes posteriores se ignoran |
| Varios dispositivos propios | Los mensajes propios se recogen desde Enviados |

### 3.5 Importación

| Funcionalidad | Detalle |
|---|---|
| Word (`.docx`) | Con mammoth (BSD-2), en el webview: títulos, formato, listas, tablas e imágenes de hasta ~1 MB codificadas |
| Hojas (`.xlsx`, `.xlsm`, `.xls`, `.ods`) | Con calamine (MIT), en el backend: una hoja de EO Docs por pestaña. Fechas incluidas; las fórmulas se importan como valores |
| Origen | Botón «Importar» (diálogo nativo) y «Abrir en EO Docs» en los adjuntos de un correo |
| Archivos en iCloud | La lectura se reintenta mientras se descarga; si no llega, aparece un mensaje claro |

## 4. Límites técnicos

| Límite | Valor | Fuente |
|---|---|---|
| Tamaño del adjunto `.eodoc` | 8 MB | `envelope.rs` `MAX_ENVELOPE_BYTES` |
| Participantes por documento | 50 | `envelope.rs` `MAX_PARTICIPANTS` |
| Longitud del título | 200 caracteres | `envelope.rs` `MAX_TITLE_CHARS` |
| Pausa antes de enviar | 120 s | `planner.rs` `FLUSH_DEBOUNCE_SECS` |
| Archivo importado | 20 MB, 5.000 filas, 100 columnas | `import.rs` |
| Imagen importada desde Word | ~1 MB (base64) | `officeImport.ts` `MAX_IMAGE_CHARS` |
| Versiones guardadas | 200 por documento | `db/shared_docs.rs` `MAX_VERSIONS` |

## 5. Limitaciones y gaps

Prioridad: **P0** bloquea un lanzamiento fuera de experimental, **P1** afecta de forma notable al uso, **P2** es una mejora.

### 5.1 Transporte y proveedores

| # | Gap | Impacto | Prioridad |
|---|---|---|---|
| T1 | **Gmail y Outlook (Graph) sin probar extremo a extremo.** La verificación real se hizo con dos instancias contra GreenMail (IMAP/SMTP local). El envío del adjunto con MIME propio, la detección en la sync y el archivado no se han ejercitado contra los proveedores reales | Puede no funcionar con los proveedores más usados | P0 |
| T2 | **Destinatarios sin EmailOps reciben cada actualización.** `planner::recipients` envía a todos los participantes; quien no tiene la app ve un correo con un adjunto `.eodoc` cada vez que hay cambios, hasta uno cada 2 minutos mientras se edita | Spam percibido; mala imagen ante terceros | P0 |
| T3 | **La copia de lectura de quien no tiene la app no se actualiza.** Solo la invitación lleva HTML legible; las actualizaciones llevan un texto fijo | Esas personas ven una versión congelada | P1 |
| T4 | **Las copias en Enviados de cada actualización siguen visibles** en la vista Enviados | Ruido en la bandeja de Enviados | P1 |
| T5 | **IMAP sin carpeta Archive:** las actualizaciones recibidas se quedan en la bandeja (leídas) y cada una registra un error | Ruido y errores en el registro con algunos proveedores IMAP | P1 |
| T6 | **Límite de 8 MB por mensaje.** El primer envío a un participante (y la invitación) lleva el estado completo. Un documento con muchas imágenes, o con mucho historial CRDT acumulado, puede pasar del límite y dejar de sincronizarse | Documentos grandes que no se pueden compartir | P1 |
| T7 | **Latencia de minutos.** Pausa de 2 min más el intervalo de sync del destinatario. No es colaboración en tiempo real | Expectativa «tipo Google Docs» que no se cumple | P2 (inherente al diseño; hay que comunicarlo) |
| T8 | Borrador con un EO Doc adjunto y adjuntos de fichero ya guardados: el marcador `doc-ref` no viaja ni se guarda en el borrador (`send_draft`) | Caso borde: el documento no se comparte | P2 |

### 5.2 Seguridad y privacidad

| # | Gap | Impacto | Prioridad |
|---|---|---|---|
| S1 | **Verificación del remitente solo en Gmail y Outlook.** Resuelto en parte el 05/10/2026: se rechazan las llegadas con DMARC en `fail`, o sin política DMARC con SPF en `fail` y sin DKIM válido (`planner::sender_rejection`). En cuentas IMAP no se puede atribuir el `Authentication-Results` al servidor propio (`junk::auth::expected_authserv` devuelve `None`), así que ahí un `From` falsificado sigue pasando | Un tercero podría inyectar cambios en cuentas IMAP | P1 |
| S2 | **Sin cifrado de extremo a extremo.** El contenido es tan privado como el resto del correo. El sobre tiene versión (`v`) para añadirlo después | Riesgo con documentos sensibles | P1 |
| S3 | **Sin roles ni propietario.** Cualquier participante activo puede añadir personas, y nadie puede quitar a un participante | Difusión no controlada | P1 |
| S4 | Abandonar o eliminar no avisa a los demás, que siguen enviándote cambios (tu app los ignora y los archiva) | Correo inútil; los demás no saben que te fuiste | P2 |

### 5.3 Editor de texto

| # | Gap | Prioridad |
|---|---|---|
| D1 | Exportación a PDF resuelta (05/10/2026): se genera con pdfmake y se guarda en Descargas. Sigue sin haber exportación a Word o Excel | P2 |
| D2 | Sin comentarios, sugerencias ni menciones | P2 |
| D3 | Sin presencia (quién está editando) ni cursores remotos: el transporte por correo no lo permite en tiempo real | P2 |
| D4 | No se puede renombrar un documento una vez creado (solo las carpetas) | P1 |
| D5 | Sin restaurar una versión del historial (solo consulta, por decisión de producto) | P2 |

### 5.4 Hojas de cálculo

| # | Gap | Prioridad |
|---|---|---|
| H1 | Solo cinco funciones de agregación: no hay aritmética (`=A1+B1`), ni `SI`, ni referencias a otras hojas | P1 |
| H2 | Sin formato de celda (moneda, decimales, fechas, negrita, colores) ni alineación configurable | P1 |
| H3 | Sin ordenar, sin inmovilizar filas/columnas y sin selección de rangos para copiar hacia fuera | P1 |
| H4 | Una hoja por documento: un libro con varias pestañas se importa como varios documentos | P2 |
| H5 | Sin virtualización: la tabla pinta todas las filas. No se ha medido el rendimiento con miles de filas (la importación permite hasta 5.000) | P1 |
| H6 | Si dos personas insertan filas a la vez, una fórmula puede quedar desplazada una fila: la reescritura A1 es de último en escribir, gana (registrado en DECISIONS) | P2 |
| H7 | Los filtros suponen que la primera fila es la cabecera y no se guardan entre sesiones | P2 |

### 5.5 Producto e integración

| # | Gap | Prioridad |
|---|---|---|
| P1 | **La IA no conoce EO Docs:** el chat y la búsqueda semántica no leen documentos ni hojas | P1 |
| P2 | Documentación de usuario añadida (05/10/2026, sección «EO Docs» de `features.md` en 4 idiomas). Falta la entrada en el ROADMAP y que la ayuda integrada pueda abrir la vista (no está en `help_docs::nav::VIEWS`) | P2 |
| P3 | La justificación de `gmail.send` y `gmail.modify` ante Google (verificación OAuth, CASA) no menciona el envío automático de documentos | P0 si se publica en una versión verificada |
| P4 | Sin plantillas ni duplicar documento | P2 |

## 6. Estado de verificación

| Área | Cómo se verificó | Resultado |
|---|---|---|
| Convergencia CRDT, mensajes perdidos o duplicados, remitente ajeno, abandonar, eliminar con lápida | Tests de integración con dos o tres instalaciones y `FakeEmailProvider` (`services/shared_docs/tests.rs`) | Pasan |
| Sincronización real entre dos instancias | Dos apps contra GreenMail con TLS (04/10/2026) | Convergen |
| Importación de Word y Excel | En la app, con un fichero sintético | Funciona |
| Mover arrastrando y eliminar | En la app, con eventos `DragEvent` simulados por JavaScript (WebDriver no hace un arrastre real) | Funciona; falta un arrastre real con el ratón |
| Deshacer con el menú nativo de macOS | Test con el evento `historyUndo` simulado | Falta probarlo en la app |
| Reintento de lectura de iCloud | Tests unitarios con un lector falso | Falta reproducirlo en la app |
| Gmail / Outlook reales | No verificado | — (ver T1) |
| Suites completas | `make gates SET=all` (05/10/2026): `rust-test` 4136 pasan, `vitest` 2867 pasan, clippy, fmt, biome y tsc en verde | El gate `outdated` falla por 7 dependencias que no tienen que ver con EO Docs |

## 7. Riesgos de integración y publicación

- **Numeración de migraciones:** otra rama tiene `V036__agent_draft_review.sql`, que choca con `V036__shared_docs.sql`. Hay que renumerar V036–V038 al integrar.
- **Acoplamiento de release:** V036, V037 y V038 crean tablas nuevas. Una versión instalada sin ellas no abre una base de datos que ya las tenga, el problema conocido de las migraciones aplicadas por builds de desarrollo.
- **Gate `outdated`:** bloquea el pre-push de la rama hasta actualizar `ammonia`, `libc`, `mailparse`, `reedline`, `refinery`, `tokio` y `uuid`. `main` ya tiene la subida de `refinery` 0.10.
- **Dependencias nuevas:** `yrs`, `calamine`, `yjs`, `@tiptap/extension-collaboration`, `@tiptap/y-tiptap`, `mammoth` y `pdfmake`.

## 8. Siguientes pasos

### Fase 1: hacerlo fiable con proveedores reales (antes de salir de experimental)

1. **Prueba extremo a extremo con Gmail y Outlook** entre dos cuentas de prueba (T1). Requiere permiso explícito porque envía correo real. Hay que comprobar el envío del `.eodoc`, la detección en la sync, el archivado y la copia en Enviados.
2. **No enviar actualizaciones a quien no tiene EmailOps** (T2). Propuesta: tratar a un participante como «lector» hasta recibir de él un mensaje `.eodoc`, y enviarle solo un resumen periódico con la copia HTML actualizada (cierra también T3).
3. **Verificar el remitente en IMAP** (S1): Gmail y Outlook ya están cubiertos; para IMAP, aprender el `authserv-id` del servidor de la cuenta o firmar los sobres entre instalaciones.
4. **Ocultar o archivar las copias en Enviados** de las actualizaciones (T4) y decidir el comportamiento sin carpeta Archive en IMAP (T5).
5. **Justificación OAuth** (P3) y entrada en el ROADMAP (P2).

### Fase 2: escalar y completar la experiencia

6. **Documentos grandes** (T6): medir cuánto crece el estado; dividir un envío que pase de 8 MB en varios mensajes, o mandar las imágenes como adjuntos aparte.
7. **Renombrar documentos** (D4) y **exportar** a `.docx`/`.xlsx`/PDF (D1).
8. **Hojas:** aritmética y referencias simples (H1), formato de número y moneda (H2), ordenar (H3) y virtualización con `@tanstack/react-virtual`, que ya está en el proyecto, midiendo antes con 5.000 filas (H5).
9. **Control de participantes** (S3): quitar participantes y avisar al abandonar (S4), con un tipo de mensaje nuevo en el sobre.

### Fase 3: diferenciación

10. **EO Docs en la IA** (P1): exponer los documentos a la búsqueda semántica y al chat como fuente («¿qué pusimos en el presupuesto?»). Pasa por `build-ai-feature` y su evaluación.
11. **Cifrado de extremo a extremo** (S2) con intercambio de claves entre participantes de EmailOps, usando el campo de versión del sobre.
12. Comentarios (D2), plantillas y duplicar (P4), y libros con varias pestañas (H4).

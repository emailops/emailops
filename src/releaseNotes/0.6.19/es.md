**¿Te gusta EmailOps?** Apoya el proyecto con una [estrella en GitHub](https://github.com/emailops/emailops): ayuda a que más gente lo encuentre.

### Novedades

- **Cco y asuntos editables**: todas las ventanas de redacción tienen campo Cco, y el asunto de una respuesta o un reenvío se puede cambiar antes de enviarlo. Las respuestas van a la dirección del Reply-To, como la de una lista de correo.
- **Los borradores conservan sus adjuntos**: un archivo se guarda con el borrador en cuanto lo añades, y los reenvíos también se guardan como borradores.
- **Búsqueda booleana**: comillas para una frase exacta, `OR`, `-palabra` o `NOT` para excluir y paréntesis para agrupar. Además, `cc:` y `has:attachment` para filtrar.
- **El correo nuevo está listo antes para la búsqueda con IA**: los Embeddings se generan del más reciente al más antiguo, en todas tus cuentas.

### Correcciones

- IMAP: un mensaje eliminado aparece una sola vez en Eliminados, y en Proton Mail Bridge ya no se borra para siempre.
- IMAP: las cuentas conectan aunque la primera dirección del servidor no responda.
- IMAP: en servidores antiguos, mover un mensaje ya no borra otros marcados para eliminar por otra app.
- IMAP: una respuesta del servidor que EmailOps no sabe leer ya no bloquea la sincronización.
- Los borradores de IA respetan tus instrucciones con cualquier plantilla, y una instrucción larga ya no se corta sin avisar.
- Los adjuntos de Gmail guardados con el mensaje vuelven a abrirse.
- Un borrador que Gmail rechaza ya no se reenvía cada minuto.

---
title: 'Línea de comandos (emailops-cli)'
description: 'Automatiza tu bandeja desde la terminal, con salida JSON estable para scripts y agentes.'
weight: 50
---

`emailops-cli` mueve el mismo motor local que la app de escritorio — tu correo, tus cuentas,
tu IA local — desde una terminal. Lee la base de datos que la app ya sincronizó, así que no
hay una configuración aparte ni una segunda copia de tu correo.

Por ahora solo macOS.

## Instalación

Descarga `EmailOps-CLI-macos.dmg` desde la
[última versión](https://github.com/emailops/emailops/releases/latest), móntalo y pon el
binario en tu `PATH`:

```bash
hdiutil attach ~/Downloads/EmailOps-CLI-macos.dmg
cp /Volumes/EmailOps\ CLI/emailops-cli /usr/local/bin/emailops-cli
hdiutil detach /Volumes/EmailOps\ CLI

emailops-cli doctor    # confirma que ve tus datos y cuentas
```

El binario es universal (Apple Silicon + Intel), firmado y notarizado, así que Gatekeeper lo
deja pasar sin avisos.

## Inicio rápido

```bash
emailops-cli accounts                     # qué cuentas están conectadas
emailops-cli emails --limit 10            # los 10 correos más recientes
emailops-cli search "factura"             # búsqueda de texto completo
emailops-cli chat "¿qué dijo Acme sobre el contrato?"
emailops-cli                              # sin subcomando → REPL interactivo
```

En el REPL, el texto normal es un turno de chat (los tokens llegan en directo) y las líneas
que empiezan por `/` corresponden a los subcomandos: `/search`, `/account`, `/sync`, `/help`,
`/quit`.

## Comandos

| Comando | Para qué sirve |
|---|---|
| `accounts` | Lista las cuentas configuradas |
| `emails [--limit N] [--mailbox inbox\|sent\|spam\|trash]` | Lista los correos recientes |
| `show <id>` | Muestra un correo (cabeceras y cuerpo) |
| `search <consulta> [--limit N]` | Búsqueda de texto completo |
| `chat <pregunta> [--trace]` | Hace una pregunta; `--trace` añade tiempos de enrutado y recuperación |
| `sync [cuenta]` | Descarga el correo nuevo |
| `calendar [--days N] [--next] [--sync]` | Próximos eventos (`--next` = solo la siguiente reunión) |
| `classify [--all]` | Clasifica los correos nuevos — o todos |
| `embed [--batch N]` | Genera los embeddings de búsqueda |
| `doctor` | Informe de estado de solo lectura (base de datos, cuentas, configuración de IA) |

Las opciones globales funcionan antes o después del subcomando: `--json`, `--quiet`,
`--account <id|email>`, `--model <modelo>`, `--data-dir <dir>`.

Los comandos de lectura son seguros con la app abierta. Las escrituras pesadas (`sync`,
`classify`, `embed`) van mejor con la app cerrada.

## Scripts con `--json`

Con `--json`, cada comando imprime exactamente un sobre por stdout — con la misma forma tanto
si va bien como si falla — mientras los registros van a stderr:

```jsonc
{ "ok": true,  "data": { /* resultado */ }, "error": null }
{ "ok": false, "data": null, "error": { "code": "not_found", "message": "…", "params": {} } }
```

```bash
# Asuntos de los 20 correos más recientes
emailops-cli emails --limit 20 --json | jq -r '.data[].subject'

# Solo el texto de la respuesta de una pregunta al chat
emailops-cli chat "resume mi correo sin leer" --json | jq -r '.data.answer'

# Remitente y asunto de cada resultado de búsqueda, en TSV
emailops-cli search "from:ana factura" --json | jq -r '.data[] | [.sender, .subject] | @tsv'
```

Los códigos de salida están agrupados por lo que harías al respecto: `0` éxito, `2` entrada
inválida, `3` no encontrado, `4` autenticación, `5` red/sincronización, `6` IA, `130`
cancelado, `1` cualquier otra cosa — así los scripts pueden ramificar por el código en lugar
de analizar texto.

Si tienes más de una cuenta, guarda una por defecto en vez de repetir `--account`:

```bash
emailops-cli config set default-account tu@ejemplo.com
```

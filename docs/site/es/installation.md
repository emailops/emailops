---
title: 'Instalación'
description: 'Descarga e instala EmailOps en macOS, Windows o Linux.'
weight: 10
---

## Requisitos del sistema

<!-- claim:inst-system-requirements-1 -->
Ejecutar la IA **en local** es la parte que requiere un hardware más potente, y es opcional. Puedes desactivarla en el
asistente inicial y usar EmailOps como un cliente de correo normal, o conservar todas las
funciones de IA y dirigirlas a un proveedor remoto. Los dos modos tienen requisitos muy
distintos.

### Con IA local {#with-local-ai}

<!-- claim:inst-system-requirements-local-ai-1 -->
Uno de los requisitos más importante para ejecutar la IA local es la memoria disponible para cargar el modelo y los contextos. Dependiendo de nuestra máquina estamos hablando de un tipo de memoria o de otro:

| | Mac con Apple Silicon | Windows / Linux |
|---|---|---|
| Ejecuta el modelo en | La GPU integrada, vía Metal | Tu GPU, vía Vulkan — o la CPU si no hay GPU |
| Memoria en la que debe caber | Memoria unificada, compartida con el sistema | La **VRAM** de la GPU, o la RAM del sistema al usar la CPU |
| Mínimo | 8 GB unificados | 8 GB de VRAM, o 16 GB de RAM sin GPU |
| Recomendado | 16 GB unificados o más | 12–16 GB de VRAM |
| Disco libre | ~3 GB — app más el modelo de chat predeterminado | ~3 GB — app más el modelo de chat predeterminado |

<!-- claim:inst-system-requirements-local-ai-2 -->
**Regla de dimensionado:** el modelo tiene que caber entero en la memoria en la que se
ejecuta. El **Qwen 3.5 4B** predeterminado necesita unos 8 GB para que la app lo ofrezca (mientras
responde usa menos de 4 GB); el modelo más grande del catálogo pide 32 GB. La cifra de cada modelo está en el
[catálogo de modelos](../ai-features/#the-model-catalog).

- **Apple Silicon** tiene memoria unificada — la GPU direcciona el mismo bloque que la CPU,
  así que la cifra a comparar es la memoria total del sistema. Un Mac de 16 GB ejecuta con
  holgura los modelos hasta la fila de 16 GB, restando lo que ya usan macOS y tus otras apps. <!-- claim:inst-system-requirements-local-ai-3 -->
- **Una GPU en Windows o Linux** tiene su propia VRAM independiente, y esa es la cifra que
  cuenta — 32 GB de RAM del sistema no ayudan si la tarjeta solo tiene 8 GB. Un modelo que no
  cabe se desborda a la CPU, lo que funciona pero es varias veces más lento. <!-- claim:inst-system-requirements-local-ai-4 -->
- **Sin GPU** también está soportado y no requiere una descarga distinta. La app recurre a la
  CPU y a la RAM del sistema; reserva la cifra del modelo en RAM y cuenta con respuestas
  notablemente más lentas. <!-- claim:inst-system-requirements-local-ai-5 -->

<!-- claim:inst-system-requirements-local-ai-6 -->
Los Mac Intel son la excepción: el runtime de IA integrado necesita un chip Apple Silicon
(M1 o posterior) y no puede ejecutarse en ellos — mira la [nota más abajo](#direct-download).

### Sin IA local

<!-- claim:inst-system-requirements-without-local-1 -->
| | Mínimo | Recomendado |
|---|---|---|
| RAM | 2 GB | 4 GB |
| Disco libre | ~500 MB, más espacio para el correo que sincronices | Según el tamaño de tu buzón |
| Procesador | 64 bits, 2 núcleos | — |
| Gráficos | Ninguno | Ninguno |

<!-- claim:inst-system-requirements-without-local-2 -->
Estos son los requisitos en dos casos: con la IA desactivada del todo, y con la IA activada
**pero dirigida a OpenRouter**. La inferencia remota ocurre en el hardware de otros, así que
basta con un portátil antiguo — a cambio de una clave de API, un coste por uso y el contenido
de tu correo saliendo del dispositivo. Consulta
[elegir un backend](../ai-features/#choosing-a-backend).

### Sistema operativo

<!-- claim:inst-system-requirements-operating-system-1 -->
Ambos modos necesitan uno de estos:

- **macOS** Monterey (12) o posterior — Apple Silicon o Intel. <!-- claim:inst-system-requirements-operating-system-2 -->
- **Windows** 10 u 11, 64 bits. <!-- claim:inst-system-requirements-operating-system-3 -->
- **Linux** 64 bits, con WebKitGTK y un llavero Secret Service — mira
  [Linux](#linux) más abajo. <!-- claim:inst-system-requirements-operating-system-4 -->

## macOS

### Homebrew

<!-- claim:inst-macos-homebrew-1 -->
```bash
brew install --cask emailops/tap/emailops
```

<!-- claim:inst-macos-homebrew-2 -->
Actualiza después con `brew upgrade --cask emailops`.

### Descarga directa {#direct-download}

1. Descarga **EmailOps-macos.dmg** desde la
   [última versión](https://github.com/emailops/emailops/releases/latest). <!-- claim:inst-macos-direct-download-1 -->
2. Abre el DMG y arrastra **EmailOps.app** a tu carpeta Aplicaciones. <!-- claim:inst-macos-direct-download-2 -->
3. Ábrelo desde Aplicaciones. <!-- claim:inst-macos-direct-download-3 -->

<!-- claim:inst-macos-direct-download-4 -->
> **Mac con Intel:** esta única descarga funciona en cualquier Mac — no hay una compilación
> aparte para Intel. Las funciones de IA son la excepción: la IA integrada necesita un chip
> Apple Silicon (M1 o posterior). En un Mac Intel permanece desactivada y EmailOps te explica
> por qué. Todo lo demás funciona con normalidad. Para la IA, apunta EmailOps a
> [OpenRouter](../ai-features/#choosing-a-backend).

## Windows

1. Descarga **EmailOps-windows-setup.exe** desde la
   [última versión](https://github.com/emailops/emailops/releases/latest). <!-- claim:inst-windows-1 -->
2. Ejecuta el instalador y sigue los pasos. <!-- claim:inst-windows-2 -->
3. Abre EmailOps desde el menú Inicio. <!-- claim:inst-windows-3 -->

### "Windows protegió su PC" {#smartscreen}

<!-- claim:inst-windows-smartscreen-1 -->
Al ejecutar el instalador, Windows puede mostrar una pantalla azul de **Microsoft Defender
SmartScreen** con el mensaje *"Windows protegió su PC"*. Es lo esperado: el instalador todavía
no está firmado, porque firmar en Windows requiere un certificado de pago que el proyecto no
tiene. El aviso no dice nada del archivo en sí; SmartScreen lo muestra con cualquier descarga
sin firmar que no haya visto muchas veces.

<!-- claim:inst-windows-smartscreen-2 -->
Para continuar:

1. Haz clic en **Más información**. <!-- claim:inst-windows-smartscreen-3 -->
2. Comprueba que la aplicación se llama **EmailOps** y haz clic en **Ejecutar de todas formas**. <!-- claim:inst-windows-smartscreen-4 -->

<!-- claim:inst-windows-smartscreen-5 -->
Si antes quieres confirmar que la descarga es la auténtica, compara su hash SHA-256
(`Get-FileHash .\EmailOps-windows-setup.exe` en PowerShell) con los checksums de la
[página de la versión](https://github.com/emailops/emailops/releases/latest).

### Aceleración por GPU

<!-- claim:inst-windows-gpu-acceleration-1 -->
No hay que instalar nada más. La versión de Windows lleva un backend **Vulkan** que se
carga en tiempo de ejecución siempre que haya un controlador gráfico funcionando, y recurre a
la CPU cuando no lo hay — la misma descarga en ambos casos.

<!-- claim:inst-windows-gpu-acceleration-2 -->
Se eligió Vulkan en lugar de CUDA precisamente para que esto siga siendo simple: cubre AMD,
Intel y NVIDIA con el controlador gráfico que ya tienes, sin ningún kit del fabricante que
instalar. Mantén el controlador de la GPU razonablemente al día y funcionará.

## Linux {#linux}

1. Descarga **EmailOps-linux.AppImage** desde la
   [última versión](https://github.com/emailops/emailops/releases/latest). <!-- claim:inst-linux-1 -->
2. Dale permisos de ejecución y ábrelo: <!-- claim:inst-linux-2 -->

```bash
chmod +x EmailOps-linux.AppImage
./EmailOps-linux.AppImage
```

### Aceleración por GPU

<!-- claim:inst-linux-gpu-acceleration-1 -->
Igual que en Windows: la AppImage lleva un backend **Vulkan** que se usa automáticamente
cuando hay un controlador gráfico presente y recurre a la CPU cuando no lo hay. Sin kit CUDA,
sin SDK del fabricante, sin tener que elegir una compilación distinta.

<!-- claim:inst-linux-gpu-acceleration-2 -->
Lo que necesitas es el stack normal de controladores Vulkan de tu tarjeta —
`mesa-vulkan-drivers` en AMD e Intel, el controlador propietario de NVIDIA en NVIDIA — que la
mayoría de distribuciones de escritorio ya instalan. Si `vulkaninfo` detecta un dispositivo,
EmailOps lo usará.

### Hace falta un llavero

<!-- claim:inst-linux-keyring-required-1 -->
EmailOps nunca escribe las credenciales de las cuentas en un archivo: los tokens OAuth y las
contraseñas IMAP van al almacén de credenciales del sistema. macOS y Windows traen uno
(el Llavero y el Administrador de credenciales); en Linux tienes que aportarlo tú.

<!-- claim:inst-linux-keyring-required-2 -->
Necesitas un proveedor de **Secret Service** instalado y desbloqueado. Cualquiera de estos
sirve:

- **GNOME Keyring** (`gnome-keyring`) — el predeterminado en GNOME, Ubuntu, Fedora Workstation. <!-- claim:inst-linux-keyring-required-3 -->
- **KWallet** (`kwalletmanager` con la interfaz Secret Service) — el equivalente en KDE. <!-- claim:inst-linux-keyring-required-4 -->
- **KeePassXC** con *Ajustes → Integración con Secret Service* activada. <!-- claim:inst-linux-keyring-required-5 -->

<!-- claim:inst-linux-keyring-required-6 -->
En un gestor de ventanas mínimo o en una sesión sin escritorio a menudo no hay ningún llavero
en marcha. Instala uno de los anteriores y asegúrate de que está desbloqueado cuando arranca
EmailOps — de lo contrario, añadir una cuenta falla, porque no hay dónde guardar las
credenciales de forma segura.

<!-- claim:inst-linux-keyring-required-7 -->
```bash
# Debian / Ubuntu
sudo apt install gnome-keyring

# Fedora
sudo dnf install gnome-keyring

# Arch
sudo pacman -S gnome-keyring
```

## Dónde viven tus datos

<!-- claim:inst-where-data-1 -->
Todo lo que EmailOps guarda está en tu máquina, en el directorio de datos de aplicación de tu
sistema:

- **Correo, contactos, eventos de calendario, embeddings** — una base de datos SQLite local. <!-- claim:inst-where-data-2 -->
- **Modelos de IA descargados** — una carpeta `models/` junto a la base de datos. <!-- claim:inst-where-data-3 -->
- **Tokens OAuth y contraseñas** — el llavero del sistema, nunca un archivo en claro. <!-- claim:inst-where-data-4 -->

<!-- claim:inst-where-data-5 -->
Para mover o compartir un directorio de datos (para pruebas o un segundo perfil), define la
variable de entorno `EMAILOPS_DATA_DIR` antes de arrancar. Las rutas exactas por plataforma, y
qué se escribe dónde, están en
[Privacidad y seguridad](../privacy-security/#where-your-data-is-stored).

## Desinstalar

<!-- claim:inst-uninstalling-1 -->
Quitar la app deja a propósito tu base de datos de correo y los modelos descargados, de modo
que una reinstalación retoma donde lo dejaste. Borra también el directorio de datos si quieres
empezar de cero.

<!-- claim:inst-uninstalling-2 -->
En ningún caso se elimina nada de tu proveedor de correo — desinstalar EmailOps nunca toca el
correo de Gmail, Outlook o tu servidor IMAP.

### macOS

<!-- claim:inst-uninstalling-macos-1 -->
Con Homebrew, un solo comando elimina la app y sus datos:

```bash
brew uninstall --zap --cask emailops
```

<!-- claim:inst-uninstalling-macos-2 -->
Sin `--zap` solo se va la app. Para hacerlo a mano: arrastra **EmailOps.app** de Aplicaciones a
la Papelera y luego borra:

```
~/Library/Application Support/com.emailops.app
~/Library/Caches/com.emailops.app
~/Library/HTTPStorages/com.emailops.app
~/Library/Preferences/com.emailops.app.plist
~/Library/Saved Application State/com.emailops.app.savedState
~/Library/WebKit/com.emailops.app
```

### Windows

<!-- claim:inst-uninstalling-windows-1 -->
Desinstala desde **Configuración → Aplicaciones → Aplicaciones instaladas → EmailOps**, o
ejecuta el desinstalador desde la entrada del menú Inicio. Después borra el directorio de
datos:

```
%APPDATA%\com.emailops.app
```

### Linux

<!-- claim:inst-uninstalling-linux-1 -->
Borra el archivo AppImage. Después borra los directorios de datos y configuración:

```bash
rm -rf ~/.local/share/com.emailops.app
rm -rf ~/.config/com.emailops.app
```

### Credenciales guardadas

<!-- claim:inst-uninstalling-stored-credentials-1 -->
En todas las plataformas, los tokens OAuth y las contraseñas IMAP viven en el llavero del
sistema y no en el directorio de datos, así que sobreviven a todo lo anterior. Elimina las
entradas `com.emailops.app` de Acceso a Llaveros (macOS), el Administrador de credenciales
(Windows) o tu gestor de llavero (Linux) si también quieres deshacerte de ellas.

## Compilar desde el código fuente

<!-- claim:inst-building-from-1 -->
Si prefieres compilarlo tú, el README del repositorio cubre la cadena de herramientas de Rust
y Node, los requisitos previos de Tauri y el flujo `make dev`. Ten en cuenta que las
compilaciones desde código necesitan tus **propias** credenciales OAuth de Gmail / Microsoft
en `.env.local`; los binarios publicados ya vienen configurados.

<!-- claim:inst-building-from-2 -->
Dos notas de compilación sobre el runtime de IA:

- Las versiones de Windows y Linux se compilan con `DYNAMIC_BACKENDS=1` y
  `CARGO_FEATURES=vulkan`, que es lo que produce un único artefacto capaz de aprovechar una
  GPU en tiempo de ejecución. Compilar el backend Vulkan requiere el SDK de Vulkan — una
  dependencia solo de compilación; los usuarios nunca lo instalan. <!-- claim:inst-building-from-3 -->
- También existe una característica de Cargo `cuda` que genera una variante solo para
  NVIDIA. El pipeline de publicación la publica para Windows como un instalador aparte,
  `EmailOps-windows-cuda.msi`, junto a la versión Vulkan predeterminada, que cubre todas las
  marcas de GPU. <!-- claim:inst-building-from-4 -->

# AUR Installer (Rust CLI)

Gestor, buscador, instalador y desinstalador ultrarrápido para el **Arch User Repository (AUR)** escrito en **Rust**.

Permite buscar entre los más de 120,000 paquetes de AUR en tiempo real a medida que escribes, priorizando coincidencias exactas y recomendando paquetes por descripción.

---

## Características

- ⚡ **Rendimiento Nativo en Rust**: Lectura, descompresión gzip y filtrado instantáneo en memoria sin lag.
- 🕒 **Caché Inteligente (1 día)**: Guarda los metadatos oficiales en `~/.cache/aur_installer/packages-meta-ext-v1.json.gz` y sólo los renueva si han pasado más de 24 horas.
- 🔍 **Búsqueda Interactiva en Tiempo Real (TUI)**:
  - Coincidencia exacta con el nombre primero.
  - Paquetes cuyo nombre empiece con el término.
  - Paquetes que contengan el término en el nombre.
  - Recomendaciones por coincidencias dentro de la descripción.
- 📦 **Instalación Automatizada**: Clona el repositorio de AUR en un directorio temporal en `/tmp`, ejecuta `makepkg -si` y limpia los residuos al terminar.
- 🗑️ **Desinstalación Rápida (`-Rs`)**: Búsqueda interactiva entre los paquetes instalados en el sistema o desinstalación directa mediante `pacman -Rs`.

---

## Instalación

### Opción 1: Con el script `install.sh`
Para compilar la versión optimizada e instalar el binario `aur` en `/usr/local/bin`:

```bash
chmod +x install.sh
./install.sh
```

### Opción 2: Compilación manual con Cargo
```bash
cargo build --release
sudo install -Dm755 target/release/aur /usr/local/bin/aur
```

---

## Uso y Comandos

Una vez instalado, puedes ejecutar `aur` directamente desde cualquier terminal:

| Comando | Descripción |
|---|---|
| `aur -h` o `aur --help` | Muestra el menú de ayuda y opciones disponibles. |
| `aur -Ss` | Inicia la búsqueda interactiva en tiempo real sobre todos los paquetes de AUR. Pulsa <kbd>Enter</kbd> para instalar el seleccionado. |
| `aur -S <paquete>` | Instala directamente el paquete indicado desde AUR sin pasar por la búsqueda interactiva. |
| `aur -Rs` | Abre la búsqueda interactiva entre los paquetes instalados localmente en el sistema. Pulsa <kbd>Enter</kbd> para desinstalarlo con `sudo pacman -Rs`. |
| `aur -Rs <paquete>` | Desinstala directamente el paquete indicado usando `sudo pacman -Rs`. |

---

## Controles en la Búsqueda Interactiva

- **Escribir**: Filtra los resultados al instante según tecleas.
- **<kbd>↑</kbd> / <kbd>↓</kbd>**: Navega hacia arriba o abajo en la lista de resultados.
- **<kbd>Enter</kbd>**: Selecciona el paquete para instalar o desinstalar.
- **<kbd>x</kbd>** *(con la barra vacía)*, **<kbd>Esc</kbd>** o **<kbd>Ctrl + C</kbd>**: Sale de la búsqueda de inmediato sin realizar cambios.

---

## Estructura del Proyecto

```text
├── Cargo.toml         # Configuración del paquete y dependencias en Rust
├── src/
│   └── main.rs        # Código fuente completo en Rust (TUI, AUR parser, makepkg, pacman)
├── target/release/aur # Binario ejecutable compilado
├── install.sh         # Script de instalación para copiar a /usr/local/bin
├── functions/
│   └── aur.py         # Versión previa en Python
└── README.md          # Documentación del proyecto
```

#!/usr/bin/env bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_PATH="${SCRIPT_DIR}/target/release/aur"
INSTALL_DEST="/usr/local/bin/aur"

echo "==> Compilando versión release de 'aur'..."
cd "${SCRIPT_DIR}"
cargo build --release

if [ ! -f "${BIN_PATH}" ]; then
    echo "Error: No se encontró el binario en ${BIN_PATH}"
    exit 1
fi

echo "==> Instalando binario en ${INSTALL_DEST} (requiere sudo)..."
sudo install -Dm755 "${BIN_PATH}" "${INSTALL_DEST}"

echo ""
echo "==> ¡Instalación completada con éxito!"
echo "Ahora puedes usar 'aur' globalmente desde cualquier terminal:"
echo "    aur -h          # Ver ayuda"
echo "    aur -Ss         # Buscar e instalar interactivamente"
echo "    aur -S <pkg>    # Instalar directamente"
echo "    aur -Rs [pkg]   # Desinstalar con pacman -Rs"

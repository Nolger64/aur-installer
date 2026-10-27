"""Módulo para interactuar con el Arch User Repository (AUR)."""

import argparse
import curses
import gzip
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple, Union

# Archivo oficial de metadatos comprimidos que provee el AUR con todos los paquetes existentes
AUR_METADATA_URL = "https://aur.archlinux.org/packages-meta-ext-v1.json.gz"
DEFAULT_USER_AGENT = "aur-installer-python/1.0"
ONE_DAY_SECONDS = 86400  # 24 horas (1 día)


def _get_default_cache_path() -> Path:
    """Retorna la ruta del archivo de caché según el estándar XDG."""
    cache_base = os.environ.get("XDG_CACHE_HOME")
    if not cache_base:
        cache_base = os.path.expanduser("~/.cache")
    return Path(cache_base) / "aur_installer" / "packages-meta-ext-v1.json.gz"


def get_all_aur(
    format_type: str = "dict",
    use_cache: bool = True,
    cache_ttl: int = ONE_DAY_SECONDS,
    as_json_string: bool = False,
    cache_path: Optional[Union[str, Path]] = None,
    timeout: int = 30,
) -> Union[Dict[str, Any], List[Dict[str, Any]], str]:
    """Descarga y retorna todos los paquetes del AUR junto con su descripción.

    Utiliza el volcado oficial comprimido `packages-meta-ext-v1.json.gz` provisto
    por los servidores de Arch Linux, el cual contiene la totalidad de paquetes
    del AUR (más de 120,000 paquetes).

    Args:
        format_type: Formato de retorno de los datos:
            - 'dict': Diccionario {nombre_paquete: descripcion}. O(1) para búsquedas.
            - 'list': Lista de objetos [{"name": ..., "description": ...}].
            - 'full': Diccionario con la información completa del paquete proporcionada por AUR.
        use_cache: Si es True, guarda una copia local en caché para evitar descargas
            repetidas y acelerar llamadas subsecuentes.
        cache_ttl: Tiempo de vida de la caché en segundos (por defecto 1 día / 86400s).
        as_json_string: Si es True, serializa y retorna el resultado como una cadena JSON.
            Si es False (por defecto), retorna estructuras de datos nativas de Python (dict o list).
        cache_path: Ruta personalizada para el archivo de caché (opcional).
        timeout: Tiempo máximo de espera en segundos para la descarga por red.

    Returns:
        Diccionario, lista o cadena JSON con todos los paquetes y sus descripciones.

    Raises:
        RuntimeError: Si ocurre un error al descargar o procesar los datos.
    """
    target_cache_file = Path(cache_path) if cache_path else _get_default_cache_path()

    # Verificar si tenemos una copia en caché válida y dentro del TTL
    cache_valid = False
    if use_cache and target_cache_file.is_file():
        file_age = time.time() - target_cache_file.stat().st_mtime
        if file_age < cache_ttl:
            cache_valid = True

    # Si la caché no es válida o no existe, descargamos el archivo oficial
    if not cache_valid:
        req = urllib.request.Request(
            AUR_METADATA_URL,
            headers={"User-Agent": DEFAULT_USER_AGENT},
        )
        try:
            with urllib.request.urlopen(req, timeout=timeout) as response:
                if use_cache:
                    target_cache_file.parent.mkdir(parents=True, exist_ok=True)
                    # Descargamos directamente al archivo de caché
                    with open(target_cache_file, "wb") as f_out:
                        while chunk := response.read(64 * 1024):
                            f_out.write(chunk)
                else:
                    # Si no se usa caché, leemos en memoria directamente
                    raw_data = response.read()
        except (urllib.error.URLError, TimeoutError, OSError) as e:
            # Si falla la red pero existe una caché previa (aunque haya expirado), la usamos como respaldo
            if use_cache and target_cache_file.is_file():
                cache_valid = True
            else:
                raise RuntimeError(f"Error al descargar la lista de paquetes del AUR: {e}") from e

    # Procesar y parsear los datos línea por línea para minimizar el uso de memoria
    results_dict: Dict[str, Any] = {}
    results_list: List[Dict[str, Any]] = []

    def process_item(item: Dict[str, Any]) -> None:
        name = item.get("Name", "")
        desc = item.get("Description") or ""
        if format_type == "dict":
            results_dict[name] = desc
        elif format_type == "list":
            results_list.append({"name": name, "description": desc})
        elif format_type == "full":
            results_dict[name] = item

    open_func = (
        gzip.open(target_cache_file, "rt", encoding="utf-8")
        if (use_cache and target_cache_file.is_file())
        else gzip.GzipFile(fileobj=raw_data)
    )

    with open_func as gz_file:
        for line in gz_file:
            line_str = line.strip()
            if line_str in ("[", "]") or not line_str:
                continue
            if line_str.endswith(","):
                line_str = line_str[:-1]
            try:
                pkg_data = json.loads(line_str)
                process_item(pkg_data)
            except json.JSONDecodeError:
                continue

    result_data: Union[Dict[str, Any], List[Dict[str, Any]]] = (
        results_list if format_type == "list" else results_dict
    )

    if as_json_string:
        return json.dumps(result_data, ensure_ascii=False)

    return result_data


def filter_packages(
    query: str,
    packages_items: List[Tuple[str, str, str, str]],
    max_results: int = 50,
) -> List[Tuple[str, str]]:
    """Filtra y ordena paquetes según la consulta del usuario.

    Prioridad:
    1. Coincidencia exacta con el nombre.
    2. Nombre comienza con la consulta (prefijo).
    3. Consulta contenida dentro del nombre.
    4. Consulta contenida dentro de la descripción (recomendaciones).

    Retorna una lista de tuplas (nombre, descripcion).
    """
    clean_query = query.strip().lower()
    if not clean_query:
        return [(name, desc) for name, desc, _, _ in packages_items[:max_results]]

    exact_matches: List[Tuple[str, str]] = []
    starts_matches: List[Tuple[str, str]] = []
    name_matches: List[Tuple[str, str]] = []
    desc_matches: List[Tuple[str, str]] = []

    for name, desc, name_lower, desc_lower in packages_items:
        if name_lower == clean_query:
            exact_matches.append((name, desc))
        elif name_lower.startswith(clean_query):
            starts_matches.append((name, desc))
        elif clean_query in name_lower:
            name_matches.append((name, desc))
        elif clean_query in desc_lower:
            desc_matches.append((name, desc))

        # Corte temprano para no recorrer innecesariamente si ya tenemos suficientes
        if len(exact_matches) + len(starts_matches) + len(name_matches) + len(desc_matches) >= max_results * 4:
            break

    ordered = exact_matches + starts_matches + name_matches + desc_matches
    return ordered[:max_results]


def _tui_search(
    stdscr: Any,
    packages: Dict[str, str],
    action_verb: str = "instalar",
    prompt_label: str = "Buscar AUR",
) -> Optional[str]:
    """Interfaz interactiva basada en curses para búsqueda en tiempo real."""
    curses.curs_set(1)
    stdscr.nodelay(False)
    curses.use_default_colors()

    # Precomputamos minúsculas para búsquedas ultrarrápidas
    packages_items = [
        (name, desc, name.lower(), desc.lower())
        for name, desc in packages.items()
    ]

    query = ""
    selected_idx = 0
    results: List[Tuple[str, str]] = filter_packages(query, packages_items)

    while True:
        stdscr.erase()
        max_y, max_x = stdscr.getmaxyx()

        # Encabezado e input de búsqueda
        title = f" Búsqueda interactiva ('x', ESC o Ctrl+C para salir, Enter para {action_verb}) "
        try:
            stdscr.addstr(0, 0, title[: max_x - 1], curses.A_REVERSE)
            prompt = f"{prompt_label}: {query}"
            stdscr.addstr(1, 0, prompt[: max_x - 1], curses.A_BOLD)
            stdscr.addstr(2, 0, "-" * min(max_x - 1, 80))
        except curses.error:
            pass

        # Mostrar resultados disponibles
        header_lines = 3
        avail_lines = max_y - header_lines - 1

        if results and selected_idx >= len(results):
            selected_idx = max(0, len(results) - 1)

        scroll_offset = 0
        if selected_idx >= avail_lines:
            scroll_offset = selected_idx - avail_lines + 1

        for idx, (pkg_name, pkg_desc) in enumerate(
            results[scroll_offset : scroll_offset + avail_lines]
        ):
            line_pos = header_lines + idx
            real_idx = scroll_offset + idx
            is_selected = real_idx == selected_idx

            # Cortar y dar formato a la línea
            line_text = f"{pkg_name} - {pkg_desc}"
            if len(line_text) >= max_x - 1:
                line_text = line_text[: max_x - 4] + "..."

            attr = curses.A_REVERSE if is_selected else curses.A_NORMAL
            try:
                stdscr.addstr(line_pos, 0, line_text[: max_x - 1], attr)
            except curses.error:
                pass

        if not results:
            try:
                stdscr.addstr(header_lines, 0, "No se encontraron coincidencias.", curses.A_DIM)
            except curses.error:
                pass

        # Posicionar cursor al final del prompt
        cursor_x = min(len(f"{prompt_label}: ") + len(query), max_x - 1)
        try:
            stdscr.move(1, cursor_x)
        except curses.error:
            pass

        stdscr.refresh()

        try:
            key = stdscr.get_wch()
        except curses.error:
            continue

        if key in (curses.KEY_UP, "\x1b[A"):
            if selected_idx > 0:
                selected_idx -= 1
        elif key in (curses.KEY_DOWN, "\x1b[B"):
            if selected_idx < len(results) - 1:
                selected_idx += 1
        elif key in ("\n", "\r", curses.KEY_ENTER):
            if results and 0 <= selected_idx < len(results):
                return results[selected_idx][0]
            return None
        elif key in (curses.KEY_BACKSPACE, "\x7f", "\b", 127):
            if query:
                query = query[:-1]
                selected_idx = 0
                results = filter_packages(query, packages_items)
        elif key == 27:  # ESC o Secuencia de escape
            stdscr.nodelay(True)
            next_ch = stdscr.getch()
            stdscr.nodelay(False)
            if next_ch == -1:
                return None
        elif key == 3:  # Ctrl + C
            return None
        elif key in ("x", "X") and not query:
            # Salir con 'x' cuando la barra de búsqueda está vacía
            return None
        elif isinstance(key, str) and key.isprintable():
            query += key
            selected_idx = 0
            results = filter_packages(query, packages_items)


def instalar(pkg_name: str) -> bool:
    """Clona un paquete de AUR en /tmp, ejecuta makepkg -si y elimina los archivos temporales.

    Args:
        pkg_name: Nombre exacto del paquete en AUR.

    Returns:
        True si la instalación finalizó con éxito, False en caso contrario.
    """
    clean_name = pkg_name.strip()
    if not clean_name:
        print("Error: Debe especificar un nombre de paquete válido.")
        return False

    clone_url = f"https://aur.archlinux.org/{clean_name}.git"
    temp_dir = tempfile.mkdtemp(prefix=f"aur_{clean_name}_", dir="/tmp")
    clone_path = Path(temp_dir) / clean_name

    print(f"\n==> Instalando paquete '{clean_name}' desde AUR...")
    print(f"==> Clonando {clone_url} en {clone_path}...")

    try:
        # Clonar el repositorio del paquete
        clone_res = subprocess.run(
            ["git", "clone", clone_url, str(clone_path)],
            check=False,
        )
        if clone_res.returncode != 0:
            print(f"Error: No se pudo clonar el repositorio de AUR para '{clean_name}'.")
            return False

        # Ejecutar makepkg -si en el directorio clonado
        print("\n==> Ejecutando makepkg -si...")
        makepkg_res = subprocess.run(
            ["makepkg", "-si"],
            cwd=str(clone_path),
            check=False,
        )
        if makepkg_res.returncode != 0:
            print(f"\nError: 'makepkg -si' falló con código {makepkg_res.returncode}.")
            return False

        print(f"\n==> ¡'{clean_name}' instalado exitosamente!")
        return True

    except Exception as e:
        print(f"Error inesperado durante la instalación: {e}")
        return False
    finally:
        # Limpieza del directorio temporal en /tmp
        if os.path.exists(temp_dir):
            print(f"==> Limpiando archivos temporales en {temp_dir}...")
            shutil.rmtree(temp_dir, ignore_errors=True)


def get_installed_packages() -> Dict[str, str]:
    """Obtiene el diccionario de todos los paquetes instalados en el sistema {nombre: descripcion}."""
    installed: Dict[str, str] = {}
    try:
        output = subprocess.check_output(
            ["pacman", "-Qi"],
            text=True,
            errors="replace",
            env={**os.environ, "LC_ALL": "C"},
        )
        cur_name: Optional[str] = None
        cur_desc: Optional[str] = None
        for line in output.splitlines():
            if line.startswith("Name"):
                cur_name = line.split(":", 1)[1].strip()
            elif line.startswith("Description"):
                cur_desc = line.split(":", 1)[1].strip()
            elif not line.strip():
                if cur_name and cur_desc:
                    installed[cur_name] = cur_desc
                cur_name = None
                cur_desc = None
        if cur_name and cur_desc:
            installed[cur_name] = cur_desc
    except Exception as e:
        print(f"Error al consultar paquetes instalados con pacman: {e}")
    return installed


def desinstalar(pkg_name: str) -> bool:
    """Desinstala un paquete usando pacman -Rs.

    Args:
        pkg_name: Nombre del paquete a desinstalar.

    Returns:
        True si se desinstaló exitosamente, False de lo contrario.
    """
    clean_name = pkg_name.strip()
    if not clean_name:
        print("Error: Debe especificar un nombre de paquete válido.")
        return False

    print(f"\n==> Desinstalando paquete '{clean_name}' (pacman -Rs)...")
    try:
        res = subprocess.run(
            ["sudo", "pacman", "-Rs", clean_name],
            check=False,
        )
        if res.returncode == 0:
            print(f"\n==> ¡'{clean_name}' desinstalado exitosamente!")
            return True
        else:
            print(f"\nError: Falló la desinstalación con código {res.returncode}.")
            return False
    except Exception as e:
        print(f"Error inesperado al desinstalar: {e}")
        return False


def buscar_desinstalar() -> Optional[str]:
    """Inicia la búsqueda interactiva en tiempo real sobre los paquetes instalados para desinstalar."""
    print("Obteniendo lista de paquetes instalados...")
    installed_pkgs = get_installed_packages()
    if not installed_pkgs:
        print("No se encontraron paquetes instalados o pacman no está disponible.")
        return None

    selected = curses.wrapper(
        _tui_search,
        installed_pkgs,
        "desinstalar",
        "Buscar instalados",
    )
    if selected:
        print(f"Paquete seleccionado para desinstalar: {selected}")
        desinstalar(selected)
    return selected


def buscar(cache_ttl: int = ONE_DAY_SECONDS, auto_install: bool = True) -> Optional[str]:
    """Inicia la búsqueda interactiva en tiempo real sobre los paquetes de AUR.

    Verifica si el último volcado de paquetes de AUR está vigente (1 día por defecto).
    Si no lo está, lo descarga automáticamente actualizando la caché.
    Si se selecciona un paquete con Enter y auto_install es True, procede a instalarlo.
    """
    cache_file = _get_default_cache_path()
    if cache_file.is_file():
        file_age = time.time() - cache_file.stat().st_mtime
        if file_age < cache_ttl:
            remaining_hours = int((cache_ttl - file_age) / 3600)
            print(f"Caché de AUR vigente ({remaining_hours}h restantes). Cargando...")
        else:
            print("Caché de AUR expirada (+1 día). Actualizando metadatos oficiales...")
    else:
        print("Descargando metadatos de AUR por primera vez...")

    packages = get_all_aur(format_type="dict", use_cache=True, cache_ttl=cache_ttl)

    selected = curses.wrapper(
        _tui_search,
        packages,
        "instalar",
        "Buscar AUR",
    )
    if selected:
        print(f"Paquete seleccionado: {selected}")
        if auto_install:
            instalar(selected)
    return selected


def main() -> None:
    parser = argparse.ArgumentParser(description="Gestor, buscador, instalador y desinstalador para Arch Linux / AUR")
    parser.add_argument(
        "-Ss",
        "--search",
        action="store_true",
        help="Iniciar búsqueda interactiva de paquetes AUR en tiempo real (Enter para instalar)",
    )
    parser.add_argument(
        "-S",
        "--install",
        metavar="PAQUETE",
        type=str,
        help="Instalar directamente un paquete de AUR por su nombre",
    )
    parser.add_argument(
        "-Rs",
        "--remove",
        nargs="?",
        const="",
        metavar="PAQUETE",
        help="Desinstalar un paquete (pacman -Rs). Si no se indica paquete, abre la búsqueda interactiva para elegirlo.",
    )
    args = parser.parse_args()

    if args.install:
        instalar(args.install)
    elif args.remove is not None:
        if args.remove:
            desinstalar(args.remove)
        else:
            buscar_desinstalar()
    elif args.search:
        buscar()
    else:
        print("Probando get_all_aur()...")
        start = time.time()
        packages = get_all_aur(format_type="dict", use_cache=True)
        elapsed = time.time() - start

        print(f"Cargados {len(packages):,} paquetes en {elapsed:.2f} segundos.")
        sample_pkgs = ["yay", "google-chrome", "visual-studio-code-bin", "spotify"]
        for pkg in sample_pkgs:
            if pkg in packages:
                print(f"- {pkg}: {packages[pkg]}")


if __name__ == "__main__":
    main()
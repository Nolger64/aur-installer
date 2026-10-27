use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::SystemTime;

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{
        self, disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use flate2::read::GzDecoder;
use serde::Deserialize;
use tempfile::Builder;

const AUR_METADATA_URL: &str = "https://aur.archlinux.org/packages-meta-ext-v1.json.gz";
const ONE_DAY_SECS: u64 = 86400; // 24 horas

#[derive(Debug, Clone)]
pub struct PkgItem {
    pub name: String,
    pub desc: String,
    pub name_lower: String,
    pub desc_lower: String,
}

#[derive(Deserialize)]
struct AurRawItem {
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "Description")]
    description: Option<String>,
}

fn get_cache_path() -> PathBuf {
    let cache_base = match env::var("XDG_CACHE_HOME") {
        Ok(val) if !val.is_empty() => PathBuf::from(val),
        _ => {
            let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".cache")
        }
    };
    cache_base.join("aur_installer").join("packages-meta-ext-v1.json.gz")
}

fn download_aur_dump(target: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }

    println!("==> Descargando metadatos oficiales de AUR (packages-meta-ext-v1.json.gz)...");
    let mut response = ureq::get(AUR_METADATA_URL)
        .header("User-Agent", "aur-installer-rust/1.0")
        .call()?;

    let mut reader = response.body_mut().as_reader();
    let mut file = File::create(target)?;
    io::copy(&mut reader, &mut file)?;
    println!("==> Metadatos descargados exitosamente.");
    Ok(())
}

fn get_all_aur(cache_ttl_secs: u64) -> Result<Vec<PkgItem>, Box<dyn std::error::Error>> {
    let cache_file = get_cache_path();
    let mut cache_valid = false;

    if cache_file.is_file() {
        if let Ok(metadata) = fs::metadata(&cache_file) {
            if let Ok(mtime) = metadata.modified() {
                if let Ok(elapsed) = SystemTime::now().duration_since(mtime) {
                    if elapsed.as_secs() < cache_ttl_secs {
                        let remaining_hours = (cache_ttl_secs - elapsed.as_secs()) / 3600;
                        println!("==> Caché de AUR vigente ({}h restantes). Cargando...", remaining_hours);
                        cache_valid = true;
                    } else {
                        println!("==> Caché de AUR expirada (+1 día). Actualizando...");
                    }
                }
            }
        }
    }

    if !cache_valid {
        if let Err(err) = download_aur_dump(&cache_file) {
            if cache_file.is_file() {
                eprintln!("Aviso: Falló la descarga en red ({}), usando copia de respaldo en caché.", err);
            } else {
                return Err(format!("Error descargando datos de AUR: {}", err).into());
            }
        }
    }

    let file = File::open(&cache_file)?;
    let gz = GzDecoder::new(file);
    let reader = BufReader::with_capacity(128 * 1024, gz);

    let mut packages = Vec::with_capacity(130_000);

    for line_res in reader.lines() {
        let line = line_res?;
        let trimmed = line.trim();
        if trimmed == "[" || trimmed == "]" || trimmed.is_empty() {
            continue;
        }
        let clean = if trimmed.ends_with(',') {
            &trimmed[..trimmed.len() - 1]
        } else {
            trimmed
        };

        if let Ok(raw) = serde_json::from_str::<AurRawItem>(clean) {
            if let Some(name) = raw.name {
                let desc = raw.description.unwrap_or_default();
                let name_lower = name.to_lowercase();
                let desc_lower = desc.to_lowercase();
                packages.push(PkgItem {
                    name,
                    desc,
                    name_lower,
                    desc_lower,
                });
            }
        }
    }

    Ok(packages)
}

fn get_installed_packages() -> Vec<PkgItem> {
    let mut items = Vec::new();
    let output = Command::new("pacman")
        .arg("-Qi")
        .env("LC_ALL", "C")
        .output();

    if let Ok(out) = output {
        let text = String::from_utf8_lossy(&out.stdout);
        let mut cur_name: Option<String> = None;
        let mut cur_desc: Option<String> = None;

        for line in text.lines() {
            if line.starts_with("Name") {
                if let Some((_, val)) = line.split_once(':') {
                    cur_name = Some(val.trim().to_string());
                }
            } else if line.starts_with("Description") {
                if let Some((_, val)) = line.split_once(':') {
                    cur_desc = Some(val.trim().to_string());
                }
            } else if line.trim().is_empty() {
                if let (Some(name), Some(desc)) = (cur_name.take(), cur_desc.take()) {
                    let name_lower = name.to_lowercase();
                    let desc_lower = desc.to_lowercase();
                    items.push(PkgItem {
                        name,
                        desc,
                        name_lower,
                        desc_lower,
                    });
                }
            }
        }
        if let (Some(name), Some(desc)) = (cur_name.take(), cur_desc.take()) {
            let name_lower = name.to_lowercase();
            let desc_lower = desc.to_lowercase();
            items.push(PkgItem {
                name,
                desc,
                name_lower,
                desc_lower,
            });
        }
    }
    items
}

pub fn filter_packages<'a>(query: &str, items: &'a [PkgItem], max_results: usize) -> Vec<&'a PkgItem> {
    let clean = query.trim().to_lowercase();
    if clean.is_empty() {
        return items.iter().take(max_results).collect();
    }

    let mut exact = Vec::new();
    let mut starts = Vec::new();
    let mut in_name = Vec::new();
    let mut in_desc = Vec::new();

    for item in items {
        if item.name_lower == clean {
            exact.push(item);
        } else if item.name_lower.starts_with(&clean) {
            starts.push(item);
        } else if item.name_lower.contains(&clean) {
            in_name.push(item);
        } else if item.desc_lower.contains(&clean) {
            in_desc.push(item);
        }

        if exact.len() + starts.len() + in_name.len() + in_desc.len() >= max_results * 4 {
            break;
        }
    }

    exact
        .into_iter()
        .chain(starts)
        .chain(in_name)
        .chain(in_desc)
        .take(max_results)
        .collect()
}

struct RawTerminalGuard;
impl RawTerminalGuard {
    fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, cursor::Show)?;
        Ok(RawTerminalGuard)
    }
}
impl Drop for RawTerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), LeaveAlternateScreen, cursor::Show);
        let _ = disable_raw_mode();
    }
}

fn tui_search(
    items: &[PkgItem],
    prompt_label: &str,
    action_verb: &str,
) -> io::Result<Option<String>> {
    let _guard = RawTerminalGuard::new()?;
    let mut stdout = io::stdout();

    let mut query = String::new();
    let mut selected_idx = 0;
    let mut results = filter_packages(&query, items, 60);

    loop {
        let (cols, rows) = terminal::size()?;
        execute!(stdout, cursor::MoveTo(0, 0), Clear(ClearType::All))?;

        // 1. Título
        let title = format!(
            " Búsqueda interactiva ('x', ESC o Ctrl+C para salir, Enter para {}) ",
            action_verb
        );
        let title_padded = if title.len() < cols as usize {
            format!("{:<width$}", title, width = cols as usize)
        } else {
            title[..cols as usize].to_string()
        };

        execute!(
            stdout,
            cursor::MoveTo(0, 0),
            SetBackgroundColor(Color::DarkGrey),
            SetForegroundColor(Color::White),
            Print(&title_padded),
            ResetColor
        )?;

        // 2. Prompt
        let prompt = format!("{}: {}", prompt_label, query);
        let prompt_display = if prompt.len() >= cols as usize {
            &prompt[..cols as usize]
        } else {
            &prompt
        };
        execute!(
            stdout,
            cursor::MoveTo(0, 1),
            SetForegroundColor(Color::Cyan),
            Print(prompt_display),
            ResetColor
        )?;

        // Separador
        let sep = "-".repeat(cols.min(80) as usize);
        execute!(stdout, cursor::MoveTo(0, 2), Print(&sep))?;

        // 3. Resultados
        let header_lines = 3u16;
        let avail_lines = rows.saturating_sub(header_lines + 1);

        if !results.is_empty() && selected_idx >= results.len() {
            selected_idx = results.len() - 1;
        }

        let scroll_offset = if selected_idx >= avail_lines as usize {
            selected_idx - avail_lines as usize + 1
        } else {
            0
        };

        let visible = results
            .iter()
            .skip(scroll_offset)
            .take(avail_lines as usize)
            .enumerate();

        for (idx, item) in visible {
            let row_pos = header_lines + idx as u16;
            let real_idx = scroll_offset + idx;
            let is_sel = real_idx == selected_idx;

            let line = format!("{} - {}", item.name, item.desc);
            let display_str = if line.chars().count() >= cols as usize {
                let truncated: String = line.chars().take(cols.saturating_sub(4) as usize).collect();
                format!("{}...", truncated)
            } else {
                line
            };

            execute!(stdout, cursor::MoveTo(0, row_pos))?;
            if is_sel {
                let padded = format!("{:<width$}", display_str, width = cols as usize);
                execute!(
                    stdout,
                    SetBackgroundColor(Color::Blue),
                    SetForegroundColor(Color::White),
                    Print(padded),
                    ResetColor
                )?;
            } else {
                execute!(
                    stdout,
                    SetForegroundColor(Color::Green),
                    Print(&item.name),
                    ResetColor,
                    Print(" - "),
                    Print(&item.desc)
                )?;
            }
        }

        if results.is_empty() {
            execute!(
                stdout,
                cursor::MoveTo(0, header_lines),
                SetForegroundColor(Color::DarkGrey),
                Print("No se encontraron coincidencias."),
                ResetColor
            )?;
        }

        // Posicionar cursor
        let cursor_x = (format!("{}: ", prompt_label).chars().count() + query.chars().count())
            .min(cols.saturating_sub(1) as usize) as u16;
        execute!(stdout, cursor::MoveTo(cursor_x, 1))?;
        stdout.flush()?;

        // Manejo de eventos
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            // Ctrl+C
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                return Ok(None);
            }

            match key.code {
                KeyCode::Esc => return Ok(None),
                KeyCode::Char('x') | KeyCode::Char('X') if query.is_empty() => return Ok(None),
                KeyCode::Up => {
                    if selected_idx > 0 {
                        selected_idx -= 1;
                    }
                }
                KeyCode::Down => {
                    if !results.is_empty() && selected_idx < results.len() - 1 {
                        selected_idx += 1;
                    }
                }
                KeyCode::Enter => {
                    if !results.is_empty() && selected_idx < results.len() {
                        return Ok(Some(results[selected_idx].name.clone()));
                    }
                }
                KeyCode::Backspace => {
                    if !query.is_empty() {
                        query.pop();
                        selected_idx = 0;
                        results = filter_packages(&query, items, 60);
                    }
                }
                KeyCode::Char(c) => {
                    query.push(c);
                    selected_idx = 0;
                    results = filter_packages(&query, items, 60);
                }
                _ => {}
            }
        }
    }
}

pub fn instalar(pkg_name: &str) -> bool {
    let clean = pkg_name.trim();
    if clean.is_empty() {
        eprintln!("Error: Debe especificar un nombre de paquete válido.");
        return false;
    }

    let temp_dir = match Builder::new().prefix(&format!("aur_{}_", clean)).tempdir_in("/tmp") {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("Error al crear directorio temporal en /tmp: {}", e);
            return false;
        }
    };

    let clone_path = temp_dir.path().join(clean);
    let clone_url = format!("https://aur.archlinux.org/{}.git", clean);

    println!("\n==> Instalando paquete '{}' desde AUR...", clean);
    println!("==> Clonando {} en {:?}...", clone_url, clone_path);

    let clone_status = Command::new("git")
        .args(["clone", &clone_url, clone_path.to_str().unwrap()])
        .status();

    match clone_status {
        Ok(status) if status.success() => {}
        _ => {
            eprintln!("Error: No se pudo clonar el repositorio de AUR para '{}'.", clean);
            return false;
        }
    }

    println!("\n==> Ejecutando makepkg -si...");
    let makepkg_status = Command::new("makepkg")
        .arg("-si")
        .current_dir(&clone_path)
        .status();

    match makepkg_status {
        Ok(status) if status.success() => {
            println!("\n==> ¡'{}' instalado exitosamente!", clean);
            true
        }
        Ok(status) => {
            eprintln!("\nError: makepkg -si finalizó con código: {:?}", status.code());
            false
        }
        Err(e) => {
            eprintln!("\nError ejecutando makepkg: {}", e);
            false
        }
    }
    // temp_dir se borra automáticamente al salir de scope gracias a RAII de tempfile
}

pub fn desinstalar(pkg_name: &str) -> bool {
    let clean = pkg_name.trim();
    if clean.is_empty() {
        eprintln!("Error: Debe especificar un nombre de paquete válido.");
        return false;
    }

    println!("\n==> Desinstalando paquete '{}' con pacman -Rs...", clean);
    let status = Command::new("sudo")
        .args(["pacman", "-Rs", clean])
        .status();

    match status {
        Ok(s) if s.success() => {
            println!("\n==> ¡'{}' desinstalado exitosamente!", clean);
            true
        }
        Ok(s) => {
            eprintln!("\nError: Falló pacman -Rs con código: {:?}", s.code());
            false
        }
        Err(e) => {
            eprintln!("\nError ejecutando sudo pacman -Rs: {}", e);
            false
        }
    }
}

fn print_help() {
    println!(
        r#"aur - Gestor, buscador, instalador y desinstalador para Arch Linux / AUR

USO:
    aur [OPCIÓN] [PAQUETE]

OPCIONES:
    -Ss               Búsqueda interactiva en tiempo real en AUR (Enter para instalar)
    -S <paquete>      Instalar un paquete de AUR directamente por su nombre
    -Rs [paquete]     Desinstalar un paquete (pacman -Rs). Si no se pasa nombre,
                      inicia búsqueda interactiva entre los paquetes instalados
    -h, --help        Muestra esta ayuda

TECLAS EN LA BÚSQUEDA INTERACTIVA:
    Escribir          Filtra en tiempo real (prioriza nombre exacto, prefijo y descripción)
    ↑ / ↓             Navegar por la lista
    Enter             Seleccionar el paquete (instalar o desinstalar)
    x (con input vacío), ESC, Ctrl+C
                      Salir de la búsqueda
"#
    );
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        print_help();
        return ExitCode::SUCCESS;
    }

    match args[1].as_str() {
        "-h" | "--help" => {
            print_help();
            ExitCode::SUCCESS
        }
        "-Ss" => {
            println!("==> Obteniendo paquetes de AUR...");
            match get_all_aur(ONE_DAY_SECS) {
                Ok(packages) => {
                    match tui_search(&packages, "Buscar AUR", "instalar") {
                        Ok(Some(pkg)) => {
                            println!("Paquete seleccionado: {}", pkg);
                            if instalar(&pkg) {
                                ExitCode::SUCCESS
                            } else {
                                ExitCode::FAILURE
                            }
                        }
                        Ok(None) => ExitCode::SUCCESS,
                        Err(e) => {
                            eprintln!("Error en la interfaz: {}", e);
                            ExitCode::FAILURE
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    ExitCode::FAILURE
                }
            }
        }
        "-S" => {
            if args.len() < 3 {
                eprintln!("Error: Debe indicar el nombre del paquete para instalar. Ejemplo: aur -S yay");
                return ExitCode::FAILURE;
            }
            if instalar(&args[2]) {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        "-Rs" => {
            if args.len() >= 3 {
                if desinstalar(&args[2]) {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                }
            } else {
                println!("==> Consultando paquetes instalados en el sistema...");
                let installed = get_installed_packages();
                if installed.is_empty() {
                    eprintln!("No se encontraron paquetes instalados o pacman no está disponible.");
                    return ExitCode::FAILURE;
                }
                match tui_search(&installed, "Buscar instalados", "desinstalar") {
                    Ok(Some(pkg)) => {
                        println!("Paquete seleccionado para desinstalar: {}", pkg);
                        if desinstalar(&pkg) {
                            ExitCode::SUCCESS
                        } else {
                            ExitCode::FAILURE
                        }
                    }
                    Ok(None) => ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("Error en la interfaz: {}", e);
                        ExitCode::FAILURE
                    }
                }
            }
        }
        other => {
            eprintln!("Opción desconocida: '{}'. Usa 'aur -h' para ver las opciones disponibles.", other);
            ExitCode::FAILURE
        }
    }
}

//! Finds — and when missing, installs — the two executables the app drives.
//!
//! This is what makes the app usable by someone who has never installed a
//! developer tool: yt-dlp and FFmpeg are fetched into the app's own data
//! folder on demand, so nothing has to be on the PATH and nothing is written
//! outside the user profile. Keeping yt-dlp downloadable also matters for a
//! different reason: it needs frequent updates to keep working with YouTube,
//! and a copy frozen into the installer would rot within weeks.

use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

const YTDLP_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";

/// yt-dlp's own FFmpeg builds, which are the ones it is tested against.
const FFMPEG_URL: &str =
    "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip";

#[cfg(windows)]
const YTDLP_NAMES: [&str; 1] = ["yt-dlp.exe"];
#[cfg(not(windows))]
const YTDLP_NAMES: [&str; 1] = ["yt-dlp"];

#[cfg(windows)]
const FFMPEG_NAMES: [&str; 1] = ["ffmpeg.exe"];
#[cfg(not(windows))]
const FFMPEG_NAMES: [&str; 1] = ["ffmpeg"];

#[derive(Clone, Debug, Default)]
pub struct Tools {
    pub ytdlp: Option<PathBuf>,
    pub ffmpeg: Option<PathBuf>,
}

impl Tools {
    /// The directory holding FFmpeg, to pass to yt-dlp as `--ffmpeg-location`.
    /// `None` when FFmpeg is only reachable through the PATH, where yt-dlp
    /// finds it by itself.
    pub fn ffmpeg_dir(&self) -> Option<&Path> {
        self.ffmpeg.as_deref().and_then(Path::parent)
    }

    pub fn require_ytdlp(&self) -> Result<&Path, String> {
        self.ytdlp.as_deref().ok_or_else(|| {
            "O yt-dlp ainda não está instalado. Use \"Instalar ferramentas\" para baixá-lo."
                .to_string()
        })
    }
}

/// Where downloaded tools live: inside the app's data folder, so an install
/// needs no administrator rights and an uninstall takes it with it.
pub fn bin_dir(app_data: &Path) -> PathBuf {
    app_data.join("bin")
}

/// Looks for an executable in the places it can legitimately be, nearest
/// first: the folder we download into, next to the app itself (a portable
/// copy dropped in by hand), and finally the PATH.
fn locate(app_data: &Path, names: &[&str]) -> Option<PathBuf> {
    let mut roots = vec![bin_dir(app_data)];
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
    {
        roots.push(exe_dir);
    }
    for root in &roots {
        for name in names {
            let candidate = root.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.is_file())
}

pub fn discover(app_data: &Path) -> Tools {
    Tools {
        ytdlp: locate(app_data, &YTDLP_NAMES),
        ffmpeg: locate(app_data, &FFMPEG_NAMES),
    }
}

// ---------------------------------------------------------------- install ---

/// Downloads a file, reporting whole percentages through `log` so a slow
/// connection still shows something happening.
fn download(
    url: &str,
    destination: &Path,
    label: &str,
    log: &dyn Fn(String),
) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("prism")
        .build()
        .map_err(|err| format!("Não foi possível iniciar o download: {err}"))?;
    let mut response = client
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|err| format!("Falha ao baixar {label}: {err}"))?;
    let total = response.content_length();

    let file = File::create(destination)
        .map_err(|err| format!("Não foi possível gravar o arquivo: {err}"))?;
    let mut writer = BufWriter::new(file);
    let mut buffer = vec![0u8; 64 * 1024];
    let mut done: u64 = 0;
    let mut reported = 0;
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|err| format!("Download interrompido: {err}"))?;
        if read == 0 {
            break;
        }
        writer
            .write_all(&buffer[..read])
            .map_err(|err| format!("Não foi possível gravar o arquivo: {err}"))?;
        done += read as u64;
        if let Some(total) = total.filter(|value| *value > 0) {
            let percent = (done * 100 / total) as u32;
            if percent >= reported + 10 {
                reported = percent - percent % 10;
                log(format!("    {label}: {reported}%"));
            }
        }
    }
    writer
        .flush()
        .map_err(|err| format!("Não foi possível gravar o arquivo: {err}"))?;
    Ok(())
}

/// Pulls just the two executables we need out of the FFmpeg archive, ignoring
/// its documentation and nested folder layout.
fn extract_ffmpeg(archive_path: &Path, destination: &Path) -> Result<(), String> {
    let file = File::open(archive_path).map_err(|err| format!("Arquivo não encontrado: {err}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|err| format!("Arquivo .zip inválido: {err}"))?;
    let mut extracted = 0;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| format!("Falha ao ler o .zip: {err}"))?;
        let name = entry.name().to_string();
        let base = name.rsplit('/').next().unwrap_or_default();
        if !FFMPEG_NAMES
            .iter()
            .chain(["ffprobe.exe", "ffprobe"].iter())
            .any(|wanted| base.eq_ignore_ascii_case(wanted))
        {
            continue;
        }
        let target = destination.join(base);
        let mut output = File::create(&target)
            .map_err(|err| format!("Não foi possível gravar {}: {err}", target.display()))?;
        std::io::copy(&mut entry, &mut output)
            .map_err(|err| format!("Falha ao extrair {base}: {err}"))?;
        extracted += 1;
    }
    if extracted == 0 {
        return Err("O pacote do FFmpeg não contém o executável esperado.".into());
    }
    Ok(())
}

/// Downloads whatever is missing and reports each step through `log`.
///
/// `force` re-downloads yt-dlp even when it is already there, which is how the
/// interface offers an update.
pub fn install(app_data: &Path, force: bool, log: &dyn Fn(String)) -> Result<Tools, String> {
    let bin = bin_dir(app_data);
    fs::create_dir_all(&bin)
        .map_err(|err| format!("Não foi possível criar {}: {err}", bin.display()))?;
    let mut tools = discover(app_data);

    if force || tools.ytdlp.is_none() {
        log(">>> Baixando o yt-dlp (cerca de 18 MB)…".into());
        // Downloaded aside and swapped in, so a failed download never leaves a
        // truncated executable behind.
        let staged = bin.join("yt-dlp.exe.part");
        download(YTDLP_URL, &staged, "yt-dlp", log)?;
        let final_path = bin.join(YTDLP_NAMES[0]);
        fs::rename(&staged, &final_path)
            .map_err(|err| format!("Não foi possível salvar o yt-dlp: {err}"))?;
        tools.ytdlp = Some(final_path);
        log(">>> yt-dlp instalado.".into());
    }

    if tools.ffmpeg.is_none() {
        log(">>> Baixando o FFmpeg, necessário para juntar vídeo e áudio (cerca de 80 MB)…".into());
        let archive = bin.join("ffmpeg.zip");
        download(FFMPEG_URL, &archive, "FFmpeg", log)?;
        log(">>> Extraindo o FFmpeg…".into());
        let result = extract_ffmpeg(&archive, &bin);
        let _ = fs::remove_file(&archive);
        result?;
        tools.ffmpeg = Some(bin.join(FFMPEG_NAMES[0]));
        log(">>> FFmpeg instalado.".into());
    }

    log(">>> Tudo pronto.".into());
    Ok(tools)
}

/// Reads the version yt-dlp reports, for the log line on startup.
pub fn ytdlp_version(ytdlp: &Path) -> Option<String> {
    let output = crate::ytdlp::command(ytdlp)
        .arg("--version")
        .output()
        .ok()?;
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!version.is_empty()).then_some(version)
}

/// Opens a folder in the system file manager.
pub fn reveal(folder: &Path) -> Result<(), String> {
    if !folder.is_dir() {
        return Err("A pasta de destino ainda não existe.".into());
    }
    #[cfg(windows)]
    let mut command = {
        // explorer.exe returns a non-zero code even when it worked, so the
        // status is deliberately not checked below.
        let mut command = std::process::Command::new("explorer.exe");
        command.arg(folder);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg(folder);
        command
    };
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(folder);
        command
    };
    command
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("Não foi possível abrir a pasta: {err}"))
}

/// Keeps a download from writing outside the folder the user chose, which a
/// crafted file-name pattern could otherwise try.
pub fn ensure_folder(folder: &str) -> Result<PathBuf, String> {
    let trimmed = folder.trim();
    if trimmed.is_empty() {
        return Err("Informe a pasta de destino.".into());
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err("Use um caminho completo para a pasta de destino.".into());
    }
    fs::create_dir_all(&path).map_err(|err| format!("Não foi possível usar esta pasta: {err}"))?;
    Ok(path)
}

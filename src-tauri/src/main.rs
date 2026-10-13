// Hides the console window that would otherwise sit behind the app on Windows,
// while keeping it in debug builds so `cargo tauri dev` can print.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Wiring: the commands the window calls, and the state they share.
//!
//! Everything substantial lives in the modules below — this file only
//! validates what comes in from the interface and hands it over.

mod config;
mod job;
mod tools;
mod ytdlp;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

use job::{Job, JobEvent};

/// Containers and audio formats offered by the interface. Anything else is
/// refused rather than passed on to yt-dlp.
const CONTAINERS: [&str; 3] = ["mp4", "mkv", "original"];
const AUDIO_EXPORT_FORMATS: [&str; 6] = ["original", "mp3", "m4a", "wav", "flac", "opus"];

struct AppState {
    app_data: PathBuf,
    config_path: PathBuf,
    config: Mutex<config::AppConfig>,
    tools: Mutex<tools::Tools>,
    js_runtime: Vec<String>,
    /// Analyses already done, keyed by link: a download uses the formats from
    /// the analysis the user was actually looking at.
    infos: Mutex<HashMap<String, ytdlp::VideoInfo>>,
    job: Arc<Job>,
    default_folder: PathBuf,
    installing: AtomicBool,
}

#[derive(Clone, Serialize)]
struct Status {
    ytdlp: bool,
    ffmpeg: bool,
    js_runtime: bool,
    default_folder: String,
    config: config::AppConfig,
}

fn status_of(state: &AppState) -> Status {
    let tools = state.tools.lock().unwrap();
    Status {
        ytdlp: tools.ytdlp.is_some(),
        ffmpeg: tools.ffmpeg.is_some(),
        js_runtime: !state.js_runtime.is_empty(),
        default_folder: state.default_folder.to_string_lossy().into_owned(),
        config: state.config.lock().unwrap().clone(),
    }
}

#[tauri::command]
fn status(state: tauri::State<AppState>) -> Status {
    status_of(&state)
}

/// The state and log of the current download, for a window that just opened.
#[tauri::command]
fn snapshot(state: tauri::State<AppState>) -> JobEvent {
    state.job.snapshot()
}

#[tauri::command]
fn analyze(url: String, state: tauri::State<AppState>) -> Result<ytdlp::VideoInfo, String> {
    let url = url.trim().to_string();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("O link precisa começar com http:// ou https://".into());
    }
    let ytdlp_path = state.tools.lock().unwrap().require_ytdlp()?.to_path_buf();
    let info = ytdlp::fetch_info(&ytdlp_path, &state.js_runtime, &url)?;
    state.infos.lock().unwrap().insert(url, info.clone());
    Ok(info)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadRequest {
    url: String,
    mode: ytdlp::Mode,
    video_format_id: Option<String>,
    audio_format_id: Option<String>,
    folder: String,
    container: String,
    audio_export_format: String,
    embed_metadata: bool,
    embed_thumbnail: bool,
    write_subs: bool,
    sub_langs: String,
    filename_pattern: String,
}

#[tauri::command]
fn start_download(request: DownloadRequest, state: tauri::State<AppState>) -> Result<(), String> {
    if !CONTAINERS.contains(&request.container.as_str()) {
        return Err("Formato de saída inválido.".into());
    }
    if !AUDIO_EXPORT_FORMATS.contains(&request.audio_export_format.as_str()) {
        return Err("Formato de áudio inválido.".into());
    }
    let folder = tools::ensure_folder(&request.folder)?;

    let info = state
        .infos
        .lock()
        .unwrap()
        .get(request.url.trim())
        .cloned()
        .ok_or("Analise o link antes de iniciar o download.")?;

    let video = request
        .video_format_id
        .as_deref()
        .and_then(|id| info.format(id));
    let audio = request
        .audio_format_id
        .as_deref()
        .and_then(|id| info.format(id));
    let selector = ytdlp::format_selector(video, audio, request.mode)?;

    let (ytdlp_path, ffmpeg_dir) = {
        let tools = state.tools.lock().unwrap();
        (
            tools.require_ytdlp()?.to_path_buf(),
            tools.ffmpeg_dir().map(PathBuf::from),
        )
    };

    let pattern = match request.filename_pattern.trim() {
        "" => ytdlp::DEFAULT_PATTERN.to_string(),
        value => value.to_string(),
    };
    let args = ytdlp::download_args(&ytdlp::DownloadOptions {
        url: request.url.trim(),
        format_selector: &selector,
        folder: &folder,
        mode: request.mode,
        container: &request.container,
        audio_export_format: &request.audio_export_format,
        embed_metadata: request.embed_metadata,
        embed_thumbnail: request.embed_thumbnail,
        write_subs: request.write_subs,
        sub_langs: &request.sub_langs,
        filename_pattern: &pattern,
        ffmpeg_dir: ffmpeg_dir.as_deref(),
        js_runtime: &state.js_runtime,
    });

    // Remember the choices for next time before anything can go wrong.
    {
        let mut stored = state.config.lock().unwrap();
        stored.last_folder = folder.to_string_lossy().into_owned();
        stored.container = request.container.clone();
        stored.audio_export_format = request.audio_export_format.clone();
        stored.embed_metadata = request.embed_metadata;
        stored.embed_thumbnail = request.embed_thumbnail;
        stored.write_subs = request.write_subs;
        stored.sub_langs = request.sub_langs.clone();
        stored.filename_pattern = pattern;
        config::save(&state.config_path, &stored);
    }

    state.job.log(format!(">>> Formato: {selector}"));
    state.job.log(format!(">>> Destino: {}", folder.display()));
    if ffmpeg_dir.is_none() && state.tools.lock().unwrap().ffmpeg.is_none() {
        state.job.log(
            ">>> AVISO: FFmpeg não encontrado. Juntar vídeo e áudio ou converter pode falhar.",
        );
    }
    state.job.log(">>> Iniciando download…");
    state.job.start(&ytdlp_path, &args, info.title)
}

#[tauri::command]
fn pause(state: tauri::State<AppState>) -> Result<(), String> {
    state.job.pause()
}

#[tauri::command]
fn resume(state: tauri::State<AppState>) -> Result<(), String> {
    state.job.resume()
}

#[tauri::command]
fn cancel(state: tauri::State<AppState>) -> Result<(), String> {
    state.job.cancel()
}

/// Opens the system folder picker. `None` means the user dismissed it.
#[tauri::command]
fn pick_folder(app: AppHandle, initial: String) -> Option<String> {
    let mut dialog = app.dialog().file().set_title("Escolha a pasta de destino");
    let start = PathBuf::from(initial.trim());
    if start.is_dir() {
        dialog = dialog.set_directory(start);
    }
    dialog
        .blocking_pick_folder()
        .map(|folder| folder.to_string())
}

#[tauri::command]
fn open_folder(folder: String) -> Result<(), String> {
    tools::reveal(&tools::ensure_folder(&folder)?)
}

/// Downloads yt-dlp and FFmpeg into the app's data folder.
///
/// Returns right away and reports progress through the log, because the
/// download runs into tens of megabytes and the window must stay responsive.
#[tauri::command]
fn install_tools(app: AppHandle, force: bool, state: tauri::State<AppState>) -> Result<(), String> {
    if state.installing.swap(true, Ordering::SeqCst) {
        return Err("A instalação já está em andamento.".into());
    }
    let app_data = state.app_data.clone();
    let job = Arc::clone(&state.job);
    std::thread::spawn(move || {
        let log = {
            let job = Arc::clone(&job);
            move |line: String| job.log(line)
        };
        let outcome = tools::install(&app_data, force, &log);
        let state = app.state::<AppState>();
        match outcome {
            Ok(found) => {
                *state.tools.lock().unwrap() = found;
            }
            Err(message) => job.log(format!(">>> ERRO: {message}")),
        }
        state.installing.store(false, Ordering::SeqCst);
        // Tells the window to re-read the status, so the header and the
        // FFmpeg warning reflect what is now installed.
        let _ = app.emit("tools", status_of(&state));
    });
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let app_data = handle.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data)?;

            let default_folder = handle
                .path()
                .download_dir()
                .unwrap_or_else(|_| app_data.clone())
                .join("yt-dlp");
            let config_path = app_data.join("config.json");
            let mut stored = config::load(&config_path);
            if stored.last_folder.trim().is_empty() {
                stored.last_folder = default_folder.to_string_lossy().into_owned();
            }

            let found = tools::discover(&app_data);
            let js_runtime = ytdlp::js_runtime_args();
            let job = Arc::new(Job::new(handle.clone()));

            match &found.ytdlp {
                Some(path) => {
                    let version = tools::ytdlp_version(path)
                        .unwrap_or_else(|| "versão desconhecida".to_string());
                    job.log(format!(">>> yt-dlp {version} pronto."));
                }
                None => job.log(
                    "O yt-dlp ainda não foi baixado. Clique em \"Instalar ferramentas\" para começar.",
                ),
            }
            if found.ffmpeg.is_none() {
                job.log(
                    "AVISO: FFmpeg não encontrado. Ele é necessário para juntar vídeo e áudio em alta resolução e para converter áudio.",
                );
            }
            if !js_runtime.is_empty() {
                job.log(
                    ">>> Runtime JS (Deno) localizado — ajuda o yt-dlp a resolver formatos do YouTube que dependem de assinatura.",
                );
            }

            app.manage(AppState {
                app_data,
                config_path,
                config: Mutex::new(stored),
                tools: Mutex::new(found),
                js_runtime,
                infos: Mutex::new(HashMap::new()),
                job,
                default_folder,
                installing: AtomicBool::new(false),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            status,
            snapshot,
            analyze,
            start_download,
            pause,
            resume,
            cancel,
            pick_folder,
            open_folder,
            install_tools,
        ])
        .run(tauri::generate_context!())
        .expect("erro ao iniciar a janela do aplicativo");
}

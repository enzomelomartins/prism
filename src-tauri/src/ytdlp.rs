//! Everything that talks to the yt-dlp executable: metadata queries, format
//! selection and the command line of a download. No state and no UI here, so
//! each rule can be read (and tested) on its own.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Keeps a console window from flashing on every subprocess on Windows.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Marker that tells the structured `--progress-template` lines apart from the
/// rest of the yt-dlp output, which keeps going to the log as-is.
pub(crate) const PROGRESS_MARKER: &str = "~~YTDLPGUI_PROGRESS~~";

const PROGRESS_TEMPLATE: &str = concat!(
    "download:~~YTDLPGUI_PROGRESS~~",
    "|%(progress.status)s|%(progress.downloaded_bytes)s|%(progress.total_bytes)s",
    "|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s|%(progress.elapsed)s",
);

/// Log prefixes that mean yt-dlp moved on to post-processing (merging, audio
/// extraction, metadata…), used only to update the state shown in the UI.
const POSTPROCESS_TAGS: [&str; 9] = [
    "[Merger]",
    "[VideoRemuxer]",
    "[ExtractAudio]",
    "[Metadata]",
    "[EmbedThumbnail]",
    "[SubtitlesConvertor]",
    "[FixupM3u8]",
    "[FixupM4a]",
    "[ffmpeg]",
];

pub const DEFAULT_PATTERN: &str = "%(title).180B [%(id)s] [%(format_id)s].%(ext)s";

/// The four download modes offered by the interface.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    VideoAudio,
    VideoOnly,
    AudioOnly,
    Separate,
}

// ----------------------------------------------------------------- helpers ---

/// A command that never pops up a console window of its own.
pub fn command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// `Command::output` with a deadline: the std library has none, and a yt-dlp
/// query that hangs would otherwise freeze the analysis forever.
fn output_with_timeout(mut cmd: Command, timeout: Duration) -> Result<Output, String> {
    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("Não foi possível executar o yt-dlp: {err}"))?;
    let pid = child.id();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(child.wait_with_output());
    });
    match receiver.recv_timeout(timeout) {
        Ok(result) => result.map_err(|err| format!("Falha ao ler a saída do yt-dlp: {err}")),
        Err(_) => {
            kill_tree(pid);
            Err("O yt-dlp demorou demais para responder. Verifique sua conexão.".into())
        }
    }
}

/// Ends a process and everything it started (yt-dlp spawns ffmpeg).
pub fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = command(Path::new("taskkill"))
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
}

pub fn human_size(bytes: Option<f64>) -> String {
    let Some(mut value) = bytes.filter(|v| *v > 0.0) else {
        return "desconhecido".to_string();
    };
    for unit in ["B", "KiB", "MiB"] {
        if value < 1024.0 {
            return format!("{value:.1} {unit}");
        }
        value /= 1024.0;
    }
    format!("{value:.1} GiB")
}

pub fn human_speed(speed: Option<f64>) -> String {
    match speed.filter(|v| *v > 0.0) {
        Some(value) => format!("{}/s", human_size(Some(value))),
        None => "-".to_string(),
    }
}

pub fn human_eta(eta: Option<f64>) -> String {
    let Some(value) = eta.filter(|v| *v >= 0.0) else {
        return "-".to_string();
    };
    let total = value as u64;
    let (minutes, seconds) = (total / 60 % 60, total % 60);
    match total / 3600 {
        0 => format!("{minutes:02}:{seconds:02}"),
        hours => format!("{hours:02}:{minutes:02}:{seconds:02}"),
    }
}

fn duration_label(duration: Option<f64>) -> String {
    let Some(value) = duration.filter(|v| *v > 0.0) else {
        return "desconhecida".to_string();
    };
    let total = value as u64;
    let (minutes, seconds) = (total / 60 % 60, total % 60);
    match total / 3600 {
        0 => format!("{minutes:02}:{seconds:02}"),
        hours => format!("{hours:02}:{minutes:02}:{seconds:02}"),
    }
}

// ------------------------------------------------------------- video info ---

/// One downloadable stream, with the derived fields the interface displays
/// computed here so the frontend only has to render them.
#[derive(Clone, Debug, Serialize)]
pub struct VideoFormat {
    pub format_id: String,
    pub ext: String,
    pub resolution: String,
    pub height: Option<i64>,
    pub fps: Option<f64>,
    pub vcodec: String,
    pub acodec: String,
    pub abr: Option<f64>,
    pub tbr: Option<f64>,
    pub filesize: Option<i64>,
    pub filesize_approx: Option<i64>,
    pub format_note: String,
    pub has_video: bool,
    pub has_audio: bool,
    pub size_label: String,
    pub quality_rank: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct VideoInfo {
    pub title: String,
    pub duration: Option<f64>,
    pub duration_label: String,
    pub uploader: String,
    pub thumbnail: Option<String>,
    pub formats: Vec<VideoFormat>,
}

impl VideoInfo {
    pub fn format(&self, format_id: &str) -> Option<&VideoFormat> {
        self.formats.iter().find(|f| f.format_id == format_id)
    }
}

/// Reads a yt-dlp JSON field that may legitimately be `null`.
fn opt_f64(value: &serde_json::Value, key: &str) -> Option<f64> {
    value.get(key).and_then(serde_json::Value::as_f64)
}

fn opt_i64(value: &serde_json::Value, key: &str) -> Option<i64> {
    value.get(key).and_then(serde_json::Value::as_i64)
}

fn text(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn size_label(filesize: Option<i64>, approx: Option<i64>) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    match (filesize, approx) {
        (Some(exact), _) if exact > 0 => format!("{:.1} MB", exact as f64 / MB),
        (_, Some(guess)) if guess > 0 => format!("~{:.1} MB", guess as f64 / MB),
        _ => "desconhecido".to_string(),
    }
}

fn parse_format(raw: &serde_json::Value) -> Option<VideoFormat> {
    let format_id = match raw.get("format_id")? {
        serde_json::Value::String(id) => id.clone(),
        serde_json::Value::Number(id) => id.to_string(),
        _ => return None,
    };
    let height = opt_i64(raw, "height");
    let note = text(raw, "format_note");
    let resolution = match raw.get("resolution").and_then(serde_json::Value::as_str) {
        Some(value) if !value.is_empty() => value.to_string(),
        _ => match height {
            Some(value) => format!("{value}p"),
            None if !note.is_empty() => note.clone(),
            None => "n/d".to_string(),
        },
    };
    let vcodec = match text(raw, "vcodec") {
        value if value.is_empty() => "none".to_string(),
        value => value,
    };
    let acodec = match text(raw, "acodec") {
        value if value.is_empty() => "none".to_string(),
        value => value,
    };
    let filesize = opt_i64(raw, "filesize");
    let filesize_approx = opt_i64(raw, "filesize_approx");
    let (abr, tbr) = (opt_f64(raw, "abr"), opt_f64(raw, "tbr"));
    let ext = match text(raw, "ext") {
        value if value.is_empty() => "?".to_string(),
        value => value,
    };
    Some(VideoFormat {
        has_video: vcodec != "none",
        has_audio: acodec != "none",
        size_label: size_label(filesize, filesize_approx),
        quality_rank: tbr
            .or(abr)
            .unwrap_or_else(|| filesize.unwrap_or(0) as f64 / 1024.0),
        format_id,
        ext,
        resolution,
        height,
        fps: opt_f64(raw, "fps"),
        vcodec,
        acodec,
        abr,
        tbr,
        filesize,
        filesize_approx,
        format_note: note,
    })
}

/// Asks yt-dlp for everything it knows about a link, without downloading.
pub fn fetch_info(ytdlp: &Path, js_runtime: &[String], url: &str) -> Result<VideoInfo, String> {
    let mut cmd = command(ytdlp);
    cmd.args(["--dump-single-json", "--no-warnings", "--no-playlist"])
        .args(js_runtime)
        .args(["--", url]);

    let output = output_with_timeout(cmd, Duration::from_secs(90))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let message = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        return Err(if message.is_empty() {
            "Não foi possível consultar este link.".to_string()
        } else {
            message
        });
    }

    let data: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|err| format!("Resposta inesperada do yt-dlp: {err}"))?;
    let formats: Vec<VideoFormat> = data
        .get("formats")
        .and_then(serde_json::Value::as_array)
        .map(|list| list.iter().filter_map(parse_format).collect())
        .unwrap_or_default();
    if formats.is_empty() {
        return Err("O yt-dlp não retornou nenhum formato para este link.".to_string());
    }

    let duration = opt_f64(&data, "duration");
    let uploader = match text(&data, "uploader") {
        value if value.is_empty() => text(&data, "channel"),
        value => value,
    };
    Ok(VideoInfo {
        title: match text(&data, "title") {
            value if value.is_empty() => "Sem título".to_string(),
            value => value,
        },
        duration,
        duration_label: duration_label(duration),
        uploader,
        thumbnail: data
            .get("thumbnail")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        formats,
    })
}

// --------------------------------------------------------------- download ---

/// The `-f` expression for a mode and the streams picked in the interface.
pub fn format_selector(
    video: Option<&VideoFormat>,
    audio: Option<&VideoFormat>,
    mode: Mode,
) -> Result<String, String> {
    if mode == Mode::AudioOnly {
        return Ok(match audio {
            Some(format) => format.format_id.clone(),
            None => "bestaudio/best".to_string(),
        });
    }
    let video = video.ok_or("Escolha um formato de vídeo.")?;
    Ok(match mode {
        Mode::Separate => {
            let chosen = audio.map_or("bestaudio", |format| format.format_id.as_str());
            format!("{},{}", video.format_id, chosen)
        }
        Mode::VideoOnly => video.format_id.clone(),
        // Video with audio: a muxed stream is already complete; otherwise the
        // best audio track is merged in by FFmpeg.
        _ if video.has_audio => video.format_id.clone(),
        _ => format!("{}+bestaudio/best", video.format_id),
    })
}

pub struct DownloadOptions<'a> {
    pub url: &'a str,
    pub format_selector: &'a str,
    pub folder: &'a Path,
    pub mode: Mode,
    pub container: &'a str,
    pub audio_export_format: &'a str,
    pub embed_metadata: bool,
    pub embed_thumbnail: bool,
    pub write_subs: bool,
    pub sub_langs: &'a str,
    pub filename_pattern: &'a str,
    pub ffmpeg_dir: Option<&'a Path>,
    pub js_runtime: &'a [String],
}

/// The full yt-dlp argument list for a download.
pub fn download_args(options: &DownloadOptions) -> Vec<String> {
    let pattern = match options.filename_pattern.trim() {
        "" => DEFAULT_PATTERN,
        value => value,
    };
    let mut args: Vec<String> = [
        "--no-playlist",
        "--newline",
        "--progress",
        "--progress-template",
        PROGRESS_TEMPLATE,
        "--windows-filenames",
        "--continue",
        "--no-overwrites",
        "-f",
        options.format_selector,
        "-o",
    ]
    .iter()
    .map(|arg| arg.to_string())
    .collect();
    args.push(options.folder.join(pattern).to_string_lossy().into_owned());
    args.extend(options.js_runtime.iter().cloned());

    // A bundled FFmpeg is not on the PATH, so yt-dlp has to be pointed at it.
    if let Some(dir) = options.ffmpeg_dir {
        args.push("--ffmpeg-location".into());
        args.push(dir.to_string_lossy().into_owned());
    }

    match options.mode {
        Mode::AudioOnly if options.audio_export_format != "original" => {
            args.push("--extract-audio".into());
            args.push("--audio-format".into());
            args.push(options.audio_export_format.into());
        }
        Mode::VideoAudio | Mode::VideoOnly if options.container != "original" => {
            args.push("--merge-output-format".into());
            args.push(options.container.into());
            args.push("--remux-video".into());
            args.push(options.container.into());
        }
        _ => {}
    }

    if options.embed_metadata {
        args.push("--embed-metadata".into());
    }
    if options.embed_thumbnail {
        args.push("--embed-thumbnail".into());
    }
    if options.write_subs && options.mode != Mode::AudioOnly {
        args.push("--write-subs".into());
        args.push("--sub-langs".into());
        args.push(options.sub_langs.into());
        args.push("--embed-subs".into());
    }

    args.push("--".into());
    args.push(options.url.into());
    args
}

// --------------------------------------------------------------- progress ---

#[derive(Clone, Debug)]
pub struct ProgressInfo {
    pub status: String,
    pub downloaded_bytes: Option<f64>,
    pub total_bytes: Option<f64>,
    pub total_bytes_estimate: Option<f64>,
    pub speed: Option<f64>,
    pub eta: Option<f64>,
}

impl ProgressInfo {
    pub fn best_total(&self) -> Option<f64> {
        self.total_bytes
            .filter(|v| *v > 0.0)
            .or(self.total_bytes_estimate)
    }

    pub fn fraction(&self) -> Option<f64> {
        let total = self.best_total().filter(|v| *v > 0.0)?;
        let done = self.downloaded_bytes?;
        Some((done / total).clamp(0.0, 1.0))
    }
}

pub fn is_progress_line(line: &str) -> bool {
    line.starts_with(PROGRESS_MARKER)
}

pub fn is_postprocess_line(line: &str) -> bool {
    POSTPROCESS_TAGS.iter().any(|tag| line.starts_with(tag))
}

/// yt-dlp prints "NA" for fields a given stream does not report.
fn parse_number(raw: &str) -> Option<f64> {
    match raw {
        "" | "NA" | "None" => None,
        value => value.parse().ok(),
    }
}

pub fn parse_progress(line: &str) -> Option<ProgressInfo> {
    let parts: Vec<&str> = line.split('|').collect();
    if parts.len() < 8 {
        return None;
    }
    Some(ProgressInfo {
        status: parts[1].to_string(),
        downloaded_bytes: parse_number(parts[2]),
        total_bytes: parse_number(parts[3]),
        total_bytes_estimate: parse_number(parts[4]),
        speed: parse_number(parts[5]),
        eta: parse_number(parts[6]),
    })
}

// ------------------------------------------------------------- js runtime ---

/// Finds a real `deno.exe` to hand to yt-dlp through `--js-runtimes`.
///
/// yt-dlp needs a JS runtime to solve the signature challenge of some YouTube
/// streams; without one, formats can go missing. When Deno was installed
/// through npm, Windows ends up with an extension-less POSIX shim next to the
/// real `deno.cmd`, and yt-dlp's own PATH lookup can match that useless shim
/// first — reporting "no runtime found" on a machine where Deno works fine.
/// Pointing at the actual executable never makes things worse: when nothing is
/// found we simply pass no argument at all.
pub fn js_runtime_args() -> Vec<String> {
    for key in ["APPDATA", "LOCALAPPDATA"] {
        let Some(base) = std::env::var_os(key).map(PathBuf::from) else {
            continue;
        };
        let direct = base.join("npm/node_modules/deno/deno.exe");
        if direct.is_file() {
            return vec!["--js-runtimes".into(), format!("deno:{}", direct.display())];
        }
        // nvm-windows keeps one node_modules tree per installed version.
        let Ok(entries) = std::fs::read_dir(base.join("nvm")) else {
            continue;
        };
        for entry in entries.flatten() {
            let candidate = entry.path().join("node_modules/deno/deno.exe");
            if candidate.is_file() {
                return vec![
                    "--js-runtimes".into(),
                    format!("deno:{}", candidate.display()),
                ];
            }
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(id: &str, video: bool, audio: bool) -> VideoFormat {
        VideoFormat {
            format_id: id.into(),
            ext: "mp4".into(),
            resolution: "1920x1080".into(),
            height: Some(1080),
            fps: Some(60.0),
            vcodec: if video { "avc1".into() } else { "none".into() },
            acodec: if audio { "mp4a".into() } else { "none".into() },
            abr: None,
            tbr: Some(3000.0),
            filesize: Some(10 * 1024 * 1024),
            filesize_approx: None,
            format_note: String::new(),
            has_video: video,
            has_audio: audio,
            size_label: "10.0 MB".into(),
            quality_rank: 3000.0,
        }
    }

    #[test]
    fn muxed_video_needs_no_merge() {
        let muxed = format("18", true, true);
        let selector = format_selector(Some(&muxed), None, Mode::VideoAudio).unwrap();
        assert_eq!(selector, "18");
    }

    #[test]
    fn silent_video_is_merged_with_best_audio() {
        let silent = format("137", true, false);
        let selector = format_selector(Some(&silent), None, Mode::VideoAudio).unwrap();
        assert_eq!(selector, "137+bestaudio/best");
    }

    #[test]
    fn separate_mode_pairs_both_streams() {
        let video = format("137", true, false);
        let audio = format("251", false, true);
        let selector = format_selector(Some(&video), Some(&audio), Mode::Separate).unwrap();
        assert_eq!(selector, "137,251");
    }

    #[test]
    fn audio_mode_ignores_the_video_stream() {
        let video = format("137", true, false);
        let audio = format("251", false, true);
        let selector = format_selector(Some(&video), Some(&audio), Mode::AudioOnly).unwrap();
        assert_eq!(selector, "251");
    }

    #[test]
    fn audio_export_only_converts_when_asked() {
        let folder = PathBuf::from("C:/dl");
        let base = DownloadOptions {
            url: "https://example.com/v",
            format_selector: "251",
            folder: &folder,
            mode: Mode::AudioOnly,
            container: "mp4",
            audio_export_format: "mp3",
            embed_metadata: false,
            embed_thumbnail: false,
            write_subs: false,
            sub_langs: "pt",
            filename_pattern: "%(title)s.%(ext)s",
            ffmpeg_dir: None,
            js_runtime: &[],
        };
        let args = download_args(&base);
        assert!(args.contains(&"--extract-audio".to_string()));

        let untouched = download_args(&DownloadOptions {
            audio_export_format: "original",
            ..base
        });
        assert!(!untouched.contains(&"--extract-audio".to_string()));
    }

    #[test]
    fn subtitles_are_skipped_for_audio_downloads() {
        let folder = PathBuf::from("C:/dl");
        let options = DownloadOptions {
            url: "https://example.com/v",
            format_selector: "251",
            folder: &folder,
            mode: Mode::AudioOnly,
            container: "mp4",
            audio_export_format: "mp3",
            embed_metadata: false,
            embed_thumbnail: false,
            write_subs: true,
            sub_langs: "pt",
            filename_pattern: DEFAULT_PATTERN,
            ffmpeg_dir: None,
            js_runtime: &[],
        };
        assert!(!download_args(&options).contains(&"--write-subs".to_string()));
    }

    #[test]
    fn progress_lines_are_parsed_and_clamped() {
        let line = format!("{PROGRESS_MARKER}|downloading|512|1024|NA|256|30|12");
        assert!(is_progress_line(&line));
        let progress = parse_progress(&line).unwrap();
        assert_eq!(progress.status, "downloading");
        assert_eq!(progress.fraction(), Some(0.5));
        assert_eq!(human_speed(progress.speed), "256.0 B/s");
        assert_eq!(human_eta(progress.eta), "00:30");
    }

    #[test]
    fn missing_totals_leave_the_fraction_unknown() {
        let line = format!("{PROGRESS_MARKER}|downloading|512|NA|NA|NA|NA|1");
        let progress = parse_progress(&line).unwrap();
        assert_eq!(progress.fraction(), None);
        assert_eq!(human_speed(progress.speed), "-");
    }

    #[test]
    fn human_helpers_match_the_interface_wording() {
        assert_eq!(human_size(None), "desconhecido");
        assert_eq!(human_size(Some(0.0)), "desconhecido");
        assert_eq!(human_size(Some(1536.0)), "1.5 KiB");
        assert_eq!(human_eta(Some(3661.0)), "01:01:01");
        assert_eq!(duration_label(Some(75.0)), "01:15");
    }
}

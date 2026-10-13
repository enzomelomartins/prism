//! The one download in flight: its state, its log, and the events the window
//! is kept up to date with.
//!
//! Only one download runs at a time, which is what the interface offers, so a
//! single guarded record is enough. Every change is pushed to the frontend as
//! an event, and a fresh window asks for a `snapshot` to catch up — that is
//! what lets a reopened window pick a running download back up.

use serde::Serialize;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

use crate::ytdlp;

/// Where a job's events go. The application sends them to the window; the
/// tests collect them, which is what makes the whole download pipeline —
/// spawning, reading, parsing, cancelling — testable without a window.
pub trait EventSink: Send + Sync + 'static {
    fn send(&self, event: JobEvent);
}

impl EventSink for AppHandle {
    fn send(&self, event: JobEvent) {
        let _ = self.emit(EVENT, event);
    }
}

/// How many log lines are kept; the rest scrolls out of history.
const LOG_LIMIT: usize = 500;

/// Progress arrives far faster than anyone can read it, so it is forwarded at
/// most this often while downloading.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// The channel the frontend listens on.
pub const EVENT: &str = "job";

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    Idle,
    Downloading,
    Merging,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl JobState {
    /// States in which a download is still going on and can be acted upon.
    fn is_active(self) -> bool {
        matches!(self, Self::Downloading | Self::Merging | Self::Paused)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Progress {
    pub fraction: Option<f64>,
    pub size: String,
    pub speed: String,
    pub eta: String,
}

impl Progress {
    fn starting() -> Self {
        Self {
            fraction: Some(0.0),
            size: "-- / --".into(),
            speed: "--".into(),
            eta: "--".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum JobEvent {
    Snapshot {
        state: JobState,
        title: String,
        progress: Option<Progress>,
        log: Vec<String>,
    },
    State {
        state: JobState,
        title: String,
    },
    /// Flattened on purpose: the frontend reads the same shape as `Progress`.
    Progress {
        fraction: Option<f64>,
        size: String,
        speed: String,
        eta: String,
    },
    Log {
        line: String,
    },
}

struct Inner {
    state: JobState,
    title: String,
    progress: Option<Progress>,
    log: VecDeque<String>,
    child: Option<Child>,
    /// Set by `cancel` so the reader thread reports a cancellation rather than
    /// the failure exit code that killing yt-dlp produces.
    cancelled: bool,
    last_progress: Option<Instant>,
}

pub struct Job {
    inner: Mutex<Inner>,
    sink: Box<dyn EventSink>,
}

impl Job {
    pub fn new(sink: impl EventSink) -> Self {
        Self {
            sink: Box::new(sink),
            inner: Mutex::new(Inner {
                state: JobState::Idle,
                title: String::new(),
                progress: None,
                log: VecDeque::new(),
                child: None,
                cancelled: false,
                last_progress: None,
            }),
        }
    }

    fn emit(&self, event: JobEvent) {
        self.sink.send(event);
    }

    pub fn log(&self, line: impl Into<String>) {
        let line = line.into();
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.log.len() == LOG_LIMIT {
                inner.log.pop_front();
            }
            inner.log.push_back(line.clone());
        }
        self.emit(JobEvent::Log { line });
    }

    /// The current state and log, for a window that just opened.
    pub fn snapshot(&self) -> JobEvent {
        let inner = self.inner.lock().unwrap();
        JobEvent::Snapshot {
            state: inner.state,
            title: inner.title.clone(),
            progress: inner.progress.clone(),
            log: inner.log.iter().cloned().collect(),
        }
    }

    pub fn state(&self) -> JobState {
        self.inner.lock().unwrap().state
    }

    fn set_state(&self, state: JobState) {
        let title = {
            let mut inner = self.inner.lock().unwrap();
            if inner.state == state {
                return;
            }
            inner.state = state;
            inner.title.clone()
        };
        self.emit(JobEvent::State { state, title });
    }

    fn set_progress(&self, progress: Progress) {
        self.inner.lock().unwrap().progress = Some(progress.clone());
        self.emit(JobEvent::Progress {
            fraction: progress.fraction,
            size: progress.size,
            speed: progress.speed,
            eta: progress.eta,
        });
    }

    // ------------------------------------------------------------- start ---

    /// Spawns yt-dlp and starts reading its output on a background thread.
    pub fn start(
        self: &Arc<Self>,
        ytdlp_path: &Path,
        args: &[String],
        title: String,
    ) -> Result<(), String> {
        if self.state().is_active() {
            return Err("Já existe um download em andamento.".into());
        }

        let mut command = ytdlp::command(ytdlp_path);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            // yt-dlp reports problems on stderr; both belong in the same log.
            .stderr(Stdio::piped());

        let mut child = command
            .spawn()
            .map_err(|err| format!("Falha ao iniciar o yt-dlp: {err}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or("Não foi possível ler o yt-dlp.")?;
        let stderr = child.stderr.take();

        {
            let mut inner = self.inner.lock().unwrap();
            inner.title = title;
            inner.cancelled = false;
            inner.last_progress = None;
            inner.child = Some(child);
        }
        self.set_progress(Progress::starting());
        self.set_state(JobState::Downloading);

        if let Some(stderr) = stderr {
            let job = Arc::clone(self);
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if !line.trim().is_empty() {
                        job.log(line);
                    }
                }
            });
        }

        let job = Arc::clone(self);
        std::thread::spawn(move || job.read_output(stdout));
        Ok(())
    }

    fn read_output(self: Arc<Self>, stdout: std::process::ChildStdout) {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let line = line.trim_end();
            if line.is_empty() {
                continue;
            }
            if ytdlp::is_progress_line(line) {
                self.handle_progress(line);
            } else {
                self.log(line);
                if ytdlp::is_postprocess_line(line) {
                    // Merging and converting report no progress of their own.
                    self.set_state(JobState::Merging);
                }
            }
        }
        self.finish();
    }

    fn handle_progress(&self, line: &str) {
        let Some(info) = ytdlp::parse_progress(line) else {
            return;
        };
        let downloading = info.status == "downloading";
        let fraction = {
            let mut inner = self.inner.lock().unwrap();
            if inner.state == JobState::Paused {
                return;
            }
            let now = Instant::now();
            if downloading
                && inner
                    .last_progress
                    .is_some_and(|last| now.duration_since(last) < PROGRESS_INTERVAL)
            {
                return;
            }
            inner.last_progress = Some(now);
            // A chunk with no known total should not reset the bar to zero.
            info.fraction()
                .or_else(|| inner.progress.as_ref().and_then(|p| p.fraction))
        };

        self.set_progress(Progress {
            fraction,
            size: format!(
                "{} / {}",
                ytdlp::human_size(info.downloaded_bytes),
                ytdlp::human_size(info.best_total())
            ),
            speed: ytdlp::human_speed(info.speed),
            eta: ytdlp::human_eta(info.eta),
        });
        if downloading {
            self.set_state(JobState::Downloading);
        }
    }

    /// Called once yt-dlp has closed its output: decides how it ended.
    fn finish(&self) {
        let (child, cancelled) = {
            let mut inner = self.inner.lock().unwrap();
            (inner.child.take(), inner.cancelled)
        };
        // stdout is closed, so the process has already exited and this returns
        // immediately.
        let code = child
            .and_then(|mut child| child.wait().ok())
            .and_then(|status| status.code());

        if cancelled {
            self.log(">>> Download cancelado.");
            self.set_state(JobState::Cancelled);
            return;
        }
        match code {
            Some(0) => {
                let size = {
                    let inner = self.inner.lock().unwrap();
                    inner
                        .progress
                        .as_ref()
                        .map_or_else(|| "-- / --".to_string(), |p| p.size.clone())
                };
                self.set_progress(Progress {
                    fraction: Some(1.0),
                    size,
                    speed: "-".into(),
                    eta: "-".into(),
                });
                self.log(">>> Download concluído com sucesso.");
                self.set_state(JobState::Completed);
            }
            Some(code) => {
                self.log(format!(">>> ERRO: o yt-dlp terminou com código {code}."));
                self.set_state(JobState::Failed);
            }
            None => {
                self.log(">>> ERRO: o yt-dlp foi encerrado inesperadamente.");
                self.set_state(JobState::Failed);
            }
        }
    }

    // ------------------------------------------------------ pause / stop ---

    pub fn pause(&self) -> Result<(), String> {
        if self.state() != JobState::Downloading {
            return Err("Nenhum download em andamento para pausar.".into());
        }
        {
            let inner = self.inner.lock().unwrap();
            let child = inner.child.as_ref().ok_or("O download já terminou.")?;
            suspend(child, true)?;
        }
        self.log(">>> Download pausado.");
        self.set_state(JobState::Paused);
        Ok(())
    }

    pub fn resume(&self) -> Result<(), String> {
        if self.state() != JobState::Paused {
            return Err("Nenhum download pausado para continuar.".into());
        }
        {
            let inner = self.inner.lock().unwrap();
            let child = inner.child.as_ref().ok_or("O download já terminou.")?;
            suspend(child, false)?;
        }
        self.log(">>> Download retomado.");
        self.set_state(JobState::Downloading);
        Ok(())
    }

    pub fn cancel(&self) -> Result<(), String> {
        let pid = {
            let mut inner = self.inner.lock().unwrap();
            if !inner.state.is_active() {
                return Err("Nenhum download em andamento para cancelar.".into());
            }
            inner.cancelled = true;
            let child = inner.child.as_ref().ok_or("O download já terminou.")?;
            // A paused process would never notice it was asked to stop.
            if inner.state == JobState::Paused {
                let _ = suspend(child, false);
            }
            child.id()
        };
        // Takes ffmpeg and the other children with it; the reader thread then
        // sees stdout close and reports the cancellation.
        ytdlp::kill_tree(pid);
        Ok(())
    }
}

/// Suspends or resumes a running process.
///
/// `NtSuspendProcess` is the only way on Windows to freeze a process without
/// its cooperation, which is what pausing a download means here: the TCP
/// connection stays open and yt-dlp carries on where it left off. It is also
/// why pausing is offered while downloading but not while merging — the pause
/// stops yt-dlp itself, not the ffmpeg it spawns.
#[cfg(windows)]
fn suspend(child: &Child, pause: bool) -> Result<(), String> {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;

    #[link(name = "ntdll")]
    extern "system" {
        fn NtSuspendProcess(handle: *mut c_void) -> i32;
        fn NtResumeProcess(handle: *mut c_void) -> i32;
    }

    let handle = child.as_raw_handle();
    // Safety: the handle belongs to the `Child` we are borrowing, so it stays
    // valid for the duration of the call.
    let status = unsafe {
        if pause {
            NtSuspendProcess(handle)
        } else {
            NtResumeProcess(handle)
        }
    };
    if status == 0 {
        Ok(())
    } else if pause {
        Err("Não foi possível pausar o download.".into())
    } else {
        Err("Não foi possível retomar o download.".into())
    }
}

#[cfg(not(windows))]
fn suspend(child: &Child, pause: bool) -> Result<(), String> {
    let signal = if pause { "-STOP" } else { "-CONT" };
    std::process::Command::new("kill")
        .args([signal, &child.id().to_string()])
        .status()
        .map_err(|err| format!("Não foi possível pausar o download: {err}"))?;
    Ok(())
}

/// These exercise the whole pipeline against a stand-in for yt-dlp: a batch
/// file that prints the same progress lines. That covers spawning, reading,
/// parsing, the state machine, pausing and cancelling without touching the
/// network.
#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::ytdlp::PROGRESS_MARKER;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[derive(Clone)]
    struct Collected(Arc<Mutex<Vec<JobEvent>>>);

    impl EventSink for Collected {
        fn send(&self, event: JobEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    /// A fake yt-dlp. Batch needs `|` escaped, hence the `^|` in the callers.
    fn fake_ytdlp(body: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "prism-test-{}-{}.bat",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::write(&path, body).unwrap();
        path
    }

    fn start(body: &str) -> (Arc<Job>, Arc<Mutex<Vec<JobEvent>>>, PathBuf) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let job = Arc::new(Job::new(Collected(Arc::clone(&events))));
        let script = fake_ytdlp(body);
        let shell = PathBuf::from(std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into()));
        let args = vec!["/c".to_string(), script.to_string_lossy().into_owned()];
        job.start(&shell, &args, "Vídeo de teste".into()).unwrap();
        (job, events, script)
    }

    fn wait_for(job: &Job, state: JobState) -> bool {
        for _ in 0..400 {
            if job.state() == state {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    }

    fn logs(events: &Arc<Mutex<Vec<JobEvent>>>) -> Vec<String> {
        events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                JobEvent::Log { line } => Some(line.clone()),
                _ => None,
            })
            .collect()
    }

    fn states(events: &Arc<Mutex<Vec<JobEvent>>>) -> Vec<JobState> {
        events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                JobEvent::State { state, .. } => Some(*state),
                _ => None,
            })
            .collect()
    }

    fn fractions(events: &Arc<Mutex<Vec<JobEvent>>>) -> Vec<Option<f64>> {
        events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                JobEvent::Progress { fraction, .. } => Some(*fraction),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_finished_download_reports_progress_merging_and_completion() {
        let (job, events, script) = start(&format!(
            "@echo off\r\n\
             echo {PROGRESS_MARKER}^|downloading^|512^|1024^|NA^|256^|30^|1\r\n\
             echo [download] Destination: teste.mp4\r\n\
             echo [Merger] Merging formats into \"teste.mp4\"\r\n\
             exit /b 0\r\n"
        ));
        assert!(wait_for(&job, JobState::Completed), "não concluiu");

        assert!(
            fractions(&events).contains(&Some(0.5)),
            "progresso não chegou"
        );
        // The bar is filled in on success, whatever the last chunk reported.
        assert_eq!(fractions(&events).last().copied().flatten(), Some(1.0));
        assert!(
            states(&events).contains(&JobState::Merging),
            "mesclagem não detectada"
        );
        assert_eq!(states(&events).last(), Some(&JobState::Completed));

        let log = logs(&events);
        assert!(log
            .iter()
            .any(|line| line.contains("Destination: teste.mp4")));
        assert!(log
            .iter()
            .any(|line| line.contains("concluído com sucesso")));
        // Progress lines are structured data, never log noise.
        assert!(!log.iter().any(|line| line.contains(PROGRESS_MARKER)));

        let snapshot = job.snapshot();
        match snapshot {
            JobEvent::Snapshot { state, title, .. } => {
                assert_eq!(state, JobState::Completed);
                assert_eq!(title, "Vídeo de teste");
            }
            _ => panic!("snapshot inesperado"),
        }
        let _ = std::fs::remove_file(script);
    }

    #[test]
    fn a_download_can_be_paused_resumed_and_cancelled() {
        let (job, events, script) =
            start("@echo off\r\nping -n 30 127.0.0.1 >nul\r\nexit /b 0\r\n");
        assert_eq!(job.state(), JobState::Downloading);

        job.pause().unwrap();
        assert_eq!(job.state(), JobState::Paused);
        assert!(job.pause().is_err(), "pausar duas vezes deveria falhar");

        job.resume().unwrap();
        assert_eq!(job.state(), JobState::Downloading);

        job.cancel().unwrap();
        assert!(wait_for(&job, JobState::Cancelled), "não cancelou");
        assert!(logs(&events).iter().any(|line| line.contains("cancelado")));
        assert!(
            job.cancel().is_err(),
            "cancelar sem download deveria falhar"
        );
        let _ = std::fs::remove_file(script);
    }

    #[test]
    fn a_failing_yt_dlp_surfaces_its_exit_code() {
        let (job, events, script) =
            start("@echo off\r\necho ERROR: link indisponivel\r\nexit /b 2\r\n");
        assert!(wait_for(&job, JobState::Failed), "não falhou");
        let log = logs(&events);
        assert!(log.iter().any(|line| line.contains("link indisponivel")));
        assert!(log.iter().any(|line| line.contains("código 2")));
        let _ = std::fs::remove_file(script);
    }

    #[test]
    fn only_one_download_runs_at_a_time() {
        let (job, _events, script) =
            start("@echo off\r\nping -n 30 127.0.0.1 >nul\r\nexit /b 0\r\n");
        let shell = PathBuf::from(std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into()));
        let second = job.start(&shell, &["/c".into(), "exit".into()], "Outro".into());
        assert!(second.is_err(), "um segundo download não deveria iniciar");
        job.cancel().unwrap();
        assert!(wait_for(&job, JobState::Cancelled));
        let _ = std::fs::remove_file(script);
    }
}

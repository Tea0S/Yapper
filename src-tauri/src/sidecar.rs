use crate::trace_log::ipc_log;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use std::time::Duration;

/// Decoding options for faster-whisper (sent on `init`; sidecar may ignore for non-Whisper engines).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WhisperDecodeOptions {
    pub beam_size: i32,
    pub best_of: i32,
    pub patience: f64,
    pub temperature: f64,
    pub no_speech_threshold: f64,
    pub log_prob_threshold: f64,
    pub compression_ratio_threshold: f64,
    pub hallucination_silence_threshold: f64,
    pub condition_on_previous_text: bool,
    pub initial_prompt: String,
    /// Empty string → auto language detection in the sidecar.
    pub language: String,
    /// Silero VAD inside faster-whisper on live chunks (Silero v6; Rust energy gate runs first).
    pub vad_filter_pcm: bool,
    /// VAD for file transcription (usually helpful on files).
    pub vad_filter_file: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SidecarIn {
    Init {
        model: String,
        device: String,
        compute_type: String,
        model_dir: Option<String>,
        mock: bool,
        #[serde(default)]
        engine: String,
        /// If true, sidecar starts without loading weights; first `EnsureModel` loads (saves VRAM).
        #[serde(default)]
        lazy_load: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        whisper: Option<WhisperDecodeOptions>,
    },
    Chunk {
        seq: u64,
        sample_rate: u32,
        audio_b64: String,
        is_final: bool,
    },
    TranscribeFile {
        path: String,
    },
    /// Drop Whisper weights; process stays alive. Next `EnsureModel` reloads from disk cache.
    UnloadModel,
    /// Load weights if unloaded (no-op if already loaded).
    EnsureModel,
    StartStream {
        session_id: u64,
        engine: String,
        model: String,
    },
    FeedAudio {
        session_id: u64,
        sample_rate: u32,
        audio_b64: String,
        engine: String,
    },
    EndStream {
        session_id: u64,
        engine: String,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SidecarOut {
    Ready {
        engines: Vec<String>,
        #[serde(default)]
        inference_device: Option<String>,
        #[serde(default)]
        compute_type: Option<String>,
    },
    Partial { text: String, seq: u64 },
    Final { text: String, seq: u64, rtf: Option<f64> },
    StreamStarted { session_id: u64 },
    StreamPartial {
        session_id: u64,
        text: String,
        #[serde(default)]
        is_stable: bool,
    },
    StreamFinal { session_id: u64, text: String },
    Error { message: String },
    FileProgress { path: String, percent: f32 },
    FileStarted {
        path: String,
        #[serde(default)]
        duration_secs: Option<f64>,
    },
    FileDone { path: String, text: String },
    ModelState { loaded: bool },
}

/// Remove the first `Partial` for `seq`, or the first `Error`. Other events stay queued.
pub(crate) fn pop_sidecar_partial_for_seq(
    q: &mut VecDeque<SidecarOut>,
    seq: u64,
) -> Result<Option<String>, String> {
    let mut i = 0usize;
    while i < q.len() {
        match &q[i] {
            SidecarOut::Error { message } => {
                let msg = message.clone();
                q.remove(i);
                ipc_log(format!(
                    "pop_partial: taking error from queue: {}",
                    msg.chars().take(120).collect::<String>()
                ));
                return Err(msg);
            }
            SidecarOut::Partial {
                text,
                seq: s,
            } if *s == seq => {
                let t = text.clone();
                q.remove(i);
                ipc_log(format!(
                    "pop_partial: matched seq={seq}, text_chars={}",
                    t.len()
                ));
                return Ok(Some(t));
            }
            _ => i += 1,
        }
    }
    Ok(None)
}

/// Remove the first `StreamPartial` or `StreamFinal` for `session_id`.
pub(crate) fn pop_sidecar_stream_for_session(
    q: &mut VecDeque<SidecarOut>,
    session_id: u64,
) -> Result<Option<(String, bool)>, String> {
    let mut i = 0usize;
    while i < q.len() {
        match &q[i] {
            SidecarOut::Error { message } => {
                let msg = message.clone();
                q.remove(i);
                return Err(msg);
            }
            SidecarOut::StreamFinal {
                session_id: sid,
                text,
            } if *sid == session_id => {
                let t = text.clone();
                q.remove(i);
                return Ok(Some((t, true)));
            }
            SidecarOut::StreamPartial {
                session_id: sid,
                text,
                ..
            } if *sid == session_id => {
                let t = text.clone();
                q.remove(i);
                return Ok(Some((t, false)));
            }
            _ => i += 1,
        }
    }
    Ok(None)
}

/// Wait until `StreamStarted` for `session_id` is seen (and removed).
pub(crate) fn take_stream_started_for_session(
    q: &mut VecDeque<SidecarOut>,
    session_id: u64,
) -> bool {
    let mut i = 0usize;
    while i < q.len() {
        if let SidecarOut::StreamStarted {
            session_id: sid,
        } = &q[i]
        {
            if *sid == session_id {
                q.remove(i);
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Remove the first `Final` for `seq`, or the first `Error`. Other events stay queued.
pub(crate) fn pop_sidecar_transcript_for_seq(
    q: &mut VecDeque<SidecarOut>,
    seq: u64,
) -> Result<Option<String>, String> {
    let mut i = 0usize;
    while i < q.len() {
        match &q[i] {
            SidecarOut::Error { message } => {
                let msg = message.clone();
                q.remove(i);
                ipc_log(format!("pop: taking error from queue: {}", msg.chars().take(120).collect::<String>()));
                return Err(msg);
            }
            SidecarOut::Final {
                text,
                seq: s,
                ..
            } if *s != seq => {
                ipc_log(format!(
                    "pop: skipping final seq={s} (waiting for {seq}), text_chars={}",
                    text.len()
                ));
                i += 1;
            }
            SidecarOut::Final { text, .. } => {
                let t = text.clone();
                q.remove(i);
                ipc_log(format!("pop: matched final seq={seq}, text_chars={}", t.len()));
                return Ok(Some(t));
            }
            _ => i += 1,
        }
    }
    Ok(None)
}

/// Remove the first `FileStarted` for `path`.
/// - `None` — event not seen yet
/// - `Some(None)` — seen, duration unknown
/// - `Some(Some(d))` — seen with duration hint
pub(crate) fn take_file_started_for_path(
    q: &mut VecDeque<SidecarOut>,
    path: &str,
) -> Option<Option<f64>> {
    let mut i = 0usize;
    while i < q.len() {
        if let SidecarOut::FileStarted {
            path: p,
            duration_secs,
        } = &q[i]
        {
            if p == path {
                let d = *duration_secs;
                q.remove(i);
                return Some(d);
            }
        }
        i += 1;
    }
    None
}

/// Remove all `FileProgress` for `path` and return the highest percent seen.
pub(crate) fn take_file_progress_for_path(
    q: &mut VecDeque<SidecarOut>,
    path: &str,
) -> Option<f32> {
    let mut best: Option<f32> = None;
    let mut i = 0usize;
    while i < q.len() {
        if let SidecarOut::FileProgress {
            path: p,
            percent,
        } = &q[i]
        {
            if p == path {
                best = Some(best.map_or(*percent, |b| b.max(*percent)));
                q.remove(i);
                continue;
            }
        }
        i += 1;
    }
    best
}

/// Like transcript pop: first `Error` fails the wait; remove matching `FileDone` or drop matching `FileProgress`.
pub(crate) fn pop_sidecar_file_done_for_path(
    q: &mut VecDeque<SidecarOut>,
    path: &str,
) -> Result<Option<String>, String> {
    let mut i = 0usize;
    while i < q.len() {
        match &q[i] {
            SidecarOut::Error { message } => {
                let m = message.clone();
                q.remove(i);
                return Err(m);
            }
            SidecarOut::FileDone {
                path: p,
                text,
            } if p == path => {
                let t = text.clone();
                q.remove(i);
                return Ok(Some(t));
            }
            SidecarOut::FileProgress { path: p, .. } if p == path => {
                q.remove(i);
            }
            SidecarOut::FileStarted { path: p, .. } if p == path => {
                q.remove(i);
            }
            _ => i += 1,
        }
    }
    Ok(None)
}

pub(crate) fn sidecar_out_one_liner(m: &SidecarOut) -> String {
    match m {
        SidecarOut::Ready {
            engines,
            inference_device,
            ..
        } => format!(
            "ready(engines={}, dev={:?})",
            engines.len(),
            inference_device
        ),
        SidecarOut::Partial { seq, text } => format!("partial(seq={seq}, chars={})", text.len()),
        SidecarOut::Final { seq, text, .. } => format!("final(seq={seq}, chars={})", text.len()),
        SidecarOut::StreamStarted { session_id } => format!("stream_started(id={session_id})"),
        SidecarOut::StreamPartial {
            session_id,
            text,
            ..
        } => format!("stream_partial(id={session_id}, chars={})", text.len()),
        SidecarOut::StreamFinal {
            session_id,
            text,
        } => format!("stream_final(id={session_id}, chars={})", text.len()),
        SidecarOut::Error { message } => {
            let short: String = message.chars().take(100).collect();
            format!("error({short})")
        }
        SidecarOut::FileProgress { path, percent } => {
            format!("file_progress({percent:.0}% path_len={})", path.len())
        }
        SidecarOut::FileStarted {
            path,
            duration_secs,
        } => format!(
            "file_started(duration_secs={duration_secs:?}, path_len={})",
            path.len()
        ),
        SidecarOut::FileDone { path, text } => {
            format!("file_done(chars={}, path_len={})", text.len(), path.len())
        }
        SidecarOut::ModelState { loaded } => format!("model_state(loaded={loaded})"),
    }
}

/// Optional environment merged into the sidecar process (NVIDIA runtime search path).
#[derive(Default, Clone)]
pub struct SidecarSpawnEnv {
    /// Windows: prepend to `PATH` (folder containing cuBLAS/cuDNN DLLs).
    pub path_prepend_windows: Option<PathBuf>,
    /// Unix: prepend to `LD_LIBRARY_PATH` (e.g. pip `nvidia-*-cu12` lib dirs).
    pub ld_library_path_prepend_unix: Option<String>,
}

pub struct SidecarSession {
    #[allow(dead_code)]
    pub child: Child,
    writer: Arc<Mutex<tokio::process::ChildStdin>>,
    pub pending: Arc<Mutex<VecDeque<SidecarOut>>>,
}

impl SidecarSession {
    pub async fn spawn(
        python: &str,
        script: PathBuf,
        env: Option<SidecarSpawnEnv>,
    ) -> Result<Self, String> {
        if !script.exists() {
            return Err(format!("Sidecar script not found: {}", script.display()));
        }
        let mut cmd = Command::new(python);
        // Embeddable CPython (Windows release bundle) does not put the script directory on
        // sys.path; server.py inserts it, but set PYTHONPATH for non-embeddable fallbacks.
        if let Some(parent) = script.parent() {
            let sidecar_dir = parent.to_string_lossy();
            #[cfg(windows)]
            let sep = ';';
            #[cfg(not(windows))]
            let sep = ':';
            let merged = match std::env::var("PYTHONPATH") {
                Ok(old) if !old.is_empty() => format!("{sidecar_dir}{sep}{old}"),
                _ => sidecar_dir.into_owned(),
            };
            cmd.env("PYTHONPATH", merged);
        }
        cmd.arg(script.as_os_str())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .env("PYTHONUNBUFFERED", "1");
        if let Ok(v) = std::env::var("YAPPER_VERBOSE") {
            cmd.env("YAPPER_VERBOSE", v);
        }
        if let Some(e) = env {
            if cfg!(windows) {
                if let Some(bin) = e.path_prepend_windows {
                    use std::ffi::OsString;
                    let old = std::env::var_os("PATH").unwrap_or_default();
                    let mut new = OsString::from(bin.as_os_str());
                    new.push(";");
                    new.push(old);
                    cmd.env("PATH", new);
                }
            }
            if cfg!(unix) {
                if let Some(fragment) = e.ld_library_path_prepend_unix {
                    if cfg!(target_os = "macos") {
                        let old = std::env::var("DYLD_LIBRARY_PATH").unwrap_or_default();
                        let merged = if old.is_empty() {
                            fragment
                        } else {
                            format!("{fragment}:{old}")
                        };
                        cmd.env("DYLD_LIBRARY_PATH", merged);
                    } else {
                        let old = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
                        let merged = if old.is_empty() {
                            fragment
                        } else {
                            format!("{fragment}:{old}")
                        };
                        cmd.env("LD_LIBRARY_PATH", merged);
                    }
                }
            }
        }
        crate::win_spawn::hide_console_tokio_python(&mut cmd, python);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("spawn sidecar: {e}"))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "no stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "no stdout".to_string())?;
        // If stderr is piped but never read, the sidecar deadlocks once the (small) pipe buffer fills
        // (common with torch/CUDA import warnings on Windows).
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "no stderr".to_string())?;

        let pending = Arc::new(Mutex::new(VecDeque::new()));
        let pending_reader = Arc::clone(&pending);
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                match serde_json::from_str::<SidecarOut>(&line) {
                    Ok(msg) => {
                        ipc_log(format!(
                            "stdout parsed {} (line_len={})",
                            sidecar_out_one_liner(&msg),
                            line.len()
                        ));
                        let mut q = pending_reader.lock().await;
                        q.push_back(msg);
                    }
                    Err(e) if !line.trim().is_empty() => {
                        let preview: String = line.chars().take(240).collect();
                        eprintln!(
                            "[yapper-sidecar] JSON parse error (sidecar stdout): {e}; len={} preview={preview}",
                            line.len()
                        );
                    }
                    Err(_) => {}
                }
            }
        });
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                eprintln!("[yapper-sidecar] {line}");
            }
        });

        Ok(Self {
            child,
            writer: Arc::new(Mutex::new(stdin)),
            pending,
        })
    }

    fn map_pipe_write_err(e: io::Error) -> String {
        #[cfg(windows)]
        if e.raw_os_error() == Some(232) {
            return "The inference process closed its input pipe (often it exited or crashed). Restart the engine from Home or Settings.".into();
        }
        e.to_string()
    }

    pub async fn send(&self, msg: &SidecarIn) -> Result<(), String> {
        let line = serde_json::to_string(msg).map_err(|e| e.to_string())?;
        let mut w = self.writer.lock().await;
        tokio::time::timeout(Duration::from_secs(120), async {
            w.write_all(line.as_bytes())
                .await
                .map_err(Self::map_pipe_write_err)?;
            w.write_all(b"\n")
                .await
                .map_err(Self::map_pipe_write_err)?;
            w.flush().await.map_err(Self::map_pipe_write_err)?;
            Ok::<(), String>(())
        })
        .await
        .map_err(|_| {
            "Timed out writing to sidecar (it may still be loading the model — wait for \"Engine running\" after a full load, then retry)."
                .to_string()
        })??;
        Ok(())
    }

    /// Remove the first `Error` from the queue (leaves other events). Used to fail fast on init.
    pub async fn take_first_error(&self) -> Option<String> {
        let mut q = self.pending.lock().await;
        let mut i = 0usize;
        while i < q.len() {
            if let SidecarOut::Error { message } = &q[i] {
                let m = message.clone();
                q.remove(i);
                return Some(m);
            }
            i += 1;
        }
        None
    }

    pub async fn has_ready_event(&self) -> bool {
        let q = self.pending.lock().await;
        q.iter()
            .any(|m| matches!(m, SidecarOut::Ready { .. }))
    }

    /// Removes only `ModelState` and `Error` messages. **Does not** remove `Final` / `Partial` / file events
    /// — the old `drain_ready` popped the entire queue and could steal chunk transcripts mid-flight.
    pub async fn take_model_load_events(&self) -> Vec<SidecarOut> {
        let mut out = Vec::new();
        let mut q = self.pending.lock().await;
        let mut i = 0usize;
        while i < q.len() {
            let take = matches!(
                &q[i],
                SidecarOut::ModelState { .. } | SidecarOut::Error { .. }
            );
            if take {
                out.push(q.remove(i).expect("in-bounds deque remove"));
            } else {
                i += 1;
            }
        }
        out
    }

    /// Removes the first `Ready` from the queue and returns its device metadata; keeps order of other events.
    pub async fn take_ready_metadata(&self) -> Option<(Option<String>, Option<String>)> {
        let mut q = self.pending.lock().await;
        let mut kept = VecDeque::new();
        let mut meta = None;
        while let Some(m) = q.pop_front() {
            match m {
                SidecarOut::Ready {
                    inference_device,
                    compute_type,
                    ..
                } if meta.is_none() => {
                    meta = Some((inference_device, compute_type));
                }
                other => kept.push_back(other),
            }
        }
        *q = kept;
        meta
    }

    pub async fn pop_stream_for_session(
        &self,
        session_id: u64,
    ) -> Result<Option<(String, bool)>, String> {
        let mut q = self.pending.lock().await;
        pop_sidecar_stream_for_session(&mut q, session_id)
    }

    pub async fn pop_transcript_for_seq(&self, seq: u64) -> Result<Option<String>, String> {
        let mut q = self.pending.lock().await;
        pop_sidecar_transcript_for_seq(&mut q, seq)
    }

    pub async fn pop_partial_for_seq(&self, seq: u64) -> Result<Option<String>, String> {
        let mut q = self.pending.lock().await;
        pop_sidecar_partial_for_seq(&mut q, seq)
    }

    pub async fn pop_file_done_for_path(&self, path: &str) -> Result<Option<String>, String> {
        let mut q = self.pending.lock().await;
        pop_sidecar_file_done_for_path(&mut q, path)
    }

    pub async fn take_file_started_for_path(&self, path: &str) -> Option<Option<f64>> {
        let mut q = self.pending.lock().await;
        take_file_started_for_path(&mut q, path)
    }

    pub async fn take_file_progress_for_path(&self, path: &str) -> Option<f32> {
        let mut q = self.pending.lock().await;
        take_file_progress_for_path(&mut q, path)
    }

    pub async fn pending_debug_line(&self) -> String {
        let q = self.pending.lock().await;
        if q.is_empty() {
            return "(queue empty)".into();
        }
        let parts: Vec<String> = q.iter().map(sidecar_out_one_liner).collect();
        parts.join(" | ")
    }

    pub async fn pending_len(&self) -> usize {
        self.pending.lock().await.len()
    }
}

/// Interpreter for the sidecar and Yapper Node.
/// Order: `YAPPER_PYTHON`, bundled embeddable runtime (release layout), then Windows `py` launcher / `python`.
pub fn python_executable(app: &tauri::AppHandle) -> String {
    if let Ok(p) = std::env::var("YAPPER_PYTHON") {
        let t = p.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    if let Some(p) = crate::paths::bundled_python_exe(app) {
        return p.to_string_lossy().into_owned();
    }
    system_python_fallback()
}

fn system_python_fallback() -> String {
    #[cfg(windows)]
    {
        for ver in ["-3.12", "-3.11", "-3.10"] {
            let mut py_cmd = std::process::Command::new("py");
            crate::win_spawn::hide_console(&mut py_cmd);
            let out = py_cmd
                .args([ver, "-c", "import sys; print(sys.executable)"])
                .output();
            if let Ok(o) = out {
                if o.status.success() {
                    let s = String::from_utf8_lossy(&o.stdout);
                    let exe = s.trim();
                    if !exe.is_empty() {
                        return exe.to_string();
                    }
                }
            }
        }
    }
    #[cfg(not(windows))]
    {
        for cmd in ["python3", "python"] {
            let out = std::process::Command::new(cmd)
                .args(["-c", "import sys; print(sys.executable)"])
                .output();
            if let Ok(o) = out {
                if o.status.success() {
                    let s = String::from_utf8_lossy(&o.stdout);
                    let exe = s.trim();
                    if !exe.is_empty() {
                        return exe.to_string();
                    }
                }
            }
        }
    }
    "python".to_string()
}

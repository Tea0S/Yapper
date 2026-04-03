use crate::audio::PttController;
use crate::remote_engine::RemoteBridge;
use crate::sidecar::SidecarSession;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;
use tauri::async_runtime::JoinHandle;
use tokio::process::Child;
use tokio::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HudPhase {
    /// Widget not shown (engine stopped).
    Hidden,
    /// Small pill — engine running, not dictating.
    Idle,
    Listening,
    Transcribing,
}

pub struct AppState {
    pub tone_dir: PathBuf,
    pub ptt: PttController,
    /// `Arc` so commands can `clone()` the session and `.await` without holding `sidecar`'s mutex
    /// (otherwise `ptt_stop` deadlocks: wait retries `sidecar.lock()` while the lock is still held).
    pub sidecar: Arc<Mutex<Option<Arc<SidecarSession>>>>,
    pub remote: Arc<Mutex<Option<RemoteBridge>>>,
    pub seq: Arc<AtomicU64>,
    pub last_transcript: Arc<Mutex<String>>,
    /// Last reported Whisper/device line from sidecar `ready` (local) or a remote placeholder.
    pub inference_line: Arc<Mutex<Option<String>>>,
    /// Local sidecar: Whisper weights are resident (false when lazy-loading or after idle unload).
    pub local_model_in_memory: Arc<AtomicBool>,
    /// Last user inference activity (PTT / file job) for idle unload.
    pub last_model_activity: Arc<std::sync::Mutex<Instant>>,
    /// Bumps on each local `engine_start` so stale idle-supervisor tasks exit.
    pub idle_run_id: Arc<AtomicU64>,
    /// True after successful `ptt_start` until `ptt_stop` returns.
    pub ptt_session_active: Arc<AtomicBool>,
    /// True during an active file transcription job.
    pub inference_busy: Arc<AtomicBool>,
    /// Push-to-talk overlay (`hud` window) — polled by the HUD webview.
    pub hud_phase: std::sync::Mutex<HudPhase>,
    /// Latest `ptt_start` task started by the global shortcut handler (release awaits it).
    pub ptt_hotkey_start_pending: Arc<StdMutex<Option<JoinHandle<Result<(), String>>>>>,
    /// Optional Yapper Node WebSocket server (`yapper-node/main.py`).
    pub yapper_node: Arc<Mutex<Option<Child>>>,
    pub yapper_node_logs: Arc<Mutex<VecDeque<String>>>,
}

impl AppState {
    pub fn new(tone_dir: PathBuf, ptt: PttController) -> Self {
        Self {
            tone_dir,
            ptt,
            sidecar: Arc::new(Mutex::new(None)),
            remote: Arc::new(Mutex::new(None)),
            seq: Arc::new(AtomicU64::new(0)),
            last_transcript: Arc::new(Mutex::new(String::new())),
            inference_line: Arc::new(Mutex::new(None)),
            local_model_in_memory: Arc::new(AtomicBool::new(false)),
            last_model_activity: Arc::new(std::sync::Mutex::new(Instant::now())),
            idle_run_id: Arc::new(AtomicU64::new(0)),
            ptt_session_active: Arc::new(AtomicBool::new(false)),
            inference_busy: Arc::new(AtomicBool::new(false)),
            hud_phase: std::sync::Mutex::new(HudPhase::Hidden),
            ptt_hotkey_start_pending: Arc::new(StdMutex::new(None)),
            yapper_node: Arc::new(Mutex::new(None)),
            yapper_node_logs: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
}

pub fn next_seq(seq: &Arc<AtomicU64>) -> u64 {
    seq.fetch_add(1, Ordering::SeqCst)
}

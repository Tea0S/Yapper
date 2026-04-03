mod audio;
mod db;
mod global_shortcuts;
mod hud;
mod nvidia_libs;
mod paths;
mod paste;
mod postprocess;
mod remote_engine;
mod sidecar;
mod state;
mod trace_log;
mod node_server;

use crate::db::{
    check_keybind_conflicts, get_setting, import_dictionary_merge, import_dictionary_replace,
    list_corrections, list_dictionary, list_keybinds, load_corrections_for_postprocess,
    load_dictionary_for_postprocess, set_keybind, set_setting, upsert_correction, upsert_dictionary,
    CorrectionEntry, DictionaryEntry, DictionaryExportFile, DictionaryExportItem, DictionaryImportRoot,
    KeybindRow,
};
use crate::paths::{db_path, model_cache_dir, sidecar_script_path};
use crate::sidecar::{
    pop_sidecar_transcript_for_seq, python_executable, SidecarIn, SidecarOut, SidecarSession,
    SidecarSpawnEnv, WhisperDecodeOptions,
};
use crate::state::{next_seq, AppState, HudPhase};
use crate::trace_log::ptt_log;
use audio::{
    condition_speech_signal, f32_to_i16_le_bytes, list_input_devices, resample_to_whisper_16k_mono,
    vad_segments, AudioInputDevice, InputLevelState,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use postprocess::pipeline;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};

fn whisper_decode_options_from_db(conn: &rusqlite::Connection) -> WhisperDecodeOptions {
    let s = |k: &str, def: &str| {
        get_setting(conn, k)
            .ok()
            .flatten()
            .unwrap_or_else(|| def.to_string())
    };
    let truthy = |k: &str, def_true: bool| {
        let v = s(
            k,
            if def_true {
                "true"
            } else {
                "false"
            },
        )
        .to_ascii_lowercase();
        matches!(v.as_str(), "1" | "true" | "yes" | "on")
    };
    WhisperDecodeOptions {
        beam_size: s("whisper_beam_size", "5").parse::<i32>().unwrap_or(5).clamp(1, 10),
        best_of: s("whisper_best_of", "1").parse::<i32>().unwrap_or(1).clamp(1, 5),
        patience: s("whisper_patience", "1")
            .parse::<f64>()
            .unwrap_or(1.0)
            .clamp(0.0, 2.0),
        temperature: s("whisper_temperature", "0")
            .parse::<f64>()
            .unwrap_or(0.0)
            .clamp(0.0, 1.0),
        no_speech_threshold: s("whisper_no_speech_threshold", "0.78")
            .parse::<f64>()
            .unwrap_or(0.78)
            .clamp(0.0, 1.0),
        log_prob_threshold: s("whisper_log_prob_threshold", "-0.55")
            .parse::<f64>()
            .unwrap_or(-0.55)
            .clamp(-2.0, 0.0),
        compression_ratio_threshold: s("whisper_compression_ratio_threshold", "1.9")
            .parse::<f64>()
            .unwrap_or(1.9)
            .clamp(1.0, 4.0),
        hallucination_silence_threshold: s("whisper_hallucination_silence_threshold", "1.6")
            .parse::<f64>()
            .unwrap_or(1.6)
            .clamp(0.0, 3.0),
        condition_on_previous_text: truthy("whisper_condition_on_previous_text", false),
        initial_prompt: s("whisper_initial_prompt", ""),
        language: s("whisper_language", ""),
        vad_filter_pcm: truthy("whisper_vad_filter_pcm", false),
        vad_filter_file: truthy("whisper_vad_filter_file", true),
    }
}

fn inference_model_for_init(conn: &rusqlite::Connection) -> rusqlite::Result<String> {
    let engine = get_setting(conn, "engine")?.unwrap_or_else(|| "whisper".into());
    if engine == "parakeet" {
        Ok(get_setting(conn, "parakeet_model")?
            .unwrap_or_else(|| "nvidia/parakeet-tdt-0.6b-v3".into()))
    } else {
        Ok(get_setting(conn, "whisper_model")?.unwrap_or_else(|| "base".into()))
    }
}
use tauri::{Emitter, Manager, State};

struct ClearPttSession(Arc<AtomicBool>);
impl Drop for ClearPttSession {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

struct ClearInferenceBusy(Arc<AtomicBool>);
impl Drop for ClearInferenceBusy {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn touch_model_activity(state: &AppState) {
    if let Ok(mut g) = state.last_model_activity.lock() {
        *g = Instant::now();
    }
}

async fn ensure_local_model_loaded(_app: &tauri::AppHandle, state: &AppState) -> Result<(), String> {
    if state.remote.lock().await.is_some() {
        ptt_log("ensure_model: skip (remote node)");
        return Ok(());
    }
    if state.local_model_in_memory.load(Ordering::SeqCst) {
        ptt_log("ensure_model: skip (local_model_in_memory already true)");
        return Ok(());
    }
    ptt_log("ensure_model: sending EnsureModel to sidecar");
    {
        let side = state.sidecar.lock().await.clone();
        let Some(side) = side else {
            return Err("Engine not started".into());
        };
        side.send(&SidecarIn::EnsureModel).await?;
    }

    // 720 × 500ms = 6 min — first Hugging Face download can be large/slow on slow links.
    for round in 0..720 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let drained = match state.sidecar.lock().await.clone() {
            Some(side) => side.take_model_load_events().await,
            None => return Err("Engine stopped".into()),
        };
        if !drained.is_empty() {
            ptt_log(format!(
                "ensure_model: round {} drained {} msg(s): {}",
                round + 1,
                drained.len(),
                drained
                    .iter()
                    .map(|m| crate::sidecar::sidecar_out_one_liner(m))
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
        } else if round % 10 == 9 {
            ptt_log(format!(
                "ensure_model: round {} still waiting (no events drained yet)",
                round + 1
            ));
        }
        for m in drained {
            match m {
                SidecarOut::ModelState { loaded: true } => {
                    state.local_model_in_memory.store(true, Ordering::SeqCst);
                    ptt_log("ensure_model: ModelState loaded=true → success");
                    return Ok(());
                }
                SidecarOut::ModelState { loaded: false } => {}
                SidecarOut::Error { message } => {
                    ptt_log(format!("ensure_model: sidecar error: {message}"));
                    return Err(message);
                }
                _ => {}
            }
        }
    }
    ptt_log("ensure_model: timed out after 720×500ms");
    Err("Timed out loading Whisper (first use may download several GB from Hugging Face)".into())
}

async fn model_idle_supervisor(app: tauri::AppHandle, run_id: u64) {
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let state = app.state::<AppState>();
        if state.idle_run_id.load(Ordering::SeqCst) != run_id {
            break;
        }
        if state.sidecar.lock().await.is_none() {
            break;
        }
        if state.ptt_session_active.load(Ordering::SeqCst)
            || state.inference_busy.load(Ordering::SeqCst)
        {
            continue;
        }
        let mins = match open_db(&app).ok().and_then(|c| {
            get_setting(&c, "model_idle_unload_mins")
                .ok()
                .flatten()
                .and_then(|s| s.parse::<u64>().ok())
        }) {
            Some(m) => m,
            None => 0,
        };
        if mins == 0 {
            continue;
        }
        let last = state
            .last_model_activity
            .lock()
            .map(|g| *g)
            .unwrap_or_else(|_| Instant::now());
        if last.elapsed() < Duration::from_secs(mins * 60) {
            continue;
        }
        if !state.local_model_in_memory.load(Ordering::SeqCst) {
            continue;
        }
        let send_ok = match state.sidecar.lock().await.clone() {
            Some(side) => side.send(&SidecarIn::UnloadModel).await.is_ok(),
            None => false,
        };
        if !send_ok {
            continue;
        }
        for _ in 0..80 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if state.idle_run_id.load(Ordering::SeqCst) != run_id {
                break;
            }
            let drained = match state.sidecar.lock().await.clone() {
                Some(side) => side.take_model_load_events().await,
                None => break,
            };
            for m in drained {
                if let SidecarOut::ModelState { loaded } = m {
                    if !loaded {
                        state.local_model_in_memory.store(false, Ordering::SeqCst);
                    }
                }
            }
            if !state.local_model_in_memory.load(Ordering::SeqCst) {
                break;
            }
        }
    }
}

#[derive(Serialize)]
struct EngineStatus {
    ready: bool,
    mode: String,
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inference_detail: Option<String>,
}

/// Where faster-whisper is told to store weights (`app_cache_dir/models`) and current DB settings.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelCacheSettingsSnapshot {
    whisper_model: String,
    mock_transcription: bool,
    lazy_load_whisper: bool,
    whisper_device: String,
    compute_type: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelCacheDiagnostic {
    cache_dir: String,
    cache_dir_exists: bool,
    /// Top-level names under the cache folder (often `hub` or `models--Systran--faster-whisper-*`).
    top_level_entries: Vec<String>,
    settings: ModelCacheSettingsSnapshot,
}

fn format_inference_line(dev: Option<String>, compute: Option<String>) -> Option<String> {
    let mut line = String::new();
    if let Some(d) = dev.filter(|s| !s.is_empty()) {
        if d == "pending_first_use" {
            line.push_str("Model: loads on first dictation or file job (lazy mode)");
        } else {
            line.push_str("Whisper device: ");
            line.push_str(&d);
        }
    }
    if let Some(c) = compute.filter(|s| !s.is_empty()) {
        if !line.is_empty() {
            line.push_str(" · ");
        }
        line.push_str("compute ");
        line.push_str(&c);
    }
    if line.is_empty() {
        None
    } else {
        Some(line)
    }
}

async fn poll_local_ready_metadata(state: &AppState) {
    for _ in 0..100 {
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        if let Some(side) = state.sidecar.lock().await.clone() {
            if let Some((dev, ct)) = side.take_ready_metadata().await {
                *state.inference_line.lock().await = format_inference_line(dev, ct);
                return;
            }
        }
    }
}

pub(crate) fn open_db(app: &tauri::AppHandle) -> Result<rusqlite::Connection, String> {
    let p = db_path(app)?;
    db::open(&p).map_err(|e| e.to_string())
}

#[tauri::command]
fn model_cache_diagnostic(app: tauri::AppHandle) -> Result<ModelCacheDiagnostic, String> {
    let conn = open_db(&app)?;
    let cache_dir = model_cache_dir(&app)?;
    let cache_dir_str = cache_dir.display().to_string();
    let (cache_dir_exists, top_level_entries) = if cache_dir.is_dir() {
        let mut names: Vec<String> = std::fs::read_dir(&cache_dir)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .take(48)
            .collect();
        names.sort();
        (true, names)
    } else {
        (false, Vec::new())
    };
    let whisper_model = get_setting(&conn, "whisper_model")
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| "base".into());
    let mock_transcription = get_setting(&conn, "mock_transcription")
        .map_err(|e| e.to_string())?
        .as_deref()
        == Some("true");
    let lazy_load_whisper = get_setting(&conn, "lazy_load_whisper")
        .map_err(|e| e.to_string())?
        .as_deref()
        == Some("true");
    let whisper_device = get_setting(&conn, "whisper_device")
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| "auto".into());
    let compute_type = get_setting(&conn, "compute_type")
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| "int8".into());
    Ok(ModelCacheDiagnostic {
        cache_dir: cache_dir_str,
        cache_dir_exists,
        top_level_entries,
        settings: ModelCacheSettingsSnapshot {
            whisper_model,
            mock_transcription,
            lazy_load_whisper,
            whisper_device,
            compute_type,
        },
    })
}

#[tauri::command]
fn get_setting_cmd(app: tauri::AppHandle, key: String) -> Result<Option<String>, String> {
    let conn = open_db(&app)?;
    get_setting(&conn, &key).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_setting_cmd(app: tauri::AppHandle, key: String, value: String) -> Result<(), String> {
    let conn = open_db(&app)?;
    set_setting(&conn, &key, &value).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_keybinds_cmd(app: tauri::AppHandle) -> Result<Vec<KeybindRow>, String> {
    let conn = open_db(&app)?;
    list_keybinds(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_keybind_cmd(
    app: tauri::AppHandle,
    action: String,
    shortcut: String,
) -> Result<Vec<String>, String> {
    let conn = open_db(&app)?;
    let conflicts = check_keybind_conflicts(&conn, &action, &shortcut).map_err(|e| e.to_string())?;
    if !conflicts.is_empty() {
        return Ok(conflicts);
    }
    set_keybind(&conn, &action, &shortcut).map_err(|e| e.to_string())?;
    let _ = global_shortcuts::refresh(&app);
    Ok(vec![])
}

#[tauri::command]
fn refresh_global_shortcuts(app: tauri::AppHandle) -> Result<String, String> {
    global_shortcuts::refresh(&app)
}

#[tauri::command]
fn list_dictionary_cmd(app: tauri::AppHandle) -> Result<Vec<DictionaryEntry>, String> {
    let conn = open_db(&app)?;
    list_dictionary(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn upsert_dictionary_cmd(app: tauri::AppHandle, entry: DictionaryEntry) -> Result<i64, String> {
    let conn = open_db(&app)?;
    upsert_dictionary(&conn, &entry).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_dictionary_cmd(app: tauri::AppHandle, id: i64) -> Result<(), String> {
    let conn = open_db(&app)?;
    db::delete_dictionary(&conn, id).map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct DictionaryImportSummary {
    inserted: usize,
    updated: usize,
}

#[tauri::command]
fn export_dictionary_to_path(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let conn = open_db(&app)?;
    let rows = list_dictionary(&conn).map_err(|e| e.to_string())?;
    let file = DictionaryExportFile {
        format: "yapper-dictionary".into(),
        version: 1,
        dictionary: rows
            .into_iter()
            .map(|e| DictionaryExportItem {
                term: e.term,
                replacement: e.replacement,
                priority: e.priority,
                scope: e.scope,
            })
            .collect(),
    };
    let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

#[tauri::command]
fn import_dictionary_from_path(
    app: tauri::AppHandle,
    path: String,
    replace: bool,
) -> Result<DictionaryImportSummary, String> {
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    let root: DictionaryImportRoot =
        serde_json::from_str(&text).map_err(|e| format!("Invalid dictionary file: {e}"))?;
    let items = root.into_items();
    if items.is_empty() {
        return Err("File contains no dictionary entries.".into());
    }
    let conn = open_db(&app)?;
    if replace {
        import_dictionary_replace(&conn, &items).map_err(|e| e.to_string())?;
        let inserted = items.iter().filter(|e| !e.term.trim().is_empty()).count();
        Ok(DictionaryImportSummary {
            inserted,
            updated: 0,
        })
    } else {
        let (inserted, updated) =
            import_dictionary_merge(&conn, &items).map_err(|e| e.to_string())?;
        Ok(DictionaryImportSummary { inserted, updated })
    }
}

#[tauri::command]
fn list_corrections_cmd(app: tauri::AppHandle) -> Result<Vec<CorrectionEntry>, String> {
    let conn = open_db(&app)?;
    list_corrections(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn upsert_correction_cmd(app: tauri::AppHandle, entry: CorrectionEntry) -> Result<i64, String> {
    let conn = open_db(&app)?;
    upsert_correction(&conn, &entry).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_correction_cmd(app: tauri::AppHandle, id: i64) -> Result<(), String> {
    let conn = open_db(&app)?;
    db::delete_correction(&conn, id).map_err(|e| e.to_string())
}

#[tauri::command]
fn paste_text(text: String) -> Result<(), String> {
    paste::paste_text_at_focus(&text)
}

#[tauri::command]
fn list_audio_input_devices() -> Result<Vec<AudioInputDevice>, String> {
    list_input_devices()
}

#[tauri::command]
fn get_mic_input_level(state: State<'_, AppState>) -> InputLevelState {
    state.ptt.snapshot_input_levels()
}

#[tauri::command]
async fn engine_status(state: State<'_, AppState>) -> Result<EngineStatus, String> {
    let side = state.sidecar.lock().await;
    let rem = state.remote.lock().await;
    let detail = state.inference_line.lock().await.clone();
    if side.is_some() {
        return Ok(EngineStatus {
            ready: true,
            mode: "local".into(),
            message: Some("Sidecar running — use push-to-talk or Transcribe.".into()),
            inference_detail: detail,
        });
    }
    if rem.is_some() {
        return Ok(EngineStatus {
            ready: true,
            mode: "remote".into(),
            message: Some("Connected to Yapper Node — use push-to-talk or Transcribe.".into()),
            inference_detail: detail,
        });
    }
    Ok(EngineStatus {
        ready: false,
        mode: "none".into(),
        message: Some("Engine not started".into()),
        inference_detail: None,
    })
}

#[tauri::command]
async fn engine_start(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<EngineStatus, String> {
    let conn = open_db(&app)?;
    let host = get_setting(&conn, "inference_host")
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| "local".into());

    *state.sidecar.lock().await = None;
    *state.remote.lock().await = None;
    *state.inference_line.lock().await = None;
    state.ptt_session_active.store(false, Ordering::SeqCst);
    state.inference_busy.store(false, Ordering::SeqCst);
    state.idle_run_id.fetch_add(1, Ordering::SeqCst);
    let idle_supervisor_run = state.idle_run_id.load(Ordering::SeqCst);

    if host == "remote" {
        let url = get_setting(&conn, "remote_url")
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| "ws://127.0.0.1:8765".into());
        let token = get_setting(&conn, "remote_token")
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let bridge = remote_engine::spawn_remote(&url, &token).await?;
        let model = inference_model_for_init(&conn).map_err(|e| e.to_string())?;
        let compute = get_setting(&conn, "compute_type")
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| "int8".into());
        let mock = get_setting(&conn, "mock_transcription")
            .map_err(|e| e.to_string())?
            .as_deref()
            == Some("true");
        let cache = model_cache_dir(&app).ok().map(|p| p.to_string_lossy().to_string());
        let engine = get_setting(&conn, "engine")
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| "whisper".into());
        let whisper = whisper_decode_options_from_db(&conn);
        let init = SidecarIn::Init {
            model,
            device: "cpu".into(),
            compute_type: compute,
            model_dir: cache,
            mock,
            engine,
            lazy_load: false,
            whisper: Some(whisper),
        };
        bridge
            .tx
            .send(init)
            .map_err(|e| format!("remote send: {e}"))?;
        *state.remote.lock().await = Some(bridge);
        state.local_model_in_memory.store(true, Ordering::SeqCst);
        *state.last_model_activity.lock().unwrap() = Instant::now();
        *state.inference_line.lock().await =
            Some("Remote node (GPU/CPU depends on the server host)".into());
        if let Ok(mut g) = state.hud_phase.lock() {
            *g = HudPhase::Idle;
        }
        let _ = hud::ensure_collapsed_visible(&app);
        return Ok(EngineStatus {
            ready: true,
            mode: "remote".into(),
            message: Some("Connected to Yapper Node — use push-to-talk or Transcribe.".into()),
            inference_detail: state.inference_line.lock().await.clone(),
        });
    }

    let script = sidecar_script_path(&app);
    let py = python_executable(&app);
    #[cfg(target_os = "windows")]
    let sidecar_env = nvidia_libs::windows_bin_dir(&app).map(|bin| SidecarSpawnEnv {
        path_prepend_windows: Some(bin),
        ..Default::default()
    });
    #[cfg(not(target_os = "windows"))]
    let sidecar_env = nvidia_libs::unix_ld_library_path(&app).map(|ld| SidecarSpawnEnv {
        ld_library_path_prepend_unix: Some(ld),
        ..Default::default()
    });
    let session = SidecarSession::spawn(&py, script, sidecar_env).await?;
    let model = inference_model_for_init(&conn).map_err(|e| e.to_string())?;
    let compute = get_setting(&conn, "compute_type")
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| "int8".into());
    let mock = get_setting(&conn, "mock_transcription")
        .map_err(|e| e.to_string())?
        .as_deref()
        == Some("true");
    let cache = model_cache_dir(&app).ok().map(|p| p.to_string_lossy().to_string());
    let whisper_dev_pref = get_setting(&conn, "whisper_device")
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| "auto".into());
    let cuda = std::process::Command::new("nvidia-smi")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let device = match whisper_dev_pref.as_str() {
        "cpu" => "cpu".to_string(),
        "cuda" => {
            if cuda {
                "cuda".into()
            } else {
                "cpu".into()
            }
        }
        _ => {
            if cuda {
                "cuda".into()
            } else {
                "cpu".into()
            }
        }
    };
    let engine = get_setting(&conn, "engine")
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| "whisper".into());
    let lazy_whisper = get_setting(&conn, "lazy_load_whisper")
        .map_err(|e| e.to_string())?
        .as_deref()
        == Some("true");
    let model_dir_disp = cache
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or("(none)");
    let whisper = whisper_decode_options_from_db(&conn);
    eprintln!(
        "[yapper] sidecar Init: model={model} device={device} compute={compute} mock={mock} lazy_load={lazy_whisper} model_dir={model_dir_disp}"
    );
    session
        .send(&SidecarIn::Init {
            model,
            device: device.into(),
            compute_type: compute,
            model_dir: cache,
            mock,
            engine,
            lazy_load: lazy_whisper,
            whisper: Some(whisper),
        })
        .await?;

    *state.sidecar.lock().await = Some(Arc::new(session));

    // Python processes `init` synchronously: while `load_whisper` runs it does not read stdin.
    // If we return "engine ready" early, the next `Chunk` write can fill the stdin pipe and block forever.
    const INIT_WAIT: Duration = Duration::from_secs(1800);
    let init_deadline = Instant::now() + INIT_WAIT;
    loop {
        if Instant::now() > init_deadline {
            *state.sidecar.lock().await = None;
            return Err(
                "Sidecar init timed out after 30 minutes (model download or GPU load stuck?).".into(),
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        let s = state.sidecar.lock().await.clone();
        let Some(side) = s else {
            return Err("Engine stopped during init".into());
        };
        if let Some(e) = side.take_first_error().await {
            *state.sidecar.lock().await = None;
            return Err(e);
        }
        if side.has_ready_event().await {
            break;
        }
    }

    poll_local_ready_metadata(&state).await;
    // Only `ensure_local_model_loaded` / idle supervisor may set this true after a real `ModelState`.
    state.local_model_in_memory.store(false, Ordering::SeqCst);
    *state.last_model_activity.lock().unwrap() = Instant::now();
    let app_spawn = app.clone();
    tokio::spawn(async move {
        model_idle_supervisor(app_spawn, idle_supervisor_run).await;
    });
    if let Ok(mut g) = state.hud_phase.lock() {
        *g = HudPhase::Idle;
    }
    let _ = hud::ensure_collapsed_visible(&app);
    Ok(EngineStatus {
        ready: true,
        mode: "local".into(),
        message: Some("Sidecar running — use push-to-talk or Transcribe.".into()),
        inference_detail: state.inference_line.lock().await.clone(),
    })
}

#[tauri::command]
async fn engine_stop(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.idle_run_id.fetch_add(1, Ordering::SeqCst);
    state.ptt_session_active.store(false, Ordering::SeqCst);
    state.inference_busy.store(false, Ordering::SeqCst);
    state.local_model_in_memory.store(false, Ordering::SeqCst);
    if let Some(s) = state.sidecar.lock().await.take() {
        let _ = s.send(&SidecarIn::Shutdown).await;
    }
    if let Some(r) = state.remote.lock().await.take() {
        let _ = r.tx.send(SidecarIn::Shutdown);
    }
    if let Ok(mut g) = state.hud_phase.lock() {
        *g = HudPhase::Hidden;
    }
    hud::hide(&app);
    Ok(())
}

/// Wait until the sidecar emits `Final` for this chunk (Whisper can take many seconds).
/// Pass `local_sidecar` when using the Python sidecar so this never blocks on `state.sidecar.lock()`
/// while another task already holds that mutex (see `ptt_stop`).
async fn wait_ptt_chunk_transcript(
    app: &tauri::AppHandle,
    state: &AppState,
    seq: u64,
    local_sidecar: Option<Arc<SidecarSession>>,
) -> Result<String, String> {
    // First GPU/CPU run can compile kernels and download weights; keep generous.
    const TIMEOUT: Duration = Duration::from_secs(600);
    let deadline = Instant::now() + TIMEOUT;
    let mut iter: u32 = 0;
    ptt_log(format!("wait_chunk: waiting for final seq={seq} (timeout {:?})", TIMEOUT));
    loop {
        if Instant::now() > deadline {
            if let Some(side) = local_sidecar.as_ref() {
                ptt_log(format!(
                    "wait_chunk: TIMEOUT seq={seq} pending_len={} snapshot: {}",
                    side.pending_len().await,
                    side.pending_debug_line().await
                ));
            }
            return Err(
                "Transcription timed out waiting for Whisper (first run may load the model for a long time)"
                    .into(),
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        iter = iter.wrapping_add(1);

        let raw = if let Some(side) = local_sidecar.as_ref() {
            side.pop_transcript_for_seq(seq).await?
        } else if let Some(rem) = state.remote.lock().await.as_ref() {
            let mut q = rem.pending.lock().await;
            pop_sidecar_transcript_for_seq(&mut q, seq)?
        } else {
            return Err("Engine not started".into());
        };

        if iter % 40 == 0 {
            if let Some(side) = local_sidecar.as_ref() {
                ptt_log(format!(
                    "wait_chunk: seq={seq} ~{:.1}s elapsed pending_len={} | {}",
                    iter as f32 * 0.05,
                    side.pending_len().await,
                    side.pending_debug_line().await
                ));
            }
        }

        if let Some(mut text) = raw {
            ptt_log(format!(
                "wait_chunk: got result seq={seq} raw_chars={} (postprocess next if non-empty)",
                text.len()
            ));
            if !text.is_empty() {
                let tone = open_db(app)
                    .ok()
                    .and_then(|c| get_setting(&c, "tone_preset").ok().flatten())
                    .unwrap_or_else(|| "standard".into());
                let conn = open_db(app)?;
                let corrections =
                    load_corrections_for_postprocess(&conn).map_err(|e| e.to_string())?;
                let dictionary =
                    load_dictionary_for_postprocess(&conn).map_err(|e| e.to_string())?;
                text = pipeline(&text, &corrections, &dictionary, &tone, &state.tone_dir);
            }
            ptt_log(format!("wait_chunk: returning seq={seq} final_chars={}", text.len()));
            return Ok(text);
        }
    }
}

pub(crate) async fn ptt_start_inner(app: &tauri::AppHandle, state: &AppState) -> Result<(), String> {
    ptt_log("ptt_start: begin");
    let conn = open_db(app)?;
    let device_name = get_setting(&conn, "input_device_name")
        .map_err(|e| e.to_string())?
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let ptt = state.ptt.clone();
    tokio::task::spawn_blocking(move || ptt.start(device_name))
        .await
        .map_err(|e| e.to_string())??;
    state.ptt_session_active.store(true, Ordering::SeqCst);
    touch_model_activity(state);
    ptt_log("ptt_start: capture started");
    {
        let mut g = state.hud_phase.lock().map_err(|e| e.to_string())?;
        *g = HudPhase::Listening;
    }
    if hud::set_expanded(app, true).is_err() {
        let _ = hud::ensure_collapsed_visible(app);
    }
    if let Err(e) = hud::set_expanded(app, true) {
        if let Ok(mut g) = state.hud_phase.lock() {
            *g = HudPhase::Idle;
        }
        let ptt = state.ptt.clone();
        let _ = tokio::task::spawn_blocking(move || ptt.stop()).await;
        state.ptt_session_active.store(false, Ordering::SeqCst);
        return Err(e);
    }
    Ok(())
}

#[tauri::command]
async fn ptt_start(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    ptt_start_inner(&app, &state).await
}

pub(crate) async fn ptt_stop_inner(app: &tauri::AppHandle, state: &AppState) -> Result<String, String> {
    ptt_log("ptt_stop: begin");
    let _hud_collapse = hud::HudCollapseAfterPtt::new(&app);
    {
        let mut g = state.hud_phase.lock().map_err(|e| e.to_string())?;
        *g = HudPhase::Transcribing;
    }
    let _clear_ptt = ClearPttSession(state.ptt_session_active.clone());
    let ptt = state.ptt.clone();
    let stop_handle = tokio::task::spawn_blocking(move || ptt.stop());
    let (samples, rate) = tokio::time::timeout(Duration::from_secs(45), stop_handle)
        .await
        .map_err(|_| {
            "Microphone stop timed out — the capture thread may be stuck; restart the inference engine."
                .to_string()
        })?
        .map_err(|e| e.to_string())??;

    let conn = open_db(&app)?;
    let mic_peak: f32 = get_setting(&conn, "mic_normalize_peak")
        .map_err(|e| e.to_string())?
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(0.88)
        .clamp(0.05, 0.99);
    let mic_max_gain: f32 = get_setting(&conn, "mic_max_gain")
        .map_err(|e| e.to_string())?
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(12.0)
        .clamp(1.0, 48.0);
    let samples = condition_speech_signal(&samples, mic_peak, mic_max_gain);

    ptt_log(format!(
        "ptt_stop: mic stopped samples={} rate={} duration_s≈{:.2}",
        samples.len(),
        rate,
        samples.len() as f32 / rate.max(1) as f32
    ));

    let threshold: f32 = get_setting(&conn, "vad_energy_threshold")
        .map_err(|e| e.to_string())?
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(0.008)
        .clamp(0.0001, 0.25);

    let vad_min_silence_ms: u32 = get_setting(&conn, "vad_min_silence_ms")
        .map_err(|e| e.to_string())?
        .and_then(|s| s.parse().ok())
        .unwrap_or(300)
        .clamp(50, 3000);

    ptt_log(format!(
        "ptt_stop: vad_energy_threshold={threshold} vad_min_silence_ms={vad_min_silence_ms} mic_peak={mic_peak} mic_max_gain={mic_max_gain}"
    ));

    if samples.is_empty() {
        ptt_log("ptt_stop: empty buffer → return Ok(\"\")");
        return Ok(String::new());
    }

    ensure_local_model_loaded(&app, &state).await?;

    let segments = vad_segments(&samples, threshold, vad_min_silence_ms, rate);
    ptt_log(format!(
        "ptt_stop: vad_segments count={} (Rust energy gate before Whisper)",
        segments.len()
    ));

    // One Whisper decode for the whole utterance avoids repeated short runs (hallucination cascades).
    const GAP_16K_MS: u32 = 70;
    const MERGED_MAX_16K_SAMPLES: usize = 16_000 * 240;
    let gap_16k = (16_000u32 * GAP_16K_MS / 1000) as usize;
    let mut pcm16k_merged: Vec<f32> = Vec::new();
    for (a, b) in &segments {
        let chunk = &samples[*a..*b];
        let pcm16k = resample_to_whisper_16k_mono(chunk, rate);
        if !pcm16k_merged.is_empty() {
            pcm16k_merged.extend(std::iter::repeat(0.0f32).take(gap_16k));
        }
        pcm16k_merged.extend_from_slice(&pcm16k);
    }
    let use_merged = !pcm16k_merged.is_empty() && pcm16k_merged.len() <= MERGED_MAX_16K_SAMPLES;

    let mut combined = String::new();

    let local_side = state.sidecar.lock().await.clone();
    if let Some(side) = local_side {
        if use_merged {
            let bytes = f32_to_i16_le_bytes(&pcm16k_merged);
            let audio_b64 = B64.encode(&bytes);
            let seq = next_seq(&state.seq);
            ptt_log(format!(
                "ptt_stop: merged {} vad span(s) → single chunk pcm16k_len={} b64_len={} seq={seq}",
                segments.len(),
                pcm16k_merged.len(),
                audio_b64.len(),
            ));
            let msg = SidecarIn::Chunk {
                seq,
                sample_rate: 16_000,
                audio_b64,
                is_final: true,
            };
            ptt_log(format!(
                "ptt_stop: sending Chunk seq={seq} (json approx {} bytes)",
                serde_json::to_string(&msg).map(|s| s.len()).unwrap_or(0)
            ));
            side.send(&msg).await?;
            combined = wait_ptt_chunk_transcript(&app, &state, seq, Some(Arc::clone(&side))).await?;
            ptt_log(format!("ptt_stop: merged transcript_chars={}", combined.len()));
        } else {
            ptt_log(format!(
                "ptt_stop: merged span too long ({} samples) or empty — falling back to per-segment chunks",
                pcm16k_merged.len()
            ));
            for (si, (a, b)) in segments.iter().enumerate() {
                let chunk = &samples[*a..*b];
                let pcm16k = resample_to_whisper_16k_mono(chunk, rate);
                let bytes = f32_to_i16_le_bytes(&pcm16k);
                let audio_b64 = B64.encode(bytes);
                let seq = next_seq(&state.seq);
                ptt_log(format!(
                    "ptt_stop: segment {si} sample_range {}..{} pcm16k_len={} b64_len={} seq={seq}",
                    a,
                    b,
                    pcm16k.len(),
                    audio_b64.len(),
                ));
                let msg = SidecarIn::Chunk {
                    seq,
                    sample_rate: 16_000,
                    audio_b64,
                    is_final: true,
                };

                ptt_log(format!(
                    "ptt_stop: sending Chunk seq={seq} (json approx {} bytes)",
                    serde_json::to_string(&msg).map(|s| s.len()).unwrap_or(0)
                ));
                side.send(&msg).await?;
                let piece =
                    wait_ptt_chunk_transcript(&app, &state, seq, Some(Arc::clone(&side))).await?;
                ptt_log(format!(
                    "ptt_stop: segment {si} piece_chars={}",
                    piece.len()
                ));
                if !piece.is_empty() {
                    if !combined.is_empty() {
                        combined.push(' ');
                    }
                    combined.push_str(&piece);
                }
            }
        }
    } else if state.remote.lock().await.is_some() {
        if use_merged {
            let bytes = f32_to_i16_le_bytes(&pcm16k_merged);
            let audio_b64 = B64.encode(bytes);
            let seq = next_seq(&state.seq);
            ptt_log(format!(
                "ptt_stop: remote merged {} vad span(s) pcm16k_len={} seq={seq}",
                segments.len(),
                pcm16k_merged.len(),
            ));
            let msg = SidecarIn::Chunk {
                seq,
                sample_rate: 16_000,
                audio_b64,
                is_final: true,
            };
            {
                let rem = state.remote.lock().await;
                let Some(rem) = rem.as_ref() else {
                    return Err("Engine not started".into());
                };
                rem.tx.send(msg).map_err(|e| e.to_string())?;
            }
            combined = wait_ptt_chunk_transcript(&app, &state, seq, None).await?;
        } else {
            for (si, (a, b)) in segments.iter().enumerate() {
                let chunk = &samples[*a..*b];
                let pcm16k = resample_to_whisper_16k_mono(chunk, rate);
                let bytes = f32_to_i16_le_bytes(&pcm16k);
                let audio_b64 = B64.encode(bytes);
                let seq = next_seq(&state.seq);
                ptt_log(format!(
                    "ptt_stop: segment {si} sample_range {}..{} pcm16k_len={} b64_len={} seq={seq}",
                    a,
                    b,
                    pcm16k.len(),
                    audio_b64.len(),
                ));
                let msg = SidecarIn::Chunk {
                    seq,
                    sample_rate: 16_000,
                    audio_b64,
                    is_final: true,
                };
                {
                    let rem = state.remote.lock().await;
                    let Some(rem) = rem.as_ref() else {
                        return Err("Engine not started".into());
                    };
                    ptt_log(format!("ptt_stop: remote Chunk seq={seq}"));
                    rem.tx.send(msg).map_err(|e| e.to_string())?;
                }
                let piece = wait_ptt_chunk_transcript(&app, &state, seq, None).await?;
                ptt_log(format!("ptt_stop: remote segment {si} piece_chars={}", piece.len()));
                if !piece.is_empty() {
                    if !combined.is_empty() {
                        combined.push(' ');
                    }
                    combined.push_str(&piece);
                }
            }
        }
    } else {
        return Err("Engine not started".into());
    }

    ptt_log(format!(
        "ptt_stop: done combined_chars={}",
        combined.len()
    ));
    *state.last_transcript.lock().await = combined.clone();
    let _ = app.emit("transcript", combined.clone());
    touch_model_activity(state);
    Ok(combined)
}

#[tauri::command]
async fn ptt_stop(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    ptt_stop_inner(&app, &state).await
}

#[tauri::command]
async fn transcribe_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    let _busy = ClearInferenceBusy(state.inference_busy.clone());
    state.inference_busy.store(true, Ordering::SeqCst);
    ensure_local_model_loaded(&app, &state).await?;
    let msg = SidecarIn::TranscribeFile { path: path.clone() };
    if let Some(side) = state.sidecar.lock().await.clone() {
        side.send(&msg).await?;
    } else if let Some(rem) = state.remote.lock().await.as_ref() {
        rem.tx.send(msg).map_err(|e| e.to_string())?;
    } else {
        return Err("Engine not started".into());
    }

    for _ in 0..600 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if let Some(side) = state.sidecar.lock().await.clone() {
            match side.pop_file_done_for_path(&path).await {
                Ok(Some(text)) => {
                    let tone = open_db(&app)
                        .ok()
                        .and_then(|c| get_setting(&c, "tone_preset").ok().flatten())
                        .unwrap_or_else(|| "standard".into());
                    let conn = open_db(&app)?;
                    let corrections =
                        load_corrections_for_postprocess(&conn).map_err(|e| e.to_string())?;
                    let dictionary =
                        load_dictionary_for_postprocess(&conn).map_err(|e| e.to_string())?;
                    let out = pipeline(&text, &corrections, &dictionary, &tone, &state.tone_dir);
                    *state.last_transcript.lock().await = out.clone();
                    touch_model_activity(&state);
                    return Ok(out);
                }
                Err(message) => return Err(message),
                Ok(None) => {}
            }
        }
        if let Some(rem) = state.remote.lock().await.as_ref() {
            let mut q = rem.pending.lock().await;
            while let Some(m) = q.pop_front() {
                match m {
                    SidecarOut::FileDone { path: p, text } if p == path => {
                        let tone = open_db(&app)
                            .ok()
                            .and_then(|c| get_setting(&c, "tone_preset").ok().flatten())
                            .unwrap_or_else(|| "standard".into());
                        let conn = open_db(&app)?;
                        let corrections =
                            load_corrections_for_postprocess(&conn).map_err(|e| e.to_string())?;
                        let dictionary =
                            load_dictionary_for_postprocess(&conn).map_err(|e| e.to_string())?;
                        let out = pipeline(&text, &corrections, &dictionary, &tone, &state.tone_dir);
                        *state.last_transcript.lock().await = out.clone();
                        touch_model_activity(&state);
                        return Ok(out);
                    }
                    SidecarOut::Error { message } => return Err(message),
                    _ => {}
                }
            }
        }
    }
    Err("Transcription timed out".into())
}

#[derive(Serialize)]
struct HudSnapshot {
    phase: HudPhase,
}

#[tauri::command]
fn hud_snapshot(state: State<'_, AppState>) -> Result<HudSnapshot, String> {
    let g = state.hud_phase.lock().map_err(|e| e.to_string())?;
    Ok(HudSnapshot { phase: *g })
}

#[tauri::command]
fn focus_main_window(app: tauri::AppHandle) -> Result<(), String> {
    let w = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;
    w.unminimize().map_err(|e| e.to_string())?;
    w.show().map_err(|e| e.to_string())?;
    w.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn cuda_available() -> bool {
    std::process::Command::new("nvidia-smi")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[tauri::command]
async fn install_nvidia_whisper_libs(app: tauri::AppHandle) -> Result<String, String> {
    tokio::task::spawn_blocking(move || nvidia_libs::install_blocking(&app))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let tone_dir = if cfg!(debug_assertions) {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("resources")
                    .join("tones")
            } else {
                let res = handle.path().resource_dir().expect("resource dir");
                let a = res.join("tones");
                let b = res.join("resources").join("tones");
                if a.join("standard.yaml").exists() {
                    a
                } else {
                    b
                }
            };
            let _ = std::fs::create_dir_all(&tone_dir);

            let state = AppState::new(tone_dir.clone(), audio::PttController::spawn());
            app.manage(state);

            let show_i = MenuItem::with_id(app, "show", "Show Yapper", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &quit_i])?;

            let _tray = TrayIconBuilder::new()
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "quit" => app.exit(0),
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { .. } = event {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                })
                .build(app)?;

            let _ = global_shortcuts::refresh(&handle);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            model_cache_diagnostic,
            get_setting_cmd,
            set_setting_cmd,
            list_keybinds_cmd,
            set_keybind_cmd,
            refresh_global_shortcuts,
            list_dictionary_cmd,
            upsert_dictionary_cmd,
            delete_dictionary_cmd,
            export_dictionary_to_path,
            import_dictionary_from_path,
            list_corrections_cmd,
            upsert_correction_cmd,
            delete_correction_cmd,
            paste_text,
            engine_start,
            engine_stop,
            engine_status,
            ptt_start,
            ptt_stop,
            transcribe_file,
            cuda_available,
            list_audio_input_devices,
            get_mic_input_level,
            install_nvidia_whisper_libs,
            hud_snapshot,
            focus_main_window,
            node_server::yapper_node_status,
            node_server::yapper_node_start,
            node_server::yapper_node_stop,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

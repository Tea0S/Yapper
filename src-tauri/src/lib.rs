mod audio;
mod dictation;
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
mod win_spawn;
#[cfg(windows)]
mod win_taskbar_icon;

use crate::db::{
    check_keybind_conflicts, get_setting, import_corrections_merge, import_corrections_replace,
    import_dictionary_merge, import_dictionary_replace, list_corrections, list_dictionary,
    list_keybinds, load_corrections_for_postprocess, load_dictionary_for_postprocess, set_keybind,
    set_setting, upsert_correction, upsert_dictionary, CorrectionEntry, CorrectionExportItem,
    DictionaryEntry, DictionaryExportFile, DictionaryExportItem, DictionaryImportRoot, KeybindRow,
};
use crate::paths::{db_path, model_cache_dir, sidecar_script_path};
use crate::sidecar::{
    pop_sidecar_file_done_for_path, pop_sidecar_restored_for_seq,
    pop_sidecar_stream_for_session, pop_sidecar_transcript_for_seq, python_executable,
    take_file_progress_for_path, take_file_started_for_path, SidecarIn, SidecarOut,
    SidecarSession, SidecarSpawnEnv, WhisperDecodeOptions,
};
use crate::state::{next_seq, AppState, HudPhase};
use crate::trace_log::ptt_log;
use audio::{
    condition_speech_signal, f32_to_i16_le_bytes, list_input_devices, resample_to_whisper_16k_mono,
    vad_segments, AudioInputDevice, InputLevelState,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use postprocess::{looks_underpunctuated, pipeline, pipeline_after_restore, pipeline_before_restore};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
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
        initial_prompt: dictionary_prompt(conn, &s("whisper_initial_prompt", "")),
        language: s("whisper_language", ""),
        vad_filter_pcm: truthy("whisper_vad_filter_pcm", false),
        vad_filter_file: truthy("whisper_vad_filter_file", true),
    }
}

fn dictionary_prompt(conn: &rusqlite::Connection, user_prompt: &str) -> String {
    if get_setting(conn, "dictionary_recognition_hints").ok().flatten().as_deref() == Some("false") {
        return user_prompt.to_string();
    }
    let mut entries = list_dictionary(conn).unwrap_or_default();
    entries.sort_by(|a, b| b.priority.cmp(&a.priority));
    let mut terms = Vec::new();
    let mut remaining = 400usize;
    for entry in entries {
        let term = if entry.replacement.trim().is_empty() { entry.term.trim() } else { entry.replacement.trim() };
        let term = term.replace(['\n', '\r'], " ");
        let n = term.chars().count();
        if n == 0 || n > remaining || terms.contains(&term) { continue; }
        remaining = remaining.saturating_sub(n + 2);
        terms.push(term);
        if terms.len() >= 32 { break; }
    }
    if terms.is_empty() { return user_prompt.to_string(); }
    format!("{}\nVocabulary: {}.", user_prompt.trim(), terms.join(", ")).trim().to_string()
}

fn inference_model_for_init(conn: &rusqlite::Connection) -> rusqlite::Result<String> {
    let engine = get_setting(conn, "engine")?.unwrap_or_else(|| "whisper".into());
    if engine == "parakeet" {
        Ok(get_setting(conn, "parakeet_model")?
            .unwrap_or_else(|| "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8".into()))
    } else {
        Ok(get_setting(conn, "whisper_model")?.unwrap_or_else(|| "base".into()))
    }
}
use tauri::webview::{NewWindowResponse, WebviewWindowBuilder};
use tauri::{Emitter, Manager, Runtime, RunEvent, State, Url, WindowEvent};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

fn live_preview_wanted(conn: &rusqlite::Connection) -> bool {
    get_setting(conn, "live_dictation_experimental")
        .ok()
        .flatten()
        .as_deref()
        == Some("true")
        && get_setting(conn, "mock_transcription")
            .ok()
            .flatten()
            .as_deref()
            != Some("true")
}

const DICTATION_OUTCOME_TTL: Duration = Duration::from_secs(3);

fn set_dictation_outcome(state: &AppState, message: impl Into<String>) {
    if let Ok(mut g) = state.last_dictation_outcome.lock() {
        *g = Some((message.into(), Instant::now()));
    }
}

fn take_dictation_outcome_if_fresh(state: &AppState) -> (String, bool) {
    let Ok(mut g) = state.last_dictation_outcome.lock() else {
        return (String::new(), false);
    };
    match g.as_ref() {
        Some((msg, at)) if at.elapsed() < DICTATION_OUTCOME_TTL => (msg.clone(), false),
        Some(_) => {
            *g = None;
            (String::new(), true)
        }
        None => (String::new(), false),
    }
}

fn live_streaming_engine(conn: &rusqlite::Connection) -> String {
    get_setting(conn, "live_streaming_engine")
        .ok()
        .flatten()
        .unwrap_or_else(|| "moonshine".into())
}

fn live_streaming_model(conn: &rusqlite::Connection) -> String {
    get_setting(conn, "live_streaming_model")
        .ok()
        .flatten()
        .unwrap_or_else(|| "small_streaming".into())
}

async fn send_sidecar_msg(state: &AppState, msg: SidecarIn) -> Result<(), String> {
    let local_side = state.sidecar.lock().await.clone();
    if let Some(ref side) = local_side {
        side.send(&msg).await
    } else if let Some(rem) = state.remote.lock().await.as_ref() {
        rem.tx.send(msg).map_err(|e| e.to_string())
    } else {
        Err("Engine not started".into())
    }
}

async fn wait_stream_started(state: &AppState, session_id: u64) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        if Instant::now() > deadline {
            return Err("Stream start timed out".into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        let local_side = state.sidecar.lock().await.clone();
        if let Some(side) = local_side {
            let mut q = side.pending.lock().await;
            let mut i = 0usize;
            while i < q.len() {
                match &q[i] {
                    SidecarOut::Error { message } => {
                        let m = message.clone();
                        q.remove(i);
                        return Err(m);
                    }
                    SidecarOut::StreamStarted { session_id: sid } if *sid == session_id => {
                        q.remove(i);
                        return Ok(());
                    }
                    _ => i += 1,
                }
            }
        } else if let Some(rem) = state.remote.lock().await.as_ref() {
            let mut q = rem.pending.lock().await;
            let mut i = 0usize;
            while i < q.len() {
                match &q[i] {
                    SidecarOut::Error { message } => {
                        let m = message.clone();
                        q.remove(i);
                        return Err(m);
                    }
                    SidecarOut::StreamStarted { session_id: sid } if *sid == session_id => {
                        q.remove(i);
                        return Ok(());
                    }
                    _ => i += 1,
                }
            }
        } else {
            return Err("Engine not started".into());
        }
    }
}

async fn wait_stream_final(state: &AppState, session_id: u64) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if Instant::now() > deadline {
            return Err("Stream final timed out".into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        let local_side = state.sidecar.lock().await.clone();
        let raw = if let Some(side) = local_side {
            side.pop_stream_for_session(session_id).await?
        } else if let Some(rem) = state.remote.lock().await.as_ref() {
            let mut q = rem.pending.lock().await;
            pop_sidecar_stream_for_session(&mut q, session_id)?
        } else {
            return Err("Engine not started".into());
        };
        if let Some((text, is_final)) = raw {
            if is_final {
                return Ok(text);
            }
        }
    }
}

async fn update_live_hud_preview(app: &tauri::AppHandle, state: &AppState, raw: String) {
    let t = raw.trim().to_string();
    if t.is_empty() {
        return;
    }
    let mut g = state.live_hud_preview.lock().await;
    let first_preview = g.is_empty();
    if *g != t {
        *g = t.clone();
    }
    *state.live_last_partial_text.lock().await = t;
    drop(g);
    // Grow the pill only once live text actually arrives (not for every PTT).
    if first_preview {
        let _ = hud::set_layout(app, hud::HudLayout::Preview);
    }
}

async fn abort_live_preview_task(state: &AppState) {
    let mut g = state.live_loop_handle.lock().await;
    if let Some(h) = g.take() {
        h.abort();
        let _ = h.await;
    }
}

async fn try_spawn_live_preview(app: &tauri::AppHandle, state: &AppState) {
    let conn = match open_db(app) {
        Ok(c) => c,
        Err(_) => return,
    };
    if !live_preview_wanted(&conn) {
        return;
    }
    if state.sidecar.lock().await.is_none() && state.remote.lock().await.is_none() {
        return;
    }
    let mut g = state.live_loop_handle.lock().await;
    if let Some(h) = g.take() {
        h.abort();
        let _ = h.await;
    }
    let app = app.clone();
    *g = Some(tokio::spawn(async move {
        live_streaming_loop(app).await;
    }));
}

async fn live_streaming_loop(app: tauri::AppHandle) {
    let state = app.state::<AppState>();
    let conn = match open_db(&app) {
        Ok(c) => c,
        Err(_) => return,
    };
    if !live_preview_wanted(&conn) {
        return;
    }
    let engine = live_streaming_engine(&conn);
    let model = live_streaming_model(&conn);
    let session_id = state.live_stream_session_id.load(Ordering::SeqCst);
    if session_id == 0 {
        return;
    }

    *state.live_preview_status.lock().await = "Preparing live preview; the first use may download a model...".into();
    if let Err(e) = send_sidecar_msg(
        &state,
        SidecarIn::StartStream {
            session_id,
            engine: engine.clone(),
            model: model.clone(),
        },
    )
    .await
    {
        *state.live_preview_status.lock().await = format!("Preview unavailable: {e}. Final transcription is still available.");
        ptt_log(format!("live_dictation: start_stream: {e}"));
        return;
    }
    if let Err(e) = wait_stream_started(&state, session_id).await {
        *state.live_preview_status.lock().await = format!("Preview unavailable: {e}. Final transcription is still available.");
        ptt_log(format!("live_dictation: wait stream_started: {e}"));
        return;
    }

    *state.live_preview_status.lock().await = "Listening for preview...".into();
    let mut first_tick = true;
    loop {
        let interval_ms: u64 = {
            let conn = match open_db(&app) {
                Ok(c) => c,
                Err(_) => break,
            };
            if !live_preview_wanted(&conn) {
                break;
            }
            get_setting(&conn, "live_feed_interval_ms")
                .ok()
                .flatten()
                .and_then(|s| s.parse().ok())
                .or_else(|| {
                    get_setting(&conn, "live_chunk_interval_ms")
                        .ok()
                        .flatten()
                        .and_then(|s| s.parse().ok())
                })
                .unwrap_or(200)
                .clamp(100, 30_000)
        };

        if !first_tick {
            tokio::time::sleep(Duration::from_millis(interval_ms)).await;
        }
        first_tick = false;

        if !state.ptt_session_active.load(Ordering::SeqCst) {
            break;
        }

        let conn = match open_db(&app) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if !live_preview_wanted(&conn) {
            break;
        }

        let min_audio_ms: u32 = get_setting(&conn, "live_min_audio_ms")
            .ok()
            .flatten()
            .and_then(|s| s.parse().ok())
            .unwrap_or(800)
            .clamp(200, 10_000);

        let ptt = state.ptt.clone();
        let snapshot = match tokio::task::spawn_blocking(move || ptt.snapshot_buffer()).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                ptt_log(format!("live_dictation: snapshot: {e}"));
                continue;
            }
            Err(e) => {
                ptt_log(format!("live_dictation: spawn_blocking: {e}"));
                continue;
            }
        };
        let (samples, rate) = snapshot;
        if samples.is_empty() || rate == 0 {
            continue;
        }
        let cursor = state.live_audio_cursor.load(Ordering::SeqCst) as usize;
        if cursor >= samples.len() {
            continue;
        }
        let delta = samples[cursor..].to_vec();
        let ms = (delta.len() as u64 * 1000 / rate as u64) as u32;
        if ms < min_audio_ms.saturating_div(4).max(50) && cursor == 0 {
            continue;
        }

        let mic_peak: f32 = get_setting(&conn, "mic_normalize_peak")
            .ok()
            .flatten()
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(0.88)
            .clamp(0.05, 0.99);
        let mic_max_gain: f32 = get_setting(&conn, "mic_max_gain")
            .ok()
            .flatten()
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(12.0)
            .clamp(1.0, 48.0);
        let delta = condition_speech_signal(&delta, mic_peak, mic_max_gain);
        let pcm16k = resample_to_whisper_16k_mono(&delta, rate);
        if pcm16k.is_empty() {
            continue;
        }
        let bytes = f32_to_i16_le_bytes(&pcm16k);
        let audio_b64 = B64.encode(&bytes);

        let Ok(_io_guard) = state.inference_io_lock.try_lock() else {
            continue;
        };

        if let Err(e) = send_sidecar_msg(
            &state,
            SidecarIn::FeedAudio {
                session_id,
                sample_rate: 16_000,
                audio_b64,
                engine: engine.clone(),
            },
        )
        .await
        {
            ptt_log(format!("live_dictation: feed_audio: {e}"));
            *state.live_preview_status.lock().await = "Preview interrupted. Final transcription is still available.".into();
            break;
        }
        // Never advance until FeedAudio was actually sent; a busy lock must not discard speech.
        state.live_audio_cursor.store(samples.len() as u32, Ordering::SeqCst);

        tokio::time::sleep(Duration::from_millis(30)).await;
        for _ in 0..256 {
        let result = {
            let local_side = state.sidecar.lock().await.clone();
            if let Some(side) = local_side {
                side.pop_stream_for_session(session_id).await
            } else if let Some(rem) = state.remote.lock().await.as_ref() {
                let mut q = rem.pending.lock().await;
                pop_sidecar_stream_for_session(&mut q, session_id)
            } else {
                Ok(None)
            }
        };
        match result {
            Ok(Some((text, _))) if !text.trim().is_empty() => {
                *state.live_preview_status.lock().await = String::new();
                update_live_hud_preview(&app, &state, text).await;
            }
            Ok(Some(_)) => {},
            Ok(None) => break,
            Err(error) => {
                *state.live_preview_status.lock().await = format!("Preview unavailable: {error}");
                return;
            }
        }
        }
    }
}

/// Tear down the live streaming session. Discarded text — paste uses batch ASR.
/// Must not block PTT release: waiting on stream_final previously hung the HUD for up to 30s.
async fn end_live_stream(app: &tauri::AppHandle, state: &AppState) {
    let conn = match open_db(app) {
        Ok(c) => c,
        Err(_) => return,
    };
    if !live_preview_wanted(&conn) {
        return;
    }
    let session_id = state.live_stream_session_id.swap(0, Ordering::SeqCst);
    if session_id == 0 {
        return;
    }
    let engine = live_streaming_engine(&conn);
    {
        let _guard = state.inference_io_lock.lock().await;
        let _ = send_sidecar_msg(
            state,
            SidecarIn::EndStream {
                session_id,
                engine,
            },
        )
        .await;
    }
    // Brief drain only — never stall the batch Whisper commit / paste path.
    match tokio::time::timeout(Duration::from_millis(750), wait_stream_final(state, session_id))
        .await
    {
        Ok(Err(e)) => ptt_log(format!("live_dictation: stream final (ignored): {e}")),
        Err(_) => ptt_log("live_dictation: stream final drain timed out (ok, preview-only)"),
        Ok(Ok(_)) => {}
    }
}

/// Dev server, `tauri://`, and packaged `https://tauri.localhost` (and loopback IPs).
pub(crate) fn allow_in_app_navigation(url: &Url) -> bool {
    match url.scheme() {
        "tauri" => true,
        "about" => matches!(url.path(), "" | "blank"),
        "http" | "https" => {
            if let Some(host) = url.host_str() {
                if host.eq_ignore_ascii_case("localhost") || host == "tauri.localhost" {
                    return true;
                }
                if let Ok(ip) = host.parse::<std::net::IpAddr>() {
                    return ip.is_loopback();
                }
            }
            false
        }
        _ => false,
    }
}

/// `true` keeps navigation inside the webview; `false` cancels it and opens http(s)/mailto externally.
pub(crate) fn allow_navigation_in_webview(url: &Url) -> bool {
    if allow_in_app_navigation(url) {
        return true;
    }
    if matches!(url.scheme(), "http" | "https" | "mailto") {
        let _ = open::that(url.as_str());
        return false;
    }
    true
}

pub(crate) fn handle_new_window_request<R: Runtime>(url: Url) -> NewWindowResponse<R> {
    if !allow_in_app_navigation(&url) && matches!(url.scheme(), "http" | "https" | "mailto") {
        let _ = open::that(url.as_str());
    }
    NewWindowResponse::Deny
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

async fn apply_dictation_postprocess(
    app: &tauri::AppHandle,
    state: &AppState,
    mut text: String,
    underpunctuated: bool,
    local_sidecar: Option<Arc<SidecarSession>>,
) -> Result<String, String> {
    if text.is_empty() {
        return Ok(text);
    }
    let tone = open_db(app)
        .ok()
        .and_then(|c| get_setting(&c, "tone_preset").ok().flatten())
        .unwrap_or_else(|| "standard".into());
    let conn = open_db(app)?;
    let corrections = load_corrections_for_postprocess(&conn).map_err(|e| e.to_string())?;
    let dictionary = load_dictionary_for_postprocess(&conn).map_err(|e| e.to_string())?;
    let grammar_mode = get_setting(&conn, "grammar_restore")
        .ok()
        .flatten()
        .unwrap_or_else(|| "auto".into())
        .to_ascii_lowercase();

    text = pipeline_before_restore(&text, &corrections, &dictionary);

    let want_restore = match grammar_mode.as_str() {
        "off" | "false" | "0" => false,
        "always" | "on" | "true" | "1" => true,
        _ => underpunctuated || looks_underpunctuated(&text),
    };

    if want_restore {
        let seq = next_seq(&state.seq);
        let msg = SidecarIn::RestorePunct {
            seq,
            text: text.clone(),
        };
        ptt_log(format!(
            "grammar_restore: mode={grammar_mode} underpunctuated={underpunctuated} seq={seq}"
        ));
        if let Some(side) = local_sidecar.as_ref() {
            side.send(&msg).await?;
            let deadline = Instant::now() + Duration::from_secs(90);
            loop {
                if Instant::now() > deadline {
                    ptt_log("grammar_restore: timed out — keeping pre-restore text");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(40)).await;
                match side.pop_restored_for_seq(seq).await? {
                    Some(restored) => {
                        text = restored;
                        break;
                    }
                    None => continue,
                }
            }
        } else if let Some(rem) = state.remote.lock().await.as_ref() {
            rem.tx
                .send(msg)
                .map_err(|e| format!("remote restore send: {e}"))?;
            let deadline = Instant::now() + Duration::from_secs(90);
            loop {
                if Instant::now() > deadline {
                    ptt_log("grammar_restore remote: timed out — keeping pre-restore text");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(40)).await;
                let mut q = rem.pending.lock().await;
                match pop_sidecar_restored_for_seq(&mut q, seq)? {
                    Some(restored) => {
                        text = restored;
                        break;
                    }
                    None => continue,
                }
            }
        }
    }

    text = pipeline_after_restore(&text, &tone, &state.tone_dir);
    Ok(text)
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
    let side = state.sidecar.lock().await.clone().ok_or("Engine not started")?;
    if !side.is_alive() { return Err(side.exit_error("while loading the model").await); }
    side.send(&SidecarIn::EnsureModel).await?;

    // 720 × 500ms = 6 min — first Hugging Face download can be large/slow on slow links.
    for round in 0..720 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if !side.is_alive() { return Err(side.exit_error("while loading the model").await); }
        let drained = side.take_model_load_events().await;
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


async fn sidecar_watchdog(app: tauri::AppHandle, run_id: u64) {
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let state = app.state::<AppState>();
        let Ok(_engine) = state.engine_lifecycle.try_lock() else { continue; };
        if state.idle_run_id.load(Ordering::SeqCst) != run_id {
            return;
        }
        let dead = {
            let side = state.sidecar.lock().await;
            match side.as_ref() {
                Some(s) => !s.is_alive(),
                None => false,
            }
        };
        if !dead {
            continue;
        }
        ptt_log("sidecar_watchdog: process died — clearing session");
        *state.sidecar.lock().await = None;
        state.local_model_in_memory.store(false, Ordering::SeqCst);
        let _ = app.emit(
            "engine-crashed",
            serde_json::json!({
                "message": "Inference engine exited unexpectedly.",
                "recovering": true,
            }),
        );
        // Rust alone owns restart. UI events report progress; they must not start
        // another process while this attempt is loading.
        let app_restart = app.clone();
        let mut expected_run = run_id;
        std::thread::spawn(move || {
            let mut last_error = String::new();
            for attempt in 1u32..=3 {
                std::thread::sleep(Duration::from_millis(800 * attempt as u64));
                let app_restart = app_restart.clone();
                let (ok, next_run, error) = tauri::async_runtime::block_on(async move {
                    let state = app_restart.state::<AppState>();
                    if state.idle_run_id.load(Ordering::SeqCst) != expected_run {
                        return (true, expected_run, String::new()); // superseded
                    }
                    let alive = {
                        let g = state.sidecar.lock().await;
                        g.as_ref().is_some_and(|s| s.is_alive())
                    };
                    if alive {
                        return (true, expected_run, String::new());
                    }
                    // Give a failed job time to release its work guard and let an
                    // active recording finish; a busy guard is not a failed restart.
                    let deadline = Instant::now() + Duration::from_secs(60);
                    loop {
                        if state.idle_run_id.load(Ordering::SeqCst) != expected_run {
                            return (true, expected_run, String::new());
                        }
                        let capture_idle = state.dictation.capture.try_lock().map(|c| c.is_none()).unwrap_or(false);
                        if capture_idle && !dictation::busy(&state).await { break; }
                        if Instant::now() >= deadline { return (false, expected_run, "Finish the active recording before restarting.".into()); }
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                    let Ok(_engine) = state.engine_lifecycle.try_lock() else {
                        return (false, expected_run, "An engine transition is already in progress.".into());
                    };
                    if state.idle_run_id.load(Ordering::SeqCst) != expected_run {
                        return (true, expected_run, String::new());
                    }
                    ptt_log(format!(
                        "sidecar_watchdog: auto-restart attempt {attempt}"
                    ));
                    let _ = app_restart.emit(
                        "engine-auto-restart",
                        serde_json::json!({ "attempt": attempt }),
                    );
                    // Keep ownership through reading the attempt's generation so an
                    // explicit user stop cannot be adopted as another recovery attempt.
                    match engine_start_inner(&app_restart, &state).await {
                        Ok(st) if st.ready => {
                            // Home queries status in response; publish only after
                            // the transition guard is released so it sees Ready.
                            drop(_engine);
                            let _ = app_restart.emit(
                                "engine-recovered",
                                serde_json::json!({
                                    "message": "Inference engine restarted automatically."
                                }),
                            );
                            (true, expected_run, String::new())
                        }
                        result => {
                            let reason = result.err().unwrap_or_else(|| "Engine did not become ready".into());
                            ptt_log(format!("sidecar_watchdog: auto-restart failed: {reason}"));
                            // engine_start advances the generation even when loading fails.
                            // Continue retries for our new generation instead of treating
                            // our own failed attempt as an external stop/restart.
                            (false, state.idle_run_id.load(Ordering::SeqCst), reason)
                        }
                    }
                });
                expected_run = next_run;
                if !error.is_empty() { last_error = error; }
                if ok {
                    return;
                }
            }
            let _ = app_restart.emit(
                "engine-crashed",
                serde_json::json!({
                    "message": format!("Could not restart the inference engine. {last_error}"),
                    "recovering": false,
                }),
            );
        });
        // This watchdog exits; a successful engine_start spawns a new one.
        return;
    }
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
            || dictation::busy(&state).await
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
        let Ok(_work) = state.dictation.work.try_lock() else { continue; };
        if state.ptt_session_active.load(Ordering::SeqCst) { continue; }
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
    corrections_inserted: usize,
    corrections_updated: usize,
}

#[tauri::command]
fn export_dictionary_to_path(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let conn = open_db(&app)?;
    let rows = list_dictionary(&conn).map_err(|e| e.to_string())?;
    let corr_rows = list_corrections(&conn).map_err(|e| e.to_string())?;
    let file = DictionaryExportFile {
        format: "yapper-dictionary".into(),
        version: 2,
        dictionary: rows
            .into_iter()
            .map(|e| DictionaryExportItem {
                term: e.term,
                replacement: e.replacement,
                priority: e.priority,
                scope: e.scope,
            })
            .collect(),
        corrections: Some(
            corr_rows
                .into_iter()
                .map(|e| CorrectionExportItem {
                    mishear: e.mishear,
                    intended: e.intended,
                    priority: e.priority,
                })
                .collect(),
        ),
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
    let (items, corrections) = root.into_parts();
    let has_corrections = corrections
        .as_ref()
        .is_some_and(|c| c.iter().any(|e| !e.mishear.trim().is_empty()));
    if items.iter().all(|e| e.term.trim().is_empty()) && !has_corrections {
        return Err("File contains no dictionary or correction entries.".into());
    }
    let conn = open_db(&app)?;
    if replace {
        import_dictionary_replace(&conn, &items).map_err(|e| e.to_string())?;
        let inserted = items.iter().filter(|e| !e.term.trim().is_empty()).count();
        let corrections_inserted = if let Some(ref corr) = corrections {
            import_corrections_replace(&conn, corr).map_err(|e| e.to_string())?;
            corr.iter().filter(|e| !e.mishear.trim().is_empty()).count()
        } else {
            0
        };
        Ok(DictionaryImportSummary {
            inserted,
            updated: 0,
            corrections_inserted,
            corrections_updated: 0,
        })
    } else {
        let (inserted, updated) =
            import_dictionary_merge(&conn, &items).map_err(|e| e.to_string())?;
        let (corrections_inserted, corrections_updated) = if let Some(ref corr) = corrections {
            import_corrections_merge(&conn, corr).map_err(|e| e.to_string())?
        } else {
            (0, 0)
        };
        Ok(DictionaryImportSummary {
            inserted,
            updated,
            corrections_inserted,
            corrections_updated,
        })
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
fn paste_text(app: tauri::AppHandle, text: String) -> Result<(), String> {
    let _ops = paste::paste_text_at_focus_on_main_thread(&app, text)?;
    Ok(())
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
    if state.engine_lifecycle.try_lock().is_err() {
        return Ok(EngineStatus { ready: false, mode: "starting".into(), message: Some("Engine is starting or stopping — please wait.".into()), inference_detail: None });
    }
    let detail = state.inference_line.lock().await.clone();
    {
        let side = state.sidecar.lock().await;
        if let Some(s) = side.as_ref() {
            if s.is_alive() {
                return Ok(EngineStatus {
                    ready: true,
                    mode: "local".into(),
                    message: Some("Sidecar running — use push-to-talk or Transcribe.".into()),
                    inference_detail: detail,
                });
            }
        }
    }
    {
        let rem = state.remote.lock().await;
        if rem.is_some() {
            return Ok(EngineStatus {
                ready: true,
                mode: "remote".into(),
                message: Some("Connected to Yapper Node — use push-to-talk or Transcribe.".into()),
                inference_detail: detail,
            });
        }
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
    let _engine = state.engine_lifecycle.try_lock().map_err(|_| "The engine is already starting or stopping. Please wait.".to_string())?;
    engine_start_inner(&app, &state).await
}

async fn engine_start_inner(app: &tauri::AppHandle, state: &AppState) -> Result<EngineStatus, String> {
    let _capture = state.dictation.capture.try_lock().map_err(|_| "Recording is stopping. Try again shortly.".to_string())?;
    if _capture.is_some() { return Err("Finish recording before restarting the engine".into()); }
    let _jobs = state.dictation.work.try_lock().map_err(|_| "Wait for dictation processing to finish before restarting the engine".to_string())?;
    drop(_capture);
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
    let mut cuda_cmd = std::process::Command::new("nvidia-smi");
    crate::win_spawn::hide_console(&mut cuda_cmd);
    let cuda = cuda_cmd
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

    let session = Arc::new(session);
    *state.sidecar.lock().await = Some(session.clone());

    // A dead process can never emit Ready. Fail promptly and release lifecycle/work
    // guards rather than waiting thirty minutes with the capture gate held.
    if let Err(error) = session.wait_ready(Duration::from_secs(1800)).await {
        *state.sidecar.lock().await = None;
        state.local_model_in_memory.store(false, Ordering::SeqCst);
        return Err(error);
    }

    poll_local_ready_metadata(&state).await;
    // Only `ensure_local_model_loaded` / idle supervisor may set this true after a real `ModelState`.
    state.local_model_in_memory.store(false, Ordering::SeqCst);
    *state.last_model_activity.lock().unwrap() = Instant::now();
    let app_spawn = app.clone();
    tokio::spawn(async move {
        model_idle_supervisor(app_spawn, idle_supervisor_run).await;
    });
    let app_wd = app.clone();
    tokio::spawn(async move {
        sidecar_watchdog(app_wd, idle_supervisor_run).await;
    });
    if let Ok(mut g) = state.hud_phase.lock() {
        *g = HudPhase::Idle;
    }
    let _ = hud::ensure_collapsed_visible(&app);
    let inference_detail = state.inference_line.lock().await.clone();
    Ok(EngineStatus {
        ready: true,
        mode: "local".into(),
        message: Some("Sidecar running — use push-to-talk or Transcribe.".into()),
        inference_detail,
    })
}

#[tauri::command]
async fn engine_stop(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let _engine = state.engine_lifecycle.try_lock().map_err(|_| "The engine is already starting or stopping. Please wait.".to_string())?;
    let _capture = state.dictation.capture.try_lock().map_err(|_| "Recording is stopping. Try again shortly.".to_string())?;
    if _capture.is_some() { return Err("Finish recording before stopping the engine".into()); }
    let _jobs = state.dictation.work.try_lock().map_err(|_| "Wait for processing to finish before stopping the engine".to_string())?;
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
    audio_duration_secs: Option<f64>,
) -> Result<String, String> {
    // Scale with audio length; floor 600s for cold model loads, cap 3600s.
    let timeout_secs = {
        let dur = audio_duration_secs.unwrap_or(0.0).max(0.0);
        let scaled = 90.0 + dur * 4.0;
        scaled.clamp(600.0, 3600.0) as u64
    };
    let timeout = Duration::from_secs(timeout_secs);
    let deadline = Instant::now() + timeout;
    let mut iter: u32 = 0;
    ptt_log(format!(
        "wait_chunk: waiting for final seq={seq} (timeout {:?} audio_s={audio_duration_secs:?})",
        timeout
    ));
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
            if !side.is_alive() {
                return Err(format!("{}\nYour recording is available in Home. Once the engine is ready, choose Retry recording.", side.exit_error("while transcribing").await));
            }
            side.pop_transcript_for_seq(seq).await?
        } else if let Some(rem) = state.remote.lock().await.as_ref() {
            let mut q = rem.pending.lock().await;
            pop_sidecar_transcript_for_seq(&mut q, seq)?
        } else {
            return Err("Engine not started".into());
        };

        if iter.is_multiple_of(100) && iter > 0 {
            if let Some(side) = local_sidecar.as_ref() {
                let elapsed = iter as f32 * 0.05;
                ptt_log(format!(
                    "wait_chunk: seq={seq} ~{elapsed:.0}s — sidecar still decoding (queue empty until `final` arrives) pending_len={}",
                    side.pending_len().await
                ));
            }
        }

        if let Some((text, underpunctuated)) = raw {
            ptt_log(format!(
                "wait_chunk: got result seq={seq} raw_chars={} underpunctuated={underpunctuated}",
                text.len()
            ));
            let text = apply_dictation_postprocess(
                app,
                state,
                text,
                underpunctuated,
                local_sidecar.clone(),
            )
            .await?;
            ptt_log(format!("wait_chunk: returning seq={seq} final_chars={}", text.len()));
            return Ok(text);
        }
    }
}

pub(crate) async fn ptt_start_inner(app: &tauri::AppHandle, state: &AppState) -> Result<(), String> {
    dictation::start(app, state, true).await
}

async fn start_capture(app: &tauri::AppHandle, state: &AppState) -> Result<(), String> {
    ptt_log("ptt_start: begin");
    state.live_audio_cursor.store(0, Ordering::SeqCst);
    // Zero means no live session. The global sequence starts at zero.
    let session_id = state::next_stream_seq(&state.seq);
    state
        .live_stream_session_id
        .store(session_id, Ordering::SeqCst);
    *state.live_last_partial_text.lock().await = String::new();
    *state.live_hud_preview.lock().await = String::new();
    let conn = open_db(app)?;
    let device_name = get_setting(&conn, "input_device_name")
        .map_err(|e| e.to_string())?
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let fallback = dictation::enabled(app, "microphone_auto_fallback", true);
    let ptt = state.ptt.clone();
    tokio::task::spawn_blocking(move || ptt.start(device_name, fallback))
        .await
        .map_err(|e| e.to_string())??;
    state.ptt_session_active.store(true, Ordering::SeqCst);
    touch_model_activity(state);
    ptt_log("ptt_start: capture started");
    hud::sound_cue(app, 880);
    {
        let mut g = state.hud_phase.lock().map_err(|e| e.to_string())?;
        *g = HudPhase::Listening;
    }
    if hud::set_layout(app, hud::HudLayout::Listening).is_err() {
        let _ = hud::ensure_collapsed_visible(app);
    }
    if let Err(e) = hud::set_layout(app, hud::HudLayout::Listening) {
        if let Ok(mut g) = state.hud_phase.lock() {
            *g = HudPhase::Idle;
        }
        let ptt = state.ptt.clone();
        let _ = tokio::task::spawn_blocking(move || ptt.stop()).await;
        state.ptt_session_active.store(false, Ordering::SeqCst);
        return Err(e);
    }
    *state.live_preview_status.lock().await = String::new();
    if !dictation::busy(state).await {
        try_spawn_live_preview(app, state).await;
    } else {
        state.live_stream_session_id.store(0, Ordering::SeqCst);
        if live_preview_wanted(&conn) {
            *state.live_preview_status.lock().await = "Preview unavailable while earlier dictation finishes. Audio is still recording.".into();
        }
    }
    Ok(())
}

#[tauri::command]
async fn ptt_start(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    dictation::start(&app, &state, false).await
}

pub(crate) async fn ptt_stop_inner(app: &tauri::AppHandle, state: &AppState) -> Result<String, String> {
    dictation::stop(app, state).await
}

async fn transcribe_recording(app: &tauri::AppHandle, state: &AppState, samples: &[f32], rate: u32) -> Result<String, String> {
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
        .unwrap_or(500)
        .clamp(50, 3000);

    ptt_log(format!(
        "ptt_stop: vad_energy_threshold={threshold} vad_min_silence_ms={vad_min_silence_ms} mic_peak={mic_peak} mic_max_gain={mic_max_gain}"
    ));

    if samples.is_empty() {
        ptt_log("ptt_stop: empty buffer → return Ok(\"\")");
        set_dictation_outcome(state, "Hold longer — nothing pasted");
        return Ok(String::new());
    }

    if live_preview_wanted(&conn) {
        ptt_log("ptt_stop: live preview was on — commit via batch Whisper/Parakeet (not stream_final)");
    }

    let adaptive = dictation::enabled(app, "adaptive_microphone", true);
    let threshold = if adaptive { audio::adaptive_threshold(&samples, rate, threshold) } else { threshold };
    let segments = if adaptive { audio::padded_vad_segments(&samples, threshold, vad_min_silence_ms, rate) } else { vad_segments(&samples, threshold, vad_min_silence_ms, rate) };
    ptt_log(format!(
        "ptt_stop: vad_segments count={} (Rust energy gate before Whisper)",
        segments.len()
    ));

    if segments.is_empty() {
        ptt_log("ptt_stop: no speech segments → return Ok(\"\")");
        set_dictation_outcome(state, "No speech heard — hold and speak");
        return Ok(String::new());
    }

    ensure_local_model_loaded(app, state).await?;

    // One Whisper decode for the whole utterance avoids repeated short runs (hallucination cascades).
    // Keep the stitch gap tiny: longer pads read as sentence ends and Whisper inserts periods on
    // mid-thought pauses. Speech spans already have trailing silence trimmed in vad_segments.
    const GAP_16K_MS: u32 = 20;
    const MERGED_MAX_16K_SAMPLES: usize = 16_000 * 600;
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
            let _io = state.inference_io_lock.lock().await;
            side.send(&msg).await?;
            let audio_s = pcm16k_merged.len() as f64 / 16_000.0;
            combined = wait_ptt_chunk_transcript(&app, &state, seq, Some(Arc::clone(&side)), Some(audio_s)).await?;
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
                let _io = state.inference_io_lock.lock().await;
                side.send(&msg).await?;
                let piece =
                    wait_ptt_chunk_transcript(&app, &state, seq, Some(Arc::clone(&side)), Some(pcm16k.len() as f64 / 16_000.0)).await?;
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
            let _io = state.inference_io_lock.lock().await;
            {
                let rem = state.remote.lock().await;
                let Some(rem) = rem.as_ref() else {
                    return Err("Engine not started".into());
                };
                rem.tx.send(msg).map_err(|e| e.to_string())?;
            }
            let audio_s = pcm16k_merged.len() as f64 / 16_000.0;
            combined = wait_ptt_chunk_transcript(&app, &state, seq, None, Some(audio_s)).await?;
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
                let _io = state.inference_io_lock.lock().await;
                {
                    let rem = state.remote.lock().await;
                    let Some(rem) = rem.as_ref() else {
                        return Err("Engine not started".into());
                    };
                    ptt_log(format!("ptt_stop: remote Chunk seq={seq}"));
                    rem.tx.send(msg).map_err(|e| e.to_string())?;
                }
                let piece = wait_ptt_chunk_transcript(&app, &state, seq, None, Some(pcm16k.len() as f64 / 16_000.0)).await?;
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
    touch_model_activity(state);
    Ok(combined)
}

#[tauri::command]
async fn ptt_stop(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    ptt_stop_inner(&app, &state).await
}

/// How long to wait for file transcription. Scales with media length (CPU/GPU often runs several× realtime).
fn file_transcribe_timeout(duration_secs: Option<f64>) -> Duration {
    const MIN_SECS: u64 = 600;
    const MAX_SECS: u64 = 14_400;
    const DEFAULT_SECS: u64 = 7200;
    const RTF_FACTOR: f64 = 4.0;
    const BUFFER_SECS: u64 = 300;

    let secs = match duration_secs {
        Some(d) if d.is_finite() && d > 0.0 => {
            let scaled = (d * RTF_FACTOR).ceil() as u64 + BUFFER_SECS;
            scaled.clamp(MIN_SECS, MAX_SECS)
        }
        _ => DEFAULT_SECS,
    };
    Duration::from_secs(secs)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TranscribeFileProgress {
    path: String,
    percent: f32,
}

async fn wait_file_transcript(
    app: &tauri::AppHandle,
    state: &AppState,
    path: &str,
    local_sidecar: Option<Arc<SidecarSession>>,
) -> Result<String, String> {
    let mut timeout = file_transcribe_timeout(None);
    let mut deadline = Instant::now() + timeout;
    let mut duration_applied = false;

    loop {
        if Instant::now() > deadline {
            let waited = timeout.as_secs() / 60;
            return Err(format!(
                "Transcription timed out after ~{waited} minutes. Long files can take several× realtime on CPU/GPU — retry or use a smaller Whisper model."
            ));
        }

        if !duration_applied {
            let started = if let Some(side) = local_sidecar.as_ref() {
                side.take_file_started_for_path(path).await
            } else if let Some(rem) = state.remote.lock().await.as_ref() {
                let mut q = rem.pending.lock().await;
                take_file_started_for_path(&mut q, path)
            } else {
                return Err("Engine not started".into());
            };
            if let Some(duration_secs) = started {
                duration_applied = true;
                timeout = file_transcribe_timeout(duration_secs);
                let new_deadline = Instant::now() + timeout;
                if new_deadline > deadline {
                    deadline = new_deadline;
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;

        if let Some(percent) = if let Some(side) = local_sidecar.as_ref() {
            side.take_file_progress_for_path(path).await
        } else if let Some(rem) = state.remote.lock().await.as_ref() {
            let mut q = rem.pending.lock().await;
            take_file_progress_for_path(&mut q, path)
        } else {
            None
        } {
            let _ = app.emit(
                "transcribe_file_progress",
                TranscribeFileProgress {
                    path: path.to_string(),
                    percent,
                },
            );
        }

        let raw = if let Some(side) = local_sidecar.as_ref() {
            side.pop_file_done_for_path(path).await?
        } else if let Some(rem) = state.remote.lock().await.as_ref() {
            let mut q = rem.pending.lock().await;
            pop_sidecar_file_done_for_path(&mut q, path)?
        } else {
            return Err("Engine not started".into());
        };

        if let Some(text) = raw {
            return Ok(text);
        }
    }
}

#[tauri::command]
async fn transcribe_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    let _jobs = state.dictation.work.lock().await;
    let _busy = ClearInferenceBusy(state.inference_busy.clone());
    state.inference_busy.store(true, Ordering::SeqCst);
    ensure_local_model_loaded(&app, &state).await?;
    let msg = SidecarIn::TranscribeFile { path: path.clone() };
    let local_side = state.sidecar.lock().await.clone();
    let _io = state.inference_io_lock.lock().await;
    if let Some(side) = local_side.as_ref() {
        side.send(&msg).await?;
    } else if let Some(rem) = state.remote.lock().await.as_ref() {
        rem.tx.send(msg).map_err(|e| e.to_string())?;
    } else {
        return Err("Engine not started".into());
    }
    drop(_io);

    let _ = app.emit(
        "transcribe_file_progress",
        TranscribeFileProgress {
            path: path.clone(),
            percent: 0.0,
        },
    );

    let text = wait_file_transcript(&app, &state, &path, local_side).await?;
    let tone = open_db(&app)
        .ok()
        .and_then(|c| get_setting(&c, "tone_preset").ok().flatten())
        .unwrap_or_else(|| "standard".into());
    let conn = open_db(&app)?;
    let corrections = load_corrections_for_postprocess(&conn).map_err(|e| e.to_string())?;
    let dictionary = load_dictionary_for_postprocess(&conn).map_err(|e| e.to_string())?;
    let out = pipeline(&text, &corrections, &dictionary, &tone, &state.tone_dir);
    *state.last_transcript.lock().await = out.clone();
    let _ = app.emit(
        "transcribe_file_progress",
        TranscribeFileProgress {
            path: path.clone(),
            percent: 100.0,
        },
    );
    touch_model_activity(&state);
    Ok(out)
}

#[derive(Serialize)]
struct HudSnapshot {
    phase: HudPhase,
    preview: String,
    /// Brief post-dictation status (empty hold, pasted count). Empty when idle/expired.
    outcome: String,
    pending: usize,
    microphone: audio::MicrophoneStatus,
    preview_status: String,
    engine_progress: sidecar::EngineProgress,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HudChromeInfo {
    /// From `cfg!` — do not infer from `navigator.userAgent` (breaks Windows WebView).
    macos: bool,
    /// Apple Silicon only — MLX Whisper is not available on Intel Macs or other platforms.
    apple_silicon: bool,
}

#[tauri::command]
fn hud_chrome_info() -> HudChromeInfo {
    HudChromeInfo {
        macos: cfg!(target_os = "macos"),
        apple_silicon: cfg!(all(target_os = "macos", target_arch = "aarch64")),
    }
}

async fn engine_progress_inner(state: &AppState) -> sidecar::EngineProgress {
    let side = state.sidecar.lock().await.clone();
    if let Some(side) = side { return side.progress.lock().await.clone(); }
    sidecar::EngineProgress::default()
}

#[tauri::command]
async fn engine_progress(state: State<'_, AppState>) -> Result<sidecar::EngineProgress, String> {
    Ok(engine_progress_inner(&state).await)
}

#[tauri::command]
fn microphone_status(state: State<'_, AppState>) -> Result<audio::MicrophoneStatus, String> {
    Ok(state.ptt.snapshot_status())
}

#[tauri::command]
async fn hud_toggle_recording(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let result = if state.ptt_session_active.load(Ordering::SeqCst) {
        dictation::stop(&app, &state).await.map(|_| ())
    } else {
        dictation::start(&app, &state, true).await
    };
    if let Err(ref e) = result { set_dictation_outcome(&state, e); }
    result
}

#[tauri::command]
async fn hud_snapshot(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<HudSnapshot, String> {
    let mut phase = *state.hud_phase.lock().map_err(|e| e.to_string())?;
    let pending = dictation::pending_count(&state).await;
    if phase != HudPhase::Hidden {
        if state.ptt_session_active.load(Ordering::SeqCst) { phase = HudPhase::Listening; }
        else if pending > 0 { phase = HudPhase::Transcribing; }
    }
    let preview = if phase == HudPhase::Listening { state.live_hud_preview.lock().await.clone() } else { String::new() };
    let (outcome, _) = take_dictation_outcome_if_fresh(&state);
    let microphone = state.ptt.snapshot_status();
    let preview_status = state.live_preview_status.lock().await.clone();
    let engine_progress = engine_progress_inner(&state).await;
    let needs_detail = !preview.is_empty() || !outcome.is_empty() || !microphone.error.is_empty()
        || (phase == HudPhase::Listening && (!microphone.notice.is_empty() || !preview_status.is_empty()))
        || matches!(engine_progress.stage.as_str(), "checking" | "downloading" | "loading" | "warming");
    let layout = if needs_detail { hud::HudLayout::Preview }
        else if phase == HudPhase::Listening || phase == HudPhase::Transcribing { hud::HudLayout::Listening }
        else { hud::HudLayout::Collapsed };
    let _ = hud::set_layout(&app, layout);
    Ok(HudSnapshot { phase, preview, outcome, pending, microphone, preview_status, engine_progress })
}

/// Latest dictation outcome for the main window (Home tips / status). Same TTL as HUD.
#[tauri::command]
fn last_dictation_outcome_cmd(state: State<'_, AppState>) -> Result<String, String> {
    Ok(take_dictation_outcome_if_fresh(&state).0)
}

/// Apply `hud_widget_enabled` and whether the engine is running — show, hide, or keep hidden.
#[tauri::command]
async fn hud_sync_visibility_cmd(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if !hud::widget_enabled(&app)? {
        hud::hide(&app);
        return Ok(());
    }
    let running = state.sidecar.lock().await.is_some() || state.remote.lock().await.is_some();
    if running {
        hud::ensure_collapsed_visible(&app)
    } else {
        hud::hide(&app);
        Ok(())
    }
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

/// Re-applies small + big (`ICON_BIG`) window icons on Windows — WebView2 can reset the taskbar icon after load.
#[tauri::command]
fn sync_windows_taskbar_icon(app: tauri::AppHandle) {
    #[cfg(windows)]
    {
        let Some(win) = app.get_webview_window("main") else {
            return;
        };
        let Some(icon) = app.default_window_icon().cloned() else {
            return;
        };
        let _ = win.set_icon(icon.clone());
        win_taskbar_icon::apply_taskbar_big_icon(&win, &icon);
    }
    #[cfg(not(windows))]
    let _ = app;
}

#[tauri::command]
async fn cuda_available() -> bool {
    let mut cmd = std::process::Command::new("nvidia-smi");
    crate::win_spawn::hide_console(&mut cmd);
    cmd.output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[tauri::command]
async fn install_nvidia_whisper_libs(app: tauri::AppHandle) -> Result<String, String> {
    tokio::task::spawn_blocking(move || nvidia_libs::install_blocking(&app))
        .await
        .map_err(|e| e.to_string())?
}

/// Stop engine, HUD, node server, and global shortcuts so exit does not leave ghost hotkeys.
fn cleanup_before_exit(app: &tauri::AppHandle) {
    let _ = app.global_shortcut().unregister_all();

    if let Some(hud_win) = app.get_webview_window(hud::LABEL) {
        let _ = hud_win.close();
    }

    let state = app.state::<AppState>();
    state.idle_run_id.fetch_add(1, Ordering::SeqCst);
    state.ptt_session_active.store(false, Ordering::SeqCst);
    state.inference_busy.store(false, Ordering::SeqCst);
    state.local_model_in_memory.store(false, Ordering::SeqCst);

    let _ = tauri::async_runtime::block_on(async {
        if let Some(s) = state.sidecar.lock().await.take() {
            let _ = s.send(&SidecarIn::Shutdown).await;
        }
        if let Some(r) = state.remote.lock().await.take() {
            let _ = r.tx.send(SidecarIn::Shutdown);
        }
        let mut node = state.yapper_node.lock().await;
        if let Some(mut child) = node.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
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
            let cleanup_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(15)).await;
                    dictation::expire(&cleanup_app.state::<AppState>()).await;
                }
            });

            let main_cfg = app
                .config()
                .app
                .windows
                .iter()
                .find(|w| w.label == "main")
                .ok_or_else(|| "tauri.conf.json must define a window labeled \"main\"")?;
            let main_win = WebviewWindowBuilder::from_config(app.handle(), main_cfg)?
                .on_navigation(|url| allow_navigation_in_webview(url))
                .on_new_window(|url, _| handle_new_window_request(url))
                .build()?;
            let app_for_close = app.handle().clone();
            main_win.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    cleanup_before_exit(&app_for_close);
                    app_for_close.exit(0);
                }
            });

            let bundled_icon = app.default_window_icon().cloned();
            // With `devUrl` (localhost), WebView2 on Windows often leaves the taskbar/title icon as the
            // default blue placeholder unless the window icon is applied from Rust after startup.
            #[cfg(windows)]
            if let (Some(main), Some(icon)) = (
                app.get_webview_window("main"),
                bundled_icon.as_ref(),
            ) {
                let _ = main.set_icon(icon.clone());
                win_taskbar_icon::apply_taskbar_big_icon(&main, icon);
            }

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
            dictation::dictation_jobs,
            dictation::retry_dictation,
            dictation::discard_dictation,
            dictation::benchmark_dictation,
            transcribe_file,
            cuda_available,
            list_audio_input_devices,
            get_mic_input_level,
            install_nvidia_whisper_libs,
            hud_chrome_info,
            hud_snapshot,
            hud_toggle_recording,
            engine_progress,
            microphone_status,
            last_dictation_outcome_cmd,
            hud_sync_visibility_cmd,
            focus_main_window,
            sync_windows_taskbar_icon,
            node_server::yapper_node_status,
            node_server::yapper_node_start,
            node_server::yapper_node_stop,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let RunEvent::Exit = event {
                cleanup_before_exit(app_handle);
            }
        });
}

#[cfg(test)]
mod recognition_hint_tests {
    use super::*;

    #[test]
    fn dictionary_hints_respect_priority_budget_and_opt_out() {
        let conn = db::open(std::path::Path::new(":memory:")).unwrap();
        for (term, priority) in [("low", 0), ("Yapper", 100), ("Yapper", 99)] {
            upsert_dictionary(&conn, &DictionaryEntry { id: None, term: term.into(), replacement: String::new(), priority, scope: "word".into() }).unwrap();
        }
        upsert_dictionary(&conn, &DictionaryEntry { id: None, term: "x".repeat(500), replacement: String::new(), priority: 200, scope: "word".into() }).unwrap();
        assert_eq!(dictionary_prompt(&conn, "User style."), "User style.\nVocabulary: Yapper, low.");
        set_setting(&conn, "dictionary_recognition_hints", "false").unwrap();
        assert_eq!(dictionary_prompt(&conn, "User style."), "User style.");
    }
}

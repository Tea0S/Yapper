//! Recording jobs are independent of capture, so another utterance can start while decoding.
use crate::{audio, db, paste, state::AppState};
use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager, State};
use tokio::sync::Mutex;

const KEEP_FOR: Duration = Duration::from_secs(600);
const MAX_JOBS: usize = 5;

#[derive(Default)]
pub struct Dictation {
    pub capture: Mutex<Option<Capture>>,
    pub work: Mutex<()>,
    jobs: Mutex<VecDeque<Job>>,
}

pub struct Capture {
    insert: bool,
    target: Option<paste::PasteTarget>,
    stop: Arc<AtomicBool>,
    progress: Option<tokio::task::JoinHandle<(usize, String)>>,
}

struct Job {
    id: u64,
    audio: Arc<Vec<f32>>,
    rate: u32,
    created: Instant,
    text: String,
    status: String,
    error: Option<String>,
    elapsed_ms: u64,
    app_key: Option<String>,
}

#[derive(Serialize)]
pub struct JobView {
    id: u64,
    text: String,
    status: String,
    error: Option<String>,
    audio_seconds: f64,
    elapsed_ms: u64,
    app_key: Option<String>,
}

fn prune(jobs: &mut VecDeque<Job>) {
    jobs.retain(|j| {
        j.created.elapsed() < KEEP_FOR || j.status == "processing" || j.status == "queued"
    });
}

pub fn enabled(app: &tauri::AppHandle, key: &str, default: bool) -> bool {
    crate::open_db(app)
        .ok()
        .and_then(|c| db::get_setting(&c, key).ok().flatten())
        .map(|v| v == "true")
        .unwrap_or(default)
}

pub async fn start(app: &tauri::AppHandle, state: &AppState, insert: bool) -> Result<(), String> {
    let mut capture = state.dictation.capture.lock().await;
    let _engine = state.engine_lifecycle.try_lock()
        .map_err(|_| "The engine is starting or stopping. Wait until it is ready.".to_string())?;
    // Holding capture now prevents an engine transition. Do not make an ordinary
    // microphone start appear as an engine transition to status readers.
    drop(_engine);
    if capture.is_some() {
        return Err("Already recording".into());
    }
    {
        let mut jobs = state.dictation.jobs.lock().await;
        prune(&mut jobs);
        if jobs
            .iter()
            .filter(|j| j.status == "queued" || j.status == "processing")
            .count()
            >= MAX_JOBS
        {
            return Err(
                "Five dictations are waiting. Let one finish before recording again.".into(),
            );
        }
    }
    let target_task = insert.then(|| tokio::task::spawn_blocking(paste::capture_target));
    crate::start_capture(app, state).await?;
    // Capture destination identity concurrently so UI Automation cannot delay mic opening.
    let target = match target_task {
        Some(task) => task.await.unwrap_or(None),
        None => None,
    };
    let stop = Arc::new(AtomicBool::new(false));
    let progress = if enabled(app, "background_dictation", true) {
        let app = app.clone();
        let stop = stop.clone();
        Some(tokio::spawn(async move {
            let state = app.state::<AppState>();
            let mut cursor = 0;
            let mut text = String::new();
            while !stop.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(400)).await;
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let ptt = state.ptt.clone();
                let Ok(Ok((recent, rate, total))) =
                    tokio::task::spawn_blocking(move || ptt.snapshot_tail()).await
                else {
                    break;
                };
                if cursor > total || rate == 0 {
                    break;
                }
                if total - cursor < rate as usize * 8 {
                    continue;
                }
                let tail = &recent;
                // Commit only at a sustained pause after at least eight seconds. The pause
                // remains in the next slice, preserving soft consonants at either boundary.
                let quiet = rate as usize * 3 / 4;
                let threshold = audio::adaptive_threshold(tail, rate, 0.008).min(0.008);
                let rms = (tail[tail.len() - quiet..]
                    .iter()
                    .map(|s| s * s)
                    .sum::<f32>()
                    / quiet as f32)
                    .sqrt();
                if rms >= threshold {
                    continue;
                }
                let ptt = state.ptt.clone();
                let Ok(Ok((samples, snapshot_rate))) =
                    tokio::task::spawn_blocking(move || ptt.snapshot_buffer()).await
                else {
                    break;
                };
                if snapshot_rate != rate || samples.len() < total {
                    break;
                }
                let end = total - quiet / 2;
                let Ok(_work) = state.dictation.work.try_lock() else {
                    continue;
                };
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                match crate::transcribe_recording(&app, &state, &samples[cursor..end], rate).await {
                    Ok(piece) => {
                        append_text(&mut text, &piece);
                        cursor = end;
                        // Only update the preview for this recording, never a newer one.
                        if !stop.load(Ordering::SeqCst) {
                            crate::update_live_hud_preview(&app, &state, text.clone()).await;
                        }
                    }
                    Err(_) => break, // Stop will retry the uncommitted tail from retained audio.
                }
            }
            (cursor, text)
        }))
    } else {
        None
    };
    *capture = Some(Capture {
        insert,
        target,
        stop,
        progress,
    });
    let warm_app = app.clone();
    tokio::spawn(async move {
        let state = warm_app.state::<AppState>();
        if let Ok(_work) = state.dictation.work.try_lock() {
            let _ = crate::ensure_local_model_loaded(&warm_app, &state).await;
        };
    });
    Ok(())
}

fn append_text(text: &mut String, piece: &str) {
    if !text.is_empty() && !piece.trim().is_empty() {
        text.push(' ');
    }
    text.push_str(piece.trim());
}

pub async fn stop(app: &tauri::AppHandle, state: &AppState) -> Result<String, String> {
    // Hold capture through stopping the microphone; release it before decoding.
    let mut slot = state.dictation.capture.lock().await;
    let capture = slot.take().ok_or("No recording is active")?;
    let started = Instant::now();
    capture.stop.store(true, Ordering::SeqCst);
    let ptt = state.ptt.clone();
    let result = tokio::time::timeout(
        Duration::from_secs(45),
        tokio::task::spawn_blocking(move || ptt.stop()),
    )
    .await;
    state.ptt_session_active.store(false, Ordering::SeqCst);
    let result = result
        .map_err(|_| "Microphone stop timed out. Restart Yapper.".to_string())
        .and_then(|r| r.map_err(|e| e.to_string()))
        .and_then(|r| r);
    if result.is_err() {
        if let Ok(mut phase) = state.hud_phase.lock() {
            *phase = crate::state::HudPhase::Idle;
        }
    }
    let (samples, rate) = result?;
    crate::abort_live_preview_task(state).await;
    crate::end_live_stream(app, state).await;
    *state.live_hud_preview.lock().await = String::new();
    if let Ok(mut phase) = state.hud_phase.lock() {
        *phase = crate::state::HudPhase::Transcribing;
    }
    let id = crate::state::next_seq(&state.seq);
    let audio = Arc::new(samples);
    {
        let mut jobs = state.dictation.jobs.lock().await;
        prune(&mut jobs);
        while jobs.len() >= MAX_JOBS {
            if let Some(i) = jobs
                .iter()
                .position(|j| j.status != "queued" && j.status != "processing")
            {
                jobs.remove(i);
            } else {
                break;
            }
        }
        jobs.push_back(Job {
            id,
            audio: audio.clone(),
            rate,
            created: Instant::now(),
            text: String::new(),
            status: "queued".into(),
            error: None,
            elapsed_ms: 0,
            app_key: capture.target.as_ref().map(|t| t.app_key.clone()),
        });
    }
    // Reserve FIFO order before permitting the next capture to stop.
    let work = state.dictation.work.lock();
    tokio::pin!(work);
    // Polling lock once registers this job in Tokio's FIFO queue.
    let early = tokio::select! { biased; guard = &mut work => Some(guard), _ = std::future::ready(()) => None };
    drop(slot);
    // Keep the microphone available while preparing a compact recovery recording.
    let retained = audio.clone();
    if let Ok(recovery) =
        tokio::task::spawn_blocking(move || audio::resample_to_whisper_16k_mono(&retained, rate))
            .await
    {
        if let Some(j) = state
            .dictation
            .jobs
            .lock()
            .await
            .iter_mut()
            .find(|j| j.id == id)
        {
            j.audio = Arc::new(recovery);
            j.rate = 16_000;
        }
    }
    let (cursor, mut text) = if let Some(task) = capture.progress {
        task.await.unwrap_or_default()
    } else {
        (0, String::new())
    };
    let _work = match early {
        Some(g) => g,
        None => work.await,
    };
    let _hud = crate::hud::HudCollapseAfterPtt::new(app);
    set_status(state, id, "processing", None).await;
    let result =
        crate::transcribe_recording(app, state, &audio[cursor.min(audio.len())..], rate).await;
    match result {
        Ok(tail) => {
            append_text(&mut text, &tail);
            complete(app, state, id, &text, started).await;
            if !text.is_empty() {
                if let Some(target) = capture.target {
                    let app_copy = app.clone();
                    let output = text.clone();
                    let insert = tokio::task::spawn_blocking(move || {
                        paste::paste_to_target(&app_copy, target, output)
                    })
                    .await;
                    if let Err(error) = insert.map_err(|e| e.to_string()).and_then(|v| v) {
                        set_status(state, id, "ready", Some(error)).await;
                        crate::set_dictation_outcome(
                            state,
                            "Text ready in Home — destination changed",
                        );
                    } else {
                        crate::set_dictation_outcome(state, "Dictation pasted");
                    }
                } else if capture.insert {
                    #[cfg(windows)]
                    {
                        set_status(
                            state,
                            id,
                            "ready",
                            Some(
                                "Could not identify the destination. Copy your transcript here."
                                    .into(),
                            ),
                        )
                        .await;
                        crate::set_dictation_outcome(
                            state,
                            "Transcript ready in Home — copy to insert",
                        );
                    }
                    #[cfg(not(windows))]
                    {
                        // Preserve the established macOS/Linux paste path. Native destination
                        // identity checks are currently implemented on Windows only.
                        if let Err(error) =
                            paste::paste_text_at_focus_spawn(app, text.clone()).await
                        {
                            set_status(state, id, "ready", Some(error)).await;
                        }
                    }
                }
            }
            Ok(text)
        }
        Err(error) => {
            set_status(state, id, "failed", Some(error.clone())).await;
            crate::set_dictation_outcome(state, "Dictation saved temporarily — retry from Home");
            Err(error)
        }
    }
}

async fn set_status(state: &AppState, id: u64, status: &str, error: Option<String>) {
    if let Some(j) = state
        .dictation
        .jobs
        .lock()
        .await
        .iter_mut()
        .find(|j| j.id == id)
    {
        j.status = status.into();
        j.error = error;
        if status == "ready" || status == "failed" {
            j.created = Instant::now();
        }
    }
}

async fn complete(app: &tauri::AppHandle, state: &AppState, id: u64, text: &str, started: Instant) {
    if let Some(j) = state
        .dictation
        .jobs
        .lock()
        .await
        .iter_mut()
        .find(|j| j.id == id)
    {
        j.text = text.into();
        j.status = "ready".into();
        j.error = None;
        j.created = Instant::now();
        j.elapsed_ms = started.elapsed().as_millis() as u64;
    }
    *state.last_transcript.lock().await = text.into();
    let _ = app.emit("transcript", text);
    crate::set_dictation_outcome(
        state,
        if text.is_empty() {
            "No speech detected"
        } else {
            "Transcript ready"
        },
    );
}

#[tauri::command]
pub async fn dictation_jobs(state: State<'_, AppState>) -> Result<Vec<JobView>, String> {
    let mut jobs = state.dictation.jobs.lock().await;
    prune(&mut jobs);
    Ok(jobs
        .iter()
        .rev()
        .map(|j| JobView {
            id: j.id,
            text: j.text.clone(),
            status: j.status.clone(),
            error: j.error.clone(),
            audio_seconds: j.audio.len() as f64 / j.rate.max(1) as f64,
            elapsed_ms: j.elapsed_ms,
            app_key: j.app_key.clone(),
        })
        .collect())
}

#[tauri::command]
pub async fn discard_dictation(state: State<'_, AppState>, id: u64) -> Result<(), String> {
    let mut jobs = state.dictation.jobs.lock().await;
    if jobs
        .iter()
        .any(|j| j.id == id && (j.status == "processing" || j.status == "queued"))
    {
        return Err("Wait for processing to finish before discarding".into());
    }
    jobs.retain(|j| j.id != id);
    Ok(())
}

#[tauri::command]
pub async fn retry_dictation(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: u64,
) -> Result<String, String> {
    let (audio, rate) = reserve(&state, id).await?;
    let _work = state.dictation.work.lock().await;
    set_status(&state, id, "processing", None).await;
    let started = Instant::now();
    match crate::transcribe_recording(&app, &state, &audio, rate).await {
        Ok(text) => {
            complete(&app, &state, id, &text, started).await;
            Ok(text)
        }
        Err(e) => {
            set_status(&state, id, "failed", Some(e.clone())).await;
            Err(e)
        }
    }
}

async fn reserve(state: &AppState, id: u64) -> Result<(Arc<Vec<f32>>, u32), String> {
    let mut jobs = state.dictation.jobs.lock().await;
    prune(&mut jobs);
    if state.ptt_session_active.load(Ordering::SeqCst)
        && jobs
            .iter()
            .filter(|j| j.status == "queued" || j.status == "processing")
            .count()
            >= MAX_JOBS - 1
    {
        return Err("The queue is full. Wait for a dictation to finish before retrying.".into());
    }
    reserve_job(&mut jobs, id)
}

fn reserve_job(jobs: &mut VecDeque<Job>, id: u64) -> Result<(Arc<Vec<f32>>, u32), String> {
    let j = jobs
        .iter_mut()
        .find(|j| j.id == id)
        .ok_or("Recording expired or was discarded")?;
    if j.status == "queued" || j.status == "processing" {
        return Err("This recording is already processing".into());
    }
    j.status = "queued".into();
    Ok((j.audio.clone(), j.rate))
}

#[derive(Serialize)]
pub struct Benchmark {
    audio_seconds: f64,
    processing_seconds: f64,
    realtime_factor: f64,
    recommendation: String,
}

#[tauri::command]
pub async fn benchmark_dictation(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: u64,
) -> Result<Benchmark, String> {
    if enabled(&app, "mock_transcription", false) {
        return Err("Turn off demo mode before benchmarking".into());
    }
    let (audio, rate) = reserve(&state, id).await?;
    let _work = state.dictation.work.lock().await;
    let seconds = audio.len() as f64 / rate.max(1) as f64;
    if seconds < 3.0 {
        set_status(&state, id, "ready", None).await;
        return Err("Record at least three seconds of speech for a useful benchmark".into());
    }
    set_status(&state, id, "processing", None).await;
    let result = async {
        crate::ensure_local_model_loaded(&app, &state).await?;
        let started = Instant::now();
        let text = crate::transcribe_recording(&app, &state, &audio, rate).await?;
        if text.trim().is_empty() { return Err("No speech recognized. Record a spoken sample before benchmarking.".to_string()); }
        let elapsed = started.elapsed().as_secs_f64();
        let ratio = elapsed / seconds;
        Ok(Benchmark { audio_seconds: seconds, processing_seconds: elapsed, realtime_factor: ratio,
            recommendation: if ratio > 0.7 { "Try the Fast Whisper preset. For English dictation, also compare Parakeet on this same recording." } else if ratio > 0.25 { "This setup keeps up with speech. Keep it, or try Fast for a shorter wait." } else { "This setup has speed to spare. Keep it, or compare Balanced / Accurate if you need fewer corrections." }.into() })
    }.await;
    match &result {
        Ok(_) => set_status(&state, id, "ready", None).await,
        Err(e) => set_status(&state, id, "failed", Some(e.clone())).await,
    }
    result
}

pub async fn busy(state: &AppState) -> bool {
    state.dictation.work.try_lock().is_err()
}

pub async fn expire(state: &AppState) {
    prune(&mut *state.dictation.jobs.lock().await);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: u64, status: &str) -> Job {
        Job {
            id,
            audio: Arc::new(vec![0.1; 1600]),
            rate: 16000,
            created: Instant::now(),
            text: "Previous result".into(),
            status: status.into(),
            error: None,
            elapsed_ms: 0,
            app_key: None,
        }
    }

    #[test]
    fn expired_recordings_are_removed_but_active_jobs_survive() {
        let mut jobs = VecDeque::from([
            job(1, "failed"),
            job(2, "processing"),
            job(3, "queued"),
            job(4, "ready"),
        ]);
        for j in jobs.iter_mut().take(3) {
            j.created = Instant::now() - KEEP_FOR - Duration::from_secs(1);
        }
        prune(&mut jobs);
        assert_eq!(jobs.iter().map(|j| j.id).collect::<Vec<_>>(), vec![2, 3, 4]);
    }

    #[test]
    fn retry_reservation_preserves_audio_and_prevents_duplicate_work() {
        let mut jobs = VecDeque::from([job(1, "failed")]);
        let (audio, rate) = reserve_job(&mut jobs, 1).unwrap();
        assert_eq!(rate, 16000);
        assert!(Arc::ptr_eq(&audio, &jobs[0].audio));
        assert_eq!(jobs[0].text, "Previous result");
        assert!(reserve_job(&mut jobs, 1).is_err());
        assert!(reserve_job(&mut jobs, 99).is_err());
    }

    #[tokio::test]
    async fn finishing_job_keeps_fifo_position_while_new_recording_starts() {
        let work = Arc::new(Mutex::new(()));
        let initial = work.lock().await;
        let first = work.lock();
        tokio::pin!(first);
        let early = tokio::select! { biased; guard = &mut first => Some(guard), _ = std::future::ready(()) => None };
        assert!(early.is_none());
        let next_work = work.clone();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let next = tokio::spawn(async move {
            let _guard = next_work.lock().await;
            tx.send(()).await.unwrap();
        });
        drop(initial);
        let first_guard = tokio::time::timeout(Duration::from_secs(1), first)
            .await
            .unwrap();
        assert!(rx.try_recv().is_err());
        drop(first_guard);
        tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap();
        next.await.unwrap();
    }

    #[test]
    fn completed_phrases_join_without_duplication_or_empty_spacing() {
        let mut text = String::new();
        append_text(&mut text, " First phrase. ");
        append_text(&mut text, "");
        append_text(&mut text, "Second phrase.");
        assert_eq!(text, "First phrase. Second phrase.");
    }
}

pub async fn pending_count(state: &AppState) -> usize {
    state
        .dictation
        .jobs
        .lock()
        .await
        .iter()
        .filter(|j| j.status == "queued" || j.status == "processing")
        .count()
}

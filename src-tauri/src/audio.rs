use audioadapter_buffers::direct::InterleavedSlice;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use rubato::{
    calculate_cutoff, Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

/// Last callback-sized RMS / peak for a simple input meter (0..1, float samples).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputLevelState {
    pub rms: f32,
    pub peak: f32,
}

impl Default for InputLevelState {
    fn default() -> Self {
        Self { rms: 0.0, peak: 0.0 }
    }
}

fn update_input_levels(levels: &Arc<Mutex<InputLevelState>>, chunk: &[f32]) {
    if chunk.is_empty() {
        return;
    }
    let mut pk = 0.0f32;
    let mut sum_sq = 0.0f32;
    for &s in chunk {
        let a = s.abs();
        if a > pk {
            pk = a;
        }
        sum_sq += s * s;
    }
    let rms = (sum_sq / chunk.len() as f32).sqrt();
    if let Ok(mut g) = levels.lock() {
        g.rms = g.rms * 0.82 + rms * 0.18;
        g.peak *= 0.91;
        if pk > g.peak {
            g.peak = pk;
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioInputDevice {
    /// Empty string means OS default input.
    pub id: String,
    pub label: String,
}

pub fn list_input_devices() -> Result<Vec<AudioInputDevice>, String> {
    let host = cpal::default_host();
    let mut v = vec![AudioInputDevice {
        id: String::new(),
        label: "System default".into(),
    }];
    for d in host.input_devices().map_err(|e| e.to_string())? {
        let name = d.name().map_err(|e| e.to_string())?;
        v.push(AudioInputDevice {
            id: name.clone(),
            label: name,
        });
    }
    Ok(v)
}

fn resolve_input_device(host: &cpal::Host, name: Option<&str>) -> Result<cpal::Device, String> {
    if let Some(n) = name.map(str::trim).filter(|s| !s.is_empty()) {
        for d in host.input_devices().map_err(|e| e.to_string())? {
            if d.name().map_err(|e| e.to_string())? == n {
                return Ok(d);
            }
        }
        return Err(format!("Microphone not found: {n}"));
    }
    host.default_input_device()
        .ok_or_else(|| "No default input device".into())
}

/// Commands to the dedicated microphone thread (`cpal::Stream` is not `Send` / `Sync`).
pub enum PttControlCmd {
    Start(mpsc::Sender<Result<(), String>>, Option<String>),
    Stop(mpsc::Sender<Result<(Vec<f32>, u32), String>>),
}

/// Handle cloned into `AppState`; all capture runs on a single background thread.
#[derive(Clone)]
pub struct PttController {
    tx: Arc<Mutex<Sender<PttControlCmd>>>,
    pub input_levels: Arc<Mutex<InputLevelState>>,
}

impl PttController {
    pub fn spawn() -> Self {
        let input_levels = Arc::new(Mutex::new(InputLevelState::default()));
        let levels_for_thread = Arc::clone(&input_levels);
        let (tx, rx) = mpsc::channel::<PttControlCmd>();
        std::thread::spawn(move || {
            let mut cap = PttCapture::new(levels_for_thread);
            while let Ok(cmd) = rx.recv() {
                match cmd {
                    PttControlCmd::Start(reply, device) => {
                        let res = (|| {
                            cap.start_stream(device.as_deref())?;
                            cap.set_recording(true);
                            Ok::<_, String>(())
                        })();
                        let _ = reply.send(res);
                    }
                    PttControlCmd::Stop(reply) => {
                        cap.set_recording(false);
                        let samples = cap.take_buffer_f32();
                        let rate = cap.input_sample_rate;
                        let _ = reply.send(Ok((samples, rate)));
                    }
                }
            }
        });
        Self {
            tx: Arc::new(Mutex::new(tx)),
            input_levels,
        }
    }

    pub fn snapshot_input_levels(&self) -> InputLevelState {
        self.input_levels
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    pub fn start(&self, device_name: Option<String>) -> Result<(), String> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .lock()
            .map_err(|_| "microphone thread lock poisoned".to_string())?
            .send(PttControlCmd::Start(reply_tx, device_name))
            .map_err(|_| "microphone thread stopped".to_string())?;
        reply_rx
            .recv()
            .map_err(|_| "microphone thread stopped".to_string())?
    }

    pub fn stop(&self) -> Result<(Vec<f32>, u32), String> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .lock()
            .map_err(|_| "microphone thread lock poisoned".to_string())?
            .send(PttControlCmd::Stop(reply_tx))
            .map_err(|_| "microphone thread stopped".to_string())?;
        reply_rx
            .recv()
            .map_err(|_| "microphone thread stopped".to_string())?
    }
}

pub struct PttCapture {
    pub input_sample_rate: u32,
    stream: Option<Stream>,
    /// Which device the open stream targets (`None` = default).
    stream_device_key: Option<String>,
    recording: Arc<AtomicBool>,
    buffer: Arc<Mutex<Vec<f32>>>,
    input_levels: Arc<Mutex<InputLevelState>>,
}

impl PttCapture {
    pub fn new(input_levels: Arc<Mutex<InputLevelState>>) -> Self {
        Self {
            input_sample_rate: 48_000,
            stream: None,
            stream_device_key: None,
            recording: Arc::new(AtomicBool::new(false)),
            buffer: Arc::new(Mutex::new(Vec::new())),
            input_levels,
        }
    }

    pub fn start_stream(&mut self, device_name: Option<&str>) -> Result<(), String> {
        let wanted_key = device_name
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        if self.stream.is_some() && self.stream_device_key == wanted_key {
            return Ok(());
        }
        self.stop_stream();

        let host = cpal::default_host();
        let device = resolve_input_device(&host, device_name)?;
        let cfg = device
            .default_input_config()
            .map_err(|e| e.to_string())?;
        self.input_sample_rate = cfg.sample_rate().0;

        let recording = Arc::clone(&self.recording);
        let buffer = Arc::clone(&self.buffer);
        let levels = Arc::clone(&self.input_levels);

        let stream_cfg: cpal::StreamConfig = cfg.clone().into();
        let channels = stream_cfg.channels as usize;
        if channels == 0 {
            return Err("Microphone reports zero channels".into());
        }
        let stream = match cfg.sample_format() {
            SampleFormat::F32 => build_stream_f32(&device, &stream_cfg, channels, recording, buffer, levels),
            SampleFormat::I16 => build_stream_i16(&device, &stream_cfg, channels, recording, buffer, levels),
            SampleFormat::U16 => build_stream_u16(&device, &stream_cfg, channels, recording, buffer, levels),
            f => Err(format!("Unsupported sample format {f:?}")),
        }?;

        stream.play().map_err(|e| e.to_string())?;
        self.stream = Some(stream);
        self.stream_device_key = wanted_key;
        Ok(())
    }

    pub fn stop_stream(&mut self) {
        self.stream.take();
        self.stream_device_key = None;
    }

    pub fn set_recording(&self, on: bool) {
        self.recording.store(on, Ordering::SeqCst);
        if on {
            if let Ok(mut b) = self.buffer.lock() {
                b.clear();
            }
            if let Ok(mut lv) = self.input_levels.lock() {
                *lv = InputLevelState::default();
            }
        } else if let Ok(mut lv) = self.input_levels.lock() {
            *lv = InputLevelState::default();
        }
    }

    pub fn take_buffer_f32(&self) -> Vec<f32> {
        self.buffer.lock().map(|mut b| std::mem::take(&mut *b)).unwrap_or_default()
    }
}

fn push_interleaved_to_mono(buffer: &mut Vec<f32>, data: &[f32], channels: usize) {
    if channels <= 1 {
        buffer.extend_from_slice(data);
        return;
    }
    for frame in data.chunks_exact(channels) {
        let s: f32 = frame.iter().sum::<f32>() / channels as f32;
        buffer.push(s);
    }
}

fn build_stream_f32(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    recording: Arc<AtomicBool>,
    buffer: Arc<Mutex<Vec<f32>>>,
    levels: Arc<Mutex<InputLevelState>>,
) -> Result<Stream, String> {
    let err_fn = |e| eprintln!("cpal: {e}");
    let stream = device
        .build_input_stream(
            config,
            move |data: &[f32], _| {
                if recording.load(Ordering::SeqCst) {
                    if channels <= 1 {
                        update_input_levels(&levels, data);
                    } else {
                        let mut mono = Vec::with_capacity(data.len() / channels);
                        for frame in data.chunks_exact(channels) {
                            mono.push(frame.iter().sum::<f32>() / channels as f32);
                        }
                        update_input_levels(&levels, &mono);
                    }
                    if let Ok(mut b) = buffer.lock() {
                        push_interleaved_to_mono(&mut b, data, channels);
                    }
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| e.to_string())?;
    Ok(stream)
}

fn build_stream_i16(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    recording: Arc<AtomicBool>,
    buffer: Arc<Mutex<Vec<f32>>>,
    levels: Arc<Mutex<InputLevelState>>,
) -> Result<Stream, String> {
    let err_fn = |e| eprintln!("cpal: {e}");
    let stream = device
        .build_input_stream(
            config,
            move |data: &[i16], _| {
                if recording.load(Ordering::SeqCst) {
                    let mut mono: Vec<f32> = Vec::with_capacity(data.len() / channels.max(1));
                    if channels <= 1 {
                        for &s in data {
                            mono.push(s as f32 / 32768.0);
                        }
                    } else {
                        for frame in data.chunks_exact(channels) {
                            let s: f32 = frame.iter().map(|&x| x as f32).sum::<f32>()
                                / (channels as f32 * 32768.0);
                            mono.push(s);
                        }
                    }
                    update_input_levels(&levels, &mono);
                    if let Ok(mut b) = buffer.lock() {
                        b.extend_from_slice(&mono);
                    }
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| e.to_string())?;
    Ok(stream)
}

fn build_stream_u16(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    recording: Arc<AtomicBool>,
    buffer: Arc<Mutex<Vec<f32>>>,
    levels: Arc<Mutex<InputLevelState>>,
) -> Result<Stream, String> {
    let err_fn = |e| eprintln!("cpal: {e}");
    let stream = device
        .build_input_stream(
            config,
            move |data: &[u16], _| {
                if recording.load(Ordering::SeqCst) {
                    let mut mono: Vec<f32> = Vec::with_capacity(data.len() / channels.max(1));
                    if channels <= 1 {
                        for &s in data {
                            mono.push((s as f32 / 32768.0) - 1.0);
                        }
                    } else {
                        for frame in data.chunks_exact(channels) {
                            let s: f32 = frame
                                .iter()
                                .map(|&x| (x as f32 / 32768.0) - 1.0)
                                .sum::<f32>()
                                / channels as f32;
                            mono.push(s);
                        }
                    }
                    update_input_levels(&levels, &mono);
                    if let Ok(mut b) = buffer.lock() {
                        b.extend_from_slice(&mono);
                    }
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| e.to_string())?;
    Ok(stream)
}

/// Remove DC offset and scale toward `peak_target` (e.g. 0.88) so Whisper gets consistent levels.
/// `max_gain_cap` limits boost for very quiet mics (typical 8–16).
pub fn condition_speech_signal(samples: &[f32], peak_target: f32, max_gain_cap: f32) -> Vec<f32> {
    if samples.is_empty() {
        return vec![];
    }
    let peak_target = peak_target.clamp(0.05, 0.99);
    let max_gain_cap = max_gain_cap.clamp(1.0, 48.0);
    let n = samples.len() as f32;
    let mean = samples.iter().sum::<f32>() / n;
    let mut v: Vec<f32> = samples.iter().map(|&s| s - mean).collect();
    let peak = v.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    if peak > 1e-9 {
        let g = (peak_target / peak).min(max_gain_cap);
        for s in &mut v {
            *s *= g;
        }
    }
    v
}

/// 16 kHz mono for Whisper using band-limited sinc resampling (much cleaner than linear).
pub fn resample_to_whisper_16k_mono(input: &[f32], from_rate: u32) -> Vec<f32> {
    if input.is_empty() || from_rate == 0 {
        return vec![];
    }
    if from_rate == 16_000 {
        return input.to_vec();
    }
    let ratio = 16_000f64 / from_rate as f64;
    let n_frames = input.len();
    let ch = 1usize;
    let sinc_len = 128;
    let window = WindowFunction::Blackman2;
    let params = SincInterpolationParameters {
        sinc_len,
        f_cutoff: calculate_cutoff(sinc_len, window),
        interpolation: SincInterpolationType::Cubic,
        oversampling_factor: 256,
        window,
    };
    let Ok(mut resampler) = Async::<f32>::new_sinc(ratio, 1.1, &params, 1024, ch, FixedAsync::Input)
    else {
        return resample_linear(input, from_rate, 16_000);
    };
    let Ok(adapter_in) = InterleavedSlice::new(input, ch, n_frames) else {
        return resample_linear(input, from_rate, 16_000);
    };
    let out_frames = resampler.process_all_needed_output_len(n_frames);
    let mut outdata = vec![0f32; out_frames * ch];
    let Ok(mut adapter_out) = InterleavedSlice::new_mut(&mut outdata, ch, out_frames) else {
        return resample_linear(input, from_rate, 16_000);
    };
    match resampler.process_all_into_buffer(&adapter_in, &mut adapter_out, n_frames, None) {
        Ok((_n_in, n_out)) => outdata.truncate(n_out * ch),
        Err(e) => {
            eprintln!("[yapper] rubato resample failed: {e}; using linear");
            return resample_linear(input, from_rate, 16_000);
        }
    }
    outdata
}

/// Simple RMS energy gate: returns slice ranges (start sample, end exclusive) for segments above threshold.
pub fn vad_segments(samples: &[f32], threshold: f32, min_silence_ms: u32, sample_rate: u32) -> Vec<(usize, usize)> {
    let min_silence = (sample_rate as f32 * min_silence_ms as f32 / 1000.0) as usize;
    if samples.is_empty() {
        return vec![];
    }
    let frame = (sample_rate as f32 * 0.02) as usize; // 20ms
    let frame = frame.max(1);
    let mut segments = Vec::new();
    let mut i = 0;
    while i < samples.len() {
        let end = (i + frame).min(samples.len());
        let chunk = &samples[i..end];
        let rms = (chunk.iter().map(|x| x * x).sum::<f32>() / chunk.len() as f32).sqrt();
        if rms >= threshold {
            let start = i;
            let mut j = end;
            let mut silence = 0usize;
            while j < samples.len() {
                let e = (j + frame).min(samples.len());
                let c = &samples[j..e];
                let r = (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt();
                if r < threshold {
                    silence += e - j;
                    if silence >= min_silence {
                        break;
                    }
                } else {
                    silence = 0;
                }
                j = e;
            }
            let seg_end = j.min(samples.len());
            if seg_end > start {
                segments.push((start, seg_end));
            }
            i = seg_end;
        } else {
            i = end;
        }
    }
    if segments.is_empty() && !samples.is_empty() {
        segments.push((0, samples.len()));
    }
    segments
}

pub fn resample_linear(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == 0 || input.is_empty() {
        return vec![];
    }
    if from_rate == to_rate {
        return input.to_vec();
    }
    let out_len = ((input.len() as f64) * (to_rate as f64) / (from_rate as f64)).round() as usize;
    let mut out = Vec::with_capacity(out_len.max(1));
    for i in 0..out_len.max(1) {
        let src_pos = (i as f64) * (from_rate as f64) / (to_rate as f64);
        let idx = src_pos.floor() as usize;
        let frac = src_pos - idx as f64;
        let a = input.get(idx).copied().unwrap_or(0.0);
        let b = input.get((idx + 1).min(input.len().saturating_sub(1)))
            .copied()
            .unwrap_or(a);
        out.push(a + (b - a) * frac as f32);
    }
    out
}

pub fn f32_to_i16_le_bytes(samples: &[f32]) -> Vec<u8> {
    let mut v = Vec::with_capacity(samples.len() * 2);
    for &s in samples {
        let x = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        v.extend_from_slice(&x.to_le_bytes());
    }
    v
}

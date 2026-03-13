use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use std::fs::{remove_file, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use uuid::Uuid;

const TARGET_RATE: u32 = 48_000;

type I16Sink = Arc<Mutex<BufWriter<File>>>;

#[derive(Debug, Clone, Copy, Default)]
pub struct LiveLevels {
    pub mic: f32,
    pub system: f32,
}

#[derive(Clone)]
pub struct SharedLevels {
    mic: Arc<AtomicU32>,
    system: Arc<AtomicU32>,
}

impl Default for SharedLevels {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedLevels {
    pub fn new() -> Self {
        Self {
            mic: Arc::new(AtomicU32::new(0.0f32.to_bits())),
            system: Arc::new(AtomicU32::new(0.0f32.to_bits())),
        }
    }

    pub fn reset(&self) {
        self.set_mic(0.0);
        self.set_system(0.0);
    }

    pub fn set_mic(&self, value: f32) {
        store_level(&self.mic, value);
    }

    pub fn set_system(&self, value: f32) {
        store_level(&self.system, value);
    }

    pub fn snapshot(&self) -> LiveLevels {
        LiveLevels {
            mic: load_level(&self.mic),
            system: load_level(&self.system),
        }
    }

    fn mic_meter(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.mic)
    }

    fn system_meter(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.system)
    }
}

fn store_level(target: &AtomicU32, value: f32) {
    target.store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
}

fn load_level(target: &AtomicU32) -> f32 {
    let raw = f32::from_bits(target.load(Ordering::Relaxed));
    if raw.is_finite() {
        raw.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn normalize_signal_level(linear_rms: f32) -> f32 {
    if !linear_rms.is_finite() {
        return 0.0;
    }
    let linear = linear_rms.abs().clamp(0.0, 1.0);
    if linear <= 1.0e-6 {
        return 0.0;
    }

    // Map linear RMS into a perceptual dB scale close to the macOS input meter feel.
    let db = 20.0 * linear.log10();
    let db_floor = -55.0_f32;
    let normalized = ((db - db_floor) / (0.0 - db_floor)).clamp(0.0, 1.0);
    normalized.powf(0.72)
}

fn update_meter_level(target: &AtomicU32, measured_linear_rms: f32) {
    let mapped = normalize_signal_level(measured_linear_rms);
    let prev = load_level(target);
    let attack = 0.84_f32;
    let release = 0.38_f32;
    let alpha = if mapped > prev { attack } else { release };
    let next = prev + (mapped - prev) * alpha;
    store_level(target, next);
}

pub struct ContinuousCapture {
    stop_tx: Sender<()>,
    join: Option<thread::JoinHandle<Result<CaptureArtifacts, String>>>,
}

pub struct CaptureArtifacts {
    pub mic_path: PathBuf,
    pub mic_rate: u32,
    pub system_path: Option<PathBuf>,
    pub system_rate: u32,
}

impl ContinuousCapture {
    pub fn start(
        mic_name: Option<String>,
        system_name: Option<String>,
        levels: SharedLevels,
    ) -> Result<Self, String> {
        let host = cpal::default_host();
        select_input_device(&host, mic_name.as_deref())
            .or_else(|| host.default_input_device())
            .ok_or_else(|| "No input device available for microphone".to_string())?;

        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let join = thread::spawn(move || capture_until_stopped(stop_rx, mic_name, system_name, levels));

        Ok(Self {
            stop_tx,
            join: Some(join),
        })
    }

    pub fn stop_and_take_artifacts(mut self) -> Result<CaptureArtifacts, String> {
        let _ = self.stop_tx.send(());
        let result = match self.join.take() {
            Some(handle) => handle
                .join()
                .map_err(|_| "Audio capture thread panicked".to_string())??,
            None => {
                return Err("Audio capture thread is missing".to_string());
            }
        };
        Ok(result)
    }
}

fn capture_until_stopped(
    stop_rx: mpsc::Receiver<()>,
    mic_name: Option<String>,
    system_name: Option<String>,
    levels: SharedLevels,
) -> Result<CaptureArtifacts, String> {
    let host = cpal::default_host();
    levels.reset();

    let mic_device = select_input_device(&host, mic_name.as_deref())
        .or_else(|| host.default_input_device())
        .ok_or_else(|| "No input device available for microphone".to_string())?;

    let mic_device_name = mic_device.name().ok();
    let device_names = list_input_devices_for_host(&host).unwrap_or_default();
    let resolved_system_name = resolve_system_source_name(
        system_name.as_deref(),
        mic_device_name.as_deref(),
        &device_names,
    );
    let system_device = resolved_system_name
        .as_deref()
        .and_then(|name| select_input_device(&host, Some(name)));

    let mic_path = temp_raw_path("mic");
    let mic_file = File::create(&mic_path).map_err(|e| e.to_string())?;
    let mic_sink = Arc::new(Mutex::new(BufWriter::new(mic_file)));

    let (mic_stream, mic_rate) =
        build_capture_stream(&mic_device, Arc::clone(&mic_sink), levels.mic_meter())?;
    mic_stream.play().map_err(|e| e.to_string())?;

    let mut system_path = None;
    let mut system_stream = None;
    let mut system_rate = TARGET_RATE;
    let mut maybe_system_sink: Option<I16Sink> = None;

    if let Some(dev) = system_device {
        let path = temp_raw_path("sys");
        let file = File::create(&path).map_err(|e| e.to_string())?;
        let sink = Arc::new(Mutex::new(BufWriter::new(file)));
        let (stream, rate) = build_capture_stream(&dev, Arc::clone(&sink), levels.system_meter())?;
        stream.play().map_err(|e| e.to_string())?;
        system_path = Some(path);
        system_stream = Some(stream);
        system_rate = rate;
        maybe_system_sink = Some(sink);
    }

    loop {
        match stop_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(_) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    drop(mic_stream);
    drop(system_stream);
    levels.reset();

    if let Ok(mut writer) = mic_sink.lock() {
        writer.flush().map_err(|e| e.to_string())?;
    }
    if let Some(system_sink) = maybe_system_sink {
        if let Ok(mut writer) = system_sink.lock() {
            writer.flush().map_err(|e| e.to_string())?;
        }
    }

    Ok(CaptureArtifacts {
        mic_path,
        mic_rate,
        system_path,
        system_rate,
    })
}

pub fn list_input_devices() -> Result<Vec<String>, String> {
    let host = cpal::default_host();
    list_input_devices_for_host(&host)
}

fn list_input_devices_for_host(host: &cpal::Host) -> Result<Vec<String>, String> {
    let devices = host.input_devices().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for d in devices {
        if let Ok(name) = d.name() {
            out.push(name);
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

pub fn detect_system_source_device() -> Result<Option<String>, String> {
    let devices = list_input_devices()?;
    Ok(select_best_system_source(&devices))
}

pub fn probe_levels(mic_name: Option<&str>, system_name: Option<&str>) -> Result<LiveLevels, String> {
    let host = cpal::default_host();
    let mic_device = select_input_device(&host, mic_name)
        .or_else(|| host.default_input_device())
        .ok_or_else(|| "No input device available for microphone".to_string())?;

    let mic_device_name = mic_device.name().ok();
    let device_names = list_input_devices_for_host(&host).unwrap_or_default();
    let resolved_system_name = resolve_system_source_name(system_name, mic_device_name.as_deref(), &device_names);
    let system_device = resolved_system_name
        .as_deref()
        .and_then(|name| select_input_device(&host, Some(name)));

    let mic_meter = Arc::new(AtomicU32::new(0.0f32.to_bits()));
    let system_meter = Arc::new(AtomicU32::new(0.0f32.to_bits()));

    let mic_stream = build_probe_level_stream(&mic_device, Arc::clone(&mic_meter))?;
    mic_stream.play().map_err(|e| e.to_string())?;

    let mut system_stream = None;
    if let Some(dev) = system_device {
        let stream = build_probe_level_stream(&dev, Arc::clone(&system_meter))?;
        stream.play().map_err(|e| e.to_string())?;
        system_stream = Some(stream);
    }

    std::thread::sleep(Duration::from_millis(72));
    drop(mic_stream);
    drop(system_stream);

    Ok(LiveLevels {
        mic: load_level(&mic_meter),
        system: load_level(&system_meter),
    })
}

fn resolve_system_source_name(
    preferred_name: Option<&str>,
    mic_name: Option<&str>,
    devices: &[String],
) -> Option<String> {
    let mic = mic_name.map(|v| v.trim().to_lowercase()).unwrap_or_default();
    let is_mic = |name: &str| !mic.is_empty() && name.trim().to_lowercase() == mic;

    if let Some(preferred) = preferred_name {
        let preferred = preferred.trim();
        if !preferred.is_empty() {
            for device in devices {
                if device.eq_ignore_ascii_case(preferred) && !is_mic(device) {
                    return Some(device.clone());
                }
            }
        }
    }

    let filtered = devices
        .iter()
        .filter(|name| !is_mic(name))
        .cloned()
        .collect::<Vec<_>>();
    select_best_system_source(&filtered)
}

fn select_best_system_source(devices: &[String]) -> Option<String> {
    let os = std::env::consts::OS;
    let mut best: Option<(i32, String)> = None;
    for name in devices {
        let score = score_system_source_name(name, os);
        if score <= 0 {
            continue;
        }
        match &best {
            Some((best_score, _)) if *best_score >= score => {}
            _ => best = Some((score, name.clone())),
        }
    }
    best.map(|(_, name)| name)
}

fn score_system_source_name(name: &str, os: &str) -> i32 {
    let n = name.to_lowercase();
    let mut score = 0;

    // Cross-platform strong indicators.
    let strong_hits = ["loopback", "stereo mix", "what u hear", "monitor of"];
    let weak_hits = ["mix", "virtual", "aggregate", "capture", "output"];

    // Platform-specific preferred devices.
    let platform_strong_hits: &[&str] = match os {
        "macos" => &["blackhole", "soundflower", "loopback audio", "aggregate device"],
        "windows" => &["vb-cable", "cable output", "stereo mix", "what u hear", "loopback"],
        _ => &["loopback", "monitor of", "stereo mix", "vb-cable", "blackhole"],
    };

    for key in strong_hits {
        if n.contains(key) {
            score += 10;
        }
    }
    for key in platform_strong_hits {
        if n.contains(key) {
            score += 16;
        }
    }
    for key in weak_hits {
        if n.contains(key) {
            score += 3;
        }
    }

    if n.contains("microphone") || n.contains("mic") {
        score -= 4;
    }
    if n.contains("default") {
        score -= 1;
    }

    score
}

fn select_input_device(host: &cpal::Host, preferred_name: Option<&str>) -> Option<Device> {
    if let Some(name) = preferred_name {
        let needle = name.trim().to_lowercase();
        if needle.is_empty() {
            return None;
        }
        if let Ok(devices) = host.input_devices() {
            for d in devices {
                if d.name().map(|n| n.to_lowercase() == needle).unwrap_or(false) {
                    return Some(d);
                }
            }
        }
        None
    } else {
        None
    }
}

fn build_capture_stream(
    device: &Device,
    sink: I16Sink,
    level_output: Arc<AtomicU32>,
) -> Result<(Stream, u32), String> {
    let default_cfg = device
        .default_input_config()
        .map_err(|e| format!("default input config error: {e}"))?;

    let mut cfg: StreamConfig = default_cfg.clone().into();
    cfg.channels = cfg.channels.max(1);

    let sample_rate = cfg.sample_rate.0;
    let channels = cfg.channels as usize;

    let err_fn = |err| eprintln!("audio capture stream error: {err}");

    let stream = match default_cfg.sample_format() {
        SampleFormat::F32 => {
            let sink = Arc::clone(&sink);
            let level_output = Arc::clone(&level_output);
            device
                .build_input_stream(
                    &cfg,
                    move |data: &[f32], _| append_mono_f32_as_i16(data, channels, &sink, &level_output),
                    err_fn,
                    None,
                )
                .map_err(|e| e.to_string())?
        }
        SampleFormat::I16 => {
            let sink = Arc::clone(&sink);
            let level_output = Arc::clone(&level_output);
            device
                .build_input_stream(
                    &cfg,
                    move |data: &[i16], _| append_mono_i16(data, channels, &sink, &level_output),
                    err_fn,
                    None,
                )
                .map_err(|e| e.to_string())?
        }
        SampleFormat::U16 => {
            let sink = Arc::clone(&sink);
            let level_output = Arc::clone(&level_output);
            device
                .build_input_stream(
                    &cfg,
                    move |data: &[u16], _| append_mono_u16_as_i16(data, channels, &sink, &level_output),
                    err_fn,
                    None,
                )
                .map_err(|e| e.to_string())?
        }
        _ => return Err("Unsupported input sample format".to_string()),
    };

    Ok((stream, sample_rate))
}

fn build_probe_level_stream(device: &Device, level_output: Arc<AtomicU32>) -> Result<Stream, String> {
    let default_cfg = device
        .default_input_config()
        .map_err(|e| format!("default input config error: {e}"))?;
    let mut cfg: StreamConfig = default_cfg.clone().into();
    cfg.channels = cfg.channels.max(1);
    let channels = cfg.channels as usize;
    let err_fn = |err| eprintln!("audio probe stream error: {err}");

    let stream = match default_cfg.sample_format() {
        SampleFormat::F32 => {
            let level_output = Arc::clone(&level_output);
            device
                .build_input_stream(
                    &cfg,
                    move |data: &[f32], _| update_peak_level_f32(data, channels, &level_output),
                    err_fn,
                    None,
                )
                .map_err(|e| e.to_string())?
        }
        SampleFormat::I16 => {
            let level_output = Arc::clone(&level_output);
            device
                .build_input_stream(
                    &cfg,
                    move |data: &[i16], _| update_peak_level_i16(data, channels, &level_output),
                    err_fn,
                    None,
                )
                .map_err(|e| e.to_string())?
        }
        SampleFormat::U16 => {
            let level_output = Arc::clone(&level_output);
            device
                .build_input_stream(
                    &cfg,
                    move |data: &[u16], _| update_peak_level_u16(data, channels, &level_output),
                    err_fn,
                    None,
                )
                .map_err(|e| e.to_string())?
        }
        _ => return Err("Unsupported input sample format".to_string()),
    };

    Ok(stream)
}

fn append_mono_f32_as_i16(data: &[f32], channels: usize, sink: &I16Sink, level_output: &Arc<AtomicU32>) {
    if channels == 0 {
        return;
    }
    let mut bytes = Vec::with_capacity((data.len() / channels) * 2);
    let mut sum_sq = 0.0_f32;
    let mut samples = 0_usize;
    for frame in data.chunks(channels) {
        if let Some(first) = frame.first() {
            let v = (*first).clamp(-1.0, 1.0);
            sum_sq += v * v;
            samples += 1;
            let s = (v * i16::MAX as f32) as i16;
            bytes.extend_from_slice(&s.to_le_bytes());
        }
    }
    let rms = if samples > 0 {
        (sum_sq / samples as f32).sqrt()
    } else {
        0.0
    };
    update_meter_level(level_output, rms);
    if let Ok(mut writer) = sink.lock() {
        let _ = writer.write_all(&bytes);
    }
}

fn append_mono_i16(data: &[i16], channels: usize, sink: &I16Sink, level_output: &Arc<AtomicU32>) {
    if channels == 0 {
        return;
    }
    let mut bytes = Vec::with_capacity((data.len() / channels) * 2);
    let mut sum_sq = 0.0_f32;
    let mut samples = 0_usize;
    for frame in data.chunks(channels) {
        if let Some(first) = frame.first() {
            let v = *first as f32 / i16::MAX as f32;
            sum_sq += v * v;
            samples += 1;
            bytes.extend_from_slice(&first.to_le_bytes());
        }
    }
    let rms = if samples > 0 {
        (sum_sq / samples as f32).sqrt()
    } else {
        0.0
    };
    update_meter_level(level_output, rms);
    if let Ok(mut writer) = sink.lock() {
        let _ = writer.write_all(&bytes);
    }
}

fn append_mono_u16_as_i16(data: &[u16], channels: usize, sink: &I16Sink, level_output: &Arc<AtomicU32>) {
    if channels == 0 {
        return;
    }
    let mut bytes = Vec::with_capacity((data.len() / channels) * 2);
    let mut sum_sq = 0.0_f32;
    let mut samples = 0_usize;
    for frame in data.chunks(channels) {
        if let Some(first) = frame.first() {
            let f = (*first as f32 / u16::MAX as f32) * 2.0 - 1.0;
            sum_sq += f * f;
            samples += 1;
            let s = (f.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            bytes.extend_from_slice(&s.to_le_bytes());
        }
    }
    let rms = if samples > 0 {
        (sum_sq / samples as f32).sqrt()
    } else {
        0.0
    };
    update_meter_level(level_output, rms);
    if let Ok(mut writer) = sink.lock() {
        let _ = writer.write_all(&bytes);
    }
}

fn update_peak_level_f32(data: &[f32], channels: usize, level_output: &Arc<AtomicU32>) {
    if channels == 0 {
        return;
    }
    let mut sum_sq = 0.0_f32;
    let mut samples = 0_usize;
    for frame in data.chunks(channels) {
        if let Some(first) = frame.first() {
            let v = first.clamp(-1.0, 1.0);
            sum_sq += v * v;
            samples += 1;
        }
    }
    let rms = if samples > 0 {
        (sum_sq / samples as f32).sqrt()
    } else {
        0.0
    };
    update_meter_level(level_output, rms);
}

fn update_peak_level_i16(data: &[i16], channels: usize, level_output: &Arc<AtomicU32>) {
    if channels == 0 {
        return;
    }
    let mut sum_sq = 0.0_f32;
    let mut samples = 0_usize;
    for frame in data.chunks(channels) {
        if let Some(first) = frame.first() {
            let v = *first as f32 / i16::MAX as f32;
            sum_sq += v * v;
            samples += 1;
        }
    }
    let rms = if samples > 0 {
        (sum_sq / samples as f32).sqrt()
    } else {
        0.0
    };
    update_meter_level(level_output, rms);
}

fn update_peak_level_u16(data: &[u16], channels: usize, level_output: &Arc<AtomicU32>) {
    if channels == 0 {
        return;
    }
    let mut sum_sq = 0.0_f32;
    let mut samples = 0_usize;
    for frame in data.chunks(channels) {
        if let Some(first) = frame.first() {
            let f = (*first as f32 / u16::MAX as f32) * 2.0 - 1.0;
            sum_sq += f * f;
            samples += 1;
        }
    }
    let rms = if samples > 0 {
        (sum_sq / samples as f32).sqrt()
    } else {
        0.0
    };
    update_meter_level(level_output, rms);
}

pub fn cleanup_artifacts(result: &CaptureArtifacts) {
    let _ = remove_file(&result.mic_path);
    if let Some(path) = &result.system_path {
        let _ = remove_file(path);
    }
}

#[cfg(test)]
fn resample_i16(input: &[i16], src_rate: u32, target_rate: u32) -> Vec<i16> {
    if input.is_empty() {
        return vec![];
    }
    if src_rate == 0 || src_rate == target_rate {
        return input.to_vec();
    }

    let out_len = ((input.len() as f64) * (target_rate as f64 / src_rate as f64)).round() as usize;
    let out_len = out_len.max(1);
    let mut out = Vec::with_capacity(out_len);

    for i in 0..out_len {
        let src_pos = (i as f64) * (src_rate as f64 / target_rate as f64);
        let idx = src_pos.floor() as usize;
        let frac = (src_pos - idx as f64) as f32;
        let a = *input.get(idx).unwrap_or(&0) as f32;
        let b = *input.get(idx + 1).unwrap_or_else(|| input.last().unwrap_or(&0)) as f32;
        let v = a + (b - a) * frac;
        out.push(v.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16);
    }

    out
}

fn temp_raw_path(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("bigecho_{}_{}.raw", prefix, Uuid::new_v4()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_keeps_non_empty_signal() {
        let src = vec![0_i16, 1200, -1200, 400, -400, 0];
        let dst = resample_i16(&src, 24_000, 48_000);
        assert!(!dst.is_empty());
    }

    #[test]
    fn detects_best_system_source_by_name() {
        let items = vec![
            "Built-in Microphone".to_string(),
            "BlackHole 2ch".to_string(),
            "Stereo Mix (Realtek)".to_string(),
        ];
        let best = select_best_system_source(&items);
        assert!(best.is_some());
        let picked = best.unwrap_or_default().to_lowercase();
        assert!(picked.contains("blackhole") || picked.contains("stereo mix"));
    }

    #[test]
    fn prefers_blackhole_on_macos_profile() {
        let blackhole = score_system_source_name("BlackHole 2ch", "macos");
        let stereo_mix = score_system_source_name("Stereo Mix (Realtek)", "macos");
        assert!(blackhole >= stereo_mix);
    }

    #[test]
    fn prefers_stereo_mix_on_windows_profile() {
        let blackhole = score_system_source_name("BlackHole 2ch", "windows");
        let stereo_mix = score_system_source_name("Stereo Mix (Realtek)", "windows");
        assert!(stereo_mix >= blackhole);
    }

    #[test]
    fn prefers_explicit_system_source_when_present() {
        let items = vec![
            "Built-in Microphone".to_string(),
            "BlackHole 2ch".to_string(),
            "Stereo Mix (Realtek)".to_string(),
        ];
        let selected = resolve_system_source_name(Some("Stereo Mix (Realtek)"), Some("Built-in Microphone"), &items);
        assert_eq!(selected.as_deref(), Some("Stereo Mix (Realtek)"));
    }

    #[test]
    fn falls_back_to_detected_source_when_preferred_missing() {
        let items = vec![
            "Built-in Microphone".to_string(),
            "BlackHole 2ch".to_string(),
            "MacBook Pro Microphone".to_string(),
        ];
        let selected = resolve_system_source_name(
            Some("Missing Device"),
            Some("Built-in Microphone"),
            &items,
        );
        assert_eq!(selected.as_deref(), Some("BlackHole 2ch"));
    }

    #[test]
    fn does_not_select_microphone_as_system_source() {
        let items = vec![
            "Built-in Microphone".to_string(),
            "External Mic".to_string(),
        ];
        let selected = resolve_system_source_name(Some("Built-in Microphone"), Some("Built-in Microphone"), &items);
        assert!(selected.is_none());
    }

    #[test]
    fn normalized_level_is_monotonic_and_boosts_mid_signal() {
        let low = normalize_signal_level(0.01);
        let mid = normalize_signal_level(0.03);
        let high = normalize_signal_level(0.25);
        assert!(low < mid && mid < high);
        assert!(mid > 0.35);
    }
}

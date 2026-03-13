use ogg::writing::{PacketWriteEndInfo, PacketWriter};
use opus::{Application, Bitrate, Channels, Encoder};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read};
use std::path::{Path, PathBuf};

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u8 = 1;
const FRAME_SIZE: usize = 960; // 20ms @ 48kHz

#[allow(dead_code)]
pub fn write_silence_opus(path: &Path, duration_ms: u32, bitrate_kbps: u32) -> Result<(), String> {
    let frames = ((duration_ms as f32 / 20.0).ceil() as usize).max(1);
    let pcm = vec![0i16; frames * FRAME_SIZE];
    write_pcm_opus(path, SAMPLE_RATE, &pcm, bitrate_kbps)
}

pub fn write_pcm_opus(
    path: &Path,
    sample_rate: u32,
    pcm_mono_i16: &[i16],
    bitrate_kbps: u32,
) -> Result<(), String> {
    if pcm_mono_i16.is_empty() {
        let pcm = vec![0i16; FRAME_SIZE];
        return write_pcm_opus(path, SAMPLE_RATE, &pcm, bitrate_kbps);
    }

    let sr = if matches!(sample_rate, 8_000 | 12_000 | 16_000 | 24_000 | 48_000) {
        sample_rate
    } else {
        SAMPLE_RATE
    };

    let samples = if sr == sample_rate {
        pcm_mono_i16.to_vec()
    } else {
        resample_i16(pcm_mono_i16, sample_rate, sr)
    };

    let file = File::create(path).map_err(|e| e.to_string())?;
    let mut writer = PacketWriter::new(BufWriter::new(file));
    let serial = 1;

    let head = opus_head_packet();
    writer
        .write_packet(head, serial, PacketWriteEndInfo::EndPage, 0)
        .map_err(|e| e.to_string())?;

    let tags = opus_tags_packet("BigEcho");
    writer
        .write_packet(tags, serial, PacketWriteEndInfo::EndPage, 0)
        .map_err(|e| e.to_string())?;

    let frame_size = (sr as usize) / 50; // 20ms
    let mut encoder = Encoder::new(sr, Channels::Mono, Application::Voip)
        .map_err(|e| format!("failed to create opus encoder: {e}"))?;
    encoder
        .set_bitrate(Bitrate::Bits((bitrate_kbps.clamp(12, 128) * 1000) as i32))
        .map_err(|e| format!("failed to set bitrate: {e}"))?;

    let mut packet_buf = vec![0u8; 4000];
    let mut granule: u64 = 0;
    let total_frames = (samples.len() as f32 / frame_size as f32).ceil() as usize;
    let total_frames = total_frames.max(1);

    for i in 0..total_frames {
        let from = i * frame_size;
        let to = ((i + 1) * frame_size).min(samples.len());
        let mut frame = vec![0i16; frame_size];
        if from < to {
            frame[..(to - from)].copy_from_slice(&samples[from..to]);
        }

        let encoded = encoder
            .encode(&frame, &mut packet_buf)
            .map_err(|e| format!("encode failed: {e}"))?;

        granule += frame_size as u64;
        let end = if i == total_frames - 1 {
            PacketWriteEndInfo::EndStream
        } else {
            PacketWriteEndInfo::NormalPacket
        };

        writer
            .write_packet(packet_buf[..encoded].to_vec(), serial, end, granule)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

pub fn write_mixed_raw_i16_to_opus(
    output_path: &Path,
    mic_raw_path: &Path,
    mic_rate: u32,
    system_raw_path: Option<&PathBuf>,
    system_rate: u32,
    bitrate_kbps: u32,
) -> Result<(), String> {
    if mic_rate == 0 {
        return Err("Mic sample rate must be > 0".to_string());
    }
    if system_raw_path.is_some() && system_rate == 0 {
        return Err("System sample rate must be > 0".to_string());
    }

    let file = File::create(output_path).map_err(|e| e.to_string())?;
    let mut writer = PacketWriter::new(BufWriter::new(file));
    let serial = 1;

    writer
        .write_packet(opus_head_packet(), serial, PacketWriteEndInfo::EndPage, 0)
        .map_err(|e| e.to_string())?;
    writer
        .write_packet(opus_tags_packet("BigEcho"), serial, PacketWriteEndInfo::EndPage, 0)
        .map_err(|e| e.to_string())?;

    let mut encoder = Encoder::new(SAMPLE_RATE, Channels::Mono, Application::Voip)
        .map_err(|e| format!("failed to create opus encoder: {e}"))?;
    encoder
        .set_bitrate(Bitrate::Bits((bitrate_kbps.clamp(12, 128) * 1000) as i32))
        .map_err(|e| format!("failed to set bitrate: {e}"))?;

    let mut mic_resampler = StreamResampler::new(mic_raw_path, mic_rate, SAMPLE_RATE)?;
    let mut system_resampler = match system_raw_path {
        Some(path) => Some(StreamResampler::new(path, system_rate, SAMPLE_RATE)?),
        None => None,
    };

    let mut packet_buf = vec![0u8; 4000];
    let mut granule: u64 = 0;
    let mut emitted_any = false;

    loop {
        let mic_frame = mic_resampler.read_frame(FRAME_SIZE).map_err(|e| e.to_string())?;
        let system_frame = if let Some(resampler) = system_resampler.as_mut() {
            resampler.read_frame(FRAME_SIZE).map_err(|e| e.to_string())?
        } else {
            Vec::new()
        };

        if mic_frame.is_empty() && system_frame.is_empty() {
            if !emitted_any {
                let zeros = vec![0i16; FRAME_SIZE];
                let encoded = encoder
                    .encode(&zeros, &mut packet_buf)
                    .map_err(|e| format!("encode failed: {e}"))?;
                granule += FRAME_SIZE as u64;
                writer
                    .write_packet(
                        packet_buf[..encoded].to_vec(),
                        serial,
                        PacketWriteEndInfo::EndStream,
                        granule,
                    )
                    .map_err(|e| e.to_string())?;
            }
            break;
        }

        let mut mixed = vec![0i16; FRAME_SIZE];
        for i in 0..FRAME_SIZE {
            let a = *mic_frame.get(i).unwrap_or(&0) as f32;
            let b = *system_frame.get(i).unwrap_or(&0) as f32;
            let v = if i < mic_frame.len() && i < system_frame.len() {
                (a + b) * 0.5
            } else {
                a + b
            };
            mixed[i] = v.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        }

        let encoded = encoder
            .encode(&mixed, &mut packet_buf)
            .map_err(|e| format!("encode failed: {e}"))?;
        granule += FRAME_SIZE as u64;
        emitted_any = true;

        let is_last = mic_frame.len() < FRAME_SIZE && system_frame.len() < FRAME_SIZE;
        let end = if is_last {
            PacketWriteEndInfo::EndStream
        } else {
            PacketWriteEndInfo::NormalPacket
        };

        writer
            .write_packet(packet_buf[..encoded].to_vec(), serial, end, granule)
            .map_err(|e| e.to_string())?;

        if is_last {
            break;
        }
    }

    Ok(())
}

struct StreamResampler {
    reader: BufReader<File>,
    src_rate: u32,
    dst_rate: u32,
    src_pos: f64,
    buf: Vec<i16>,
    buf_start_idx: usize,
    eof: bool,
}

impl StreamResampler {
    fn new(path: &Path, src_rate: u32, dst_rate: u32) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        Ok(Self {
            reader: BufReader::new(file),
            src_rate,
            dst_rate,
            src_pos: 0.0,
            buf: Vec::new(),
            buf_start_idx: 0,
            eof: false,
        })
    }

    fn read_frame(&mut self, frame_size: usize) -> std::io::Result<Vec<i16>> {
        let mut out = Vec::with_capacity(frame_size);
        for _ in 0..frame_size {
            match self.next_sample()? {
                Some(s) => out.push(s),
                None => break,
            }
        }
        Ok(out)
    }

    fn next_sample(&mut self) -> std::io::Result<Option<i16>> {
        let idx = self.src_pos.floor() as usize;
        if !self.ensure_index_available(idx + 1)? {
            if !self.ensure_index_available(idx)? {
                return Ok(None);
            }
        }

        let a = match self.sample_at(idx)? {
            Some(v) => v,
            None => return Ok(None),
        };
        let b = self.sample_at(idx + 1)?.unwrap_or(a);
        let frac = (self.src_pos - idx as f64) as f32;
        let v = a as f32 + (b as f32 - a as f32) * frac;

        let step = self.src_rate as f64 / self.dst_rate as f64;
        self.src_pos += step;
        self.compact_buffer();

        Ok(Some(v.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16))
    }

    fn sample_at(&mut self, idx: usize) -> std::io::Result<Option<i16>> {
        if !self.ensure_index_available(idx)? {
            return Ok(None);
        }
        if idx < self.buf_start_idx {
            return Ok(None);
        }
        let local = idx - self.buf_start_idx;
        Ok(self.buf.get(local).copied())
    }

    fn ensure_index_available(&mut self, idx: usize) -> std::io::Result<bool> {
        while !self.eof && idx >= self.buf_start_idx + self.buf.len() {
            let mut bytes = vec![0u8; 4096 * 2];
            let n = self.reader.read(&mut bytes)?;
            if n == 0 {
                self.eof = true;
                break;
            }
            for chunk in bytes[..(n - (n % 2))].chunks_exact(2) {
                self.buf.push(i16::from_le_bytes([chunk[0], chunk[1]]));
            }
        }
        Ok(idx < self.buf_start_idx + self.buf.len())
    }

    fn compact_buffer(&mut self) {
        // Keep a tiny lookback window for interpolation continuity.
        let keep_from = self.src_pos.floor().max(1.0) as usize - 1;
        if keep_from > self.buf_start_idx + 1024 {
            let drain_to = keep_from.saturating_sub(self.buf_start_idx);
            if drain_to > 0 && drain_to < self.buf.len() {
                self.buf.drain(0..drain_to);
                self.buf_start_idx += drain_to;
            }
        }
    }
}

fn resample_i16(input: &[i16], src_rate: u32, dst_rate: u32) -> Vec<i16> {
    if input.is_empty() || src_rate == 0 || dst_rate == 0 || src_rate == dst_rate {
        return input.to_vec();
    }

    let out_len = ((input.len() as f64) * (dst_rate as f64 / src_rate as f64)).round() as usize;
    let out_len = out_len.max(1);
    let mut out = Vec::with_capacity(out_len);

    for i in 0..out_len {
        let src_pos = (i as f64) * (src_rate as f64 / dst_rate as f64);
        let idx = src_pos.floor() as usize;
        let frac = (src_pos - idx as f64) as f32;
        let a = *input.get(idx).unwrap_or(&0) as f32;
        let b = *input.get(idx + 1).unwrap_or(&input[input.len() - 1]) as f32;
        let v = a + (b - a) * frac;
        out.push(v.round().clamp(i16::MIN as f32, i16::MAX as f32) as i16);
    }

    out
}

fn opus_head_packet() -> Vec<u8> {
    let mut out = Vec::with_capacity(19);
    out.extend_from_slice(b"OpusHead");
    out.push(1); // version
    out.push(CHANNELS);
    out.extend_from_slice(&0u16.to_le_bytes()); // pre-skip
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&0i16.to_le_bytes()); // output gain
    out.push(0); // channel mapping family
    out
}

fn opus_tags_packet(vendor: &str) -> Vec<u8> {
    let vendor_bytes = vendor.as_bytes();
    let mut out = Vec::with_capacity(16 + vendor_bytes.len());
    out.extend_from_slice(b"OpusTags");
    out.extend_from_slice(&(vendor_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(vendor_bytes);
    out.extend_from_slice(&0u32.to_le_bytes()); // user comment list length
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_non_empty_opus_with_ogg_header() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("audio.opus");
        write_silence_opus(&path, 1000, 24).expect("write opus");

        let bytes = std::fs::read(&path).expect("read file");
        assert!(bytes.len() > 128);
        assert_eq!(&bytes[..4], b"OggS");
    }

    #[test]
    fn writes_pcm_as_opus() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("audio_pcm.opus");
        let tone: Vec<i16> = (0..48_000)
            .map(|i| {
                let t = i as f32 / 48_000.0;
                ((2.0 * std::f32::consts::PI * 440.0 * t).sin() * 12_000.0) as i16
            })
            .collect();

        write_pcm_opus(&path, 48_000, &tone, 24).expect("write opus from pcm");
        let bytes = std::fs::read(&path).expect("read file");
        assert!(bytes.len() > 128);
        assert_eq!(&bytes[..4], b"OggS");
    }

    #[test]
    fn writes_streamed_raw_to_opus() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mic = temp.path().join("mic.raw");
        let out = temp.path().join("streamed.opus");
        std::fs::write(
            &mic,
            (0..(FRAME_SIZE * 4))
                .flat_map(|i| ((i as i16 % 500) - 250).to_le_bytes())
                .collect::<Vec<u8>>(),
        )
        .expect("write mic raw");

        write_mixed_raw_i16_to_opus(&out, &mic, 48_000, None, 48_000, 24).expect("streamed opus");
        let bytes = std::fs::read(&out).expect("read opus");
        assert!(bytes.len() > 32);
        assert_eq!(&bytes[..4], b"OggS");
    }

    #[test]
    fn writes_streamed_raw_to_opus_with_resampling() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mic = temp.path().join("mic_44k.raw");
        let out = temp.path().join("streamed_44k.opus");

        // 44.1kHz, ~1 second tone
        let samples: Vec<i16> = (0..44_100)
            .map(|i| {
                let t = i as f32 / 44_100.0;
                ((2.0 * std::f32::consts::PI * 220.0 * t).sin() * 8_000.0) as i16
            })
            .collect();
        let bytes: Vec<u8> = samples.into_iter().flat_map(|s| s.to_le_bytes()).collect();
        std::fs::write(&mic, bytes).expect("write mic raw");

        write_mixed_raw_i16_to_opus(&out, &mic, 44_100, None, 48_000, 24).expect("streamed opus");
        let out_bytes = std::fs::read(&out).expect("read opus");
        assert!(out_bytes.len() > 32);
        assert_eq!(&out_bytes[..4], b"OggS");
    }
}

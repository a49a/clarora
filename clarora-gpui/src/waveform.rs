use crate::{
    data::{Library, id},
    process::{self, Cancel},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, sync::atomic::Ordering, time::Duration};

#[derive(Clone, Serialize, Deserialize)]
pub struct Waveform {
    pub duration: f64,
    pub peaks: Vec<f32>,
}
/// Decode to a low-rate mono file, then scan samples once with bounded memory.
pub fn extract(lib: &Library, path: &Path, cancel: &Cancel) -> Result<Waveform> {
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    let metadata = fs::metadata(path)?;
    let identity = format!(
        "{}:{:?}:{}",
        path.canonicalize()?.display(),
        metadata.modified()?,
        metadata.len()
    );
    let directory = lib.root.join("waveforms");
    fs::create_dir_all(&directory)?;
    let cache = directory.join(format!(
        "{}.json",
        hex::encode(Sha256::digest(identity.as_bytes()))
    ));
    if let Ok(bytes) = fs::read(&cache) {
        if let Ok(waveform) = serde_json::from_slice::<Waveform>(&bytes) {
            if waveform.duration > 0.
                && waveform.duration.is_finite()
                && !waveform.peaks.is_empty()
                && waveform.peaks.len() <= 1024
            {
                return Ok(waveform);
            }
        }
    }
    let temp = directory.join(format!("{}.wav", id()));
    let result = (|| {
        process::run(
            &process::tool("ffmpeg")?,
            &[
                "-nostdin".into(),
                "-v".into(),
                "error".into(),
                "-y".into(),
                "-i".into(),
                path.to_string_lossy().into(),
                "-vn".into(),
                "-ac".into(),
                "1".into(),
                "-ar".into(),
                "1000".into(),
                "-c:a".into(),
                "pcm_s16le".into(),
                temp.to_string_lossy().into(),
            ],
            cancel,
            Duration::from_secs(600),
        )?;
        let mut wav = hound::WavReader::open(&temp)?;
        let total = wav.duration() as usize;
        ensure!(total > 0, "音频没有可解码采样");
        let per_bin = total.div_ceil(512);
        let mut peaks = Vec::with_capacity(512);
        let (mut energy, mut count) = (0., 0usize);
        for sample in wav.samples::<i16>() {
            if count == 0 {
                ensure!(!cancel.load(Ordering::SeqCst), "已取消");
            }
            let sample = f64::from(sample?) / 32768.;
            energy += sample * sample;
            count += 1;
            if count == per_bin {
                peaks.push((energy / count as f64).sqrt() as f32);
                energy = 0.;
                count = 0;
            }
        }
        if count > 0 {
            peaks.push((energy / count as f64).sqrt() as f32);
        }
        let maximum = peaks.iter().copied().fold(0.001f32, f32::max);
        for peak in &mut peaks {
            *peak = (*peak / maximum).clamp(0., 1.);
        }
        let waveform = Waveform {
            duration: total as f64 / 1000.,
            peaks,
        };
        ensure!(!cancel.load(Ordering::SeqCst), "已取消");
        fs::write(cache, serde_json::to_vec(&waveform)?)?;
        Ok(waveform)
    })();
    let _ = fs::remove_file(temp);
    result
}

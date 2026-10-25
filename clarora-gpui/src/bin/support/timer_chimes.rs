use anyhow::{Result, bail, ensure};
use clarora_gpui::{
    data::Library,
    media::Player,
    timer_chimes::{Chimes, Cue},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};
fn finished(sound: &Chimes, path: &Path) -> Result<()> {
    let end = Instant::now() + Duration::from_secs(8);
    loop {
        let s = sound.snapshot().unwrap();
        ensure!(s.error.is_empty(), "提示音解码失败：{}", s.error);
        if s.path == path.to_string_lossy() && s.eof {
            return Ok(());
        }
        if Instant::now() > end {
            bail!("提示音未正常播放结束")
        }
        thread::sleep(Duration::from_millis(20));
    }
}
pub fn run(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    let root = lib.root.join("timer-chimes");
    let mut sounds = Chimes::without_audio_device(root.clone());
    let first = sounds.play(Cue::Start)?;
    let reader = hound::WavReader::open(&first)?;
    ensure!(
        reader.spec().channels == 1 && reader.spec().sample_rate == 44100,
        "提示音规格错误"
    );
    let samples = reader
        .into_samples::<i16>()
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let peak = samples
        .iter()
        .map(|v| i32::from(*v).abs())
        .max()
        .unwrap_or(0);
    ensure!(
        samples.len() > 44100 && peak > 1000 && peak < 16000,
        "开始提示音空白/削波"
    );
    // Play the start cue concurrently on a separate normal learning player.
    let learning = Player::without_audio_device();
    learning.open(&first);
    learning.set("loop-file", "inf");
    finished(&sounds, &first)?;
    let before = learning.snapshot();
    let end = sounds.play(Cue::End)?;
    finished(&sounds, &end)?;
    let after = learning.snapshot();
    ensure!(
        after.path == first.to_string_lossy()
            && after.error.is_empty()
            && !after.paused
            && after.initialized,
        "提示音打断学习播放器"
    );
    ensure!(before.path == after.path, "背景音轨被提示音替换");
    let samples = hound::WavReader::open(&end)?
        .into_samples::<i16>()
        .collect::<std::result::Result<Vec<_>, _>>()?;
    ensure!(
        samples.len() > 88200 && samples[29000..35000].iter().any(|s| s.abs() > 1000),
        "结束提示音第二声缺失"
    );
    fs::write(&first, b"broken cache")?;
    let repaired = sounds.play(Cue::Start)?;
    finished(&sounds, &repaired)?;
    ensure!(
        hound::WavReader::open(&repaired)?.duration() > 44100,
        "损坏素材未重建"
    );
    fs::remove_file(&end)?;
    let regenerated = sounds.play(Cue::End)?;
    finished(&sounds, &regenerated)?;
    ensure!(regenerated.is_file(), "丢失素材未重建");
    sounds.stop();
    fs::write(
        root.join("chimes-evidence.json"),
        serde_json::to_vec_pretty(
            &json!({"start_wav":first,"end_wav":end,"start_peak":peak,"learning_path_before":before.path,"learning_path_after":after.path,"independent_decoder":true,"physical_speaker_verified":false}),
        )?,
    )?;
    results.push(json!({"workflow":"提示音真实WAV生成/libmpv完整解码、独立播放器不打断学习、缓存损坏和丢失恢复","passed":true,"evidence":root.join("chimes-evidence.json")}));
    Ok(())
}

/// Explicit local-device validation; never included in headless default E2E.
pub fn audible(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    let root = lib.root.join("timer-chimes-device");
    let mut sounds = Chimes::new(root.clone());
    let start = sounds.play(Cue::Start)?;
    finished(&sounds, &start)?;
    let end = sounds.play(Cue::End)?;
    finished(&sounds, &end)?;
    fs::write(
        root.join("device-evidence.json"),
        serde_json::to_vec_pretty(
            &json!({"start":start,"end":end,"default_audio_device":true,"decoder_errors":false,"human_listening_verified":false}),
        )?,
    )?;
    results.push(json!({"workflow":"本机默认音频输出播放开始/结束提示音，无null输出覆盖，未验证人工听感","passed":true,"evidence":root.join("device-evidence.json")}));
    Ok(())
}

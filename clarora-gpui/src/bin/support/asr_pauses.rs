//! Real native ASR across silence; all outputs remain in the isolated E2E library.
use anyhow::{Result, ensure};
use clarora_gpui::{ai, data::Library, process, subtitles, video_subtitles};
use serde_json::{Value, json};
use std::{fs, path::Path, time::Duration};

pub fn run(lib: &Library, speech: &Path, results: &mut Vec<Value>) -> Result<()> {
    let cancel = process::cancel_token();
    let probe = process::run(
        &process::tool("ffprobe")?,
        &[
            "-v".into(),
            "error".into(),
            "-show_entries".into(),
            "format=duration".into(),
            "-of".into(),
            "default=nw=1:nk=1".into(),
            speech.to_string_lossy().into_owned(),
        ],
        &cancel,
        Duration::from_secs(30),
    )?;
    let duration: f64 = probe.trim().parse()?;
    let audio = lib.root.join("asr-long-pauses.wav");
    process::run(&process::tool("ffmpeg")?, &[
        "-v".into(), "error".into(), "-y".into(), "-i".into(), speech.to_string_lossy().into_owned(),
        "-filter_complex".into(),
        "[0:a]aresample=16000,aformat=channel_layouts=mono,asplit=2[a][b];anullsrc=r=16000:cl=mono,atrim=duration=8[s];[a][s][b]concat=n=3:v=0:a=1[out]".into(),
        "-map".into(), "[out]".into(), audio.to_string_lossy().into_owned(),
    ], &cancel, Duration::from_secs(60))?;
    let cues = ai::transcribe(lib, &audio, &cancel)?;
    ensure!(cues.iter().any(|c| c.start < duration), "首段语音丢失");
    ensure!(
        cues.iter().any(|c| c.start >= duration + 7.5),
        "长静音后语音丢失或时间偏移"
    );
    ensure!(
        !cues
            .iter()
            .any(|c| c.start > duration + 1. && c.start < duration + 7.),
        "静音区产生虚假字幕"
    );
    fs::write(lib.root.join("asr-long-pauses.srt"), subtitles::srt(&cues))?;
    results.push(json!({"workflow":"真实原生ASR → 长静音分段 → 恢复原始时间轴", "passed":true,"cues":cues,"first_duration":duration}));
    if let Some(video) = std::env::var_os("CLARORA_ASR_P9_VIDEO") {
        let model = std::env::var_os("CLARORA_ASR_P9_MODEL").expect("P9 model is required");
        fs::copy(model, lib.root.join("models/ggml-small.en.bin"))?;
        let prior = lib.setting("ai_config")?;
        lib.set_setting(
            "ai_config",
            &json!({"engine":"local","local_model":"small.en"}).to_string(),
        )?;
        let (_, cues) = video_subtitles::generate(lib, Path::new(&video), &cancel)?;
        let text = cues
            .iter()
            .map(|c| c.text.to_lowercase())
            .collect::<Vec<_>>()
            .join(" ");
        ensure!(
            !text.contains(".com")
                && text.contains("guides lead nature walks")
                && text.contains("protected habitats")
                && text.contains("trail paths")
                && text.contains("download the map for free"),
            "P9复现内容仍错误: {text}"
        );
        ensure!(
            cues.iter().any(|c| c.start >= 64.
                && c.start < 66.
                && c.text.to_lowercase().contains("guides")),
            "P9讲解时间仍落在静音区"
        );
        fs::write(lib.root.join("p9-corrected.srt"), subtitles::srt(&cues))?;
        fs::write(
            lib.root.join("p9-evidence.json"),
            serde_json::to_vec_pretty(&cues)?,
        )?;
        lib.set_setting("ai_config", &prior)?;
        results.push(json!({"workflow":"用户P9原视频 → small.en 分段真实回归", "passed":true,"cues":cues.len()}));
    }
    Ok(())
}

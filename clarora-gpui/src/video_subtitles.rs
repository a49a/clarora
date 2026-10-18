//! Video-bound ASR subtitles, independent of listening-library selection.
use crate::{
    ai,
    data::{Library, id},
    process::{self, Cancel},
    subtitles::{self, Cue},
};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, sync::atomic::Ordering, time::Duration};
fn key(video: &Path) -> Result<String> {
    let path = video.canonicalize()?;
    let meta = fs::metadata(&path)?;
    Ok(format!(
        "video_subtitles_{}",
        hex::encode(Sha256::digest(
            format!("{}:{}:{:?}", path.display(), meta.len(), meta.modified()?).as_bytes()
        ))
    ))
}
pub fn active(cues: &[Cue], time: f64) -> Option<usize> {
    cues.iter().rposition(|c| time >= c.start && time < c.end)
}
pub fn load(lib: &Library, video: &Path) -> Result<Vec<Cue>> {
    let saved = lib.setting(&key(video)?)?;
    if saved.is_empty() {
        return Ok(vec![]);
    }
    let path = std::path::PathBuf::from(saved);
    ensure!(path.starts_with(lib.root.join("media")), "字幕缓存路径无效");
    if !path.is_file() {
        return Ok(vec![]);
    }
    Ok(subtitles::parse(&fs::read_to_string(path)?))
}
pub fn save(lib: &Library, video: &Path, cues: &[Cue], cancel: &Cancel) -> Result<String> {
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    ensure!(
        !cues.is_empty()
            && cues.iter().all(|c| c.start.is_finite()
                && c.end.is_finite()
                && c.start >= 0.
                && c.end > c.start
                && !c.text.trim().is_empty()),
        "没有有效的带时间轴字幕"
    );
    let key = key(video)?;
    let target = lib.root.join("media").join(format!("video-{}.srt", id()));
    fs::write(&target, subtitles::srt(cues))?;
    let result = (|| {
        ensure!(!cancel.load(Ordering::SeqCst), "已取消");
        lib.set_setting(&key, &target.to_string_lossy())
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&target);
        return Err(e);
    }
    Ok(target.to_string_lossy().into())
}
pub fn generate(lib: &Library, video: &Path, cancel: &Cancel) -> Result<(String, Vec<Cue>)> {
    let identity = key(video)?;
    let audio = lib
        .root
        .join("media")
        .join(format!("video-asr-{}.mp3", id()));
    let result = (|| {
        process::run(
            &process::tool("ffmpeg")?,
            &[
                "-y".into(),
                "-i".into(),
                video.to_string_lossy().into(),
                "-vn".into(),
                "-ac".into(),
                "1".into(),
                "-ar".into(),
                "16000".into(),
                "-c:a".into(),
                "libmp3lame".into(),
                "-b:a".into(),
                "64k".into(),
                audio.to_string_lossy().into(),
            ],
            cancel,
            Duration::from_secs(3600),
        )
        .context("视频音轨提取失败")?;
        let mut cues = ai::transcribe(lib, &audio, cancel)?;
        cues.sort_by(|a, b| a.start.total_cmp(&b.start));
        ensure!(
            key(video)? == identity,
            "转写期间视频文件已改变，请重新生成"
        );
        let path = save(lib, video, &cues, cancel)?;
        Ok((path, cues))
    })();
    let _ = fs::remove_file(audio);
    result
}

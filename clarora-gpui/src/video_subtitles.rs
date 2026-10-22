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
    generate_with_progress(lib, video, cancel, |_| {})
}
pub fn generate_with_progress(
    lib: &Library,
    video: &Path,
    cancel: &Cancel,
    mut progress: impl FnMut(&str),
) -> Result<(String, Vec<Cue>)> {
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    let identity = key(video)?;
    let audio = lib
        .root
        .join("media")
        .join(format!("video-asr-{}.mp3", id()));
    let result = (|| {
        progress("1/3 · 正在提取视频音轨");
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
        progress("2/3 · 正在识别语音，长视频可能需要几分钟");
        let mut cues = ai::transcribe(lib, &audio, cancel)?;
        cues.sort_by(|a, b| a.start.total_cmp(&b.start));
        ensure!(
            key(video)? == identity,
            "转写期间视频文件已改变，请重新生成"
        );
        progress("3/3 · 正在保存字幕");
        let path = save(lib, video, &cues, cancel)?;
        Ok((path, cues))
    })();
    let _ = fs::remove_file(audio);
    result
}

/// Extract original English lines, excluding any existing Chinese translation.
pub fn english_source(cues: &[Cue]) -> Result<Vec<Cue>> {
    ensure!(!cues.is_empty(), "请先导入或生成英文字幕");
    cues.iter()
        .enumerate()
        .map(|(i, c)| {
            let text = c
                .text
                .lines()
                .filter(|s| {
                    s.chars().any(|ch| ch.is_ascii_alphabetic())
                        && !s.chars().any(|ch| ('\u{3400}'..='\u{9fff}').contains(&ch))
                })
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            ensure!(
                !text.is_empty(),
                "第 {} 段没有独立英文原文，请先导入英文字幕",
                i + 1
            );
            ensure!(
                text.chars().count() <= 8000,
                "第 {} 段字幕过长，请先分段",
                i + 1
            );
            Ok(Cue {
                start: c.start,
                end: c.end,
                text,
            })
        })
        .collect()
}

/// Translate completely before replacing the video-bound cache. A competing
/// subtitle import invalidates this task's revision inside the SQLite transaction.
pub fn translate(
    lib: &Library,
    video: &Path,
    cues: &[Cue],
    cancel: &Cancel,
) -> Result<(String, Vec<Cue>)> {
    translate_with_progress(lib, video, cues, cancel, |_| {})
}
pub fn translate_with_progress(
    lib: &Library,
    video: &Path,
    cues: &[Cue],
    cancel: &Cancel,
    mut progress: impl FnMut(&str),
) -> Result<(String, Vec<Cue>)> {
    use rusqlite::OptionalExtension;
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    let identity = key(video)?;
    let revision = lib.setting(&identity)?;
    let source = english_source(cues)?;
    let mut translated = Vec::with_capacity(source.len());
    let mut offset = 0;
    progress(&format!("正在翻译字幕 · 0/{} 段", source.len()));
    while offset < source.len() {
        ensure!(!cancel.load(Ordering::SeqCst), "已取消");
        ensure!(
            key(video)? == identity && lib.setting(&identity)? == revision,
            "视频或英文字幕已改变，请重新生成中文"
        );
        let mut end = offset;
        let mut chars = 0;
        while end < source.len() && end - offset < 32 {
            let count = source[end].text.chars().count();
            if end > offset && chars + count > 8000 {
                break;
            }
            chars += count;
            end += 1;
        }
        let batch = ai::translate(lib, &source[offset..end], cancel)?;
        ensure!(
            batch.iter().zip(&source[offset..end]).all(|(c, original)| {
                c.text
                    .strip_prefix(&format!("{}\n", original.text))
                    .is_some_and(|translation| !translation.trim().is_empty())
            }),
            "模型未返回有效中文字幕，原字幕未覆盖"
        );
        translated.extend(batch);
        offset = end;
        progress(&format!("正在翻译字幕 · {offset}/{} 段", source.len()));
    }
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    ensure!(key(video)? == identity, "视频文件已改变，请重新生成中文");
    progress("正在保存双语字幕");
    let path = lib.root.join("media").join(format!("video-{}.srt", id()));
    fs::write(&path, subtitles::srt(&translated))?;
    let saved = (|| -> Result<()> {
        let mut db = lib.db()?;
        let tx = db.transaction()?;
        let current = tx
            .query_row(
                "SELECT value FROM app_settings WHERE key=?",
                [&identity],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_default();
        ensure!(current == revision, "英文字幕已替换，未覆盖较新的字幕");
        ensure!(!cancel.load(Ordering::SeqCst), "已取消");
        tx.execute("INSERT INTO app_settings(key,value) VALUES (?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![identity,path.to_string_lossy()])?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(error) = saved {
        let _ = fs::remove_file(&path);
        return Err(error);
    }
    Ok((path.to_string_lossy().into_owned(), translated))
}

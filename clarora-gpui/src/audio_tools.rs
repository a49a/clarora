use crate::{
    data::{Library, Row, id, now, text},
    process::{self, Cancel},
    subtitles,
};
use anyhow::{Result, ensure};
use serde_json::json;
use std::{fs, path::Path, time::Duration};
pub fn merge(lib: &Library, rows: &[Row], practice: &str, cancel: &Cancel) -> Result<()> {
    ensure!(rows.len() >= 2, "请至少勾选两条音频");
    let mut args = vec!["-nostdin".into(), "-v".into(), "error".into(), "-y".into()];
    let mut cues = vec![];
    let mut offset = 0.;
    for row in rows {
        let file = text(row, "audio_uri");
        ensure!(Path::new(&file).is_file(), "音频文件缺失");
        let duration = process::run(
            &process::tool("ffprobe")?,
            &[
                "-v".into(),
                "error".into(),
                "-show_entries".into(),
                "format=duration".into(),
                "-of".into(),
                "default=nokey=1:noprint_wrappers=1".into(),
                file.clone(),
            ],
            cancel,
            Duration::from_secs(30),
        )?
        .trim()
        .parse::<f64>()?;
        ensure!(duration.is_finite() && duration > 0., "无法读取真实时长");
        if let Ok(body) = fs::read_to_string(text(row, "subtitle_uri")) {
            for mut cue in subtitles::parse(&body) {
                cue.start = cue.start.max(0.) + offset;
                cue.end = (cue.end + offset).min(offset + duration);
                if cue.end > cue.start {
                    cues.push(cue);
                }
            }
        }
        offset += duration;
        args.extend(["-i".into(), file]);
    }
    let normalize = (0..rows.len())
        .map(|i| {
            format!("[{i}:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo[a{i}];")
        })
        .collect::<String>();
    let filter = format!(
        "{normalize}{}concat=n={}:v=0:a=1[out]",
        (0..rows.len())
            .map(|i| format!("[a{i}]"))
            .collect::<String>(),
        rows.len()
    );
    let key = id();
    let output = lib.root.join("media").join(format!("merged-{key}.m4a"));
    let subtitle = lib.root.join("media").join(format!("merged-{key}.srt"));
    let result = (|| {
        args.extend([
            "-filter_complex".into(),
            filter,
            "-map".into(),
            "[out]".into(),
            "-c:a".into(),
            "aac".into(),
            output.to_string_lossy().into(),
        ]);
        process::run(
            &process::tool("ffmpeg")?,
            &args,
            cancel,
            Duration::from_secs(1800),
        )?;
        fs::write(&subtitle, subtitles::srt(&cues))?;
        lib.put("listening_audios",json!({"id":key,"practice_id":practice,"name":format!("合并 {} 条 · {}",rows.len(),&key[..6]),"audio_uri":output.to_string_lossy(),"subtitle_uri":subtitle.to_string_lossy(),"created_at":now()}))
    })();
    if result.is_err() {
        let _ = fs::remove_file(output);
        let _ = fs::remove_file(subtitle);
    }
    result
}

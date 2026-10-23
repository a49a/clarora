//! Local Kokoro v1.0 via the sherpa-onnx C API helper; no network during generation.
use crate::{
    data::{Library, Row, id, now},
    model_download::{self, FileSpec, Progress},
    process::{self, Cancel},
    subtitles::{self, Cue},
};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::Ordering,
    time::Duration,
};
pub const MODEL: &str = "kokoro-multi-lang-v1_0";
const ARCHIVE_SHA256: &str = "c5f7e2d2caf082bc1d20fb70334a61d99d20b484500aad32e7cf84c128ea3298";
pub const VOICES: &[(i32, &str)] = &[
    (0, "美式女声 · af_alloy"),
    (3, "美式女声 · af_heart"),
    (10, "美式女声 · af_sky"),
    (14, "美式男声 · am_fenrir"),
    (21, "英式女声 · bf_emma"),
    (26, "英式男声 · bm_george"),
    (45, "中文女声 · 晓贝"),
    (46, "中文女声 · 晓妮"),
    (47, "中文女声 · 晓晓"),
    (48, "中文女声 · 晓伊"),
    (49, "中文男声 · 云健"),
    (50, "中文男声 · 云希"),
    (51, "中文男声 · 云夏"),
    (52, "中文男声 · 云扬"),
];
pub fn model_dir(lib: &Library) -> PathBuf {
    lib.root.join("models").join(MODEL)
}
pub fn validate_model(root: &Path) -> Result<()> {
    for (file, min) in [
        ("model.onnx", 1_000_000),
        ("voices.bin", 1000),
        ("tokens.txt", 10),
        ("lexicon-us-en.txt", 10),
        ("lexicon-zh.txt", 10),
        ("espeak-ng-data/phontab", 10),
    ] {
        ensure!(
            fs::metadata(root.join(file)).is_ok_and(|m| m.is_file() && m.len() > min),
            "Kokoro 模型缺少或损坏：{file}，请先下载模型"
        );
    }
    Ok(())
}
struct Work(PathBuf);
impl Drop for Work {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
pub fn install(lib: &Library, cancel: &Cancel, notify: &mut impl FnMut(Progress)) -> Result<()> {
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    let dest = model_dir(lib);
    if validate_model(&dest).is_ok() {
        return Ok(());
    }
    let work = Work(
        lib.root
            .join("models")
            .join(format!("kokoro-download-{}", id())),
    );
    fs::create_dir_all(&work.0)?;
    let filename = format!("{MODEL}.tar.bz2");
    model_download::download(
        MODEL,
        &work.0,
        &[FileSpec {
            name: filename.clone(),
            remote: "k2-fsa/sherpa-onnx/releases/download/tts-models".into(),
            min_bytes: 300_000_000,
        }],
        &["https://github.com"],
        cancel,
        notify,
    )?;
    install_archive(lib, &work.0.join(filename), cancel, notify)
}
/// Install a previously downloaded official package, with the same checksum and path checks.
pub fn install_archive(
    lib: &Library,
    archive_path: &Path,
    cancel: &Cancel,
    notify: &mut impl FnMut(Progress),
) -> Result<()> {
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    let dest = model_dir(lib);
    let work = Work(
        lib.root
            .join("models")
            .join(format!("kokoro-extract-{}", id())),
    );
    fs::create_dir_all(&work.0)?;
    let mut file = fs::File::open(archive_path)?;
    let mut hash = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        ensure!(!cancel.load(Ordering::SeqCst), "已取消");
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    ensure!(
        hex::encode(hash.finalize()) == ARCHIVE_SHA256,
        "Kokoro 模型校验失败，未安装"
    );
    notify(Progress {
        model: MODEL.into(),
        file: format!("{MODEL}.tar.bz2"),
        file_index: 1,
        file_count: 1,
        source: "本机".into(),
        downloaded: 0,
        total: None,
        bytes_per_second: 0.,
        phase: "解压模型（可取消）".into(),
    });
    let mut archive = tar::Archive::new(bzip2::read::BzDecoder::new(fs::File::open(archive_path)?));
    let mut expanded = 0u64;
    for entry in archive.entries()? {
        ensure!(!cancel.load(Ordering::SeqCst), "已取消");
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        ensure!(
            path.components().all(|c| matches!(c, Component::Normal(_))) && path.starts_with(MODEL),
            "模型压缩包路径无效"
        );
        let kind = entry.header().entry_type();
        ensure!(
            kind.is_file() || kind.is_dir(),
            "模型压缩包包含不支持的链接或设备"
        );
        expanded = expanded.checked_add(entry.size()).context("模型体积溢出")?;
        ensure!(expanded <= 1_500_000_000, "模型解压体积过大");
        let output = work.0.join(path);
        if kind.is_dir() {
            fs::create_dir_all(&output)?;
        } else {
            fs::create_dir_all(output.parent().context("模型路径无效")?)?;
            let mut out = fs::File::create(output)?;
            loop {
                ensure!(!cancel.load(Ordering::SeqCst), "已取消");
                let n = entry.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n])?;
            }
        }
    }
    validate_model(&work.0.join(MODEL))?;
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    // Keep a damaged previous install until the new package has fully validated.
    let old = work.0.join("previous");
    let had_old = dest.exists();
    if had_old {
        fs::rename(&dest, &old)?;
    }
    if let Err(e) = fs::rename(work.0.join(MODEL), &dest) {
        if had_old {
            let _ = fs::rename(old, &dest);
        }
        return Err(e.into());
    }
    Ok(())
}
/// Split at sentence punctuation and explicit line breaks without discarding text.
/// Keep decimal points, initials and common English abbreviations inside a sentence.
fn sentences(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    for line in text.lines() {
        let chars: Vec<char> = line.chars().collect();
        let mut pending = String::new();
        let mut boundary = false;
        for (index, &ch) in chars.iter().enumerate() {
            if boundary
                && !matches!(
                    ch,
                    '!' | '?'
                        | '。'
                        | '！'
                        | '？'
                        | '.'
                        | '"'
                        | '\''
                        | '”'
                        | '’'
                        | '」'
                        | '』'
                        | ')'
                        | '）'
                )
            {
                if !pending.trim().is_empty() {
                    result.push(pending.trim().to_owned());
                }
                pending.clear();
                boundary = false;
            }
            pending.push(ch);
            if matches!(ch, '!' | '?' | '。' | '！' | '？') {
                boundary = true;
            }
            if ch == '.'
                && chars
                    .get(index + 1)
                    .is_none_or(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '”' | '’'))
            {
                let token = pending
                    .split_whitespace()
                    .last()
                    .unwrap_or("")
                    .trim_matches(|c: char| !c.is_alphabetic() && c != '.');
                let short = token.trim_end_matches('.');
                let abbreviation = [
                    "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "vs", "etc", "e.g", "i.e",
                ]
                .contains(&short.to_ascii_lowercase().as_str())
                    || short.contains('.')
                    || (short.chars().count() == 1
                        && short.chars().all(|c| c.is_ascii_uppercase()));
                if !abbreviation {
                    boundary = true;
                }
            }
        }
        if !pending.trim().is_empty() {
            result.push(pending.trim().to_owned());
        }
    }
    result
}
#[allow(clippy::too_many_arguments)]
pub fn generate(
    lib: &Library,
    model: &Path,
    practice: &str,
    name: &str,
    text: &str,
    voice: i32,
    speed: f32,
    cancel: &Cancel,
) -> Result<Row> {
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    ensure!(
        !text.trim().is_empty()
            && text.chars().count() <= 10_000
            && text.len() <= 40_000
            && !text.contains('\0'),
        "请输入 1–10,000 字的正文（不含空字符）"
    );
    ensure!((0..54).contains(&voice), "音色编号需在 0–53 之间");
    ensure!(
        speed.is_finite() && (0.5..=2.).contains(&speed),
        "语速需在 0.5–2.0 之间"
    );
    ensure!(
        !name.trim().is_empty() && name.chars().count() <= 120,
        "请输入 1–120 字的音频名称"
    );
    ensure!(
        !lib.rows(
            "SELECT id FROM listening_practices WHERE id=?",
            &[practice.to_string().into()]
        )?
        .is_empty(),
        "请先选择练习组"
    );
    validate_model(model)?;
    let helper = process::tool("clarora-tts")
        .context("未安装 Kokoro 原生运行程序 clarora-tts，请按项目说明文档构建并安装")?;
    let work = Work(lib.root.join("cache").join(format!("tts-{}", id())));
    fs::create_dir_all(&work.0)?;
    let input = work.0.join("text.txt");
    let wav = work.0.join("output.wav");
    let sentences = sentences(text);
    ensure!(!sentences.is_empty(), "没有可合成的正文");
    fs::write(&input, sentences.join("\n"))?;
    process::run(
        &helper,
        &[
            model.to_string_lossy().into(),
            input.to_string_lossy().into(),
            wav.to_string_lossy().into(),
            voice.to_string(),
            speed.to_string(),
        ],
        cancel,
        Duration::from_secs(1800),
    )?;
    let mut reader = hound::WavReader::open(&wav).context("Kokoro 未生成有效 WAV")?;
    ensure!(
        reader.duration() > 0
            && reader.spec().sample_rate > 0
            && reader.spec().channels == 1
            && reader.spec().bits_per_sample == 16,
        "Kokoro 音频格式无效"
    );
    let sample_rate = reader.spec().sample_rate;
    let sample_count = reader.duration() as u64;
    let timings = fs::read_to_string(wav.with_extension("segments"))
        .context("Kokoro 辅助程序未提供逐句时间，请重新构建并打包 clarora-tts")?;
    let mut lines = timings.lines();
    ensure!(
        lines.next().and_then(|s| s.parse::<u32>().ok()) == Some(sample_rate),
        "Kokoro 字幕采样率与音频不匹配"
    );
    let mut cues = Vec::new();
    let mut previous_end = 0;
    for (index, line) in lines.enumerate() {
        let samples = line
            .split_whitespace()
            .map(str::parse::<u64>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ensure!(
            samples.len() == 2
                && samples[0] == previous_end
                && samples[1] > samples[0]
                && samples[1] <= sample_count,
            "Kokoro 逐句时间无效"
        );
        let sentence = sentences.get(index).context("Kokoro 时间段多于原文句数")?;
        cues.push(Cue {
            start: samples[0] as f64 / sample_rate as f64,
            end: samples[1] as f64 / sample_rate as f64,
            text: sentence.clone(),
        });
        previous_end = samples[1];
    }
    ensure!(
        cues.len() == sentences.len() && previous_end == sample_count,
        "Kokoro 字幕与音频长度或原文句数不符"
    );
    let subtitle = work.0.join("output.srt");
    fs::write(&subtitle, subtitles::srt(&cues))?;
    let mut audible = false;
    for (index, sample) in reader.samples::<i16>().enumerate() {
        if index % 65536 == 0 {
            ensure!(!cancel.load(Ordering::SeqCst), "已取消");
        }
        audible |= sample?.unsigned_abs() > 30;
    }
    ensure!(audible, "Kokoro 生成了静音，请检查文字与音色");
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    let key = id();
    let output = lib.root.join("media").join(format!("tts-{key}.wav"));
    fs::create_dir_all(output.parent().context("音频路径无效")?)?;
    let output_subtitle = output.with_extension("srt");
    let row: Row = serde_json::from_value(
        json!({"id":key,"practice_id":practice,"name":name.trim(),"audio_uri":output.to_string_lossy(),"subtitle_uri":output_subtitle.to_string_lossy(),"created_at":now()}),
    )?;
    let save = (|| {
        fs::rename(wav, &output)?;
        fs::rename(subtitle, &output_subtitle)?;
        ensure!(!cancel.load(Ordering::SeqCst), "已取消");
        ensure!(
            !lib.rows(
                "SELECT id FROM listening_practices WHERE id=?",
                &[practice.to_string().into()]
            )?
            .is_empty(),
            "所选练习组已被删除"
        );
        lib.put("listening_audios", serde_json::to_value(&row)?)
    })();
    if let Err(e) = save {
        let _ = fs::remove_file(output);
        let _ = fs::remove_file(output_subtitle);
        return Err(e);
    }
    Ok(row)
}

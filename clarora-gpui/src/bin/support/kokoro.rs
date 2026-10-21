use anyhow::{Result, ensure};
use clarora_gpui::{
    backup,
    data::{Library, text},
    process, subtitles, tts,
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, sync::atomic::Ordering};

pub fn run(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    let group = lib.practice("Kokoro E2E")?;
    let missing = lib.root.join("missing-kokoro");
    let before = lib.list("listening_audios")?.len();
    for (body, voice, speed) in [
        (" ", 0, 1.),
        ("hello", -1, 1.),
        ("hello", 54, 1.),
        ("hello", 0, f32::NAN),
        ("hello", 0, 0.),
    ] {
        ensure!(
            tts::generate(
                lib,
                &missing,
                &group,
                "bad",
                body,
                voice,
                speed,
                &process::cancel_token()
            )
            .is_err(),
            "非法 TTS 输入未拒绝"
        );
    }
    ensure!(
        tts::generate(
            lib,
            &missing,
            &group,
            "missing",
            "Hello",
            0,
            1.,
            &process::cancel_token()
        )
        .is_err(),
        "缺失模型未拒绝"
    );
    let cancel = process::cancel_token();
    cancel.store(true, Ordering::SeqCst);
    ensure!(
        tts::generate(lib, &missing, &group, "cancel", "Hello", 0, 1., &cancel).is_err(),
        "预取消未拒绝"
    );
    ensure!(
        lib.list("listening_audios")?.len() == before,
        "失败 TTS 入库"
    );
    results.push(json!({"workflow":"Kokoro 非法输入/缺失模型/预取消 → 无音频入库", "passed":true}));
    let Some(model) = std::env::var_os("CLARORA_KOKORO_MODEL") else {
        results.push(json!({"workflow":"真实 Kokoro ONNX 中英文生成", "skipped":"未设置 CLARORA_KOKORO_MODEL"}));
        return Ok(());
    };
    let mut model = PathBuf::from(model);
    if let Some(archive) = std::env::var_os("CLARORA_KOKORO_ARCHIVE") {
        let bad = lib.root.join("bad-kokoro.tar.bz2");
        fs::write(&bad, b"incomplete archive")?;
        ensure!(
            tts::install_archive(lib, &bad, &process::cancel_token(), &mut |_| {}).is_err(),
            "损坏压缩包未拒绝"
        );
        tts::install_archive(
            lib,
            &PathBuf::from(archive),
            &process::cancel_token(),
            &mut |_| {},
        )?;
        model = tts::model_dir(lib);
        tts::validate_model(&model)?;
        results.push(
            json!({"workflow":"Kokoro 官方压缩包 SHA256/解压安装 + 拒绝损坏文件", "passed":true}),
        );
    }
    for (name, body, voice) in [
        (
            "kokoro-english",
            "Hello! Welcome to Clarora. Practice a little every day, and enjoy learning English.\nDr. Smith reads 3.14 pages.",
            0,
        ),
        (
            "kokoro-chinese",
            "你好，欢迎使用 Clarora。\n\n每天练习一点，享受学习语言的乐趣。",
            45,
        ),
    ] {
        let row = tts::generate(
            lib,
            &model,
            &group,
            name,
            body,
            voice,
            1.,
            &process::cancel_token(),
        )?;
        let audio = PathBuf::from(text(&row, "audio_uri"));
        let mut reader = hound::WavReader::open(&audio)?;
        let duration = reader.duration() as f64 / reader.spec().sample_rate as f64;
        let subtitle = PathBuf::from(text(&row, "subtitle_uri"));
        ensure!(subtitle.is_file(), "TTS 没有保存字幕附件");
        let cues = subtitles::parse(&fs::read_to_string(&subtitle)?);
        ensure!(cues.len() >= 2, "TTS 没有逐句字幕");
        ensure!(
            cues.iter()
                .map(|c| c.text.as_str())
                .collect::<String>()
                .split_whitespace()
                .collect::<String>()
                == body.split_whitespace().collect::<String>(),
            "TTS 字幕与原文不一致"
        );
        ensure!(
            cues[0].start == 0. && (cues.last().unwrap().end - duration).abs() <= 0.002,
            "字幕没有覆盖真实音频长度"
        );
        ensure!(
            cues.iter().all(|c| c.end > c.start)
                && cues
                    .windows(2)
                    .all(|w| (w[0].end - w[1].start).abs() < 0.002),
            "字幕时间轴不连续"
        );
        if voice == 0 {
            ensure!(
                cues.last().unwrap().text == "Dr. Smith reads 3.14 pages.",
                "英文缩写或小数被错误分句"
            );
        }
        fs::copy(&subtitle, lib.root.join(format!("{name}.srt")))?;
        ensure!(
            reader.duration() > reader.spec().sample_rate,
            "生成语音过短"
        );
        ensure!(
            reader
                .samples::<i16>()
                .any(|v| v.is_ok_and(|n| n.unsigned_abs() > 100)),
            "语音全静音"
        );
        fs::copy(&audio, lib.root.join(format!("{name}.wav")))?;
        let player = clarora_gpui::media::Player::without_audio_device();
        player.open(&audio);
        let start = std::time::Instant::now();
        loop {
            let state = player.snapshot();
            ensure!(state.error.is_empty(), "TTS 播放失败：{}", state.error);
            if state.position > 0.2 {
                break;
            }
            ensure!(start.elapsed().as_secs() < 15, "TTS 没有播放进度");
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        player.command([
            "seek".into(),
            cues[1].start.to_string(),
            "absolute+exact".into(),
        ]);
        player.set("pause", "no");
        let seek_started = std::time::Instant::now();
        loop {
            let state = player.snapshot();
            if state.position >= cues[1].start && state.position < cues[1].end && !state.paused {
                break;
            }
            ensure!(
                seek_started.elapsed().as_secs() < 10,
                "TTS 第二句无法定位播放"
            );
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
    }
    let reopened = Library::open(lib.root.clone())?;
    ensure!(
        reopened.list("listening_audios")?.len() == before + 2,
        "TTS 未持久化"
    );
    for row in reopened
        .list("listening_audios")?
        .iter()
        .filter(|r| text(r, "practice_id") == group)
    {
        ensure!(
            !subtitles::parse(&fs::read_to_string(text(row, "subtitle_uri"))?).is_empty(),
            "重开资料丢失 TTS 字幕"
        );
    }
    let archive = lib.root.join("kokoro-subtitle-backup.zip");
    backup::export_local(lib, &archive)?;
    let restored = Library::open(lib.root.join("kokoro-subtitle-restored"))?;
    backup::import_local(&restored, &archive)?;
    for name in ["kokoro-english", "kokoro-chinese"] {
        let rows = restored.list("listening_audios")?;
        let row = rows
            .iter()
            .find(|r| text(r, "name") == name)
            .ok_or_else(|| anyhow::anyhow!("恢复没有 TTS 记录"))?;
        ensure!(
            std::path::Path::new(&text(row, "audio_uri")).is_file()
                && subtitles::parse(&fs::read_to_string(text(row, "subtitle_uri"))?).len() >= 2,
            "备份恢复丢失 TTS 音频或字幕"
        );
    }
    let cancel = process::cancel_token();
    let flag = cancel.clone();
    let work = lib.clone();
    let work_model = model.clone();
    let work_group = group.clone();
    let handle = std::thread::spawn(move || {
        tts::generate(
            &work,
            &work_model,
            &work_group,
            "cancel-active",
            &"This is a long text for cancellation. ".repeat(60),
            0,
            1.,
            &flag,
        )
    });
    // Wait for a unique work directory, proving the generation has started.
    let started = std::time::Instant::now();
    loop {
        if fs::read_dir(lib.root.join("cache"))?
            .filter_map(Result::ok)
            .any(|e| e.file_name().to_string_lossy().starts_with("tts-"))
        {
            break;
        }
        ensure!(started.elapsed().as_secs() < 10, "TTS 未启动");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    cancel.store(true, Ordering::SeqCst);
    ensure!(handle.join().unwrap().is_err(), "执行中的取消失败");
    ensure!(
        !fs::read_dir(lib.root.join("cache"))?
            .filter_map(Result::ok)
            .any(|e| e.file_name().to_string_lossy().starts_with("tts-")),
        "取消残留临时文件"
    );
    ensure!(
        lib.list("listening_audios")?.len() == before + 2,
        "取消仍然入库"
    );
    fs::write(
        lib.root.join("kokoro-evidence.json"),
        serde_json::to_vec_pretty(
            &json!({"model":model,"samples":["kokoro-english.wav","kokoro-chinese.wav"],"database_reopen":true,"playback_null_audio":true,"cancel_cleanup":true,"sentence_subtitles":true,"subtitle_seek":true,"subtitle_backup_restore":true}),
        )?,
    )?;
    results.push(json!({"workflow":"真实 Kokoro 中英文 ONNX → 非静音 WAV → SQLite 重开 → libmpv 播放 → 取消清理", "passed":true, "evidence":"kokoro-evidence.json"}));
    Ok(())
}

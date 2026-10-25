use anyhow::{Result, ensure};
use clarora_gpui::{
    audio_library, command_import,
    data::{Library, text},
    process,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

fn literal(value: &str) -> String {
    if cfg!(windows) {
        format!("'{}'", value.replace('\'', "''"))
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}
fn output(path: &Path) -> String {
    let value = literal(&path.to_string_lossy());
    if cfg!(windows) {
        format!("Write-Output {value}")
    } else {
        format!("printf '%s\\n' {value}")
    }
}

pub fn run(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    let evidence = lib.root.join("command-import-evidence");
    fs::create_dir_all(&evidence)?;
    let media = evidence.join("中文 audio's sample.wav");
    let mut wav = hound::WavWriter::create(
        &media,
        hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for sample in 0..16000 {
        wav.write_sample(
            ((sample as f32 * 440. * std::f32::consts::TAU / 16000.).sin() * 1000.) as i16,
        )?;
    }
    wav.finalize()?;
    let first = lib.practice("悬浮导入 · 点击时的组")?;
    let second = lib.practice("悬浮导入 · 后来选的组")?;
    let command = output(&media);
    let request = command_import::Request::new(lib, &first, &command)?;
    lib.set_setting("library_group", &second)?;
    let mut report = request.run(lib, &process::cancel_token())?;
    ensure!(report.imported == 1, "真实命令路径未导入");
    let rows = lib.list("listening_audios")?;
    let row = rows
        .iter()
        .find(|r| text(r, "practice_id") == first)
        .unwrap();
    ensure!(
        rows.iter().all(|r| text(r, "practice_id") != second),
        "切组改变已启动任务目标"
    );
    ensure!(
        fs::read(text(row, "audio_uri"))? == fs::read(&media)?,
        "导入附件内容变化"
    );
    let reopened = Library::open(lib.root.clone())?;
    ensure!(
        reopened
            .list("listening_audios")?
            .iter()
            .any(|r| text(r, "id") == text(row, "id")),
        "导入未持久化"
    );
    let duplicate =
        command_import::Request::new(lib, &first, &command)?.run(lib, &process::cancel_token());
    ensure!(duplicate.is_err(), "全重复路径被报告成功0条");
    results.push(json!({"workflow":"真实命令 → 中文/空格/引号音频路径 → 点击时组固定 → 文件复制与SQLite重开 → 重复导入明确失败", "passed":true}));

    ensure!(report.audios.len() == 1, "报告缺少新导入条目");
    ensure!(
        text(&report.audios[0], "id") == text(row, "id"),
        "命名目标不是本次导入"
    );
    for invalid in ["  ".to_string(), "长".repeat(121)] {
        ensure!(report.rename(lib, 0, &invalid).is_err(), "非法名称被接受");
    }
    report.rename(lib, 0, "  第一段 · 听力练习  ")?;
    ensure!(
        text(&report.audios[0], "name") == "第一段 · 听力练习",
        "改名未去除首尾空格"
    );
    let another = evidence.join("second.wav");
    let third = evidence.join("third.wav");
    fs::copy(&media, &another)?;
    fs::copy(&media, &third)?;
    let batch_command = format!(
        "{}; {}; {}",
        output(&another),
        output(&another),
        output(&third)
    );
    let mut batch = command_import::Request::new(lib, &first, &batch_command)?
        .run(lib, &process::cancel_token())?;
    ensure!(
        batch.imported == 2 && batch.failed == 1 && batch.audios.len() == 2,
        "部分失败报告包含旧音频/失败音频"
    );
    ensure!(
        text(&batch.audios[0], "id") != text(&batch.audios[1], "id"),
        "报告ID重复"
    );
    let duplicate_name = text(&batch.audios[0], "name");
    ensure!(
        batch.rename(lib, 1, &duplicate_name).is_err(),
        "同组重名未拒绝"
    );
    batch.rename(lib, 1, "第二个新名称")?;
    let renamed = Library::open(lib.root.clone())?.list("listening_audios")?;
    let first_saved = renamed
        .iter()
        .find(|r| text(r, "id") == text(row, "id"))
        .unwrap();
    ensure!(
        text(first_saved, "name") == "第一段 · 听力练习",
        "重开后名称未保存"
    );
    ensure!(
        text(first_saved, "audio_uri") == text(row, "audio_uri")
            && text(first_saved, "subtitle_uri") == text(row, "subtitle_uri"),
        "改名改变附件引用"
    );
    ensure!(
        fs::read(text(first_saved, "audio_uri"))? == fs::read(&media)?,
        "改名改变媒体内容"
    );
    let moved_id = text(&batch.audios[0], "id");
    audio_library::move_records(lib, &first, &second, &[moved_id.clone()])?;
    ensure!(
        batch.rename(lib, 0, "不应改动").is_err(),
        "已移动音频仍被改名"
    );
    lib.delete("listening_audios", &text(&batch.audios[1], "id"))?;
    ensure!(
        batch.rename(lib, 1, "已删除").is_err(),
        "已删除音频仍改名成功"
    );
    ensure!(batch.rename(lib, 2, "越界").is_err(), "不存在的索引未拒绝");
    let after_names = lib.list("listening_audios")?;
    ensure!(
        after_names.iter().any(|r| text(r, "id") == moved_id
            && text(r, "name") == duplicate_name
            && text(r, "practice_id") == second),
        "拒绝后仍误改移动条目"
    );
    fs::write(
        evidence.join("naming-report.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":true,"single":report.audios,"batch":batch.audios,"after":after_names,"selected_group":second}),
        )?,
    )?;
    results.push(json!({"workflow":"导入结果ID → 切组后命名 → 批量部分失败 → 空名/过长/重名拒绝 → 移动/删除保护 → SQLite重开与附件保持", "passed":true,"evidence":evidence.join("naming-report.json")}));

    let marker = evidence.join("must-not-execute.wav");
    let mutating = if cfg!(windows) {
        format!(
            "New-Item -ItemType File {}",
            literal(&marker.to_string_lossy())
        )
    } else {
        format!("touch {}", literal(&marker.to_string_lossy()))
    };
    ensure!(
        command_import::Request::new(lib, "", &mutating).is_err(),
        "空组未拒绝"
    );
    ensure!(
        command_import::Request::new(lib, "does-not-exist", &mutating).is_err(),
        "无效组未拒绝"
    );
    ensure!(
        command_import::Request::new(lib, &first, "  ").is_err(),
        "空命令未拒绝"
    );
    ensure!(!marker.exists(), "校验失败仍执行命令");
    let bad = if cfg!(windows) {
        "Write-Error 'fixture failure'; exit 7"
    } else {
        "echo 'fixture failure' >&2; exit 7"
    };
    ensure!(
        command_import::Request::new(lib, &first, bad)?
            .run(lib, &process::cancel_token())
            .is_err(),
        "非零退出被忽略"
    );
    let none = if cfg!(windows) {
        "Write-Output 'not an audio path'"
    } else {
        "printf 'not an audio path\\n'"
    };
    ensure!(
        command_import::Request::new(lib, &first, none)?
            .run(lib, &process::cancel_token())
            .is_err(),
        "没有文件仍成功"
    );
    results.push(json!({"workflow":"空命令/空组/无效组执行前拒绝；真实shell非零退出/无路径明确失败", "passed":true}));

    let waiting = if cfg!(windows) {
        "Start-Sleep -Seconds 20"
    } else {
        "sleep 20"
    };
    let token = process::cancel_token();
    let signal = token.clone();
    let cancel_thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        signal.store(true, Ordering::SeqCst);
    });
    let started = Instant::now();
    ensure!(
        command_import::Request::new(lib, &first, waiting)?
            .run(lib, &token)
            .is_err(),
        "取消后仍成功"
    );
    cancel_thread.join().unwrap();
    ensure!(
        started.elapsed() < Duration::from_secs(5),
        "shell取消未及时完成"
    );
    fs::write(
        evidence.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":true,"target":first,"selected_after_click":second,"imported":report.imported,"cancel_ms":started.elapsed().as_millis(),"media":media,"rows":rows}),
        )?,
    )?;
    results.push(json!({"workflow":"真实长时间shell任务取消及进程组回收", "passed":true,"evidence":evidence.join("report.json")}));
    Ok(())
}

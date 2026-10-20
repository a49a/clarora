//! Integration workflow written before the blind-listening implementation.
use anyhow::{Result, ensure};
use clarora_gpui::{
    blind_listening::{State, chinese_hint, resolve, row_target, sentence_target, word_target},
    data::Library,
    media::Player,
    subtitles::{self, Cue},
};
use serde_json::{Value, json};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
pub fn run(lib: &Library, media: &std::path::Path, results: &mut Vec<Value>) -> Result<()> {
    let body = "1\n00:00:00,200 --> 00:00:01,000\nFIRST_VISIBLE_ONLY_WHEN_REVEALED\n\n2\n00:00:01,500 --> 00:00:02,500\nSECOND_MUST_STAY_HIDDEN\n";
    let path = lib.root.join("blind-timeline.srt");
    fs::write(&path, body)?;
    let cues = subtitles::parse(&fs::read_to_string(&path)?);
    ensure!(resolve(&cues, 0.1, 4.).is_none(), "首句前不应揭晓");
    ensure!(
        resolve(&cues, 1.2, 4.).unwrap().cue.text == cues[0].text,
        "句间目标错误"
    );
    ensure!(
        resolve(&cues, 3.9, 4.).unwrap().cue.text == cues[1].text,
        "末尾目标错误"
    );
    let mut overlap = cues.clone();
    overlap.push(Cue {
        start: 0.5,
        end: 0.9,
        text: "later overlap".into(),
    });
    ensure!(
        resolve(&overlap, 0.7, 4.).unwrap().cue.text == "later overlap",
        "重叠未取最晚开始"
    );
    ensure!(
        resolve(&cues, f64::NAN, 4.).is_none()
            && resolve(&cues, 1., 0.).is_none()
            && resolve(&[], 1., 4.).is_none(),
        "非法时间或空字幕未拒绝"
    );
    ensure!(
        resolve(&cues, 20., 30.).unwrap().sparse,
        "长空白未提示上一条字幕"
    );
    // Sentence controls must use timestamps, never expose text as navigation labels.
    let sentence = sentence_target(&cues, 1.2, 4., None, 0, None).unwrap();
    ensure!(sentence.cue.start == 0.2, "重听未定位刚才一句句首");
    ensure!(
        sentence_target(&cues, 0.1, 4., None, 0, None).is_none(),
        "首句前不可重听未来句"
    );
    ensure!(
        sentence_target(&cues, 0.1, 4., None, 1, None)
            .unwrap()
            .cue
            .start
            == 0.2,
        "首句前应能跳到首句"
    );
    ensure!(
        sentence_target(&cues, 0.6, 4., None, -1, None).is_none(),
        "上一句越界"
    );
    ensure!(
        sentence_target(&cues, 3.9, 4., None, 1, None).is_none(),
        "下一句越界"
    );
    ensure!(
        sentence_target(&cues, 0.6, 4., None, 1, Some((0., 1.))).is_none(),
        "跳句超出 A/B 范围"
    );
    ensure!(
        sentence_target(&cues, 2., 4., Some(&sentence), 0, None)
            .unwrap()
            .cue
            .start
            == 0.2,
        "揭晓后未锁定原句"
    );
    ensure!(
        sentence_target(&overlap, 0.7, 4., None, 0, None)
            .unwrap()
            .cue
            .start
            == 0.5,
        "重叠句重听错误"
    );
    let chinese = chinese_hint("Listen to the talk.\n听这段讲座。");
    ensure!(chinese == "听这段讲座。", "双语未仅提取中文");
    ensure!(
        chinese_hint("只有中文字幕。") == "只有中文字幕。",
        "纯中文被丢弃"
    );
    ensure!(chinese_hint("English only.").is_empty(), "无中文时泄漏英文");
    let mixed = chinese_hint("海獭 sea otters 属于 mammals。\nThis line must be hidden.");
    ensure!(
        mixed.contains("海獭") && !mixed.chars().any(|c| c.is_ascii_alphabetic()),
        "同一行中英泄漏英文"
    );
    lib.set_setting("blind_chinese_hint", "true")?;
    ensure!(
        Library::open(lib.root.clone())?.setting("blind_chinese_hint")? == "true",
        "中文偏好未落盘"
    );
    fs::write(
        lib.root.join("blind-chinese-evidence.json"),
        serde_json::to_vec_pretty(
            &json!({"bilingual":chinese,"mixed_line":mixed,"english_only":"","preference_reopened":true}),
        )?,
    )?;
    let mut chinese_state = State::default();
    chinese_state.enabled = true;
    ensure!(
        chinese_state.reference_for(true).is_empty(),
        "查看前泄漏中文"
    );
    chinese_state.revealed = Some(clarora_gpui::blind_listening::Revealed {
        cue: Cue {
            start: 0.,
            end: 1.,
            text: "Hidden English answer.\n只在查看时显示。".into(),
        },
        sparse: false,
    });
    ensure!(
        chinese_state.reference_for(true) == "只在查看时显示。",
        "仅中文揭晓／AI引用泄漏英文"
    );
    chinese_state.revealed.as_mut().unwrap().cue.text = "English only".into();
    ensure!(
        chinese_state.reference_for(true).is_empty(),
        "无中文揭晓回退英文"
    );
    chinese_state.invalidate();
    ensure!(
        chinese_state.reference_for(true).is_empty(),
        "收起后中文仍残留"
    );
    let player = Player::without_audio_device();
    player.open(media);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let p = player.snapshot();
        if p.duration > 2. && p.position >= 0.3 {
            break;
        }
        ensure!(Instant::now() < deadline, "媒体未就绪：{}", p.error);
        thread::sleep(Duration::from_millis(20));
    }
    player.command(["seek", "0.6", "absolute+exact"]);
    player.set("pause", "yes");
    let deadline = Instant::now() + Duration::from_secs(3);
    let snap = loop {
        let p = player.snapshot();
        if p.paused && (p.position - 0.6).abs() < 0.15 {
            break p;
        }
        ensure!(Instant::now() < deadline, "暂停未确认");
        thread::sleep(Duration::from_millis(20));
    };
    let mut state = State::default();
    state.enabled = true;
    ensure!(state.reference().is_empty(), "隐藏态泄漏引用");
    state.revealed = resolve(&cues, snap.position, snap.duration);
    ensure!(
        state.reference() == cues[0].text && !state.reference().contains(&cues[1].text),
        "揭晓引用范围错误"
    );
    state.invalidate();
    player.set("pause", "no");
    ensure!(state.reference().is_empty(), "恢复播放仍带引用");
    state.revealed = resolve(&cues, 1.6, 4.);
    state.invalidate();
    ensure!(state.reference().is_empty(), "切源／定位失效未清空");
    let word_cue = Cue {
        start: 0.2,
        end: 2.5,
        text: "First second third\n第一第二第三。".into(),
    };
    let word_cues = vec![word_cue.clone()];
    let word_start = word_target(&word_cues, &word_cue, 1, snap.duration).unwrap();
    ensure!(
        word_start > word_cue.start && word_start < word_cue.end,
        "词定位仍是句首或越界"
    );
    ensure!(
        word_target(&word_cues, &word_cue, 99, snap.duration).is_none(),
        "越界词号未拒绝"
    );
    ensure!(
        word_target(&[], &word_cue, 1, snap.duration).is_none(),
        "失效字幕仍能逐词播放"
    );
    player.command([
        "seek".into(),
        word_start.to_string(),
        "absolute+exact".into(),
    ]);
    player.set("pause", "no");
    let word_deadline = Instant::now() + Duration::from_secs(3);
    let word_playback = loop {
        let p = player.snapshot();
        if !p.paused && p.position >= word_start && p.position < word_cue.end {
            break p;
        }
        ensure!(Instant::now() < word_deadline, "逐词真实 seek/play 未确认");
        thread::sleep(Duration::from_millis(20));
    };
    fs::write(
        lib.root.join("blind-word-play.json"),
        serde_json::to_vec_pretty(
            &json!({"word_index":1,"target":word_start,"actual":word_playback.position,"sentence_start":word_cue.start,"playing":!word_playback.paused,"hidden_reference":state.reference()}),
        )?,
    )?;
    let clicked = cues[1].clone();
    ensure!(
        row_target(&cues, &clicked, snap.duration) == Some(clicked.start),
        "点击行未定位该行句首"
    );
    ensure!(
        row_target(&[], &clicked, snap.duration).is_none(),
        "字幕替换后旧行仍有效"
    );
    let mut invalid = clicked.clone();
    invalid.start = f64::NAN;
    ensure!(
        row_target(&cues, &invalid, snap.duration).is_none(),
        "非法行时间未拒绝"
    );
    let next = sentence_target(&cues, snap.position, snap.duration, None, 1, None).unwrap();
    player.command([
        "seek".into(),
        row_target(&cues, &clicked, snap.duration)
            .unwrap()
            .to_string(),
        "absolute+exact".into(),
    ]);
    player.set("pause", "no");
    let deadline = Instant::now() + Duration::from_secs(3);
    let replay = loop {
        let p = player.snapshot();
        if !p.paused && p.position >= next.cue.start && p.position < next.cue.end {
            break p;
        }
        ensure!(Instant::now() < deadline, "跳句后真实播放未确认");
        thread::sleep(Duration::from_millis(20));
    };
    player.set("pause", "yes");
    fs::write(
        lib.root.join("blind-listening-navigation.json"),
        serde_json::to_vec_pretty(
            &json!({"repeat_start":sentence.cue.start,"next_start":next.cue.start,"playing_position":replay.position,"playing":!replay.paused,"reference_after_seek":state.reference(),"boundary_checks":true,"loop_range_checks":true}),
        )?,
    )?;
    fs::write(
        lib.root.join("blind-listening-evidence.json"),
        serde_json::to_vec_pretty(
            &json!({"paused_position":snap.position,"paused":snap.paused,"hidden_reference":"","revealed":cues[0].text,"other_sentence_excluded":true,"srt":path}),
        )?,
    )?;
    results.push(json!({"workflow":"盲听真实媒体暂停确认 → SRT 句间/重叠/末尾定位 → 单句揭晓 → 继续/失效清除引用","passed":true,"evidence":"blind-listening-evidence.json"}));
    Ok(())
}

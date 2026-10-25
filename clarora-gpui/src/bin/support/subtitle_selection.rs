use anyhow::{Result, ensure};
use clarora_gpui::{
    data::{Library, text},
    subtitle_selection::{Document, Selection},
    subtitles,
};
use serde_json::{Value, json};
use std::{fs, sync::Arc};
pub fn run(parent: &Library, results: &mut Vec<Value>) -> Result<()> {
    let lib = Library::open(parent.root.join("subtitle-selection"))?;
    let group = lib.practice("Selection")?;
    let wav = lib.root.join("fixture.wav");
    let mut w = hound::WavWriter::create(
        &wav,
        hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for _ in 0..16000 {
        w.write_sample(0i16)?;
    }
    w.finalize()?;
    lib.import_audio(&group, &wav)?;
    let audio = lib.list("listening_audios")?.remove(0);
    let srt = lib.root.join("fixture.srt");
    fs::write(
        &srt,
        "1\n00:00:00,000 --> 00:00:02,000\nIt's a fair critique.\n这是公平的批评。\n\n2\n00:00:03,000 --> 00:00:05,000\nKeep learning 🐱 today.\n继续学习，别停。\n",
    )?;
    lib.db()?.execute(
        "UPDATE listening_audios SET subtitle_uri=? WHERE id=?",
        rusqlite::params![srt.to_string_lossy(), text(&audio, "id")],
    )?;
    let row = lib.list("listening_audios")?.remove(0);
    let cues = subtitles::parse(&fs::read_to_string(text(&row, "subtitle_uri"))?);
    let doc = Arc::new(Document::new(text(&row, "id"), &cues));
    let original = &doc.cues[0].original;
    let word = &doc.cues[0].words[2];
    ensure!(&doc.text[word.clone()] == "fair", "逐词映射错误");
    let mut selection = Selection::default();
    selection.begin(doc.clone(), word.start + 1);
    selection.extend(word.end, true);
    selection.finish();
    ensure!(selection.text() == "air", "词内字符选取失败");
    let chinese = doc.cues[0].translation.clone();
    selection.begin(doc.clone(), chinese.start + 3);
    selection.extend(chinese.start + 12, true);
    selection.finish();
    ensure!(selection.text() == "是公平", "中文边界错误");
    selection.begin(doc.clone(), doc.cues[1].translation.end);
    selection.extend(original.start, true);
    selection.finish();
    let all = selection.text();
    ensure!(all == doc.text, "跨句反向选择丢字/混入时间戳");
    let emoji = doc.text.find('🐱').unwrap();
    selection.begin(doc.clone(), emoji);
    selection.extend(emoji + 1, true);
    ensure!(selection.text().is_empty(), "半个UTF-8字符仍被复制");
    selection.extend(emoji + 4, true);
    ensure!(selection.text() == "🐱", "emoji复制损坏");
    selection.begin(doc.clone(), 0);
    selection.extend(5, false);
    ensure!(
        !selection.dragged && !selection.has_selection(),
        "轻微移动误判拖选"
    );
    selection.clear();
    ensure!(
        selection.text().is_empty() && !selection.dragged,
        "切换/盲听清空后仍能复制旧内容"
    );
    fs::write(lib.root.join("selected-text.txt"), &all)?;
    fs::write(
        lib.root.join("selection-evidence.json"),
        serde_json::to_vec_pretty(
            &json!({"document":doc.text,"selected":all,"cue_count":doc.cues.len(),"utf8_safe":true,"timestamps_excluded":true,"native_drag_verified":false}),
        )?,
    )?;
    results.push(json!({"workflow":"真实SQLite/SRT字幕拖选映射：词内/中文/emoji、跨句正反向、拖动判定与清空、纯文本复制证据","passed":true,"evidence":lib.root.join("selection-evidence.json")}));
    Ok(())
}

//! Real SQLite, copied audio and ZIP restore for grouped speaking questions.
use anyhow::{Context, Result, ensure};
use clarora_gpui::{
    ai, backup,
    data::Library,
    process,
    speaking::{self, AudioEdit, Question},
};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, sync::atomic::Ordering, time::Duration};

pub fn run(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let root = root.join(format!("speaking-groups-{}", clarora_gpui::data::id()));
    let lib = Library::open(root.join("source"))?;
    let group = speaking::create_group(&lib, "跟读")?;
    let empty_group = speaking::create_group(&lib, "待练习")?;
    ensure!(speaking::create_group(&lib, "跟读").is_err());
    ensure!(speaking::list_groups(&lib)?.contains(&group) && group.tags.is_empty());
    speaking::edit_group_tag(&lib, &group.id, "  TOEFL  ", true)?;
    ensure!(speaking::edit_group_tag(&lib, &group.id, "toefl", true).is_err());
    ensure!(speaking::edit_group_tag(&lib, &group.id, "invalid\ntag", true).is_err());
    ensure!(speaking::edit_group_tag(&lib, "missing", "托福", true).is_err());
    speaking::edit_group_tag(&lib, &group.id, "校园", true)?;
    let group = speaking::edit_group_tag(&lib, &group.id, "toefl", false)?;
    ensure!(group.tags == ["校园"]);
    ensure!(speaking::list_groups(&Library::open(lib.root.clone())?)?.contains(&group));

    let mut text = Question::draft(group.id.clone());
    text.text = "Read the sentence aloud.\n请朗读。".into();
    let (address, stop, server) = super::mock_ai()?;
    ai::save_shared(
        &lib,
        ai::Config {
            base_url: address,
            model: "shared-fixture".into(),
            engine: "compatible".into(),
            ..Default::default()
        },
        "sqlite-http-fixture",
    )?;
    let input = root.join("prompt.wav");
    let mut wav = hound::WavWriter::create(
        &input,
        hound::WavSpec {
            channels: 1,
            sample_rate: 8000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for i in 0..800 {
        wav.write_sample(i as i16)?;
    }
    wav.finalize()?;
    let cancel = process::cancel_token();
    let transcript = speaking::transcribe_prompt(&lib, &input, &cancel);
    let translated = speaking::translate_prompt(&lib, &text.text, &cancel);
    ensure!(
        transcript? == "Hello world"
            && translated
                .as_ref()
                .is_ok_and(|text| text == "请朗读这句话。")
    );
    text.translation = Some(translated?);
    let text = speaking::save(&lib, text, AudioEdit::Keep)?;
    let audio = speaking::save(
        &lib,
        Question::draft(group.id.clone()),
        AudioEdit::Import(input.clone()),
    )?;
    let mut both = Question::draft(group.id.clone());
    both.text = "Respond to the recording.".into();
    let answer_file = root.join("answer.TXT");
    fs::write(&answer_file, "\u{feff}Opening\r\n第二段")?;
    both.answer_draft = speaking::read_answer_draft(&answer_file)?;
    ensure!(both.answer_draft == "Opening\n第二段");
    fs::write(&answer_file, [0xff, 0xfe])?;
    ensure!(speaking::read_answer_draft(&answer_file).is_err());
    fs::write(
        &answer_file,
        "x".repeat(speaking::MAX_ANSWER_DRAFT_CHARS + 1),
    )?;
    ensure!(speaking::read_answer_draft(&answer_file).is_err());
    let both = speaking::save(&lib, both, AudioEdit::Import(input.clone()))?;
    let mut second = Question::draft(group.id.clone());
    second.text = "Find the science building.".into();
    let second = speaking::save(&lib, second, AudioEdit::Import(input.clone()))?;
    let other_group = speaking::create_group(&lib, "另一组")?;
    let mut other = Question::draft(other_group.id);
    other.text = "Another group marker.".into();
    speaking::save(&lib, other, AudioEdit::Import(input.clone()))?;
    ensure!(speaking::suggest_group_tags(&lib, &empty_group.id, &cancel).is_err());
    ensure!(
        speaking::suggest_group_tags(&lib, &group.id, &cancel)
            .is_ok_and(|tags| { tags == ["校园", "问路", "场景对话"] })
    );
    let tags = speaking::suggest_group_tags(&lib, &group.id, &cancel)?;
    speaking::edit_group_tag(&lib, &group.id, "手动标签", true)?;
    let (group, added) = speaking::merge_group_tags(&lib, &group.id, &tags)?;
    ensure!(added == 2 && group.tags == ["校园", "手动标签", "问路", "场景对话"]);
    ensure!(speaking::merge_group_tags(&lib, &group.id, &tags)?.1 == 0);
    stop.store(true, Ordering::SeqCst);
    let requests = server
        .join()
        .map_err(|_| anyhow::anyhow!("speaking AI fixture panicked"))?;
    ensure!(
        requests.len() == 4
            && requests[0].contains("/audio/transcriptions")
            && requests[1..]
                .iter()
                .all(|request| request.contains("/chat/completions"))
    );
    let stored_audio = both
        .audio
        .as_ref()
        .context("mixed question lost its audio")?;
    let bytes = fs::read(&input)?;
    ensure!(stored_audio.path != input && fs::read(&stored_audio.path)? == bytes);
    fs::remove_file(&input)?;
    ensure!(speaking::save(&lib, both.clone(), AudioEdit::Import(input)).is_err());
    ensure!(speaking::list(&lib)?.contains(&both));
    ensure!(speaking::save(&lib, Question::draft(group.id.clone()), AudioEdit::Keep).is_err());
    ensure!(speaking::save(&lib, Question::draft("missing".into()), AudioEdit::Keep).is_err());
    ensure!(speaking::list(&Library::open(lib.root.clone())?)?.len() == 5);
    let zip = root.join("speaking.zip");
    backup::export_local(&lib, &zip)?;
    fs::rename(&lib.root, root.join("original-unavailable"))?;
    let restored = Library::open(root.join("restored"))?;
    backup::import_local(&restored, &zip)?;
    let restored_groups = speaking::list_groups(&restored)?;
    ensure!(restored_groups.contains(&group) && restored_groups.contains(&empty_group));
    let questions = speaking::list(&restored)?;
    ensure!(questions.contains(&text) && questions.len() == 5);
    for source in [audio, both, second] {
        let restored_question = questions
            .iter()
            .find(|item| item.id == source.id)
            .context("missing restored question")?;
        let attachment = restored_question
            .audio
            .as_ref()
            .context("missing restored audio")?;
        ensure!(restored_question.group_id == group.id && restored_question.text == source.text);
        ensure!(restored_question.answer_draft == source.answer_draft);
        ensure!(
            attachment.path.starts_with(&restored.root) && fs::read(&attachment.path)? == bytes
        );
    }
    let report = root.join("report.json");
    fs::write(
        &report,
        serde_json::to_vec_pretty(&json!({
            "questions":questions,"tagged_group":group,"empty_group":empty_group,"zip":zip,"source_unavailable":true,"speaking_text_requests":requests,"passed":true
        }))?,
    )?;
    results.push(json!({"workflow":"口语题库分组、草稿导入、AI 标签及 ZIP 恢复","passed":true,"report":report}));
    if cfg!(all(debug_assertions, target_os = "macos"))
        && std::env::var("CLARORA_GUI_MEMORY_CHECK").as_deref() == Ok("1")
    {
        let out = root.join("native");
        fs::create_dir_all(&out)?;
        let mut command = Command::new(std::env::current_exe()?.with_file_name("clarora-gpui"));
        command.arg("--speaking-groups-check").arg(&out);
        let status = super::native_diagnostic::run(command, &out, Duration::from_secs(30))?;
        let report_path = out.join("diagnostic-run.json");
        let report: Value = serde_json::from_slice(&fs::read(&report_path)?)?;
        ensure!(
            status.success() && report["passed"] == true,
            "口语题库原生验证失败：{}",
            report["error"]
        );
        results.push(json!({"workflow":"原生口语题库：宽窄布局、题名保存、音轨定位与暂停续播","passed":true,"report":report_path}));
    }
    Ok(())
}

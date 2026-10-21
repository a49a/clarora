use anyhow::{Result, ensure};
use clarora_gpui::{
    audio_library::{self, State},
    data::{Library, text},
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::Path};
pub fn run(lib: &Library, fixture: &Path, results: &mut Vec<Value>) -> Result<()> {
    // An isolated real library prevents management operations affecting other E2E workflows.
    let lib = Library::open(lib.root.join("audio-manager"))?;
    let a = lib.practice("Course A")?;
    let b = lib.practice("Course B")?;
    lib.import_audio(&a, fixture)?;
    let original = lib.list("listening_audios")?.remove(0);
    let key = text(&original, "id");
    let audio = text(&original, "audio_uri");
    let subtitle = text(&original, "subtitle_uri");
    audio_library::rename(&lib, "listening_audios", &key, "  Lesson Alpha  ")?;
    let mut state = State::default();
    state.sync(&a, &lib.list("listening_audios")?);
    ensure!(
        state
            .visible(&lib.list("listening_audios")?, &a, "ALPHA")
            .len()
            == 1,
        "音频搜索失败"
    );
    state.filter = 2;
    ensure!(
        state
            .visible(&lib.list("listening_audios")?, &a, "")
            .is_empty(),
        "字幕附件筛选失败"
    );
    ensure!(!state.preview.is_empty(), "所选音频字幕预览丢失");
    audio_library::move_records(&lib, &a, &b, &[key.clone()])?;
    ensure!(
        lib.list("listening_audios")?[0]["practice_id"] == b,
        "移动组未保存"
    );
    ensure!(
        Path::new(&audio).is_file() && Path::new(&subtitle).is_file(),
        "移动丢失附件"
    );
    state.sync(&a, &lib.list("listening_audios")?);
    ensure!(state.selected.is_empty(), "空组保留旧详情");
    ensure!(
        audio_library::scoped_ids(
            &lib.list("listening_audios")?,
            &a,
            &BTreeSet::from([key.clone()])
        )
        .is_empty(),
        "跨组勾选未隔离"
    );
    ensure!(
        audio_library::delete_records(&lib, &a, &[key.clone()]).is_err(),
        "旧组删除快照可删除其他组音频"
    );
    lib.import_audio(&a, fixture)?;
    let second = lib
        .list("listening_audios")?
        .into_iter()
        .find(|r| text(r, "practice_id") == a)
        .unwrap();
    let second_id = text(&second, "id");
    audio_library::rename(&lib, "listening_audios", &second_id, "Lesson Alpha")?;
    ensure!(
        audio_library::move_records(&lib, &a, &b, &[second_id.clone()]).is_err(),
        "目标同名冲突未拒绝"
    );
    ensure!(
        lib.list("listening_audios")?
            .iter()
            .any(|r| text(r, "id") == second_id && text(r, "practice_id") == a),
        "移动失败没有回滚"
    );
    let mut beta = second.clone();
    let beta_id = clarora_gpui::data::id();
    beta.insert("id".into(), json!(beta_id));
    beta.insert("name".into(), json!("Lesson Beta"));
    lib.put("listening_audios", serde_json::to_value(&beta)?)?;
    ensure!(
        audio_library::move_records(&lib, &a, &b, &[beta_id.clone(), second_id.clone()]).is_err(),
        "批量移动同名冲突未失败"
    );
    ensure!(
        lib.list("listening_audios")?
            .iter()
            .filter(|r| text(r, "practice_id") == a)
            .count()
            == 2,
        "批量移动冲突留下部分移动"
    );
    state.filter = 0;
    state.sort = 1;
    state.sync(&a, &lib.list("listening_audios")?);
    let rows = lib.list("listening_audios")?;
    let sorted = state.visible(&rows, &a, "");
    ensure!(
        sorted[0]["name"] == "Lesson Alpha" && sorted[1]["name"] == "Lesson Beta",
        "名称排序无效"
    );
    ensure!(
        audio_library::rename(&lib, "listening_practices", &a, " Course B ").is_err(),
        "组重命名允许重名"
    );
    state.filter = 0;
    state.sort = 1;
    state.sync(&b, &lib.list("listening_audios")?);
    ensure!(
        state.visible(&lib.list("listening_audios")?, &b, "alpha")[0]["id"] == key,
        "排序后音频身份错位"
    );
    audio_library::delete_records(&lib, &b, &[key.clone()])?;
    ensure!(
        Path::new(&audio).is_file() && Path::new(&subtitle).is_file(),
        "移除记录误删附件"
    );
    let reopened = Library::open(lib.root.clone())?;
    ensure!(
        !reopened
            .list("listening_audios")?
            .iter()
            .any(|r| text(r, "id") == key),
        "删除未持久化"
    );
    fs::write(
        lib.root.join("management-evidence.json"),
        serde_json::to_vec_pretty(
            &json!({"rename":true,"search_filter":true,"group_scope":true,"move_with_subtitles":true,"duplicate_rollback":true,"delete_snapshot_scope":true,"attachments_retained":true,"reopen":true}),
        )?,
    )?;
    results.push(json!({"workflow":"音频管理：真实SQLite/文件 → 搜索筛选/分组选择 → 重命名/移动冲突回滚 → 删除作用域及附件保留 → 重开", "passed":true,"evidence":"audio-manager/management-evidence.json"}));
    Ok(())
}

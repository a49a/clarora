use anyhow::{Result, ensure};
use clarora_gpui::{
    audio_library::{self, State},
    data::{Library, text},
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::Path};
pub fn run(lib: &Library, fixture: &Path, results: &mut Vec<Value>) -> Result<()> {
    run_inline_rename(lib, fixture, results)?;
    // An isolated real library prevents management operations affecting other E2E workflows.
    let lib = Library::open(lib.root.join("audio-manager"))?;
    let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
    let first_name = audio_library::default_practice_name(&lib.list("listening_practices")?, date);
    ensure!(first_name == "2026-10-23", "新建组未建议日期名称");
    let first_group_id = lib.practice(&first_name)?;
    let second_name = audio_library::default_practice_name(&lib.list("listening_practices")?, date);
    ensure!(second_name == "2026-10-23 (2)", "同日新组建议名称未避让");
    let second_group_id = lib.practice(&second_name)?;
    ensure!(first_group_id != second_group_id, "同日新建误用已有组");
    lib.practice(" 2026-10-23 (3) ")?;
    let third = audio_library::default_practice_name(&lib.list("listening_practices")?, date);
    ensure!(third == "2026-10-23 (4)", "同日多组建议名称冲突");
    let count = lib.list("listening_practices")?.len();
    ensure!(
        audio_library::default_practice_name(&lib.list("listening_practices")?, date) == third
            && lib.list("listening_practices")?.len() == count,
        "仅打开或取消新建表单就创建了练习组"
    );
    // The suggestion remains editable; saving must persist the user's chosen name.
    let custom_id = lib.practice("My writing and listening")?;
    let next_day = audio_library::default_practice_name(
        &lib.list("listening_practices")?,
        date.succ_opt().unwrap(),
    );
    ensure!(next_day == "2026-10-24", "新日期沿用旧日期名称");
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
    let groups = reopened.list("listening_practices")?;
    ensure!(
        [
            (first_group_id.as_str(), first_name.as_str()),
            (second_group_id.as_str(), second_name.as_str()),
            (custom_id.as_str(), "My writing and listening")
        ]
        .iter()
        .all(|(id, name)| groups
            .iter()
            .any(|r| text(r, "id") == *id && text(r, "name") == *name)),
        "日期组或自定义名称重开后丢失"
    );
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
            &json!({"default_practice_names":{"first":first_name,"second":second_name,"next_available":third,"next_day":next_day,"custom_name_persisted":true,"suggestion_does_not_create":true},"rename":true,"search_filter":true,"group_scope":true,"move_with_subtitles":true,"duplicate_rollback":true,"delete_snapshot_scope":true,"attachments_retained":true,"reopen":true}),
        )?,
    )?;
    results.push(json!({"workflow":"音频管理：真实SQLite/文件 → 搜索筛选/分组选择 → 重命名/移动冲突回滚 → 删除作用域及附件保留 → 重开", "passed":true,"evidence":"audio-manager/management-evidence.json"}));
    Ok(())
}

// Failure cases are specified before the inline editor implementation: cancellation must
// not write, changing selection must not change the target, invalid names must leave the
// draft open, and a changed/moved/deleted group target must never rename another record.
fn run_inline_rename(lib: &Library, fixture: &Path, results: &mut Vec<Value>) -> Result<()> {
    let lib = Library::open(lib.root.join("audio-inline-rename"))?;
    let a = lib.practice("Inline A")?;
    let b = lib.practice("Inline B")?;
    lib.import_audio(&a, fixture)?;
    let original = lib.list("listening_audios")?.remove(0);
    let target_id = text(&original, "id");
    audio_library::rename(&lib, "listening_audios", &target_id, "Original A")?;
    let other_id = clarora_gpui::data::id();
    let remote_id = clarora_gpui::data::id();
    for (id, group, name) in [(&other_id, &a, "Other A"), (&remote_id, &b, "Shared B")] {
        let mut row = original.clone();
        row.insert("id".into(), json!(id));
        row.insert("practice_id".into(), json!(group));
        row.insert("name".into(), json!(name));
        lib.put("listening_audios", serde_json::to_value(&row)?)?;
    }
    let stored_name = |id: &str| -> Result<String> {
        let rows = lib.list("listening_audios")?;
        let row = rows.iter().find(|r| text(r, "id") == id).unwrap();
        Ok(text(row, "name"))
    };
    let mut state = State::default();
    ensure!(
        state.begin_rename(&a, &lib.list("listening_audios")?, &target_id)? == "Original A"
            && state.selected == target_id,
        "行内改名未预填原名或选中目标"
    );
    let target = state.renaming.as_ref().unwrap();
    ensure!(
        target.id == target_id && target.group == a,
        "行内改名未绑定记录及组"
    );
    state.cancel_rename();
    ensure!(
        state.renaming.is_none() && stored_name(&target_id)? == "Original A",
        "取消行内改名写入了数据库"
    );

    state.begin_rename(&a, &lib.list("listening_audios")?, &target_id)?;
    state.selected = other_id.clone();
    state.save_rename(&lib, &a, "Shared B")?;
    ensure!(
        state.renaming.is_none()
            && stored_name(&target_id)? == "Shared B"
            && stored_name(&other_id)? == "Other A"
            && stored_name(&remote_id)? == "Shared B",
        "行内改名跟随选择写错ID，或错误拒绝不同组同名"
    );
    state.begin_rename(&a, &lib.list("listening_audios")?, &target_id)?;
    let too_long = "字".repeat(121);
    for invalid in ["", "   ", too_long.as_str(), "  other a  "] {
        ensure!(
            state.save_rename(&lib, &a, invalid).is_err()
                && state.renaming.as_ref().is_some_and(|r| r.id == target_id)
                && stored_name(&target_id)? == "Shared B",
            "空白、超长或同组重名保存未失败并保留编辑"
        );
    }
    state.save_rename(&lib, &a, "  听力练习 🎧 café  ")?;
    ensure!(
        state.renaming.is_none() && stored_name(&target_id)? == "听力练习 🎧 café",
        "Unicode名称或前后空格处理错误"
    );

    state.begin_rename(&a, &lib.list("listening_audios")?, &target_id)?;
    ensure!(
        state.save_rename(&lib, &b, "Wrong group").is_err()
            && state.renaming.is_some()
            && stored_name(&target_id)? == "听力练习 🎧 café",
        "切换组后仍可保存旧组编辑"
    );
    state.sync(&b, &lib.list("listening_audios")?);
    ensure!(state.renaming.is_none(), "切换组未清理行内编辑");

    state.begin_rename(&a, &lib.list("listening_audios")?, &other_id)?;
    audio_library::move_records(&lib, &a, &b, &[other_id.clone()])?;
    ensure!(
        state.save_rename(&lib, &a, "Moved by mistake").is_err()
            && state.renaming.is_some()
            && stored_name(&other_id)? == "Other A",
        "后台移动后仍可保存旧组编辑"
    );
    state.sync(&a, &lib.list("listening_audios")?);
    ensure!(state.renaming.is_none(), "后台移动后未清理行内编辑");

    state.begin_rename(&b, &lib.list("listening_audios")?, &remote_id)?;
    audio_library::delete_records(&lib, &b, &[remote_id.clone()])?;
    state.sync(&b, &lib.list("listening_audios")?);
    ensure!(state.renaming.is_none(), "目标被删除后未清理行内编辑");
    let reopened = Library::open(lib.root.clone())?;
    ensure!(
        reopened
            .list("listening_audios")?
            .iter()
            .any(|r| text(r, "id") == target_id && text(r, "name") == "听力练习 🎧 café"),
        "行内改名重开后丢失"
    );
    fs::write(
        lib.root.join("inline-rename-evidence.json"),
        serde_json::to_vec_pretty(&json!({
            "cancel_no_write":true,"target_id_independent_of_selection":true,
            "invalid_name_keeps_editing":true,"cross_group_duplicate_allowed":true,
            "changed_group_rejected":true,"moved_target_rejected":true,
            "sync_clears_changed_moved_deleted_targets":true,
            "unicode_trimmed_name":"听力练习 🎧 café","reopen":true
        }))?,
    )?;
    results.push(json!({"workflow":"音频行内改名：真实SQLite → 目标ID及取消 → 名称校验/跨组冲突 → 移动删除失效 → Unicode重开持久化","passed":true,"evidence":"audio-inline-rename/inline-rename-evidence.json"}));
    Ok(())
}

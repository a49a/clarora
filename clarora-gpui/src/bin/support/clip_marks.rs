use anyhow::{Result, ensure};
use clarora_gpui::{
    backup,
    clip_marks::{self, Catalog, Source},
    data::{Library, id, now, text},
    process, subtitles,
};
use serde_json::Value;
use std::{fs, path::Path};

pub fn run(parent: &Library, fixture: &Path, results: &mut Vec<Value>) -> Result<()> {
    let lib = Library::open(parent.root.join("saved-clip-marks"))?;
    let group = lib.practice("Clips")?;
    lib.import_audio(&group, fixture)?;
    let audio = lib.list("listening_audios")?.remove(0);
    let source = Source::audio(&text(&audio, "id"));
    let cues = subtitles::parse(
        "1\n00:00:00,000 --> 00:00:02,000\nHello world again.\n你好世界\n\n2\n00:00:02,000 --> 00:00:04,000\nKeep learning every day.\n每天学习\n",
    );
    // Real SRT loaded through the audio record, including a gap between cues.
    let edge_path = lib.root.join("selection.srt");
    fs::write(
        &edge_path,
        "1\n00:00:00,000 --> 00:00:02,000\nHello world again.\n你好\n\n2\n00:00:03,000 --> 00:00:04,000\nKeep learning.\n继续学习\n",
    )?;
    lib.db()?.execute(
        "UPDATE listening_audios SET subtitle_uri=? WHERE id=?",
        rusqlite::params![edge_path.to_string_lossy(), source.key],
    )?;
    let stored = lib.list("listening_audios")?.remove(0);
    let edge_cues = subtitles::parse(&fs::read_to_string(text(&stored, "subtitle_uri"))?);
    let word = subtitles::words(&edge_cues[0]).remove(1);
    let a = word.start + (word.end - word.start) * 0.2;
    let b = word.start + (word.end - word.start) * 0.8;
    let narrow = subtitles::selection_edges(&edge_cues, Some(a), Some(b)).unwrap();
    ensure!(
        narrow.start.cue == 0
            && narrow.end.cue == 0
            && narrow.start.word == 1
            && narrow.end.word == 1
            && (narrow.start.fraction - 0.2).abs() < 0.001
            && (narrow.end.fraction - 0.8).abs() < 0.001,
        "同词内边界没有跟随音轨精确范围"
    );
    let cross = subtitles::selection_edges(&edge_cues, Some(a), Some(3.5)).unwrap();
    ensure!(
        cross.start.cue == 0 && cross.end.cue == 1,
        "跨句选区丢失起终点"
    );
    let gap_start = subtitles::selection_edges(&edge_cues, Some(2.5), Some(3.5)).unwrap();
    let gap_end = subtitles::selection_edges(&edge_cues, Some(a), Some(2.5)).unwrap();
    ensure!(
        gap_start.start.cue == 1
            && gap_start.start.fraction == 0.
            && gap_end.end.cue == 0
            && gap_end.end.fraction == 1.,
        "间隙边界未落在选中字幕内侧"
    );
    ensure!(
        subtitles::selection_edges(&edge_cues, Some(2.2), Some(2.8)).is_none()
            && subtitles::selection_edges(&edge_cues, None, Some(b)).is_none()
            && subtitles::selection_edges(&edge_cues, Some(b), Some(a)).is_none(),
        "空白/未设定/反向范围仍有字幕边界"
    );
    fs::write(
        lib.root.join("selection-edges.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"narrow":narrow,"cross":cross,"gap_start":gap_start,"gap_end":gap_end,"source_srt":edge_path}),
        )?,
    )?;
    results.push(serde_json::json!({"workflow":"真实音频记录/SRT选择投影：同词内A/B、跨句、字幕间隙、未设置/清除边界","passed":true,"evidence":lib.root.join("selection-edges.json")}));
    let original = text(&audio, "audio_uri");
    let cancel = process::cancel_token();
    let card = clip_marks::save(
        &lib,
        source.clone(),
        Path::new(&original),
        0.5,
        3.,
        "world again Keep learning",
        "保留译文",
        &cancel,
    )?;
    let cat = Catalog::load(&lib)?;
    let ranges = cat.ranges(&source, &cues, &original);
    ensure!(
        ranges.len() == 1 && ranges[0].start == 0.5 && ranges[0].end == 3.,
        "新片段未保存来源范围"
    );
    ensure!(
        cat.ranges(&Source::audio("other"), &cues, &original)
            .is_empty(),
        "新片段串音频"
    );
    ensure!(
        ranges[0].overlaps(1., 2.) && ranges[0].overlaps(2., 2.5) && !ranges[0].overlaps(3., 4.),
        "跨句范围边界有误"
    );
    let second = clip_marks::save(
        &lib,
        source.clone(),
        Path::new(&original),
        1.,
        2.,
        "edited by user",
        "",
        &cancel,
    )?;
    let reopened = Library::open(lib.root.clone())?;
    ensure!(
        Catalog::load(&reopened)?
            .ranges(&source, &cues, &original)
            .len()
            == 2,
        "重启/编辑原文丢失来源标记"
    );
    let token = process::cancel_token();
    token.store(true, std::sync::atomic::Ordering::SeqCst);
    let before = lib.list("clip_cards")?.len();
    ensure!(
        clip_marks::save(
            &lib,
            source.clone(),
            Path::new(&original),
            0.,
            1.,
            "cancel",
            "",
            &token
        )
        .is_err()
            && lib.list("clip_cards")?.len() == before,
        "取消导出留下卡片"
    );
    let file_count = fs::read_dir(lib.root.join("media"))?.count();
    lib.db()?.execute_batch("CREATE TRIGGER fail_origin BEFORE INSERT ON ai_records WHEN NEW.kind='clip_origin' BEGIN SELECT RAISE(ABORT,'origin failed'); END;")?;
    ensure!(
        clip_marks::save(
            &lib,
            source.clone(),
            Path::new(&original),
            0.,
            1.,
            "failed save",
            "",
            &cancel
        )
        .is_err(),
        "来源写入失败仍保存卡片"
    );
    ensure!(
        lib.list("clip_cards")?.len() == before
            && fs::read_dir(lib.root.join("media"))?.count() == file_count,
        "导出失败留下卡片或附件"
    );
    lib.db()?.execute_batch("DROP TRIGGER fail_origin;")?;
    results.push(serde_json::json!({"workflow":"真实FFmpeg片段导出、来源范围/跨句/重叠、材料隔离、编辑原文/重启与取消","passed":true}));

    // Old exported clips lack provenance. Only a unique whole phrase can be inferred.
    let legacy = id();
    lib.put("clip_cards",serde_json::json!({"id":legacy,"en_text":"Keep learning","zh_text":"","audio_uri":card.media,"start_ms":0,"end_ms":1000,"created_at":now()}))?;
    let cat = Catalog::load(&lib)?;
    let old = cat.ranges(&source, &cues, &original);
    ensure!(
        old.iter().filter(|r| r.saved.id == legacy).count() == 1,
        "旧片段未按完整原文匹配"
    );
    let repeated = subtitles::parse("Keep learning\nKeep learning");
    ensure!(
        !cat.ranges(&source, &repeated, &original)
            .iter()
            .any(|r| r.saved.id == legacy),
        "重复原文错误定位"
    );
    lib.put(
        "review_schedule",
        serde_json::json!({"card_kind":"clip","card_id":card.id,"reps":2,"created_at":now()}),
    )?;
    lib.put(
        "discover_favorites",
        serde_json::json!({"key":format!("clip:{}",card.id),"payload":"{}"}),
    )?;
    lib.db()?.execute_batch("CREATE TRIGGER fail_unmark BEFORE DELETE ON review_schedule BEGIN SELECT RAISE(ABORT,'blocked'); END;")?;
    ensure!(
        clip_marks::remove(&lib, &card).is_err()
            && lib
                .list("clip_cards")?
                .iter()
                .any(|r| text(r, "id") == card.id),
        "删除失败未原子回滚"
    );
    lib.db()?.execute_batch("DROP TRIGGER fail_unmark;")?;
    clip_marks::remove(&lib, &card)?;
    ensure!(
        !lib.cards(false)?
            .iter()
            .any(|c| c.kind == "clip" && c.id == card.id)
            && lib.list("review_schedule")?.is_empty()
            && lib.list("discover_favorites")?.is_empty(),
        "取消收藏未退出复习/收藏"
    );
    ensure!(
        Path::new(&card.media).is_file() && Path::new(&original).is_file(),
        "取消收藏删除了媒体"
    );
    ensure!(
        Catalog::load(&lib)?
            .ranges(&source, &cues, &original)
            .iter()
            .any(|r| r.saved.id == second.id),
        "取消重叠片段误删另一卡片"
    );
    clip_marks::remove(&lib, &card)?; // stale dialog is harmless
    results.push(serde_json::json!({"workflow":"旧片段唯一文本匹配/重复句回避、原子取消收藏与复习、保留源文件/其它重叠卡片","passed":true}));

    let mut videos = clarora_gpui::video_library::Collection::default();
    videos.add(&[fixture.to_path_buf()], None)?;
    videos.save(&lib)?;
    let video_source = Source::video(fixture);
    let video_card = clip_marks::save(
        &lib,
        video_source.clone(),
        fixture,
        0.,
        1.,
        "Hello world",
        "你好",
        &cancel,
    )?;
    let zip = lib.root.join("clips.zip");
    backup::export_local(&lib, &zip)?;
    let target = Library::open(lib.root.join("restored"))?;
    let existing_group = target.practice("Clips")?;
    target.import_audio(&existing_group, fixture)?; // same-name merge changes source audio ID
    backup::import_local(&target, &zip)?;
    let restored_audio = target.list("listening_audios")?.remove(0);
    let restored_source = Source::audio(&text(&restored_audio, "id"));
    ensure!(
        text(&restored_audio, "id") != source.key,
        "合并夹具未更换音频ID"
    );
    let cat = Catalog::load(&target)?;
    ensure!(
        cat.ranges(&restored_source, &cues, &text(&restored_audio, "audio_uri"))
            .iter()
            .any(|r| r.saved.id == second.id),
        "备份合并未重映射音频来源"
    );
    let collection = clarora_gpui::video_library::Collection::load(&target)?;
    let video = &collection.items[0].path;
    ensure!(
        cat.ranges(&Source::video(video), &cues, &video.to_string_lossy())
            .iter()
            .any(|r| r.saved.id == video_card.id),
        "恢复未重定位视频来源"
    );
    fs::write(
        lib.root.join("saved-clips-evidence.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"audio_ranges":cat.ranges(&restored_source,&cues,&text(&restored_audio,"audio_uri")),"video_ranges":cat.ranges(&Source::video(video),&cues,&video.to_string_lossy()),"legacy":legacy,"removed":card.id}),
        )?,
    )?;
    results.push(serde_json::json!({"workflow":"ZIP恢复后音频来源ID合并与视频路径重定位、已收藏标记与取消操作持久化","passed":true,"evidence":lib.root.join("saved-clips-evidence.json")}));
    Ok(())
}

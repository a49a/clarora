use anyhow::{Context, Result, ensure};
use clarora_gpui::{data::Library, media::Player, video_library::Collection};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};
pub fn run(lib: &Library, fixture: &Path, results: &mut Vec<Value>) -> Result<()> {
    let lib = Library::open(lib.root.join("video-collection"))?;
    let a = lib.root.join("folder-a/lesson.mp4");
    let b = lib.root.join("folder-b/lesson.mp4");
    for path in [&a, &b] {
        fs::create_dir_all(path.parent().unwrap())?;
        fs::copy(fixture, path)?;
    }
    lib.set_setting(
        "recent_videos",
        &serde_json::to_string(&vec![a.clone(), b.clone(), a.clone()])?,
    )?;
    let cues = vec![clarora_gpui::subtitles::Cue {
        start: 0.,
        end: 4.,
        text: "Video collection subtitle".into(),
    }];
    clarora_gpui::video_subtitles::save(&lib, &b, &cues, &clarora_gpui::process::cancel_token())?;
    let mut c = Collection::load(&lib)?;
    ensure!(c.items.len() == 2, "迁移两个同名视频失败或未去重");
    let ids = c.items.iter().map(|v| v.id.clone()).collect::<Vec<_>>();
    let before = c.items.len();
    c.add(&[a.clone(), b.clone()], None)?;
    c.save(&lib)?;
    ensure!(c.items.len() == before, "重复导入未去重");
    ensure!(
        c.add(&[a.clone(), lib.root.join("missing.mp4")], None)
            .is_err()
            && c.items.len() == before,
        "失败批量导入改变了收藏"
    );
    let extra = (0..13)
        .map(|i| lib.root.join(format!("extra-{i}.mp4")))
        .collect::<Vec<_>>();
    for p in &extra {
        fs::copy(fixture, p)?;
    }
    c.add(&extra, None)?;
    c.save(&lib)?;
    c = Collection::load(&lib)?;
    ensure!(c.items.len() == 15, "收藏被最近12条限制截断");
    let mut series = clarora_gpui::bilibili::Series {
        title: "整套课程标题".into(),
        base: "https://www.bilibili.com/video/BV1xx411c7mD".into(),
        parts: vec![clarora_gpui::bilibili::Part {
            number: 1,
            title: "First lesson".into(),
            duration: 4.,
            path: Some(a.clone()),
        }],
        requested: 0,
        current: Some(0),
    };
    c.add(std::slice::from_ref(&a), Some(&series))?;
    c.select(&ids[0])?;
    c.save(&lib)?;
    ensure!(
        c.active().unwrap().series.is_some(),
        "收藏丢失自己的分P队列"
    );
    series.parts.push(clarora_gpui::bilibili::Part {
        number: 2,
        title: "Second lesson".into(),
        duration: 4.,
        path: Some(extra[0].clone()),
    });
    series.current = Some(1);
    c.add(&[extra[0].clone()], Some(&series))?;
    ensure!(
        c.items
            .iter()
            .find(|v| v.id == ids[0])
            .unwrap()
            .series
            .as_ref()
            .unwrap()
            .cached(1)
            == Some(extra[0].clone()),
        "同一视频收藏未更新后续分P缓存"
    );
    ensure!(c.items.len() == 14, "同源分P仍各自占用收藏");
    ensure!(
        c.items.iter().find(|v| v.id == ids[0]).unwrap().title == "整套课程标题",
        "切分P后收藏标题变成编号或分P标题"
    );
    let grouped = c.select(&ids[0])?;
    ensure!(
        grouped.path == extra[0] && grouped.series.as_ref().unwrap().current == Some(1),
        "收藏没有保留最后选择的分P"
    );
    // Simulate the previous per-part schema in a library first opened by the old app.
    let mut legacy_group = serde_json::to_value(&c)?;
    let legacy_first = legacy_group["items"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|v| v["id"] == ids[0])
        .unwrap();
    legacy_first["path"] = json!(a);
    legacy_first["series"]["current"] = json!(0);
    let mut duplicate = serde_json::to_value(&grouped)?;
    duplicate["id"] = json!("legacy-part-two");
    legacy_group["items"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    legacy_group["selected"] = json!("legacy-part-two");
    let old_parts = Library::open(lib.root.join("old-parts"))?;
    old_parts.set_setting("video_favorites_v1", &serde_json::to_string(&legacy_group)?)?;
    c = Collection::load(&old_parts)?;
    ensure!(
        c.items.len() == 14
            && c.active().unwrap().path == extra[0]
            && c.active().unwrap().id == ids[0],
        "旧分P收藏未合并或选择丢失"
    );
    ensure!(a.is_file() && extra[0].is_file(), "合并误删分P文件");
    c.select(&ids[1])?;
    ensure!(c.active().unwrap().series.is_none(), "本地视频串用分P队列");
    c.save(&lib)?;
    c = Collection::load(&lib)?;
    ensure!(
        c.active().is_some_and(|v| v.path == b),
        "重启没有恢复第二个视频选择"
    );
    let player = Player::without_audio_device();
    player.open(&b);
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let s = player.snapshot();
        if s.position > 0.15 {
            break;
        }
        ensure!(Instant::now() < deadline, "第二个收藏无法播放");
        std::thread::sleep(Duration::from_millis(30));
    }
    c.remove(&ids[1])?;
    c.save(&lib)?;
    player.command(["stop"]);
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let s = player.snapshot();
        if s.path.is_empty() && s.frame.is_none() {
            break;
        }
        ensure!(Instant::now() < deadline, "移除当前视频后播放器没有停止");
        std::thread::sleep(Duration::from_millis(30));
    }
    ensure!(b.is_file(), "移除收藏误删本机视频");
    ensure!(
        !clarora_gpui::video_subtitles::load(&lib, &b)?.is_empty(),
        "移除收藏误删字幕缓存"
    );
    let c = Collection::load(&lib)?;
    ensure!(
        c.selected.is_none() && !c.items.iter().any(|v| v.id == ids[1]),
        "移除收藏后重启复活或自动选择其他项"
    );
    fs::remove_file(&a)?;
    fs::remove_file(&extra[0])?;
    let mut c = Collection::load(&lib)?;
    ensure!(c.select(&ids[0]).is_err(), "缺失文件仍可选择播放");
    c.remove(&ids[0])?;
    c.save(&lib)?;
    ensure!(
        !Collection::load(&lib)?.items.iter().any(|v| v.id == ids[0]),
        "缺失文件收藏无法移除"
    );
    // Bad metadata must be surfaced rather than silently reset and overwritten.
    let damaged = Library::open(lib.root.join("damaged"))?;
    damaged.set_setting("video_favorites_v1", "{broken")?;
    ensure!(Collection::load(&damaged).is_err(), "损坏收藏被默认为空");
    damaged.set_setting("video_favorites_v1", "[]")?;
    ensure!(Collection::load(&damaged).is_err(), "错误schema未拒绝");
    let groups_lib = Library::open(lib.root.join("video-groups"))?;
    let group_a = groups_lib.root.join("group-a/lesson.mp4");
    let group_b = groups_lib.root.join("group-b/interview.mp4");
    for path in [&group_a, &group_b] {
        fs::create_dir_all(path.parent().unwrap())?;
        fs::copy(fixture, path)?;
    }
    let mut grouped = Collection::default();
    let course = grouped.create_group("课程")?;
    ensure!(
        grouped.create_group(" 课程 ").is_err() && grouped.create_group(" ").is_err(),
        "分组重名或空名称未拒绝"
    );
    let first = grouped.add_in_group(std::slice::from_ref(&group_a), None, &course)?;
    let second = grouped.add_in_group(std::slice::from_ref(&group_b), None, "访谈")?;
    ensure!(
        grouped.groups == vec!["课程".to_string(), "访谈".to_string()]
            && grouped.items[0].group == "课程"
            && grouped.items[1].group == "访谈",
        "新建分组或导入分组错误"
    );
    grouped.save(&groups_lib)?;
    let mut grouped = Collection::load(&groups_lib)?;
    ensure!(
        grouped.group_count("课程") == 1
            && clarora_gpui::video_library::matches_group(&grouped.items[0], "课程")
            && !clarora_gpui::video_library::matches_group(&grouped.items[0], "访谈"),
        "分组过滤或重启恢复失败"
    );
    grouped.rename_group("课程", "课程 A")?;
    ensure!(
        grouped.items[0].group == "课程 A" && grouped.has_group("课程 A"),
        "重命名分组未同步视频"
    );
    grouped.set_group(&first[0], "访谈")?;
    ensure!(grouped.group_count("访谈") == 2, "移动视频到分组失败");
    let moved = grouped.remove_group("访谈")?;
    ensure!(
        moved == 2
            && grouped.items.iter().all(|video| video.group.is_empty())
            && !grouped.has_group("访谈"),
        "删除分组未保留视频或未移出分组"
    );
    let old_groups = Library::open(lib.root.join("old-groups"))?;
    let legacy_groups = groups_lib.setting("video_favorites_v1")?;
    old_groups.set_setting("video_favorites_v1", &legacy_groups)?;
    let legacy_groups = Collection::load(&old_groups)?;
    ensure!(
        legacy_groups.groups.is_empty()
            && legacy_groups
                .items
                .iter()
                .all(|video| video.group.is_empty()),
        "旧收藏未按未分组迁移"
    );
    // Migrate the intermediate format which stored group names inline.
    let transitional = Library::open(lib.root.join("transitional-groups"))?;
    grouped.create_group("待学习")?;
    grouped.set_group(&first[0], "待学习")?;
    transitional.set_setting("video_favorites_v1", &serde_json::to_string(&grouped)?)?;
    let migrated = Collection::load(&transitional)?;
    let mut v1: Value = serde_json::from_str(&transitional.setting("video_favorites_v1")?)?;
    ensure!(
        v1.get("groups").is_none()
            && v1["items"][0].get("group").is_none()
            && migrated.groups == ["课程 A", "待学习"]
            && migrated.items[0].group == "待学习",
        "未迁移已有分组或旧版无法读取收藏"
    );
    // An old client can still select another video or rewrite the favorites value.
    v1["selected"] = json!(second[0]);
    transitional.set_setting("video_favorites_v1", &v1.to_string())?;
    let reopened = Collection::load(&transitional)?;
    ensure!(
        reopened.active().is_some_and(|video| video.id == second[0])
            && reopened.groups == migrated.groups
            && reopened.items[0].group == "待学习",
        "旧版重写收藏后分组或归属丢失"
    );
    let progress_key = clarora_gpui::video_progress::item_key(&grouped, &group_a);
    let not_favorited = clarora_gpui::bilibili::Series {
        title: "progress".into(),
        base: "https://www.bilibili.com/video/BV1xx411c7mD".into(),
        parts: vec![clarora_gpui::bilibili::Part {
            number: 1,
            title: "P1".into(),
            duration: 10.,
            path: Some(group_a.clone()),
        }],
        requested: 0,
        current: Some(0),
    };
    ensure!(
        clarora_gpui::video_progress::series_item_key(&grouped, &not_favorited, 0).is_none(),
        "未收藏的视频生成了分P进度键"
    );
    let mut progress = clarora_gpui::video_progress::Progress::load(&groups_lib);
    progress.record(
        progress_key.clone(),
        clarora_gpui::video_progress::Item {
            position: 12.5,
            completed: false,
            sentence_repeat: Some(3),
            repeat_mode: 1,
            reading: clarora_gpui::video_progress::Reading {
                tab: 1,
                transcript_open: false,
                follow: false,
                cue_start: Some(4.),
                active_start: Some(4.),
                cue_offset: 2.,
                transcript_item: None,
                transcript_cue_start: None,
                transcript_offset: 8.,
                transcript_revision: String::new(),
            },
            updated_ms: 0,
        },
    )?;
    progress.save(&groups_lib)?;
    let restored_progress = clarora_gpui::video_progress::Progress::load(&groups_lib);
    let item = restored_progress
        .get(&progress_key)
        .context("视频进度未持久化")?;
    ensure!(
        item.position == 12.5
            && item.sentence_repeat == Some(3)
            && item.repeat_mode == 1
            && item.reading.tab == 1
            && item.reading.cue_start == Some(4.),
        "视频进度或阅读快照恢复错误"
    );
    ensure!(
        clarora_gpui::video_progress::resume(Some(item))
            == clarora_gpui::video_progress::Resume::Continue,
        "视频进度标签错误"
    );
    groups_lib.set_setting(
        "last_video_position",
        &json!({"path": group_a, "position": 7.}).to_string(),
    )?;
    let mut legacy_progress = clarora_gpui::video_progress::Progress::load(&groups_lib);
    legacy_progress.seed_legacy(&groups_lib, "f:legacy-progress", &group_a);
    ensure!(
        legacy_progress
            .get("f:legacy-progress")
            .is_some_and(|item| item.position == 7.),
        "旧版单条播放位置未迁移到单项进度"
    );
    let mut favorited = grouped.clone();
    let favorite_id = favorited.items.first().context("缺少收藏夹具")?.id.clone();
    let two_parts = clarora_gpui::bilibili::Series {
        title: "Two parts".into(),
        base: "https://www.bilibili.com/video/BV1yy411c7mD".into(),
        parts: vec![
            clarora_gpui::bilibili::Part {
                number: 1,
                title: "A".into(),
                duration: 10.,
                path: Some(group_a.clone()),
            },
            clarora_gpui::bilibili::Part {
                number: 2,
                title: "B".into(),
                duration: 10.,
                path: Some(group_b.clone()),
            },
        ],
        requested: 0,
        current: Some(0),
    };
    favorited.items[0].series = Some(two_parts.clone());
    let key_a = clarora_gpui::video_progress::item_key(&favorited, &group_a);
    let key_b = clarora_gpui::video_progress::item_key(&favorited, &group_b);
    ensure!(
        key_a == format!("v:{favorite_id}:1")
            && key_b == format!("v:{favorite_id}:2")
            && clarora_gpui::video_progress::series_item_key(&favorited, &two_parts, 0)
                == Some(key_a.clone())
            && clarora_gpui::video_progress::series_item_key(&favorited, &two_parts, 1)
                == Some(key_b.clone()),
        "同一收藏的不同分P未按分P编号隔离"
    );
    let replay_key = format!("v:{favorite_id}:9");
    let mut replay = clarora_gpui::video_progress::Progress::load(&groups_lib);
    replay.record(
        replay_key.clone(),
        clarora_gpui::video_progress::Item {
            position: 10.,
            completed: true,
            sentence_repeat: None,
            repeat_mode: 0,
            reading: clarora_gpui::video_progress::Reading::default(),
            updated_ms: 0,
        },
    )?;
    ensure!(
        clarora_gpui::video_progress::resume(replay.get(&replay_key))
            == clarora_gpui::video_progress::Resume::Replay,
        "已播完视频没有显示重播"
    );
    groups_lib.set_setting("video_progress_v1", "{broken")?;
    ensure!(
        clarora_gpui::video_progress::Progress::load(&groups_lib)
            .get(&progress_key)
            .is_none(),
        "损坏的单项进度未安全回退"
    );
    fs::write(
        lib.root.join("video-library-evidence.json"),
        serde_json::to_vec_pretty(
            &json!({"same_name_paths_distinct":true,"dedup":true,"persist_more_than_12":true,"series_isolation":true,"one_favorite_per_source":true,"legacy_part_merge_preserves_selection":true,"second_video_playback":true,"remove_active_stops":true,"file_retained":true,"no_recent_resurrection":true,"missing_file_removal":true,"malformed_rejected":true,"video_groups_create_rename_move_delete":true,"legacy_videos_ungrouped":true,"groups_survive_legacy_favorites_rewrite":true}),
        )?,
    )?;
    results.push(json!({"workflow":"视频收藏：迁移/多文件去重/15项重开 → 分P隔离/第二视频播放 → 分组创建/重命名/移动/删除/旧数据迁移 → 单项进度/阅读快照/旧位置迁移 → 当前项移除停止且文件保留 → 缺失与损坏元数据", "passed":true,"evidence":"video-collection/video-library-evidence.json"}));
    Ok(())
}

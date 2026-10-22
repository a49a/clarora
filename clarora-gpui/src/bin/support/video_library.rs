use anyhow::{Result, ensure};
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
    c.add(&[a.clone()], Some(&series))?;
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
    // Simulate the previous per-part schema; migration must retain selected P2.
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
    lib.set_setting("video_favorites_v1", &serde_json::to_string(&legacy_group)?)?;
    c = Collection::load(&lib)?;
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
    lib.set_setting("video_favorites_v1", "{broken")?;
    ensure!(Collection::load(&lib).is_err(), "损坏收藏被默认为空");
    lib.set_setting("video_favorites_v1", "[]")?;
    ensure!(Collection::load(&lib).is_err(), "错误schema未拒绝");
    fs::write(
        lib.root.join("video-library-evidence.json"),
        serde_json::to_vec_pretty(
            &json!({"same_name_paths_distinct":true,"dedup":true,"persist_more_than_12":true,"series_isolation":true,"one_favorite_per_source":true,"legacy_part_merge_preserves_selection":true,"second_video_playback":true,"remove_active_stops":true,"file_retained":true,"no_recent_resurrection":true,"missing_file_removal":true,"malformed_rejected":true}),
        )?,
    )?;
    results.push(json!({"workflow":"视频收藏：迁移/多文件去重/15项重开 → 分P隔离/第二视频播放 → 当前项移除停止且文件保留 → 缺失与损坏元数据", "passed":true,"evidence":"video-collection/video-library-evidence.json"}));
    Ok(())
}

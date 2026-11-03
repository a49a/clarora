//! Real FFmpeg, SQLite, subtitles and ZIP scenarios specified before implementation.
use anyhow::{Result, ensure};
use clarora_gpui::{
    audio_library,
    audio_tools::{self, Catalog},
    backup,
    data::{Library, Row, id, now, text},
    library_controller::{self, Key},
    process, subtitles,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

fn source(
    lib: &Library,
    group: &str,
    name: &str,
    rate: u32,
    samples: u32,
    channels: u16,
    subtitle: bool,
) -> Result<Row> {
    let path = lib.root.join("media").join(format!("{name}.wav"));
    let mut wav = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for i in 0..samples {
        for _ in 0..channels {
            wav.write_sample(if i % 100 < 50 { 2000i16 } else { -2000i16 })?;
        }
    }
    wav.finalize()?;
    let srt = path.with_extension("srt");
    if subtitle {
        fs::write(
            &srt,
            format!("1\n00:00:00,000 --> 00:00:00,500\n{name} English.\n{name} 中文。\n"),
        )?;
    }
    let row = json!({"id":id(),"practice_id":group,"name":name,"audio_uri":path,"subtitle_uri":if subtitle{srt.to_string_lossy().into_owned()}else{String::new()},"created_at":now()});
    lib.put("listening_audios", row.clone())?;
    Ok(row.as_object().unwrap().clone())
}
fn names<'a>(rows: impl IntoIterator<Item = &'a Row>) -> Vec<String> {
    rows.into_iter().map(|r| text(r, "name")).collect()
}
fn media_files(lib: &Library) -> Result<BTreeSet<String>> {
    Ok(fs::read_dir(lib.root.join("media"))?
        .map(|r| r.unwrap().file_name().to_string_lossy().into_owned())
        .collect())
}
pub fn run(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let lib = Library::open(root.join(format!("audio-merge-{}", id())))?;
    let group = lib.practice("短句学习")?;
    let other = lib.practice("其它组")?;
    let a = source(&lib, &group, "a", 16000, 20000, 1, true)?;
    let b = source(&lib, &group, "b", 44100, 33075, 2, true)?;
    let c = source(&lib, &group, "c", 24000, 24000, 1, false)?;
    let originals = [a.clone(), b.clone(), c.clone()];
    let original_bytes = originals
        .iter()
        .map(|r| fs::read(text(r, "audio_uri")))
        .collect::<std::io::Result<Vec<_>>>()?;
    let mut cases = vec![];
    let mut management = audio_library::State::default();
    management.sort = 1;
    let checked = BTreeSet::from([text(&a, "id"), text(&b, "id")]);
    let ordered = management.ordered_checked(&lib.list("listening_audios")?, &group, &checked);
    ensure!(
        names(&ordered) == vec!["a", "b"],
        "merge selection does not follow current sort"
    );
    let merged = audio_tools::merge(
        &lib,
        &[b.clone(), a.clone()],
        &group,
        &process::cancel_token(),
    )?;
    let merged_id = text(&merged.row, "id");
    ensure!(
        merged.sources == 2 && merged.subtitles_complete,
        "merge report incorrect"
    );
    let cues = subtitles::parse(&fs::read_to_string(text(&merged.row, "subtitle_uri"))?);
    ensure!(
        cues.len() == 2
            && cues[0].text.starts_with('b')
            && cues[1].text.starts_with('a')
            && (cues[1].start - 0.75).abs() < 0.002
            && cues[1].text.contains("中文"),
        "decoded durations/order/bilingual subtitle offsets incorrect"
    );
    let duration = process::run(
        &process::tool("ffprobe")?,
        &[
            "-v".into(),
            "error".into(),
            "-show_entries".into(),
            "format=duration".into(),
            "-of".into(),
            "default=nokey=1:noprint_wrappers=1".into(),
            text(&merged.row, "audio_uri"),
        ],
        &process::cancel_token(),
        Duration::from_secs(10),
    )?
    .trim()
    .parse::<f64>()?;
    ensure!(
        (duration - 2.).abs() < 0.04,
        "merged media duration incorrect"
    );
    for (i, row) in originals.iter().enumerate() {
        ensure!(
            fs::read(text(row, "audio_uri"))? == original_bytes[i],
            "original media changed"
        );
    }
    let rows = lib.list("listening_audios")?;
    let index = Catalog::load(&lib, &rows)?;
    ensure!(
        index.learning(&rows, &group).len() == 2
            && text(index.learning(&rows, &group)[0], "id") == merged_id
            && index.hidden(&text(&a, "id"))
            && index.hidden(&text(&b, "id")),
        "study catalog includes merged sources"
    );
    ensure!(
        index
            .replacement(&text(&a, "id"), &rows)
            .is_some_and(|r| text(r, "id") == merged_id),
        "source selection not redirected to merged version"
    );
    management.merges = index;
    management.sync(&group, &rows);
    ensure!(
        management.visible(&rows, &group, "").len() == 2,
        "manager did not hide originals"
    );
    management.show_sources = true;
    ensure!(
        management.visible(&rows, &group, "").len() == 4,
        "show originals unavailable"
    );
    lib.set_setting("last_listening_audio", &text(&a, "id"))?;
    let snapshot = library_controller::load(
        &lib,
        &Key {
            page: "listening".into(),
            practice_id: group.clone(),
            ..Default::default()
        },
        false,
    )?;
    ensure!(
        snapshot
            .restore_audio
            .as_ref()
            .is_some_and(|(id, position)| id == &merged_id && *position == 0.),
        "reopen restores hidden source instead of merged version"
    );
    cases.push(json!({"case":"actual FFmpeg normalization/order/subtitle offsets/originals preserved/study and management catalog/restore","passed":true,"merged":merged.row,"duration":duration}));

    let cancelled = process::cancel_token();
    cancelled.store(true, Ordering::SeqCst);
    let batch = clarora_gpui::audio_batch::Plan::new(
        &lib,
        &group,
        clarora_gpui::audio_batch::Mode::English,
    )?
    .run(&lib, &cancelled, |_| {});
    ensure!(
        batch.total == 2,
        "whole-group subtitles still process hidden originals"
    );
    let merged_path = text(&merged.row, "audio_uri");
    let offline = Path::new(&merged_path).with_extension("offline");
    fs::rename(&merged_path, &offline)?;
    let missing = Catalog::load(&lib, &rows)?;
    ensure!(
        !missing.hidden(&text(&a, "id")) && !missing.hidden(&text(&b, "id")),
        "missing merged attachment hides originals"
    );
    fs::rename(&offline, &merged_path)?;
    cases.push(json!({"case":"batch skips hidden originals; missing merged attachment makes preserved sources visible","passed":true}));
    let archive = lib.root.join("merged-backup.zip");
    backup::export_local(&lib, &archive)?;
    let restored = Library::open(lib.root.join("restored"))?;
    let target_group = restored.practice("短句学习")?;
    restored.import_audio(&target_group, Path::new(&text(&a, "audio_uri")))?;
    let remapped_a = restored.list("listening_audios")?.remove(0);
    ensure!(
        text(&remapped_a, "id") != text(&a, "id"),
        "ID remap fixture invalid"
    );
    backup::import_local(&restored, &archive)?;
    let restored_rows = restored.list("listening_audios")?;
    let restored_index = Catalog::load(&restored, &restored_rows)?;
    ensure!(
        restored_index.hidden(&text(&remapped_a, "id"))
            && restored_index.learning(&restored_rows, &target_group).len() == 2,
        "ZIP restore did not remap merge source IDs"
    );
    cases.push(json!({"case":"ZIP restore retains merge relationships with same-name audio/group ID remapping","passed":true,"archive":archive}));

    let nested = audio_tools::merge(
        &lib,
        &[merged.row.clone(), c.clone()],
        &group,
        &process::cancel_token(),
    )?;
    ensure!(
        !nested.subtitles_complete && text(&nested.row, "subtitle_uri").is_empty(),
        "partial subtitles masquerade as complete"
    );
    let rows = lib.list("listening_audios")?;
    let index = Catalog::load(&lib, &rows)?;
    ensure!(
        index.learning(&rows, &group).len() == 1
            && index
                .replacement(&text(&a, "id"), &rows)
                .is_some_and(|r| text(r, "id") == text(&nested.row, "id")),
        "nested merge not resolved"
    );
    audio_library::delete_records(&lib, &group, &[text(&nested.row, "id")])?;
    let rows = lib.list("listening_audios")?;
    ensure!(
        Catalog::load(&lib, &rows)?.learning(&rows, &group).len() == 2,
        "removing merged version did not restore prior learning entries"
    );
    audio_library::move_records(&lib, &group, &other, &[merged_id.clone()])?;
    let rows = lib.list("listening_audios")?;
    let index = Catalog::load(&lib, &rows)?;
    ensure!(
        index.learning(&rows, &group).len() == 3,
        "moving merged version hides sources in old group"
    );
    audio_library::move_records(&lib, &other, &group, &[merged_id.clone()])?;
    let reopened = Library::open(lib.root.clone())?;
    let rows = reopened.list("listening_audios")?;
    ensure!(
        Catalog::load(&reopened, &rows)?
            .learning(&rows, &group)
            .len()
            == 2,
        "merge selection not persistent"
    );
    cases.push(json!({"case":"nested merges/missing subtitles/delete/move/reopen","passed":true}));

    let before = media_files(&lib)?;
    let count = lib.list("listening_audios")?.len();
    let cancel = process::cancel_token();
    cancel.store(true, Ordering::SeqCst);
    ensure!(
        audio_tools::merge(&lib, &[a.clone(), b.clone()], &group, &cancel).is_err(),
        "pre-cancel not rejected"
    );
    ensure!(
        audio_tools::merge(
            &lib,
            &[a.clone(), a.clone()],
            &group,
            &process::cancel_token()
        )
        .is_err(),
        "duplicate source accepted"
    );
    ensure!(
        audio_tools::merge(
            &lib,
            &[a.clone(), b.clone()],
            &other,
            &process::cancel_token()
        )
        .is_err(),
        "cross-group sources accepted"
    );
    ensure!(
        lib.list("listening_audios")?.len() == count && media_files(&lib)? == before,
        "failed merge left files or rows"
    );
    let worker_lib = lib.clone();
    let worker_rows = vec![a.clone(), b.clone()];
    let worker_group = group.clone();
    let worker = thread::spawn(move || {
        audio_tools::merge(
            &worker_lib,
            &worker_rows,
            &worker_group,
            &process::cancel_token(),
        )
    });
    let start = Instant::now();
    while !fs::read_dir(lib.root.join("media"))?
        .any(|r| r.is_ok_and(|r| r.file_name().to_string_lossy().starts_with(".merge-")))
    {
        ensure!(
            start.elapsed() < Duration::from_secs(5),
            "merge work never started"
        );
        thread::sleep(Duration::from_millis(2));
    }
    audio_library::rename(
        &lib,
        "listening_audios",
        &text(&a, "id"),
        "changed-during-merge",
    )?;
    ensure!(
        worker.join().unwrap().is_err(),
        "late merge overwrote a changed source"
    );
    ensure!(
        lib.list("listening_audios")?.len() == count && media_files(&lib)? == before,
        "concurrent failure left partial result"
    );
    cases.push(json!({"case":"pre-cancel/duplicate/cross-group/concurrent source edit leave no partial artifacts","passed":true}));
    let token = process::cancel_token();
    let worker_token = token.clone();
    let worker_lib = lib.clone();
    let worker_group = group.clone();
    let worker_rows = vec![b.clone(), c.clone()];
    let worker = thread::spawn(move || {
        audio_tools::merge(&worker_lib, &worker_rows, &worker_group, &worker_token)
    });
    let started = Instant::now();
    while !fs::read_dir(lib.root.join("media"))?
        .any(|r| r.is_ok_and(|r| r.file_name().to_string_lossy().starts_with(".merge-")))
    {
        ensure!(
            started.elapsed() < Duration::from_secs(5),
            "cancel fixture did not start"
        );
        thread::sleep(Duration::from_millis(2));
    }
    token.store(true, Ordering::SeqCst);
    ensure!(
        worker.join().unwrap().is_err()
            && media_files(&lib)? == before
            && lib.list("listening_audios")?.len() == count,
        "in-flight cancellation left rows or files"
    );
    cases.push(json!({"case":"in-flight cancellation removes normalization work and unpublished output","passed":true}));
    for (target, sources) in [
        (text(&a, "id"), vec![text(&b, "id"), text(&c, "id")]),
        (text(&b, "id"), vec![text(&a, "id"), text(&c, "id")]),
    ] {
        lib.put("ai_records",json!({"id":id(),"kind":"audio_merge","payload":json!({"target":target,"sources":sources}).to_string(),"created_at":now()}))?;
    }
    let rows = lib.list("listening_audios")?;
    ensure!(
        !Catalog::load(&lib, &rows)?
            .learning(&rows, &group)
            .is_empty(),
        "cyclic imported provenance hid the entire group"
    );
    cases.push(json!({"case":"cyclic imported relationships cannot hide every learning entry","passed":true}));
    let evidence = lib.root.join("audio-merge-report.json");
    fs::write(
        &evidence,
        serde_json::to_vec_pretty(&json!({"passed":true,"cases":cases,"library":lib.root}))?,
    )?;
    results.push(json!({"workflow":"合并所选音频→解码时长与双语字幕→保留原条目→学习版本去重→嵌套/恢复/ZIP重映射/失败保护","passed":true,"report":evidence,"cases":cases.len()}));
    Ok(())
}

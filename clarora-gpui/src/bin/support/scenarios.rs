//! Public E2E modes register validation, selection and execution together.
use super::*;
use std::ffi::OsString;
type Runner = fn(&Path, &[OsString], &mut Vec<Value>) -> Result<()>;
struct Scenario {
    flag: &'static str,
    min: usize,
    max: usize,
    run: Runner,
}
macro_rules! scenarios {
    ($table:ident; $output:ident,$args:ident,$results:ident; $($flag:literal, $min:literal, $max:literal => $body:block),* $(,)?) => {
        const $table: &[Scenario]=&[$(Scenario {flag:$flag,min:$min,max:$max,
            run: |$output,$args,$results| {let _=(&$output,&$args,&$results); $body}}),*];
    };
}
scenarios! {ENTRIES; output,args,results;
    "--task-lifecycle-native-only",0,0 => {task_lifecycle_native::run(output,results)},
    "--task-runtime-only",0,0 => {task_runtime::run(output,results)},
    "--full",0,0 => { super::run(output,results) },
    "--list-scenarios",0,0 => {
        fs::write(output.join("scenario-catalog.json"),serde_json::to_vec_pretty(&catalogue())?)?;
        results.push(json!({"workflow":"E2E场景目录","passed":true}));Ok(())
    },
    "--diagnostic-entry-only",0,0 => {diagnostic_entry::run(output,results).and_then(|()|diagnostic_entry::native(output,results))},
    "--network-proxy-only",0,0 => {
network_proxy::run(output, results)
    },
    "--video-batch-translation-only",0,0 => {
Library::open(output.join("batch-translate-source"))
            .and_then(|lib| video_batch_translation::run(&lib, results))
    },
    "--assistant-only",0,0 => {
Library::open(output.join("assistant-source"))
            .and_then(|lib| assistant::run(&lib, results))
    },
    "--ui-controller-only",0,0 => {
ui_controller::run(output, results)
    },
    "--flashcard-keys-only",0,0 => {
ui_controller::flashcard(output, results)
    },
    "--subtitle-workbench-only",0,0 => {
Library::open(output.join("subtitle-workbench-source")).and_then(|lib| {
            transcript::run(&lib, results)?;
            subtitle_workbench_native::run(output, results)
        })
    },
    "--chat-controller-only",0,0 => {
Library::open(output.join("chat-controller-source"))
            .and_then(|lib| chat_controller::run(&lib, results))
    },
    "--data-loader-only",0,0 => {
data_loader::run(output, results)
    },
    "--focus-timer-only",0,0 => {
focus_timer::run(output, results)
    },
    "--credential-mode-only",0,0 => {
Library::open(output.join("credential-mode-source"))
            .and_then(|lib| credential_mode::run(&lib, results))
    },
    "--runtime-stability-only",0,1 => {
runtime_stability::run(
            output,
            results,
            args.get(3)
                .and_then(|s| s.to_str())
                .unwrap_or("32")
                .parse()?,
        )
    },
    "--gpu-memory-only",0,0 => {
gpu_memory::run(output, results)
    },
    "--subtitle-selection-only",0,0 => {
let lib = Library::open(output.join(format!("subtitle-selection-{}", data::id())))?;
        subtitle_selection::run(&lib, results)
    },
    "--timer-chimes-audible",0,0 => {
let lib = Library::open(output.join(format!("timer-chimes-device-{}", data::id())))?;
        timer_chimes::audible(&lib, results)
    },
    "--timer-chimes-only",0,0 => {
let lib = Library::open(output.join(format!("timer-chimes-{}", data::id())))?;
        timer_chimes::run(&lib, results)
    },
    "--clip-marks-only",1,1 => {
let lib = Library::open(output.join(format!("clip-marks-{}", data::id())))?;
        clip_marks::run(
            &lib,
            &PathBuf::from(args.get(3).context("缺少媒体夹具")?),
            results,
        )
    },
    "--audio-merge-only",0,0 => {
audio_merge::run(output, results)
            .and_then(|()| model_navigation_native::run(output, results))
    },
    "--sentence-search-only",0,0 => {
sentence_search::run(output, results)
            .and_then(|()| sentence_search_native::run(output, results))
    },
    "--audio-repeat-only",0,0 => {
audio_repeat_native::run(output, results)
    },
    "--audio-boundary-only",0,0 => {
audio_boundary_native::run(output, results)
    },
    "--chatgpt-only",0,0 => {
#[cfg(debug_assertions)]
        {
            Library::open(output.join("chatgpt-source"))
                .and_then(|lib| chatgpt::run(&lib, results))
                .and_then(|_| ui_controller::chatgpt(output, results))
        }
        #[cfg(not(debug_assertions))]
        {
            anyhow::bail!("ChatGPT fixture requires a debug build")
        }
    },
    "--assistant-latency-only",0,0 => {
assistant_latency_native::run(output, results)
    },
    "--audio-remove-only",0,0 => {
audio_remove_native::run(output, results)
    },
    "--audio-background-only",0,0 => {
model_navigation_native::run(output, results)
    },
    "--audio-batch-only",0,0 => {
let lib = Library::open(output.join(format!("audio-batch-{}", data::id())))?;
        audio_batch::run(&lib, results)
    },
    "--command-import-only",0,0 => {
let lib = Library::open(output.join(format!("command-import-{}", data::id())))?;
        command_import::run(&lib, results)
    },
    "--http-only",2,2 => {
let lib = Library::open(output.join(format!("http-{}", data::id())))?;
        protocol::run(
            &lib,
            &PathBuf::from(args.get(3).context("缺少媒体夹具")?),
            &PathBuf::from(args.get(4).context("缺少图片夹具")?),
            results,
        )
    },
    "--backup-library-only",1,1 => {
let lib = Library::open(output.join(format!("backup-library-{}", data::id())))?;
        backup_library::run(
            &lib,
            &PathBuf::from(
                args.get(3)
                    .context("缺少媒体夹具：--backup-library-only <video.mp4>")?,
            ),
            results,
        )
    },
    "--video-render-only",0,0 => {
let lib = Library::open(output.join(format!("video-render-{}", data::id())))?;
        video_render::run(&lib, results)
    },
    "--transcript-only",0,0 => {
let lib = Library::open(output.join(format!("transcript-{}", data::id())))?;
        transcript::run(&lib, results)
    },
    "--ebook-only",0,1 => {
let lib = Library::open(output.join(format!("ebook-{}", data::id())))?;
        ebook::run(&lib, results).and_then(|_| {
            if let Some(path) = args.get(3) {
                ebook::external_file(&lib, &PathBuf::from(path), results)?;
            }
            Ok(())
        })
    },
    "--state-only",0,0 => {
let lib = Library::open(output.join(format!("state-{}", data::id())))?;
        chat_store::run(&lib, results)
            .and_then(|_| view_data::run(&lib, results))
            .and_then(|_| playback_state::run(&lib, results))
    },
    "--playback-session-only",0,0 => {
playback_session::run(output, results)
    },
    "--playback-session-native-only",0,0 => {
playback_session_native::run(output, results)
    },
    "--video-repeat-only",0,0 => {
let lib = Library::open(output.join(format!("video-repeat-{}", data::id())))?;
        video_repeat::run(&lib, results)
    },
    "--speaking-groups-only",0,0 => {speaking_groups::run(output, results)},
    "--writing-only",0,0 => {
let lib = Library::open(output.join(format!("writing-{}", data::id())))?;
        writing::run(&lib, results)
    },
    "--storage-config-only",0,0 => {
let lib = Library::open(output.join(format!("storage-config-{}", data::id())))?;
        backup_cancel::config_messages(&lib, results)
    },
    "--storage-errors-only",0,0 => {
let lib = Library::open(output.join(format!("storage-errors-{}", data::id())))?;
        storage_errors::run(&lib, results)
    },
    "--model-navigation-only",0,0 => {
let lib = Library::open(output.join(format!("model-navigation-{}", data::id())))?;
        model_progress::run(&lib, results)
            .and_then(|()| model_navigation_native::run(output, results))
    },
    "--asr-models-only",0,0 => {
asr_models::run(&Library::open(output.join("library"))?, results)
    },
    "--focus-mode-only",0,0 => {
focus_timer::run(output, results)
            .and_then(|()| focus_mode_native::run(output, results))
    },
    "--cloud-v3-only",0,0 => {
let lib = Library::open(output.join(format!("cloud-v3-{}", data::id())))?;
        cloud_v3::run(&lib, results)
    },
    "--cloud-storage-only",0,0 => {
let lib = Library::open(output.join(format!("cloud-v3-{}", data::id())))?;
        cloud_v3::run(&lib, results)
    },
    "--backup-progress-only",0,0 => {
let lib = Library::open(output.join(format!("backup-progress-{}", data::id())))?;
        backup_progress::run(&lib, results)
            .and_then(|()| backup_progress_native::run(&lib.root, results))
    },
    "--backup-background-only",0,0 => {
backup_progress_native::run(output, results)
    },
    "--reliability-only",0,0 => {
let lib = Library::open(output.join(format!("reliability-{}", data::id())))?;
        reliability(&lib, results)
    },
}
scenarios! {WORKERS; output,args,_results;
    "--asr-models-worker",0,0 => {asr_models::worker(output)},
    "--focus-timer-worker",0,0 => {focus_timer::worker(output)},
    "--runtime-stability-worker",0,1 => {runtime_stability::worker(output,args.get(3).and_then(|s|s.to_str()).unwrap_or("32").parse()?)},
    "--gpu-memory-worker",0,1 => {gpu_memory::worker(output,args.get(3).and_then(|s|s.to_str())==Some("--unbounded"))},
}
pub fn worker(output: &Path, args: &[OsString]) -> Option<Result<()>> {
    let flag = args.get(2)?.to_str()?;
    WORKERS
        .iter()
        .find(|entry| entry.flag == flag)
        .map(|entry| {
            let count = args.len().saturating_sub(3);
            ensure!(
                count >= entry.min && count <= entry.max,
                "内部worker参数数量错误：{}",
                entry.flag
            );
            (entry.run)(output, args, &mut vec![])
        })
}
pub fn catalogue() -> Value {
    json!({"schema_version":1,"workers":WORKERS.iter().map(|w|w.flag).collect::<Vec<_>>(),"scenarios":ENTRIES.iter().map(|entry|json!({"flag":entry.flag,
        "name":entry.flag.trim_start_matches("--"),"arguments":{"min":entry.min,"max":entry.max}})).collect::<Vec<_>>()})
}
pub fn execute(output: &Path, args: &[OsString], results: &mut Vec<Value>) -> Result<()> {
    let flag = match args.get(2) {
        Some(value) => value.to_str().context("E2E模式必须为UTF-8")?,
        None => "--full",
    };
    let entry = ENTRIES
        .iter()
        .find(|e| e.flag == flag)
        .with_context(|| format!("未知E2E模式：{flag}；使用 --list-scenarios 查看可用模式"))?;
    let extra = args.get(3..).unwrap_or_default();
    ensure!(
        extra.len() >= entry.min && extra.len() <= entry.max,
        "{} 需要 {}–{} 个附加参数，实际 {} 个",
        entry.flag,
        entry.min,
        entry.max,
        extra.len()
    );
    ensure!(
        extra.iter().all(|v| !v.to_string_lossy().starts_with("--")),
        "E2E附加参数不能是另一个模式选项"
    );
    (entry.run)(output, args, results)
}

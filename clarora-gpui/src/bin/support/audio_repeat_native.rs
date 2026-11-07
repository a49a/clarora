//! Actual AppView + libmpv + isolated SQLite/SRT; not physical keyboard input.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, time::Duration};
pub fn run(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let workflow = "音频EOF原生按钮复播/逐句循环/定位与切页保留/助手选区删除与侧栏隔离";
    if !cfg!(debug_assertions)
        || !cfg!(target_os = "macos")
        || std::env::var("CLARORA_GUI_MEMORY_CHECK").as_deref() != Ok("1")
    {
        results.push(json!({"workflow":workflow,"passed":true,"skipped":true,"reason":"Requires macOS graphical session and CLARORA_GUI_MEMORY_CHECK=1"}));
        return Ok(());
    }
    let out = root.join(format!("native-audio-repeat-{}", clarora_gpui::data::id()));
    fs::create_dir_all(&out)?;
    let exe = std::env::current_exe()?
        .parent()
        .context("executable directory")?
        .join("clarora-gpui");
    let mut command = Command::new(exe);
    command.arg("--audio-repeat-check").arg(&out);
    let status = super::native_diagnostic::run(command, &out, Duration::from_secs(60))?;
    let report_path = out.join("audio-repeat-report.json");
    let report: Value = serde_json::from_slice(&fs::read(&report_path)?)?;
    let cases = report["cases"]
        .as_array()
        .context("missing audio repeat cases")?;
    ensure!(
        status.success()
            && report["passed"] == true
            && cases.len() >= 64
            && [
                "native audio play button starts ordinary playback",
                "native audio pause button preserves a nonzero position",
                "native audio play resumes before EOF without rewinding",
                "ordinary audio reaches natural EOF before replay click",
                "native audio play at EOF rejects mismatched material",
                "audio play at EOF rejects an empty material context",
                "audio EOF invalid-context checks restore original subtitle fixture",
                "native audio play button rewinds and resumes after natural EOF",
                "ordinary audio reaches natural EOF again after replay",
                "native audio play button replays a second natural EOF",
                "close waiting sentence card and reclaim subtitle space",
                "existing current sentence action reopens the card",
                "close active sentence card without moving playback",
                "restore sentence card after closing an active cue",
                "mouse selection and backspace edit assistant question",
                "mouse selection deletes Chinese and emoji without splitting characters",
                "mouse selection across wrapped multilingual text deletes correctly",
                "mouse selection and forward delete edit assistant question",
                "mouse selection in assistant reply can be quoted",
                "select all and delete multiline multilingual question",
                "ordinary backspace removes a complete emoji",
                "uncovered subtitle selection still offers an AI quote",
                "subtitle drag into assistant finishes with an AI quote",
                "background editor drag into assistant finishes selection",
                "narrow assistant mouse selection and backspace",
                "double click selects the entire multilingual assistant question",
                "single click still places the assistant question caret",
                "typing replaces the entire double-click selection",
                "audio current card native double click selects a word without punctuation",
                "audio current card double clicked word floating copy writes exact OS clipboard text",
                "audio transcript native double click selects a word without punctuation",
                "audio transcript double clicked word keyboard copy writes exact OS clipboard text",
                "audio ordinary subtitle single click still seeks after double-click copy",
                "audio current card drag selects words without seeking or leaving triple repeat",
                "audio selection floating copy button writes exact OS clipboard text",
                "audio current card ask AI attaches only selected words without sending",
                "audio transcript selects across cues Chinese and emoji exactly",
                "audio subtitle native keyboard copy preserves bilingual cross-cue selection",
                "audio transcript ask AI preserves exact multilingual selection",
                "audio blind mode clears selection and offers no hidden subtitle quote",
                "audio subtitle single click still seeks and retains triple repeat",
                "visible audio copy full button copies complete bilingual transcript without changing playback",
                "audio copy full follows the newly selected audio",
                "stale audio full copy action cannot copy old material",
                "blind listening hides and blocks full audio copy",
            ]
            .iter()
            .all(|name| cases.iter().any(|case| case["case"] == *name)),
        "audio repeat failed: {}",
        report_path.display()
    );
    results.push(json!({"workflow":workflow,"passed":true,"report":report_path}));
    Ok(())
}

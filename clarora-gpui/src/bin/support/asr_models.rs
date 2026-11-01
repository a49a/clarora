//! Real HTTP installation, SQLite reopen and native/CLI process routing.
//! Stub model bytes/helpers verify integration, not large-model inference quality.
use anyhow::{Context, Result, ensure};
use clarora_gpui::{ai, data::Library, model_download, process, view_data};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command,
    thread,
};
const MODELS: [(&str, &str); 2] = [("medium.en", "en"), ("large-v3-turbo", "auto")];
pub fn helper() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    let log = PathBuf::from(std::env::var_os("CLARORA_ASR_ROUTE_LOG").context("route log")?);
    let native = Path::new(&args[0]).file_stem().and_then(|s| s.to_str()) == Some("clarora-asr");
    let row = if native {
        ensure!(
            args.len() == 5 && args[1] == "whisper-segmented",
            "invalid native arguments"
        );
        json!({"native":true,"model":args[3],"language":args[4]})
    } else {
        let option = |name: &str| -> Result<String> {
            let at = args
                .iter()
                .position(|s| s == name)
                .context("missing option")?;
            Ok(args.get(at + 1).context("missing option value")?.clone())
        };
        fs::write(
            format!("{}.srt", option("-of")?),
            "1\n00:00:00,000 --> 00:00:01,000\nRoute fixture transcript.\n",
        )?;
        json!({"native":false,"model":option("-m")?,"language":option("-l")?})
    };
    let mut file = fs::OpenOptions::new().create(true).append(true).open(log)?;
    writeln!(file, "{row}")?;
    println!(
        "{}",
        json!({"duration":1.0,"segments":[{"start":0.0,"end":1.0,"text":"Route fixture transcript."}]})
    );
    Ok(())
}
pub fn worker(root: &Path) -> Result<()> {
    let lib = Library::open(root.join("library"))?;
    let cancel = process::cancel_token();
    let audio = root.join("speech.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut wav = hound::WavWriter::create(&audio, spec)?;
    for _ in 0..16000 {
        wav.write_sample(0i16)?;
    }
    wav.finalize()?;
    let requests = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    let seen = requests.clone();
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let url = format!("http://{}", listener.local_addr()?);
    let server = thread::spawn(move || -> Result<()> {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
            let mut input = vec![];
            let mut buf = [0; 4096];
            loop {
                let n = stream.read(&mut buf)?;
                ensure!(n > 0, "empty HTTP request");
                input.extend_from_slice(&buf[..n]);
                if input.windows(4).any(|v| v == b"\r\n\r\n") {
                    break;
                }
            }
            let line = String::from_utf8_lossy(&input)
                .lines()
                .next()
                .unwrap_or_default()
                .to_string();
            seen.lock().unwrap().push(line);
            let payload = vec![42u8; 1024 * 1024 + 128];
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            )?;
            stream.write_all(&payload)?;
        }
        Ok(())
    });
    let mut cases = vec![];
    for (id, language) in MODELS {
        let model = ai::local_model(id).context("new model not selectable")?;
        ensure!(model.language == language, "wrong registry language");
        let files = ai::local_model_files(id)?;
        ensure!(
            files.len() == 1
                && files[0].name == format!("ggml-{id}.bin")
                && files[0].remote == "ggerganov/whisper.cpp/resolve/main",
            "wrong upstream weight path"
        );
        let mut events = vec![];
        model_download::download(
            id,
            &lib.root.join("models"),
            &files,
            &[&url],
            &cancel,
            &mut |p| events.push(p),
        )?;
        ensure!(
            events.iter().any(|p| p.phase == "文件完成"),
            "download completion absent"
        );
        ai::save_shared(
            &lib,
            ai::Config {
                engine: "local".into(),
                local_model: id.into(),
                ..Default::default()
            },
            "",
        )?;
        let reopened = Library::open(lib.root.clone())?;
        let saved = ai::load_config(&reopened)?;
        ensure!(
            saved.engine == "local" && saved.local_model == id,
            "saved model did not survive reopen"
        );
        let mut state = view_data::State::default();
        state.reload(&reopened, &[])?;
        ensure!(
            state.asr_models.contains(id),
            "downloaded model missing from UI cache"
        );
        let cues = ai::transcribe(&reopened, &audio, &cancel)?;
        ensure!(
            cues.len() == 1 && cues[0].text == "Route fixture transcript.",
            "new model dispatch failed"
        );
        ensure!(
            fs::read_dir(lib.root.join("recordings"))?.next().is_none(),
            "conversion temporary files leaked"
        );
        cases.push(json!({"case":"HTTP install/selection/SQLite reopen/UI availability/native or CLI dispatch","model":id,"passed":true,"progress":events}));
    }
    server
        .join()
        .map_err(|_| anyhow::anyhow!("model fixture panicked"))??;
    for (id, _) in MODELS {
        ensure!(
            requests.lock().unwrap().iter().any(|s| s.contains(&format!(
                "/ggerganov/whisper.cpp/resolve/main/ggml-{id}.bin"
            ))),
            "wrong actual model HTTP request"
        );
    }
    ensure!(
        ai::local_model_files("../medium.en").is_err(),
        "unknown model path accepted"
    );
    let log = PathBuf::from(std::env::var_os("CLARORA_ASR_ROUTE_LOG").context("route log")?);
    let calls = fs::read_to_string(&log)?
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    ensure!(calls.len() == 2, "unexpected helper invocation count");
    for ((id, lang), call) in MODELS.iter().zip(&calls) {
        ensure!(
            call["model"]
                .as_str()
                .is_some_and(|p| p.ends_with(&format!("ggml-{id}.bin")))
                && call["language"] == *lang,
            "wrong actual native/CLI model or language argument"
        );
    }
    // Missing weights and pre-cancel must not start the subprocess.
    fs::remove_file(lib.root.join("models/ggml-large-v3-turbo.bin"))?;
    ensure!(
        ai::transcribe(&lib, &audio, &cancel).is_err(),
        "missing weights accepted"
    );
    let stop = process::cancel_token();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    ensure!(
        ai::transcribe(&lib, &audio, &stop).is_err(),
        "pre-cancel accepted"
    );
    ensure!(
        fs::read_to_string(&log)?.lines().count() == 2,
        "missing/cancelled transcription invoked helper"
    );
    fs::write(
        root.join("asr-models-report.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":true,"cases":cases,"http":requests.lock().unwrap().clone(),"calls":calls,"real_large_weights":false,"fixture_helpers":true}),
        )?,
    )?;
    Ok(())
}
pub fn run(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    let root = lib
        .root
        .join(format!("asr-model-support-{}", clarora_gpui::data::id()));
    fs::create_dir_all(&root)?;
    let exe = std::env::current_exe()?;
    let ffmpeg = process::tool("ffmpeg")?;
    let mut reports = vec![];
    for native in [true, false] {
        let dir = root.join(if native { "native" } else { "cli" });
        fs::create_dir_all(&dir)?;
        let suffix = if cfg!(windows) { ".exe" } else { "" };
        let runner = dir.join(format!("e2e{suffix}"));
        fs::hard_link(&exe, &runner).or_else(|_| fs::copy(&exe, &runner).map(|_| ()))?;
        let shim = dir.join(format!(
            "{}{suffix}",
            if native { "clarora-asr" } else { "whisper-cli" }
        ));
        fs::hard_link(&exe, &shim).or_else(|_| fs::copy(&exe, &shim).map(|_| ()))?;
        // The child executable directory precedes PATH in process::tool; this
        // keeps fixtures isolated without changing the parent environment.
        #[cfg(unix)]
        std::os::unix::fs::symlink(&ffmpeg, dir.join("ffmpeg"))?;
        #[cfg(windows)]
        fs::copy(&ffmpeg, dir.join("ffmpeg.exe"))?;
        let report = dir.join("asr-models-report.json");
        let log = dir.join("route-calls.jsonl");
        let mut search = vec![
            dir.clone(),
            ffmpeg.parent().context("ffmpeg parent")?.to_path_buf(),
        ];
        search.extend(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .filter(|p| native || !p.join(format!("clarora-asr{suffix}")).is_file()),
        );
        let status = Command::new(&runner)
            .arg(&dir)
            .arg("--asr-models-worker")
            .env("CLARORA_ASR_ROUTE_LOG", &log)
            .env("PATH", std::env::join_paths(search)?)
            .stdout(fs::File::create(dir.join("stdout.log"))?)
            .stderr(fs::File::create(dir.join("stderr.log"))?)
            .status()?;
        ensure!(
            status.success(),
            "ASR model routing failed: {}",
            dir.display()
        );
        let value: Value = serde_json::from_slice(&fs::read(&report)?)?;
        ensure!(value["passed"] == true, "ASR model report failed");
        reports.push(report);
    }
    results.push(json!({"workflow":"medium.en/turbo：真实HTTP下载/SQLite重开/界面缓存/原生与CLI路由/缺失与取消保护","passed":true,"reports":reports,"real_large_model_inference":false}));
    Ok(())
}

//! One startup selection and one running probe. Normal application state owns no test fixtures.
use super::*;

#[derive(Clone, Copy, Debug)]
enum Kind {
    Assistant,
    AudioRemove,
    AudioRepeat,
    Playback,
    Tasks,
    Models,
    Backup,
    Sentences,
    Subtitles,
    #[cfg(target_os = "macos")]
    WritingPaste,
    SpeakingGroups,
    Focus,
    Controller,
    Quit,
}
struct Entry {
    flag: &'static str,
    name: &'static str,
    kind: Kind,
}
const ENTRIES: &[Entry] = &[
    Entry {
        flag: "--speaking-groups-check",
        name: "speaking-groups",
        kind: Kind::SpeakingGroups,
    },
    Entry {
        flag: "--task-lifecycle-check",
        name: "task-lifecycle",
        kind: Kind::Tasks,
    },
    Entry {
        flag: "--assistant-latency-check",
        name: "assistant-latency",
        kind: Kind::Assistant,
    },
    Entry {
        flag: "--audio-remove-check",
        name: "audio-remove",
        kind: Kind::AudioRemove,
    },
    Entry {
        flag: "--audio-repeat-check",
        name: "audio-repeat",
        kind: Kind::AudioRepeat,
    },
    Entry {
        flag: "--playback-session-check",
        name: "playback-session",
        kind: Kind::Playback,
    },
    Entry {
        flag: "--model-navigation-check",
        name: "model-navigation",
        kind: Kind::Models,
    },
    Entry {
        flag: "--backup-progress-check",
        name: "backup-progress",
        kind: Kind::Backup,
    },
    Entry {
        flag: "--sentence-search-check",
        name: "sentence-search",
        kind: Kind::Sentences,
    },
    Entry {
        flag: "--subtitle-workbench-check",
        name: "subtitle-workbench",
        kind: Kind::Subtitles,
    },
    Entry {
        flag: "--focus-mode-check",
        name: "focus-mode",
        kind: Kind::Focus,
    },
    #[cfg(target_os = "macos")]
    Entry {
        flag: "--writing-paste-check",
        name: "writing-paste",
        kind: Kind::WritingPaste,
    },
    Entry {
        flag: "--ui-controller-check",
        name: "ui-controller",
        kind: Kind::Controller,
    },
    Entry {
        flag: "--ui-controller-quit-check",
        name: "ui-controller-quit",
        kind: Kind::Quit,
    },
];

pub(super) struct Launch {
    entry: &'static Entry,
    out: PathBuf,
}
pub(super) struct Startup {
    pub(super) data_dir: Option<PathBuf>,
    pub(super) runtime: Option<PathBuf>,
    pub(super) catalogue: Option<PathBuf>,
    pub(super) diagnostic: Option<Launch>,
}
impl Startup {
    pub(super) fn parse(args: &[String]) -> Result<Self> {
        let mut startup = Self {
            data_dir: None,
            runtime: None,
            catalogue: None,
            diagnostic: None,
        };
        let mut i = 1;
        while i < args.len() {
            let flag = &args[i];
            if flag.starts_with("-psn_") {
                i += 1;
                continue;
            }
            let entry = ENTRIES.iter().find(|e| e.flag == flag);
            ensure!(
                entry.is_some()
                    || ["--data-dir", "--runtime-check", "--list-diagnostics"]
                        .contains(&flag.as_str()),
                "未知启动参数：{flag}"
            );
            let value = args
                .get(i + 1)
                .filter(|s| !s.is_empty() && !s.starts_with("--"))
                .with_context(|| format!("{flag} 缺少路径"))?;
            let path = PathBuf::from(value);
            match flag.as_str() {
                "--data-dir" => {
                    ensure!(startup.data_dir.is_none(), "--data-dir 不能重复");
                    startup.data_dir = Some(path);
                }
                "--runtime-check" => {
                    ensure!(startup.runtime.is_none(), "--runtime-check 不能重复");
                    startup.runtime = Some(path);
                }
                "--list-diagnostics" => {
                    ensure!(startup.catalogue.is_none(), "--list-diagnostics 不能重复");
                    startup.catalogue = Some(path);
                }
                _ => {
                    ensure!(startup.diagnostic.is_none(), "每次只能运行一个原生诊断");
                    startup.diagnostic = Some(Launch {
                        entry: entry.unwrap(),
                        out: path,
                    });
                }
            }
            i += 2;
        }
        ensure!(
            usize::from(startup.runtime.is_some())
                + usize::from(startup.catalogue.is_some())
                + usize::from(startup.diagnostic.is_some())
                <= 1,
            "运行环境检查、诊断目录和原生诊断不能同时运行"
        );
        if let Some(launch) = &startup.diagnostic {
            ensure!(
                cfg!(debug_assertions),
                "原生诊断仅支持调试构建：{}",
                launch.entry.flag
            );
            if let Some(data) = &startup.data_dir {
                ensure!(
                    std::path::absolute(data)? == std::path::absolute(launch.out.join("library"))?,
                    "原生诊断只能使用证据目录下的隔离资料库"
                );
            }
        }
        Ok(startup)
    }
    pub(super) fn catalogue() -> Value {
        json!({"schema_version":1,"debug_build":cfg!(debug_assertions),"diagnostics":ENTRIES.iter().map(|e|json!({
            "name":e.name,"flag":e.flag,"kind":format!("{:?}",e.kind),"available":cfg!(debug_assertions),
            "window":{"show":false,"focus":false,"activate":false},"fixture":"library","lifecycle_report":"diagnostic-run.json"
        })).collect::<Vec<_>>()})
    }
}

#[cfg(debug_assertions)]
mod running {
    use super::*;
    pub(crate) trait Probe {
        fn tick(
            &mut self,
            app: &mut AppView,
            window: &mut Window,
            cx: &mut Context<AppView>,
        ) -> Result<bool>;
        fn failed(&mut self, error: String) -> Result<()>;
        fn completed(&mut self) -> Result<()> {
            Ok(())
        }
        fn needs_layouts(&self) -> bool {
            false
        }
        fn cleanup(
            &mut self,
            _app: &mut AppView,
            _window: &mut Window,
            _cx: &mut Context<AppView>,
        ) {
        }
        fn closing(&mut self, _app: &mut AppView) -> Result<()> {
            anyhow::bail!("诊断尚未完成即关闭")
        }
        fn layouts_ready(&mut self, _result: Result<Vec<Value>, String>) {}
    }
    pub(crate) struct Active {
        entry: &'static Entry,
        out: PathBuf,
        probe: Box<dyn Probe>,
        started: Instant,
        polls: u64,
        fail_after: Option<u64>,
    }
    impl Launch {
        pub(crate) fn prepare(&self) -> Result<Library> {
            ensure!(
                !self.out.join("library").exists(),
                "诊断资料目录已存在，请使用新的证据目录：{}",
                self.out.display()
            );
            fs::create_dir_all(&self.out)?;
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.out.join(".diagnostic-started"))
                .context("诊断目录已被使用，请换一个证据目录")?;
            match self.entry.kind {
                Kind::Assistant => assistant_latency_probe::prepare(&self.out),
                Kind::AudioRemove => audio_remove_probe::prepare(&self.out),
                Kind::AudioRepeat => audio_repeat_probe::prepare(&self.out),
                Kind::Playback => playback_session_probe::prepare(&self.out),
                Kind::Tasks => task_lifecycle_probe::prepare(&self.out),
                Kind::Models => model_navigation_probe::prepare(&self.out),
                Kind::Backup => backup_progress_probe::prepare(&self.out),
                Kind::Sentences => sentence_search_probe::prepare(&self.out),
                Kind::Subtitles => subtitle_workbench_probe::prepare(&self.out),
                #[cfg(target_os = "macos")]
                Kind::WritingPaste => writing_ui::paste_probe::prepare(&self.out),
                Kind::SpeakingGroups => speaking_ui::probe::prepare(&self.out),
                Kind::Focus => Library::open(self.out.join("library")),
                Kind::Controller | Kind::Quit => ui_controller_probe::prepare(&self.out),
            }
        }
        pub(crate) fn start(&self) -> Active {
            let probe: Box<dyn Probe> = match self.entry.kind {
                Kind::Assistant => Box::new(assistant_latency_probe::Probe::new(&self.out)),
                Kind::AudioRemove => Box::new(audio_remove_probe::Probe::new(&self.out)),
                Kind::AudioRepeat => Box::new(audio_repeat_probe::Probe::new(&self.out)),
                Kind::Playback => Box::new(playback_session_probe::Probe::new(&self.out)),
                Kind::Tasks => Box::new(task_lifecycle_probe::Probe::new(&self.out)),
                Kind::Models => Box::new(model_navigation_probe::Probe::new(&self.out)),
                Kind::Backup => Box::new(backup_progress_probe::Probe::new(&self.out)),
                Kind::Sentences => Box::new(sentence_search_probe::Probe::new(&self.out)),
                Kind::Subtitles => Box::new(subtitle_workbench_probe::Probe::new(&self.out)),
                #[cfg(target_os = "macos")]
                Kind::WritingPaste => Box::new(writing_ui::paste_probe::PasteProbe::default()),
                Kind::SpeakingGroups => Box::new(speaking_ui::probe::Probe::default()),
                Kind::Focus => Box::new(focus_mode_probe::Probe::new(&self.out)),
                Kind::Controller | Kind::Quit => Box::new(
                    ui_controller_probe::Probe::new(&self.out)
                        .with_global_quit(matches!(self.entry.kind, Kind::Quit)),
                ),
            };
            Active {
                entry: self.entry,
                out: self.out.clone(),
                probe,
                started: Instant::now(),
                polls: 0,
                fail_after: std::env::var("CLARORA_DIAGNOSTIC_FAIL_AFTER_POLLS")
                    .ok()
                    .and_then(|v| v.parse().ok()),
            }
        }
    }
    impl Active {
        fn report(&self, result: &Result<()>) {
            let report = json!({"schema_version":1,"name":self.entry.name,"flag":self.entry.flag,
                "passed":result.is_ok(),"cleanup":true,"error":result.as_ref().err().map(|e|format!("{e:#}")),
                "seconds":self.started.elapsed().as_secs_f64(),"polls":self.polls,"fixture":self.out.join("library"),
                "window":{"show":false,"focus":false,"activate":false}});
            if let Err(error) = fs::write(
                self.out.join("diagnostic-run.json"),
                serde_json::to_vec_pretty(&report).expect("diagnostic report serializes"),
            ) {
                eprintln!("诊断生命周期报告写入失败：{error}");
            }
        }
        fn finish(
            &mut self,
            result: Result<()>,
            app: &mut AppView,
            window: &mut Window,
            cx: &mut Context<AppView>,
        ) {
            self.probe.cleanup(app, window, cx);
            let result = result.and_then(|()| self.probe.completed());
            let result = match result {
                Err(error) => self
                    .probe
                    .failed(format!("{error:#}"))
                    .and_then(|()| Err(error)),
                ok => ok,
            };
            self.report(&result);
        }
        pub(crate) fn needs_layouts(&self) -> bool {
            self.probe.needs_layouts()
        }
        pub(crate) fn layouts_ready(&mut self, result: Result<Vec<Value>, String>) {
            self.probe.layouts_ready(result);
        }
    }
    impl AppView {
        pub(crate) fn poll_diagnostic(
            &mut self,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) -> bool {
            let Some(mut active) = self.diagnostic.take() else {
                return false;
            };
            active.polls += 1;
            let result = if active.fail_after == Some(active.polls) {
                Err(anyhow::anyhow!("injected diagnostic failure"))
            } else {
                active.probe.tick(self, window, cx)
            };
            match result {
                Ok(false) => {
                    self.diagnostic = Some(active);
                    false
                }
                done => {
                    active.finish(done.map(|_| ()), self, window, cx);
                    window.remove_window();
                    true
                }
            }
        }
        pub(crate) fn diagnostic_closing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            if let Some(mut active) = self.diagnostic.take() {
                let result = active.probe.closing(self);
                active.finish(result, self, window, cx);
            }
        }
    }
}
#[cfg(debug_assertions)]
pub(super) use running::{Active, Probe};

//! The main window and floating importer share one captured command/target.
use crate::{
    data::{Library, Row, text},
    process::{self, Cancel},
    speaking::{self, Question},
};
use anyhow::{Context, Result, ensure};
use std::{collections::HashSet, path::PathBuf, sync::atomic::Ordering, time::Duration};

pub struct Request {
    command: String,
    group: String,
    name: String,
}
#[derive(Clone)]
pub struct Report {
    pub imported: usize,
    pub failed: usize,
    pub group_name: String,
    pub group_id: String,
    pub audios: Vec<Row>,
}
impl Report {
    pub fn rename(&mut self, lib: &Library, index: usize, name: &str) -> Result<()> {
        let audio = self.audios.get_mut(index).context("本次导入中没有该音频")?;
        lib.rename_audio(&text(audio, "id"), name, Some(&self.group_id))?;
        audio["name"] = name.trim().into();
        Ok(())
    }
    pub fn message(&self) -> String {
        format!(
            "已导入 {} 条至「{}」{}",
            self.imported,
            self.group_name,
            if self.failed > 0 {
                format!("，{} 条未导入（可能同名或文件不可读）", self.failed)
            } else {
                String::new()
            }
        )
    }
}
impl Request {
    pub fn group_name(&self) -> &str {
        &self.name
    }
    pub fn new(lib: &Library, group: &str, command: &str) -> Result<Self> {
        ensure!(!command.trim().is_empty(), "请先在设置中填写本机导入命令");
        let row = lib
            .audio_practice(group)?
            .context("请先新建或选择有效的练习组")?;
        Ok(Self {
            command: command.into(),
            group: group.into(),
            name: row.name,
        })
    }
    pub fn run(self, lib: &Library, cancel: &Cancel) -> Result<Report> {
        // Revalidate before launching a process, including independently deleted groups.
        ensure!(
            lib.audio_practice(&self.group)?.is_some(),
            "目标练习组已不存在，请重新选择"
        );
        let paths = command_paths(self.command, cancel)?;
        let mut report = Report {
            imported: 0,
            failed: 0,
            group_name: self.name,
            group_id: self.group.clone(),
            audios: Vec::new(),
        };
        let mut last_error = None;
        for path in paths {
            ensure!(!cancel.load(Ordering::SeqCst), "已取消");
            if !path.is_file() {
                continue;
            }
            match lib.import_audio_record(&self.group, &path) {
                Ok(row) => {
                    report.imported += 1;
                    report.audios.push(row.into_row());
                }
                Err(error) => {
                    report.failed += 1;
                    last_error = Some(error);
                }
            }
        }
        ensure!(
            report.imported > 0,
            "{}",
            last_error
                .map(|error| format!("音频未导入：{error:#}"))
                .unwrap_or_else(|| {
                    "命令没有返回可导入的文件。请让命令每行输出一个本机音频文件的完整路径。".into()
                })
        );
        Ok(report)
    }
}

fn command_paths(command: String, cancel: &Cancel) -> Result<Vec<PathBuf>> {
    let (exe, args) = if cfg!(windows) {
        (
            PathBuf::from("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"),
            vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                format!(
                    "$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); {}",
                    command
                ),
            ],
        )
    } else {
        (PathBuf::from("/bin/zsh"), vec!["-c".into(), command])
    };
    let output = process::run(&exe, &args, cancel, Duration::from_secs(60))?;

    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect())
}

pub struct SpeakingRequest {
    command: String,
    group: String,
    name: String,
    question: Option<Question>,
}

pub struct SpeakingReport {
    pub group_name: String,
    pub questions: Vec<Question>,
    pub attached: bool,
    pub failed: usize,
}

impl SpeakingRequest {
    pub fn new(
        lib: &Library,
        group: &str,
        command: &str,
        question: Option<Question>,
    ) -> Result<Self> {
        ensure!(!command.trim().is_empty(), "请先在设置中填写本机导入命令");
        let group = speaking::list_groups(lib)?
            .into_iter()
            .find(|item| item.id == group)
            .context("请先新建或选择有效的口语分组")?;
        ensure!(
            question
                .as_ref()
                .is_none_or(|question| question.group_id == group.id),
            "当前题目不在目标口语组"
        );
        Ok(Self {
            command: command.into(),
            group: group.id,
            name: group.name,
            question,
        })
    }

    pub fn group_name(&self) -> &str {
        &self.name
    }

    pub fn run(self, lib: &Library, cancel: &Cancel) -> Result<SpeakingReport> {
        ensure!(
            speaking::list_groups(lib)?
                .iter()
                .any(|group| group.id == self.group),
            "目标口语分组已不存在，请重新选择"
        );
        let mut seen = HashSet::new();
        let paths: Vec<_> = command_paths(self.command, cancel)?
            .into_iter()
            .filter(|path| path.is_file() && seen.insert(path.clone()))
            .collect();
        let attached = self.question.is_some();
        if attached {
            ensure!(
                paths.len() <= 1,
                "命令返回了多条音频，请使用「命令批量导入」或让命令只返回一个文件"
            );
        }
        let mut report = SpeakingReport {
            group_name: self.name,
            questions: Vec::new(),
            attached,
            failed: 0,
        };
        let mut last_error = None;
        for path in paths {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            let question = self
                .question
                .clone()
                .unwrap_or_else(|| Question::draft(self.group.clone()));
            match speaking::import_prompt_audio(lib, question, path) {
                Ok(saved) => report.questions.push(saved),
                Err(error) => {
                    report.failed += 1;
                    last_error = Some(error);
                }
            }
        }
        if report.questions.is_empty() {
            ensure!(!cancel.load(Ordering::SeqCst), "已取消");
            if let Some(error) = last_error {
                return Err(error.context("口语题目音频未导入"));
            }
            anyhow::bail!("命令没有返回可导入的文件。请让命令每行输出一个本机音频文件的完整路径。");
        }
        Ok(report)
    }
}

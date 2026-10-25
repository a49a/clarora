//! The main window and floating importer share one captured command/target.
use crate::{
    data::{Library, Row, text},
    process::{self, Cancel},
};
use anyhow::{Context, Result, ensure};
use std::{path::PathBuf, sync::atomic::Ordering, time::Duration};

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
        crate::audio_library::rename_scoped(
            lib,
            "listening_audios",
            &text(audio, "id"),
            name,
            Some(&self.group_id),
        )?;
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
            .list("listening_practices")?
            .into_iter()
            .find(|row| text(row, "id") == group)
            .context("请先新建或选择有效的练习组")?;
        Ok(Self {
            command: command.into(),
            group: group.into(),
            name: text(&row, "name"),
        })
    }
    pub fn run(self, lib: &Library, cancel: &Cancel) -> Result<Report> {
        // Revalidate before launching a process, including independently deleted groups.
        ensure!(
            lib.list("listening_practices")?
                .iter()
                .any(|row| text(row, "id") == self.group),
            "目标练习组已不存在，请重新选择"
        );
        let (exe, args) = if cfg!(windows) {
            (
                PathBuf::from("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"),
                vec![
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-Command".into(),
                    format!(
                        "$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); {}",
                        self.command
                    ),
                ],
            )
        } else {
            (PathBuf::from("/bin/zsh"), vec!["-c".into(), self.command])
        };
        let output = process::run(&exe, &args, cancel, Duration::from_secs(60))?;
        let mut report = Report {
            imported: 0,
            failed: 0,
            group_name: self.name,
            group_id: self.group.clone(),
            audios: Vec::new(),
        };
        let mut last_error = None;
        for path in output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(PathBuf::from)
        {
            ensure!(!cancel.load(Ordering::SeqCst), "已取消");
            if !path.is_file() {
                continue;
            }
            match lib.import_audio_record(&self.group, &path) {
                Ok(row) => {
                    report.imported += 1;
                    report.audios.push(row);
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

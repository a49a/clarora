//! Serial, resumable-by-skipping subtitle work bound to a captured audio group.
use crate::{
    ai,
    data::{Library, Row, id, text},
    process::Cancel,
    subtitles::{self, Cue},
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::Duration,
};

#[derive(Clone, Copy, Serialize)]
pub enum Mode {
    English,
    Chinese,
}
impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::English => "整组生成英文字幕",
            Self::Chinese => "整组翻译中文字幕",
        }
    }
}
pub struct Plan {
    group: String,
    pub group_name: String,
    mode: Mode,
    rows: Vec<Row>,
}
#[derive(Default, Serialize)]
pub struct Report {
    pub total: usize,
    pub completed: usize,
    pub english: usize,
    pub translated: usize,
    pub skipped: usize,
    pub failed: Vec<(String, String)>,
    pub cancelled: bool,
}
impl Report {
    pub fn message(&self) -> String {
        let mut message = format!(
            "{} · 已处理 {}/{} 条，生成英文 {} 条，翻译中文 {} 条，跳过 {} 条，失败 {} 条",
            if self.cancelled {
                "已取消，完成结果已保留"
            } else {
                "整组处理结束"
            },
            self.completed,
            self.total,
            self.english,
            self.translated,
            self.skipped,
            self.failed.len()
        );
        for (name, error) in self.failed.iter().take(10) {
            message.push_str(&format!("\n{name}：{error}"));
        }
        if self.failed.len() > 10 {
            message.push_str("\n其余失败项可在再次整组处理时重试；完成项会自动跳过。");
        }
        message
    }
}
fn check(cancel: &Cancel) -> Result<()> {
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    Ok(())
}
fn chinese(s: &str) -> bool {
    s.chars().any(|c| ('\u{3400}'..='\u{9fff}').contains(&c))
}
fn english(s: &str) -> bool {
    !chinese(s) && s.chars().any(|c| c.is_ascii_alphabetic())
}
fn has_english(cues: &[Cue]) -> bool {
    cues.iter().any(|c| c.text.lines().any(english))
}
fn contents(uri: &str) -> Result<Option<Vec<u8>>> {
    if uri.is_empty() {
        return Ok(None);
    }
    match fs::metadata(uri) {
        Ok(meta) => {
            ensure!(
                meta.is_file() && meta.len() <= 16 * 1024 * 1024,
                "字幕文件无效或超过16 MiB"
            );
            Ok(Some(fs::read(uri)?))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
struct Input {
    row: Row,
    bytes: Option<Vec<u8>>,
    cues: Vec<Cue>,
}
impl Input {
    fn load(lib: &Library, group: &str, key: &str) -> Result<Self> {
        let row = lib
            .rows(
                "SELECT * FROM listening_audios WHERE id=? AND practice_id=?",
                &[key.to_string().into(), group.to_string().into()],
            )?
            .pop()
            .context("音频已移动或删除")?;
        let uri = text(&row, "subtitle_uri");
        let bytes = contents(&uri)?;
        let body = bytes
            .as_ref()
            .map(|v| std::str::from_utf8(v))
            .transpose()
            .context("字幕不是UTF-8")?
            .unwrap_or("");
        let timed = Path::new(&uri)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("srt") || e.eq_ignore_ascii_case("vtt"));
        let cues = if timed && !body.contains("-->") {
            vec![]
        } else {
            subtitles::parse(body)
        };
        Ok(Self { row, bytes, cues })
    }
    fn save(&self, lib: &Library, group: &str, cues: &[Cue], cancel: &Cancel) -> Result<()> {
        check(cancel)?;
        ensure!(!cues.is_empty(), "没有有效字幕");
        let path = lib
            .root
            .join("media")
            .join(format!("audio-batch-{}.srt", id()));
        let result = (|| -> Result<()> {
            let mut output = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)?;
            output.write_all(subtitles::srt(cues).as_bytes())?;
            output.sync_all()?;
            let mut db = lib.db()?;
            db.busy_timeout(Duration::from_millis(100))?;
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            check(cancel)?;
            ensure!(
                contents(&text(&self.row, "subtitle_uri"))? == self.bytes,
                "原字幕已改变，未覆盖较新的内容"
            );
            ensure!(tx.execute("UPDATE listening_audios SET subtitle_uri=? WHERE id=? AND practice_id=? AND audio_uri=? AND subtitle_uri=?",rusqlite::params![path.to_string_lossy(),text(&self.row,"id"),group,text(&self.row,"audio_uri"),text(&self.row,"subtitle_uri")])?==1,"音频或字幕已改变，未覆盖较新的内容");
            check(cancel)?;
            tx.commit()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&path);
        }
        result
    }
}
impl Plan {
    pub fn new(lib: &Library, group: &str, mode: Mode) -> Result<Self> {
        let group_name = lib
            .rows(
                "SELECT name FROM listening_practices WHERE id=?",
                &[group.to_string().into()],
            )?
            .pop()
            .map(|r| text(&r, "name"))
            .context("请先选择练习组")?;
        let rows = lib.rows(
            "SELECT * FROM listening_audios WHERE practice_id=? ORDER BY created_at,id",
            &[group.to_string().into()],
        )?;
        ensure!(!rows.is_empty(), "当前练习组没有音频");
        Ok(Self {
            group: group.into(),
            group_name,
            mode,
            rows,
        })
    }
    pub fn run(&self, lib: &Library, cancel: &Cancel, mut progress: impl FnMut(&str)) -> Report {
        let mut report = Report {
            total: self.rows.len(),
            ..Default::default()
        };
        for row in &self.rows {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            let name = text(row, "name");
            let done = report.completed;
            let total = report.total;
            let mut stage = |s: &str| {
                progress(&format!(
                    "正在处理第 {}/{} 条 · {name}\n{s}",
                    done + 1,
                    total
                ))
            };
            let result = self.process(lib, &text(row, "id"), cancel, &mut report, &mut stage);
            match result {
                Ok(false) => report.skipped += 1,
                Ok(true) => {}
                Err(_) if cancel.load(Ordering::SeqCst) => break,
                Err(e) => report.failed.push((name, format!("{e:#}"))),
            }
            report.completed += 1;
            progress(&format!(
                "已处理 {}/{} · 英文 {} · 中文 {} · 跳过 {} · 失败 {}",
                report.completed,
                report.total,
                report.english,
                report.translated,
                report.skipped,
                report.failed.len()
            ));
        }
        report.cancelled = cancel.load(Ordering::SeqCst) && report.completed < report.total;
        report
    }
    fn process(
        &self,
        lib: &Library,
        key: &str,
        cancel: &Cancel,
        report: &mut Report,
        progress: &mut impl FnMut(&str),
    ) -> Result<bool> {
        let mut input = Input::load(lib, &self.group, key)?;
        check(cancel)?;
        match self.mode {
            Mode::English if has_english(&input.cues) => {
                progress("已有英文字幕，跳过");
                return Ok(false);
            }
            Mode::Chinese
                if !input.cues.is_empty() && input.cues.iter().all(|c| chinese(&c.text)) =>
            {
                progress("已有中文字幕，跳过");
                return Ok(false);
            }
            _ => {}
        }
        let mut changed = false;
        if !has_english(&input.cues) {
            progress("正在识别音频，补生成英文字幕");
            let path = PathBuf::from(text(&input.row, "audio_uri"));
            let meta = fs::metadata(&path).context("音频文件不存在或不可读")?;
            let stamp = (meta.len(), meta.modified()?);
            let mut cues = ai::transcribe(lib, &path, cancel)?;
            check(cancel)?;
            for cue in &mut cues {
                cue.text = cue
                    .text
                    .lines()
                    .filter(|s| !chinese(s))
                    .collect::<Vec<_>>()
                    .join("\n");
            }
            cues.retain(|c| !c.text.trim().is_empty());
            ensure!(has_english(&cues), "未识别出英文，请确认音频语言及转写模型");
            let after = fs::metadata(&path)?;
            ensure!(
                (after.len(), after.modified()?) == stamp,
                "音频文件已改变，请重新处理"
            );
            let cues = if input.cues.is_empty() {
                cues
            } else {
                subtitles::merge(&input.cues, &cues, false)?
            };
            input.save(lib, &self.group, &cues, cancel)?;
            report.english += 1;
            changed = true;
            input = Input::load(lib, &self.group, key)?;
        }
        if matches!(self.mode, Mode::English) {
            return Ok(changed);
        }
        let missing = input
            .cues
            .iter()
            .enumerate()
            .filter(|(_, c)| !chinese(&c.text) && c.text.lines().any(english))
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return Ok(changed);
        }
        let mut output = input.cues.clone();
        let mut offset = 0;
        while offset < missing.len() {
            check(cancel)?;
            let mut end = offset;
            let mut chars = 0;
            while end < missing.len() && end - offset < 32 {
                let count = input.cues[missing[end]].text.chars().count();
                ensure!(count <= 8000, "单句字幕超过8000字，请先拆分");
                if end > offset && chars + count > 8000 {
                    break;
                }
                chars += count;
                end += 1;
            }
            progress(&format!(
                "正在补齐中文字幕 · {offset}/{} 句，已有中文的句子跳过",
                missing.len()
            ));
            let source = missing[offset..end]
                .iter()
                .map(|&i| input.cues[i].clone())
                .collect::<Vec<_>>();
            let translated = ai::translate(lib, &source, cancel)?;
            for (&index, cue) in missing[offset..end].iter().zip(translated) {
                let chinese = cue
                    .text
                    .lines()
                    .filter(|s| chinese(s))
                    .collect::<Vec<_>>()
                    .join("\n");
                ensure!(
                    !chinese.trim().is_empty(),
                    "模型未返回有效中文，原字幕未覆盖"
                );
                output[index].text = format!("{}\n{}", input.cues[index].text, chinese);
            }
            offset = end;
        }
        check(cancel)?;
        progress("正在保存双语字幕");
        input.save(lib, &self.group, &output, cancel)?;
        report.translated += 1;
        Ok(true)
    }
}

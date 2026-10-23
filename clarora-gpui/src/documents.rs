use crate::data::{Library, id, now};
use anyhow::{Context, Result, ensure};
use pdfium_render::prelude::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path};

pub struct PdfPage {
    pub image: std::sync::Arc<gpui::RenderImage>,
    pub page: u16,
    pub count: u16,
    pub bookmarks: Vec<(String, u16)>,
    pub text: String,
}

pub fn load_pdf_page(lib: &Library, path: &Path, page: u16, width: i32) -> Result<PdfPage> {
    let (rendered, count, bookmarks) = render_pdf(lib, path, page, width)?;
    let page = page.min(count - 1);
    let text = pdf_text(path, page)?;
    // GPUI's directly supplied RenderImage expects BGRA pixels. Decode before
    // publishing the page so layout never depends on a later file-image load.
    let mut pixels = image::open(rendered)?.into_rgba8();
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Ok(PdfPage {
        image: std::sync::Arc::new(gpui::RenderImage::new(vec![image::Frame::new(pixels)])),
        page,
        count,
        bookmarks,
        text,
    })
}
fn pdfium() -> Result<Pdfium> {
    if let Some(path) = std::env::var_os("PDFIUM_LIB_PATH").filter(|_| !crate::runtime::packaged())
    {
        return Ok(Pdfium::new(Pdfium::bind_to_library(
            Pdfium::pdfium_platform_library_name_at_path(&std::path::PathBuf::from(path)),
        )?));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            if let Ok(bindings) =
                Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(parent))
            {
                return Ok(Pdfium::new(bindings));
            }
        }
    }
    ensure!(
        !crate::runtime::packaged(),
        "安装包缺少或无法加载 PDFium，请重新安装完整版本"
    );
    Ok(Pdfium::new(Pdfium::bind_to_system_library().context(
        "未找到 pdfium 动态库，请安装到程序旁或设置 PDFIUM_LIB_PATH",
    )?))
}
pub fn check_pdf_runtime() -> Result<()> {
    let _ = pdfium()?;
    Ok(())
}
pub fn pdf_text(path: &Path, page: u16) -> Result<String> {
    let pdf = pdfium()?;
    let doc = pdf.load_pdf_from_file(path, None)?;
    let count = doc.pages().len();
    ensure!(count > 0, "PDF 没有页面");
    Ok(doc.pages().get(page.min(count - 1))?.text()?.all())
}
pub fn render_pdf(
    lib: &Library,
    path: &Path,
    page: u16,
    width: i32,
) -> Result<(String, u16, Vec<(String, u16)>)> {
    let pdf = pdfium()?;
    let doc = pdf.load_pdf_from_file(path, None)?;
    let count = doc.pages().len();
    ensure!(count > 0, "PDF 没有页面");
    let index = page.min(count - 1);
    let pdf_page = doc.pages().get(index)?;
    let bitmap = pdf_page
        .render_with_config(&PdfRenderConfig::new().set_target_width(width.clamp(300, 2400)))?;
    let identity = format!(
        "{}:{:?}:{index}:{width}",
        path.display(),
        fs::metadata(path)?.modified()?
    );
    let key = hex::encode(Sha256::digest(identity.as_bytes()));
    let out = lib.root.join("pdf").join(format!("{key}.png"));
    bitmap.as_image().save(&out)?;
    let bookmarks = doc
        .bookmarks()
        .iter()
        .filter_map(|b| Some((b.title()?, b.destination()?.page_index().ok()?)))
        .collect();
    Ok((out.to_string_lossy().into(), count, bookmarks))
}
fn plain(raw: &str) -> String {
    let raw = regex::Regex::new(r"(?is)<(script|style)\b[^>]*>.*?</(?:script|style)>")
        .unwrap()
        .replace_all(raw, "");
    let raw = regex::Regex::new(r"(?i)\[sound:[^\]]*\]|<(?:img|audio|video)\b[^>]*>")
        .unwrap()
        .replace_all(&raw, "");
    let raw = regex::Regex::new(r"(?i)<br\s*/?>|</(?:p|div|li|tr|h[1-6])>")
        .unwrap()
        .replace_all(&raw, "\n");
    let raw = regex::Regex::new(r"<[^>]*>")
        .unwrap()
        .replace_all(&raw, " ")
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    regex::Regex::new(r"&#(x[0-9a-fA-F]+|[0-9]+);")
        .unwrap()
        .replace_all(&raw, |cap: &regex::Captures| {
            let raw = &cap[1];
            let number = if let Some(hex) = raw.strip_prefix('x') {
                u32::from_str_radix(hex, 16).ok()
            } else {
                raw.parse::<u32>().ok()
            };
            number
                .and_then(char::from_u32)
                .map(|c| c.to_string())
                .unwrap_or_default()
        })
        .lines()
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
fn cloze(raw: &str) -> String {
    regex::Regex::new(r"\{\{c\d+::(.*?)(?:::(.*?))?\}\}")
        .unwrap()
        .replace_all(raw, |cap: &regex::Captures| {
            if let Some(hint) = cap.get(2) {
                format!("{}({})", &cap[1], hint.as_str())
            } else {
                cap[1].into()
            }
        })
        .to_string()
}
fn indexes(names: &[String]) -> Option<(usize, usize)> {
    if names.len() < 2 {
        return None;
    }
    let front = names.iter().position(|s| {
        ["front", "frontside", "正面", "前面"].contains(&s.trim().to_lowercase().as_str())
    });
    let back = names
        .iter()
        .position(|s| ["back", "背面", "后面", "答案"].contains(&s.trim().to_lowercase().as_str()));
    Some(match (front, back) {
        (Some(a), Some(b)) if a != b => (a, b),
        _ => (0, 1),
    })
}
fn anki_text(body: &str) -> Result<Vec<(String, String)>> {
    let content = body.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut headers = std::collections::HashMap::new();
    let lines = content.lines().collect::<Vec<_>>();
    let mut start = 0;
    while start < lines.len() && (lines[start].trim().is_empty() || lines[start].starts_with('#')) {
        if let Some((key, value)) = lines[start]
            .strip_prefix('#')
            .and_then(|s| s.split_once(':'))
        {
            headers.insert(key.trim().to_lowercase(), value.to_string());
        }
        start += 1;
    }
    let sep = match headers
        .get("separator")
        .map(|s| s.trim().to_lowercase())
        .as_deref()
    {
        None | Some("tab") => "\t",
        Some("comma") => ",",
        Some("semicolon") => ";",
        Some("space") => " ",
        Some("pipe") => "|",
        Some("colon") => ":",
        Some(_) => headers["separator"].as_str(),
    };
    ensure!(
        ["\t", ",", ";", " ", "|", ":"].contains(&sep),
        "Anki 分隔符无效"
    );
    let sep = sep.chars().next().unwrap();
    let html = headers
        .get("html")
        .map(|s| s.trim().to_lowercase())
        .unwrap_or("false".into());
    ensure!(
        ["true", "false"].contains(&html.as_str()),
        "Anki html 标记无效"
    );
    let body = lines[start..].join("\n");
    let mut chars = body.chars().peekable();
    let mut row = vec![];
    let mut field = String::new();
    let mut rows = vec![];
    let mut quoted = false;
    let mut closed = false;
    loop {
        let c = chars.next();
        if quoted {
            let c = c.context("Anki 文本有未闭合双引号，未导入")?;
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                    closed = true;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        if c == Some('#') && row.is_empty() && field.is_empty() && !closed {
            while chars.peek().is_some_and(|c| *c != '\n') {
                chars.next();
            }
            continue;
        }
        if c == Some(sep) || c == Some('\n') || c.is_none() {
            row.push(std::mem::take(&mut field));
            closed = false;
            if c != Some(sep) {
                if row.iter().any(|s| !s.trim().is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            if c.is_none() {
                break;
            }
        } else if closed {
            anyhow::bail!("Anki 双引号后应为分隔符或换行")
        } else if c == Some('"') && field.is_empty() {
            quoted = true
        } else {
            field.push(c.unwrap());
        }
    }
    if rows.is_empty() {
        return Ok(vec![]);
    }
    let names = headers
        .get("columns")
        .map(|s| s.split(sep).map(String::from).collect::<Vec<_>>());
    let width = names.as_ref().map_or(rows[0].len(), Vec::len);
    let mut special = std::collections::HashSet::new();
    for key in [
        "tags column",
        "deck column",
        "notetype column",
        "guid column",
    ] {
        if let Some(value) = headers.get(key) {
            let n = value.trim().parse::<usize>()?;
            ensure!(
                n > 0 && n <= width && special.insert(n - 1),
                "Anki 特殊列无效或重复"
            );
        }
    }
    let ordinary = (0..width)
        .filter(|n| !special.contains(n))
        .collect::<Vec<_>>();
    let labels = ordinary
        .iter()
        .map(|n| names.as_ref().map(|v| v[*n].clone()).unwrap_or_default())
        .collect::<Vec<_>>();
    let Some((front, back)) = indexes(&labels) else {
        return Ok(vec![]);
    };
    let mut pairs = vec![];
    for row in rows {
        ensure!(row.len() == width, "Anki 记录列数不一致，未导入");
        let clean = |raw: &str| {
            if html == "true" {
                plain(&cloze(raw))
            } else {
                cloze(raw).trim().into()
            }
        };
        let a = clean(&row[ordinary[front]]);
        let b = clean(&row[ordinary[back]]);
        if !a.is_empty() && !b.is_empty() {
            pairs.push((a, b));
        }
    }
    Ok(pairs)
}
fn import_pairs(lib: &Library, pairs: Vec<(String, String)>) -> Result<usize> {
    let mut db = lib.db()?;
    let tx = db.transaction()?;
    let mut count = 0;
    for (front, back) in pairs {
        count+=tx.execute("INSERT INTO words(word,meaning,source) VALUES(?,?,'anki') ON CONFLICT(word) DO UPDATE SET meaning=excluded.meaning",rusqlite::params![front,back])?;
    }
    tx.commit()?;
    Ok(count)
}
pub fn import_anki(lib: &Library, path: &Path) -> Result<usize> {
    if [Some("txt"), Some("tsv")].contains(&path.extension().and_then(|s| s.to_str())) {
        return import_pairs(lib, anki_text(&fs::read_to_string(path)?)?);
    }
    let mut zip = zip::ZipArchive::new(fs::File::open(path)?)?;
    ensure!(
        zip.index_for_name("collection.anki21b").is_none(),
        "新版 Anki 压缩格式请在导出时勾选支持旧版本"
    );
    let db_name = ["collection.anki21", "collection.anki2"]
        .into_iter()
        .find(|n| zip.index_for_name(n).is_some())
        .context("卡包缺少 Anki 数据库")?;
    let mut bytes = vec![];
    let entry = zip.by_name(db_name)?;
    ensure!(entry.size() <= 512 * 1024 * 1024, "Anki 数据库过大");
    entry.take(512 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    let tmp = lib.root.join(format!("anki-{}.db", id()));
    fs::write(&tmp, &bytes)?;
    let result = (|| {
        let db = rusqlite::Connection::open_with_flags(
            &tmp,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let models: String = db.query_row("SELECT models FROM col LIMIT 1", [], |r| r.get(0))?;
        let models: Value = serde_json::from_str(&models)?;
        let mut stmt = db.prepare("SELECT CAST(mid AS TEXT),flds FROM notes")?;
        let fields = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut pairs = vec![];
        for (mid, field) in fields {
            let mut flds = models[&mid]["flds"].as_array().cloned().unwrap_or_default();
            flds.sort_by_key(|f| f["ord"].as_i64().unwrap_or(0));
            let names = flds
                .iter()
                .map(|f| f["name"].as_str().unwrap_or("").to_string())
                .collect::<Vec<_>>();
            let Some((a, b)) = indexes(&names) else {
                continue;
            };
            let parts = field.split('\u{1f}').collect::<Vec<_>>();
            let front = plain(&cloze(parts.get(a).copied().unwrap_or("")));
            let back = plain(&cloze(parts.get(b).copied().unwrap_or("")));
            if !front.is_empty() && !back.is_empty() {
                pairs.push((front, back));
            }
        }
        import_pairs(lib, pairs)
    })();
    let _ = fs::remove_file(tmp);
    result
}

pub fn save_sentence(lib: &Library, text: &str, translation: &str) -> Result<()> {
    ensure!(!text.trim().is_empty(), "原文不能为空");
    lib.put(
        "sentence_cards",
        json!({"id":id(),"text":text,"translation":translation,"notes":"","created_at":now()}),
    )
}
pub fn records(lib: &Library, kind: &str) -> Result<Vec<Value>> {
    let db = lib.db()?;
    let mut statement = db.prepare(
        "SELECT payload FROM ai_records WHERE kind=?1 ORDER BY created_at ASC,rowid ASC",
    )?;
    let rows = statement
        .query_map([kind], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|payload| serde_json::from_str(&payload).ok())
        .collect())
}

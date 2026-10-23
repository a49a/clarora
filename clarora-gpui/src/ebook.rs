//! Offline reflow reader for EPUB 2/3 and Unicode TXT. No HTML code is executed.
use crate::{data::Library, process::Cancel};
use anyhow::{Context, Result, ensure};
use quick_xml::{Reader, events::Event};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
};

const SOURCE_LIMIT: u64 = 128 * 1024 * 1024;
const MEMBER_LIMIT: u64 = 16 * 1024 * 1024;
const EXPANDED_LIMIT: u64 = 256 * 1024 * 1024;
const PAGE_CHARS: usize = 6000;

fn cancelled(cancel: &Cancel) -> Result<()> {
    ensure!(!cancel.load(Ordering::SeqCst), "已取消");
    Ok(())
}
pub fn supported(path: &Path) -> bool {
    extension(path).is_some_and(|s| ["pdf", "epub", "txt"].contains(&s.as_str()))
}
pub fn display_name(path: &Path) -> String {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    stem.rsplit_once('-')
        .filter(|(_, hash)| {
            [16, 32, 64].contains(&hash.len()) && hash.bytes().all(|b| b.is_ascii_hexdigit())
        })
        .map(|(name, _)| name)
        .unwrap_or(&stem)
        .to_string()
}
pub fn is_ebook(path: &Path) -> bool {
    extension(path).is_some_and(|s| ["epub", "txt"].contains(&s.as_str()))
}
fn extension(path: &Path) -> Option<String> {
    path.extension()?.to_str().map(str::to_ascii_lowercase)
}

#[derive(Clone, Debug)]
pub struct Block {
    pub text: String,
    pub heading: u8,
    pub image: Option<String>,
    anchors: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct Page {
    pub title: String,
    pub blocks: Vec<Block>,
}
impl Page {
    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Navigation {
    pub label: String,
    pub page: usize,
}
#[derive(Clone)]
pub struct Book {
    pub path: PathBuf,
    pub identity: String,
    pub title: String,
    pub author: String,
    pub pages: Vec<Page>,
    pub toc: Vec<Navigation>,
    source: Arc<[u8]>,
}
pub struct Reading {
    pub book: Arc<Book>,
    pub page: usize,
    pub font_size: u8,
    pub images: BTreeMap<usize, Arc<gpui::RenderImage>>,
    pub warnings: Vec<String>,
    pending_import: Option<PathBuf>,
}
impl Drop for Reading {
    fn drop(&mut self) {
        if let Some(path) = &self.pending_import {
            let _ = fs::remove_file(path);
        }
    }
}
impl Reading {
    /// The UI commits a validated import only when the task result is accepted.
    pub fn accept_import(&mut self) {
        self.pending_import = None;
    }

    pub fn text(&self) -> String {
        self.book.pages[self.page].text()
    }
    pub fn load(book: Arc<Book>, page: usize, cancel: &Cancel) -> Result<Self> {
        cancelled(cancel)?;
        let page = page.min(book.pages.len() - 1);
        let mut images = BTreeMap::new();
        let mut warnings = vec![];
        if book.pages[page].blocks.iter().any(|b| b.image.is_some()) {
            let mut zip = zip::ZipArchive::new(Cursor::new(book.source.as_ref()))?;
            let mut pixels = 0u64;
            for (i, block) in book.pages[page].blocks.iter().enumerate() {
                cancelled(cancel)?;
                if let Some(resource) = &block.image {
                    let image = (|| -> Result<_> {
                        let bytes = member(&mut zip, resource, cancel)?;
                        let mut reader =
                            image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
                        let mut limits = image::Limits::default();
                        limits.max_image_width = Some(8192);
                        limits.max_image_height = Some(8192);
                        limits.max_alloc = Some(64 * 1024 * 1024);
                        reader.limits(limits);
                        let decoded = reader.decode()?;
                        pixels += u64::from(decoded.width()) * u64::from(decoded.height());
                        ensure!(pixels <= 16_000_000, "本页图片尺寸过大");
                        let mut rgba = decoded.into_rgba8();
                        for p in rgba.chunks_exact_mut(4) {
                            p.swap(0, 2)
                        }
                        Ok(Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
                            rgba,
                        )])))
                    })();
                    match image {
                        Ok(image) => {
                            images.insert(i, image);
                        }
                        Err(error) => {
                            warnings.push(format!("{}：图片未显示（{error}）", block.text))
                        }
                    }
                }
            }
        }
        cancelled(cancel)?;
        Ok(Self {
            book,
            page,
            font_size: 18,
            images,
            warnings,
            pending_import: None,
        })
    }
}

/// Validate and prepare the complete first reading before committing a library copy.
pub fn open(lib: &Library, path: &Path, import: bool, cancel: &Cancel) -> Result<Reading> {
    cancelled(cancel)?;
    ensure!(
        is_ebook(path),
        "电子书支持 EPUB、TXT；PDF 请使用 PDF 阅读器。MOBI/AZW3 暂不支持"
    );
    ensure!(
        fs::metadata(path)?.len() <= SOURCE_LIMIT,
        "电子书超过 128 MiB 限制"
    );
    let mut bytes = vec![];
    fs::File::open(path)?
        .take(SOURCE_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= SOURCE_LIMIT,
        "电子书超过 128 MiB 限制"
    );
    cancelled(cancel)?;
    let identity = hex::encode(Sha256::digest(&bytes));
    let (title, author, pages, toc) = if extension(path).as_deref() == Some("epub") {
        parse_epub(&bytes, cancel)?
    } else {
        parse_text(path, &bytes)?
    };
    ensure!(!pages.is_empty(), "电子书没有可阅读正文");
    let mut book = Arc::new(Book {
        path: path.to_path_buf(),
        identity,
        title,
        author,
        pages,
        toc,
        source: bytes.into(),
    });
    let progress = lib.setting(&format!("reader_progress_{}", book.identity))?;
    let progress = serde_json::from_str::<serde_json::Value>(&progress).unwrap_or_default();
    let page = progress["page"]
        .as_u64()
        .unwrap_or(0)
        .min(usize::MAX as u64) as usize;
    let mut reading = Reading::load(book.clone(), page, cancel)?;
    reading.font_size = progress["font_size"].as_u64().unwrap_or(18).clamp(14, 32) as u8;
    if import {
        let mut stem = display_name(path)
            .chars()
            .filter(|c| c.is_alphanumeric() || " -_".contains(*c))
            .collect::<String>();
        while stem.len() > 120 {
            stem.pop();
        }
        let suffix = format!("-{}.{}", &book.identity[..32], extension(path).unwrap());
        let existing = fs::read_dir(lib.root.join("pdf"))?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().ends_with(&suffix))
                    && p.is_file()
            });
        let target = existing.unwrap_or_else(|| {
            lib.root.join("pdf").join(format!(
                "{}{}",
                if stem.is_empty() { "book" } else { &stem },
                suffix
            ))
        });
        if target.exists() {
            ensure!(
                fs::metadata(&target)?.len() <= SOURCE_LIMIT,
                "书库已有同名文件体积异常"
            );
            let mut bytes = vec![];
            fs::File::open(&target)?
                .take(SOURCE_LIMIT + 1)
                .read_to_end(&mut bytes)?;
            ensure!(
                hex::encode(Sha256::digest(&bytes)) == book.identity,
                "书库同名文件内容已改变，请先移除损坏文件"
            );
        }
        cancelled(cancel)?;
        if !target.exists() {
            let temp =
                lib.root
                    .join("pdf")
                    .join(format!(".{}-{}.tmp", book.identity, crate::data::id()));
            let result = (|| -> Result<()> {
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temp)?;
                for chunk in book.source.chunks(128 * 1024) {
                    cancelled(cancel)?;
                    file.write_all(chunk)?;
                }
                file.sync_all()?;
                drop(file);
                cancelled(cancel)?;
                fs::rename(&temp, &target)?;
                Ok(())
            })();
            if result.is_err() {
                let _ = fs::remove_file(&temp);
            }
            result?;
            reading.pending_import = Some(target.clone());
        }
        Arc::make_mut(&mut book).path = target;
        reading.book = book;
    }
    Ok(reading)
}
pub fn save_progress(lib: &Library, reading: &Reading) -> Result<()> {
    lib.set_setting(
        &format!("reader_progress_{}", reading.book.identity),
        &json!({"page":reading.page,"font_size":reading.font_size.clamp(14,32)}).to_string(),
    )
}

#[derive(Default, Debug)]
struct Node {
    name: String,
    attrs: BTreeMap<String, String>,
    parts: Vec<Part>,
}
#[derive(Debug)]
enum Part {
    Text(String),
    Node(Node),
}
impl Node {
    fn attr(&self, key: &str) -> &str {
        self.attrs.get(key).map(String::as_str).unwrap_or("")
    }
    fn children(&self) -> impl Iterator<Item = &Node> {
        self.parts
            .iter()
            .filter_map(|p| if let Part::Node(n) = p { Some(n) } else { None })
    }
    fn descendants<'a>(&'a self, name: &str, out: &mut Vec<&'a Node>) {
        if self.name == name {
            out.push(self)
        }
        for c in self.children() {
            c.descendants(name, out)
        }
    }
    fn all(&self, name: &str) -> Vec<&Node> {
        let mut v = vec![];
        self.descendants(name, &mut v);
        v
    }
    fn text(&self) -> String {
        fn append(node: &Node, out: &mut String, remaining: &mut usize) {
            for part in &node.parts {
                if *remaining == 0 {
                    break;
                }
                match part {
                    Part::Text(text) => {
                        for c in text.chars().take(*remaining) {
                            out.push(c);
                            *remaining -= 1;
                        }
                    }
                    Part::Node(child) => append(child, out, remaining),
                }
            }
        }
        let mut text = String::new();
        append(self, &mut text, &mut 512);
        text
    }
}
fn entity(name: &str) -> String {
    if let Some(n) = name
        .strip_prefix("#x")
        .or_else(|| name.strip_prefix("#X"))
        .and_then(|x| u32::from_str_radix(x, 16).ok())
        .or_else(|| name.strip_prefix('#').and_then(|x| x.parse().ok()))
        .and_then(char::from_u32)
    {
        return n.to_string();
    }
    quick_xml::escape::resolve_html5_entity(name)
        .map(str::to_string)
        .unwrap_or_else(|| format!("&{name};"))
}
fn xml(bytes: &[u8]) -> Result<Node> {
    let raw = unicode(bytes).context("EPUB XML 需要 UTF-8 或 UTF-16 编码")?;
    let mut reader = Reader::from_str(&raw);
    let mut stack = vec![Node::default()];
    let mut nodes = 0;
    loop {
        let event = reader.read_event().context("电子书 XML 损坏")?;
        let empty = matches!(&event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                nodes += 1;
                ensure!(
                    nodes <= 200_000 && stack.len() < 128,
                    "XML 结构过大或嵌套过深"
                );
                let mut node = Node {
                    name: String::from_utf8_lossy(e.local_name().as_ref()).to_ascii_lowercase(),
                    ..Default::default()
                };
                for a in e.attributes() {
                    let a = a?;
                    node.attrs.insert(
                        String::from_utf8_lossy(a.key.local_name().as_ref()).to_ascii_lowercase(),
                        a.decode_and_unescape_value(reader.decoder())?.into_owned(),
                    );
                }
                // Empty/start share parsing, but only Start adds a stack frame.
                if empty {
                    stack.last_mut().unwrap().parts.push(Part::Node(node));
                } else {
                    stack.push(node);
                }
            }
            Event::End(_) => {
                ensure!(stack.len() > 1, "XML 结束标签无对应起点");
                let n = stack.pop().unwrap();
                stack.last_mut().unwrap().parts.push(Part::Node(n));
            }
            Event::Text(e) => stack
                .last_mut()
                .unwrap()
                .parts
                .push(Part::Text(e.xml_content()?.into_owned())),
            Event::CData(e) => stack
                .last_mut()
                .unwrap()
                .parts
                .push(Part::Text(e.decode()?.into_owned())),
            Event::GeneralRef(e) => stack
                .last_mut()
                .unwrap()
                .parts
                .push(Part::Text(entity(&e.decode()?))),
            Event::Eof => break,
            _ => {}
        }
    }
    ensure!(stack.len() == 1, "XML 标签没有闭合");
    Ok(stack.pop().unwrap())
}

fn path_from(base: &str, href: &str) -> Result<(String, String)> {
    ensure!(
        !href.contains('\\') && !href.chars().any(char::is_control),
        "电子书资源路径无效"
    );
    let (href, fragment) = href.split_once('#').unwrap_or((href, ""));
    let href = href.split('?').next().unwrap_or(href);
    ensure!(
        !href.contains(':') && !href.starts_with("//"),
        "不读取电子书中的外部资源"
    );
    let decode = |s: &str| -> Result<String> {
        let bytes = s.as_bytes();
        let mut out = vec![];
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                ensure!(i + 2 < bytes.len(), "资源路径百分号编码无效");
                let hi = (bytes[i + 1] as char)
                    .to_digit(16)
                    .context("资源路径百分号编码无效")?;
                let lo = (bytes[i + 2] as char)
                    .to_digit(16)
                    .context("资源路径百分号编码无效")?;
                out.push((hi * 16 + lo) as u8);
                i += 3
            } else {
                out.push(bytes[i]);
                i += 1
            }
        }
        Ok(String::from_utf8(out)?)
    };
    let decoded = decode(href)?;
    ensure!(!decoded.contains(['\\', '\0', ':']), "资源路径无效");
    let mut parts = if decoded.starts_with('/') {
        vec![]
    } else {
        base.rsplit_once('/')
            .map(|(d, _)| d.split('/').collect())
            .unwrap_or_default()
    };
    if decoded.is_empty() {
        return Ok((base.into(), decode(fragment)?));
    }
    for part in decoded.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                ensure!(parts.pop().is_some(), "电子书资源路径越界");
            }
            p => parts.push(p),
        }
    }
    Ok((parts.join("/"), decode(fragment)?))
}
fn member<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
    cancel: &Cancel,
) -> Result<Vec<u8>> {
    cancelled(cancel)?;
    let mut file = zip
        .by_name(name)
        .with_context(|| format!("电子书缺少资源：{name}"))?;
    ensure!(file.size() <= MEMBER_LIMIT, "电子书成员超过 16 MiB：{name}");
    let mut out = vec![];
    let mut buffer = [0; 64 * 1024];
    loop {
        cancelled(cancel)?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buffer[..n]);
        ensure!(out.len() as u64 <= MEMBER_LIMIT, "电子书成员解压过大");
    }
    Ok(out)
}
#[derive(Default)]
struct Flow {
    blocks: Vec<Block>,
    text: String,
    heading: u8,
    anchors: Vec<String>,
}
impl Flow {
    fn flush(&mut self) {
        let text = self
            .text
            .split('\n')
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("\n");
        self.text.clear();
        if !text.trim().is_empty() {
            self.blocks.push(Block {
                text,
                heading: self.heading,
                image: None,
                anchors: std::mem::take(&mut self.anchors),
            });
        }
    }
    fn walk(&mut self, node: &Node, file: &str) {
        if ["head", "script", "style", "noscript"].contains(&node.name.as_str())
            || node.attrs.contains_key("hidden")
        {
            return;
        }
        let block = [
            "p",
            "div",
            "section",
            "article",
            "li",
            "blockquote",
            "pre",
            "tr",
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
            "hr",
        ]
        .contains(&node.name.as_str());
        if block {
            self.flush();
        }
        if !node.attr("id").is_empty() {
            self.anchors.push(node.attr("id").into());
        }
        let previous = self.heading;
        if node.name.starts_with('h') && node.name.len() == 2 {
            self.heading = node.name[1..].parse::<u8>().unwrap_or(0).min(6)
        }
        if ["img", "image"].contains(&node.name.as_str()) {
            self.flush();
            let href = if node.name == "img" {
                node.attr("src")
            } else {
                node.attr("href")
            };
            let resource = path_from(file, href)
                .ok()
                .filter(|(p, _)| !p.is_empty())
                .map(|(p, _)| p);
            self.blocks.push(Block {
                text: if node.attr("alt").is_empty() {
                    "插图".into()
                } else {
                    node.attr("alt").into()
                },
                heading: 0,
                image: Some(resource.unwrap_or_default()),
                anchors: std::mem::take(&mut self.anchors),
            });
        } else if node.name == "br" {
            self.text.push('\n')
        } else {
            for part in &node.parts {
                match part {
                    Part::Text(s) => self
                        .text
                        .extend(s.chars().map(|c| if c.is_whitespace() { ' ' } else { c })),
                    Part::Node(n) => self.walk(n, file),
                }
            }
        }
        if block {
            self.flush();
        }
        self.heading = previous;
    }
}
fn paginate(
    blocks: Vec<Block>,
    title: &str,
    file: &str,
    pages: &mut Vec<Page>,
    locations: &mut BTreeMap<String, usize>,
) -> Result<()> {
    let mut current = Page {
        title: title.into(),
        blocks: vec![],
    };
    let mut count = 0;
    locations.insert(file.into(), pages.len());
    for block in blocks {
        let chars = block.text.chars().collect::<Vec<_>>();
        let chunks = chars.chunks(PAGE_CHARS).collect::<Vec<_>>();
        for (i, chunk) in chunks.iter().enumerate() {
            if count + chunk.len() > PAGE_CHARS || current.blocks.len() >= 80 {
                ensure!(pages.len() < 8192, "电子书分页过多");
                pages.push(current);
                current = Page {
                    title: title.into(),
                    blocks: vec![],
                };
                count = 0;
            }
            if i == 0 {
                for anchor in &block.anchors {
                    locations
                        .entry(format!("{file}#{anchor}"))
                        .or_insert(pages.len());
                }
            }
            let mut part = block.clone();
            part.text = chunk.iter().copied().collect();
            if i > 0 {
                part.anchors.clear();
            }
            count += chunk.len();
            current.blocks.push(part);
        }
    }
    if !current.blocks.is_empty() {
        pages.push(current);
    }
    ensure!(pages.len() <= 8192, "电子书章节／分页过多");
    Ok(())
}

fn parse_epub(
    bytes: &[u8],
    cancel: &Cancel,
) -> Result<(String, String, Vec<Page>, Vec<Navigation>)> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).context("不是可读取的 EPUB 压缩包")?;
    ensure!(zip.len() <= 10_000, "EPUB 文件条目过多");
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        cancelled(cancel)?;
        let f = zip.by_index(i)?;
        let name = f.name()?;
        ensure!(
            !name.starts_with('/')
                && !name.contains(['\\', ':', '\0'])
                && !name.split('/').any(|p| p == ".."),
            "EPUB 文件路径越界"
        );
        ensure!(names.insert(name.to_string()), "EPUB 包含重复文件路径");
        total = total.checked_add(f.size()).context("EPUB 长度溢出")?;
        ensure!(
            f.size() <= MEMBER_LIMIT && total <= EXPANDED_LIMIT,
            "EPUB 解压体积过大"
        );
    }
    ensure!(
        member(&mut zip, "mimetype", cancel)? == b"application/epub+zip",
        "EPUB mimetype 无效"
    );
    if names.contains("META-INF/encryption.xml") {
        let encryption = xml(&member(&mut zip, "META-INF/encryption.xml", cancel)?)?;
        ensure!(
            encryption.all("encryptionmethod").iter().all(|n| [
                "http://www.idpf.org/2008/embedding",
                "http://ns.adobe.com/pdf/enc#RC"
            ]
            .contains(&n.attr("algorithm"))),
            "暂不支持加密或 DRM 电子书"
        );
    }
    let container = xml(&member(&mut zip, "META-INF/container.xml", cancel)?)?;
    let raw = container
        .all("rootfile")
        .first()
        .context("EPUB 缺少 rootfile")?
        .attr("full-path")
        .to_string();
    let (opf, _) = path_from("", &raw)?;
    ensure!(!opf.is_empty(), "EPUB rootfile 为空");
    let package = xml(&member(&mut zip, &opf, cancel)?)?;
    let metadata = package
        .all("metadata")
        .first()
        .copied()
        .context("EPUB 缺少 metadata")?;
    let title = metadata
        .all("title")
        .first()
        .map(|n| n.text())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "未命名电子书".into());
    let mut author = String::new();
    for node in metadata.all("creator") {
        if !author.is_empty() {
            author.push('、');
        }
        let remaining = 1024usize.saturating_sub(author.chars().count());
        author.extend(node.text().chars().take(remaining));
        if author.chars().count() >= 1024 {
            break;
        }
    }
    let mut items = BTreeMap::new();
    let mut nav = None;
    let mut ncx = None;
    for item in package
        .all("manifest")
        .first()
        .context("EPUB 缺少 manifest")?
        .children()
        .filter(|n| n.name == "item")
    {
        let id = item.attr("id");
        ensure!(
            !id.is_empty() && !items.contains_key(id),
            "EPUB manifest ID 无效或重复"
        );
        let path = path_from(&opf, item.attr("href"))?.0;
        if item
            .attr("properties")
            .split_whitespace()
            .any(|s| s == "nav")
        {
            nav = Some(path.clone());
        }
        if item.attr("media-type") == "application/x-dtbncx+xml" {
            ncx = Some(path.clone());
        }
        items.insert(id.to_string(), (path, item.attr("media-type").to_string()));
    }
    let spine = package
        .all("spine")
        .first()
        .copied()
        .context("EPUB 缺少 spine")?;
    if let Some((p, _)) = items.get(spine.attr("toc")) {
        ncx = Some(p.clone())
    }
    let mut pages = vec![];
    let mut locations = BTreeMap::new();
    let mut text_size = 0;
    for reference in spine
        .children()
        .filter(|n| n.name == "itemref" && n.attr("linear") != "no")
    {
        cancelled(cancel)?;
        let (path, kind) = items
            .get(reference.attr("idref"))
            .context("EPUB spine 引用了缺失章节")?;
        ensure!(
            ["application/xhtml+xml", "text/html", "image/svg+xml"].contains(&kind.as_str()),
            "暂不支持此 EPUB 章节类型：{kind}"
        );
        let chapter = xml(&member(&mut zip, path, cancel)?)?;
        let mut flow = Flow::default();
        flow.walk(&chapter, path);
        flow.flush();
        text_size += flow.blocks.iter().map(|b| b.text.len()).sum::<usize>();
        ensure!(text_size <= 32 * 1024 * 1024, "EPUB 正文过大");
        let heading = flow
            .blocks
            .iter()
            .find(|b| b.heading > 0)
            .map(|b| b.text.chars().take(512).collect::<String>())
            .unwrap_or_else(|| format!("章节 {}", pages.len() + 1));
        paginate(flow.blocks, &heading, path, &mut pages, &mut locations)?;
    }
    ensure!(!pages.is_empty(), "EPUB 没有可阅读正文");
    let mut toc = vec![];
    let mut add = |base: &str, href: &str, label: String| {
        if let Ok((path, fragment)) = path_from(base, href) {
            let key = if fragment.is_empty() {
                path.clone()
            } else {
                format!("{path}#{fragment}")
            };
            if let Some(&page) = locations.get(&key).or_else(|| locations.get(&path)) {
                if !label.trim().is_empty() {
                    toc.push(Navigation {
                        label: label.split_whitespace().collect::<Vec<_>>().join(" "),
                        page,
                    });
                }
            }
        }
    };
    if let Some(path) = nav {
        let tree = xml(&member(&mut zip, &path, cancel)?)?;
        if let Some(nav) = tree
            .all("nav")
            .into_iter()
            .find(|n| n.attr("type").split_whitespace().any(|s| s == "toc"))
        {
            for link in nav.all("a") {
                add(&path, link.attr("href"), link.text());
            }
        }
    } else if let Some(path) = ncx {
        let tree = xml(&member(&mut zip, &path, cancel)?)?;
        for point in tree.all("navpoint") {
            let label = point
                .children()
                .find(|n| n.name == "navlabel")
                .map(Node::text)
                .unwrap_or_default();
            if let Some(content) = point.children().find(|n| n.name == "content") {
                add(&path, content.attr("src"), label);
            }
        }
    }
    if toc.is_empty() {
        for (i, p) in pages.iter().enumerate() {
            if i == 0 || pages[i - 1].title != p.title {
                toc.push(Navigation {
                    label: p.title.clone(),
                    page: i,
                });
            }
        }
    }
    Ok((title.trim().into(), author, pages, toc))
}
fn unicode(bytes: &[u8]) -> Result<String> {
    let text = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        ensure!(bytes.len() % 2 == 0, "TXT UTF-16 长度无效");
        let little = bytes[0] == 0xff;
        String::from_utf16(
            &bytes[2..]
                .chunks_exact(2)
                .map(|b| {
                    if little {
                        u16::from_le_bytes([b[0], b[1]])
                    } else {
                        u16::from_be_bytes([b[0], b[1]])
                    }
                })
                .collect::<Vec<_>>(),
        )
        .context("TXT UTF-16 编码无效")?
    } else {
        std::str::from_utf8(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes))
            .context("TXT 请保存为 UTF-8 或带 BOM 的 UTF-16 编码")?
            .into()
    };
    Ok(text)
}
fn parse_text(path: &Path, bytes: &[u8]) -> Result<(String, String, Vec<Page>, Vec<Navigation>)> {
    let text = unicode(bytes)?;
    ensure!(text.len() <= 32 * 1024 * 1024, "TXT 正文超过 32 MiB 限制");
    ensure!(!text.contains('\0'), "TXT 包含无效字符");
    let title = display_name(path);
    let mut pages = vec![];
    let blocks = text
        .replace("\r\n", "\n")
        .split('\n')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| Block {
            text: s.into(),
            heading: 0,
            image: None,
            anchors: vec![],
        })
        .collect();
    paginate(blocks, &title, "", &mut pages, &mut BTreeMap::new())?;
    Ok((
        title.clone(),
        String::new(),
        pages,
        vec![Navigation {
            label: title,
            page: 0,
        }],
    ))
}

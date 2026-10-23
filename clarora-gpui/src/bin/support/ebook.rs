//! Real EPUB ZIP / filesystem / SQLite / local AI workflows, designed before implementation.
use anyhow::{Result, ensure};
use clarora_gpui::{ai, data::Library, ebook, process};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

fn archive(path: &Path, entries: &[(&str, Vec<u8>)]) -> Result<()> {
    let mut zip = zip::ZipWriter::new(fs::File::create(path)?);
    for (name, bytes) in entries {
        zip.start_file(
            *name,
            zip::write::SimpleFileOptions::default().compression_method(if *name == "mimetype" {
                zip::CompressionMethod::Stored
            } else {
                zip::CompressionMethod::Deflated
            }),
        )?;
        zip.write_all(bytes)?;
    }
    zip.finish()?;
    Ok(())
}

fn fixture(path: &Path, v3: bool) -> Result<()> {
    let first = format!(
        "<html xmlns='http://www.w3.org/1999/xhtml'><head><title>Wrong head text</title><style>BAD_STYLE</style></head><body><h1 id='start'>第一章</h1><p>Hello &amp; &#x4E16;&#30028; &nbsp; &ldquo;reading&rdquo;.<br/>Line after break</p><script>BAD_SCRIPT</script><img src='../Images/p.png' alt='测试插图'/>{}<h2 id='later'>后半段</h2><p>ANCHOR_TARGET</p></body></html>",
        "<p>中文连续阅读 Mixed English text.</p>".repeat(250)
    );
    let nav = if v3 {
        "<html xmlns:epub='http://www.idpf.org/2007/ops'><body><nav epub:type='page-list'><a href='Text/b.xhtml'>IGNORE_PAGE_LIST</a></nav><nav epub:type='toc'><ol><li><a href='Text/a%20章.xhtml#start'>第一章目录</a><ol><li><a href='Text/a%20章.xhtml#later'>后半段目录</a></li></ol></li><li><a href='Text/b.xhtml#second'>第二章目录</a></li></ol></nav></body></html>"
    } else {
        "<ncx><navMap><navPoint><navLabel><text>第一章目录</text></navLabel><content src='Text/a%20章.xhtml#start'/></navPoint><navPoint><navLabel><text>后半段目录</text></navLabel><content src='Text/a%20章.xhtml#later'/></navPoint><navPoint><navLabel><text>第二章目录</text></navLabel><content src='Text/b.xhtml#second'/></navPoint></navMap></ncx>"
    };
    let nav_item = if v3 {
        "<item id='nav' href='nav.xhtml' media-type='application/xhtml+xml' properties='nav'/>"
    } else {
        "<item id='nav' href='nav.ncx' media-type='application/x-dtbncx+xml'/>"
    };
    let opf = format!(
        "<package xmlns:dc='http://purl.org/dc/elements/1.1/' version='{}'><metadata><dc:title>中文 &amp; English</dc:title><dc:creator>Fixture Author</dc:creator></metadata><manifest><item id='b' href='Text/b.xhtml' media-type='application/xhtml+xml'/>{nav_item}<item id='a' href='Text/a%20章.xhtml' media-type='application/xhtml+xml'/><item id='img' href='Images/p.png' media-type='image/png'/></manifest><spine toc='nav'><itemref idref='a'/><itemref idref='b'/></spine></package>",
        if v3 { "3.0" } else { "2.0" }
    );
    let png = path.with_extension("png");
    image::RgbaImage::from_pixel(2, 3, image::Rgba([20, 100, 200, 255])).save(&png)?;
    archive(path, &[
        ("mimetype", b"application/epub+zip".to_vec()),
        ("META-INF/container.xml", b"<container><rootfiles><rootfile full-path='OPS/book.opf' media-type='application/oebps-package+xml'/><rootfile full-path='MISSING.opf'/></rootfiles></container>".to_vec()),
        ("OPS/book.opf", opf.into_bytes()), ("OPS/Text/b.xhtml", b"<html><body><h1 id='second'>Second chapter</h1><p>SECOND_CHAPTER_ONLY</p></body></html>".to_vec()),
        ("OPS/Text/a 章.xhtml", first.into_bytes()),
        (if v3 {"OPS/nav.xhtml"} else {"OPS/nav.ncx"}, nav.as_bytes().to_vec()),
        ("OPS/Images/p.png", fs::read(png)?),
    ])
}

pub fn run(parent: &Library, results: &mut Vec<Value>) -> Result<()> {
    let root = parent.root.join("ebook");
    fs::create_dir_all(&root)?;
    let lib = Library::open(root.join("library"))?;
    let source = root.join("中文 fixture.epub");
    fixture(&source, true)?;
    let cancel = process::cancel_token();
    let mut cases = vec![];
    let mut reading = ebook::open(&lib, &source, true, &cancel)?;
    reading.accept_import();
    ensure!(
        reading.book.title == "中文 & English" && reading.book.author == "Fixture Author",
        "metadata failed"
    );
    ensure!(
        reading.text().contains("Hello & 世界")
            && !reading.text().contains("BAD_")
            && !reading.text().contains("SECOND_CHAPTER_ONLY"),
        "wrong spine/text isolation"
    );
    ensure!(
        reading.text().contains("\nLine after break"),
        "explicit line break lost"
    );
    ensure!(reading.images.len() == 1, "package image not decoded");
    ensure!(
        reading.book.pages.len() > 2 && reading.book.toc.len() == 3,
        "pagination/TOC failed"
    );
    ensure!(
        reading.book.toc[1].page > 0
            && reading.book.pages[reading.book.toc[1].page]
                .text()
                .contains("ANCHOR_TARGET"),
        "fragment TOC failed"
    );
    ensure!(
        reading
            .book
            .pages
            .last()
            .unwrap()
            .text()
            .contains("SECOND_CHAPTER_ONLY"),
        "manifest order used instead of spine"
    );
    ensure!(
        reading
            .book
            .pages
            .iter()
            .all(|p| p.text().chars().count() <= 6500),
        "unbounded page text"
    );
    let imported = reading.book.path.clone();
    let identity = reading.book.identity.clone();
    cases.push(json!({"case":"EPUB3-first-rootfile-spine-entities-head-script-TOC-anchor-pagination-image","passed":true,"pages":reading.book.pages.len(),"toc":reading.book.toc}));
    let renamed = root.join("另一文件名.epub");
    fs::copy(&source, &renamed)?;
    let renamed_reading = ebook::open(&lib, &renamed, true, &cancel)?;
    ensure!(
        renamed_reading.book.path == imported,
        "renamed content duplicate"
    );
    let duplicate = ebook::open(&lib, &source, true, &cancel)?;
    ensure!(
        duplicate.book.path == imported,
        "same book duplicate import"
    );
    reading = ebook::Reading::load(reading.book.clone(), reading.book.toc[1].page, &cancel)?;
    reading.font_size = 24;
    ebook::save_progress(&lib, &reading)?;
    fs::remove_file(&source)?;
    let reopened = Library::open(lib.root.clone())?;
    let restored = ebook::open(&reopened, &imported, false, &cancel)?;
    ensure!(
        restored.page == reading.page
            && restored.font_size == 24
            && restored.book.identity == identity,
        "progress restart failed"
    );
    let last = ebook::Reading::load(restored.book.clone(), usize::MAX, &cancel)?;
    ensure!(last.page == last.book.pages.len() - 1, "page clamp failed");
    cases.push(json!({"case":"copy-dedup-remove-original-SQLite-reopen-progress-font-page-boundaries","passed":true,"imported":imported,"page":restored.page}));

    let epub2 = root.join("legacy.epub");
    fixture(&epub2, false)?;
    let legacy = ebook::open(&lib, &epub2, false, &cancel)?;
    ensure!(
        legacy.book.toc.len() == 3 && legacy.book.toc[1].page > 0,
        "NCX failed"
    );
    cases.push(json!({"case":"EPUB2-NCX-reading-order-and-fragments","passed":true}));
    for (name, bytes) in [
        (
            "utf8.txt",
            "\u{feff}中文 TXT\n\nUTF8_TEXT".as_bytes().to_vec(),
        ),
        (
            "utf16.txt",
            [
                vec![0xff, 0xfe],
                "中文 UTF16"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect(),
            ]
            .concat(),
        ),
    ] {
        let p = root.join(name);
        fs::write(&p, bytes)?;
        let mut text = ebook::open(&lib, &p, true, &cancel)?;
        text.accept_import();
        ensure!(text.text().contains("中文"), "TXT decode failed");
        ensure!(
            text.page == 0 && text.font_size == 18,
            "book progress leaked"
        );
    }
    cases.push(json!({"case":"UTF8-BOM-UTF16-TXT-separate-book-progress","passed":true}));

    let count = || fs::read_dir(lib.root.join("pdf")).unwrap().count();
    let before = count();
    for (name, bytes) in [
        ("bad.epub", b"bad zip".to_vec()),
        ("bad.txt", vec![0xff, 0xff, 0xff]),
        ("unsupported.mobi", b"not text".to_vec()),
    ] {
        let p = root.join(name);
        fs::write(&p, bytes)?;
        ensure!(
            ebook::open(&lib, &p, true, &cancel).is_err(),
            "accepted {name}"
        );
    }
    for (name, entries) in [
        (
            "missing.epub",
            vec![("mimetype", b"application/epub+zip".to_vec())],
        ),
        ("traversal.epub", vec![("../outside", b"bad".to_vec())]),
        (
            "oversized-member.epub",
            vec![("huge.xhtml", vec![b' '; 17 * 1024 * 1024])],
        ),
    ] {
        let p = root.join(name);
        archive(&p, &entries)?;
        ensure!(
            ebook::open(&lib, &p, true, &cancel).is_err(),
            "accepted {name}"
        );
    }
    let mut zip = zip::ZipArchive::new(fs::File::open(&epub2)?)?;
    let mut original = vec![];
    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        let name = file.name()?.into_owned();
        let mut bytes = vec![];
        file.read_to_end(&mut bytes)?;
        original.push((name, bytes));
    }
    let utf16 = root.join("utf16-xml.epub");
    let mut entries = original.clone();
    for (name, bytes) in &mut entries {
        if name.ends_with("book.opf") {
            let text = format!(
                "<?xml version='1.0' encoding='UTF-16'?>{}",
                String::from_utf8(bytes.clone())?
            );
            *bytes = [
                vec![0xff, 0xfe],
                text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            ]
            .concat();
        }
    }
    archive(
        &utf16,
        &entries
            .iter()
            .map(|(n, b)| (n.as_str(), b.clone()))
            .collect::<Vec<_>>(),
    )?;
    ensure!(
        ebook::open(&lib, &utf16, false, &cancel)?.book.title == "中文 & English",
        "UTF16 OPF failed"
    );
    cases.push(json!({"case":"UTF16-EPUB-XML-and-HTML-named-entities","passed":true}));
    let mut long_labels = original.clone();
    for (name, bytes) in &mut long_labels {
        if name == "OPS/book.opf" {
            *bytes = String::from_utf8(bytes.clone())?
                .replace("中文 &amp; English", &"长标题".repeat(3000))
                .into_bytes();
        }
        if name == "OPS/Text/a 章.xhtml" {
            *bytes = format!(
                "<html><body><h1>{}</h1><p>LONG_LABEL_BODY</p></body></html>",
                "章节标题".repeat(3000)
            )
            .into_bytes();
        }
    }
    let long_path = root.join("long-labels.epub");
    archive(
        &long_path,
        &long_labels
            .iter()
            .map(|(n, b)| (n.as_str(), b.clone()))
            .collect::<Vec<_>>(),
    )?;
    let long = ebook::open(&lib, &long_path, false, &cancel)?;
    ensure!(
        long.book.title.chars().count() <= 512
            && long
                .book
                .pages
                .iter()
                .all(|p| p.title.chars().count() <= 512),
        "unbounded UI labels"
    );
    cases.push(json!({"case":"long-metadata-and-heading-bounded-before-native-UI-pagination","passed":true}));
    let mut rejection_names = vec![];
    for scenario in [
        "drm",
        "malformed-xml",
        "missing-spine",
        "missing-chapter",
        "outside-root",
        "external-resource",
        "bad-percent",
        "empty-book",
    ] {
        let mut entries = original.clone();
        if scenario == "drm" {
            entries.push(("META-INF/encryption.xml".into(),b"<encryption><EncryptedData><EncryptionMethod Algorithm='urn:DRM'/></EncryptedData></encryption>".to_vec()));
        }
        for (name, bytes) in &mut entries {
            let text = String::from_utf8_lossy(bytes);
            if name == "OPS/book.opf" {
                *bytes = match scenario {
                    "malformed-xml" => text.replace("</spine>", "</broken>"),
                    "missing-spine" => text
                        .replace("<spine ", "<other ")
                        .replace("</spine>", "</other>"),
                    "missing-chapter" => text.replace("idref='a'", "idref='unknown'"),
                    "external-resource" => {
                        text.replace("Text/a%20章.xhtml", "https://127.0.0.1/private.xhtml")
                    }
                    "bad-percent" => text.replace("Text/a%20章.xhtml", "Text/%中.xhtml"),
                    _ => text.into_owned(),
                }
                .into_bytes();
            } else if name == "META-INF/container.xml" && scenario == "outside-root" {
                *bytes = text.replace("OPS/book.opf", "../outside.opf").into_bytes();
            } else if name.starts_with("OPS/Text/") && scenario == "empty-book" {
                *bytes = b"<html><body><script>NOT_BODY</script></body></html>".to_vec();
            }
        }
        let file = root.join(format!("{scenario}.epub"));
        let refs = entries
            .iter()
            .map(|(n, b)| (n.as_str(), b.clone()))
            .collect::<Vec<_>>();
        archive(&file, &refs)?;
        ensure!(
            ebook::open(&lib, &file, true, &cancel).is_err(),
            "accepted malformed {scenario}"
        );
        rejection_names.push(scenario);
    }
    cases.push(json!({"case":"real-EPUB-DRM-XML-spine-chapter-path-resource-encoding-empty-failures","passed":true,"scenarios":rejection_names}));
    let duplicate = root.join("duplicate.epub");
    archive(
        &duplicate,
        &[("dupa", b"1".to_vec()), ("dupb", b"2".to_vec())],
    )?;
    let mut raw = fs::read(&duplicate)?;
    for i in 0..raw.len().saturating_sub(3) {
        if &raw[i..i + 4] == b"dupb" {
            raw[i..i + 4].copy_from_slice(b"dupa");
        }
    }
    fs::write(&duplicate, raw)?;
    ensure!(
        ebook::open(&lib, &duplicate, true, &cancel).is_err(),
        "duplicate ZIP accepted"
    );
    let pending = ebook::open(&lib, &epub2, true, &cancel)?;
    let pending_path = pending.book.path.clone();
    ensure!(pending_path.is_file(), "pending import missing");
    drop(pending);
    ensure!(
        !pending_path.exists(),
        "cancelled result retained new import"
    );
    let flag = process::cancel_token();
    flag.store(true, Ordering::SeqCst);
    ensure!(
        ebook::open(&lib, &epub2, true, &flag).is_err(),
        "cancelled import succeeded"
    );
    ensure!(count() == before, "failed imports left book files");
    cases.push(json!({"case":"broken-encoding-format-container-traversal-duplicate-bomb-cancel-no-import","passed":true}));

    let mut shelf = clarora_gpui::view_data::State::default();
    shelf.reload(&lib, &[])?;
    ensure!(
        shelf.document_files.contains(&imported)
            && shelf
                .document_files
                .iter()
                .any(|p| p.extension().is_some_and(|x| x == "txt")),
        "EPUB/TXT not in cached shelf"
    );
    cases.push(json!({"case":"cached-document-shelf-includes-EPUB-and-TXT","passed":true,"files":shelf.document_files}));

    // Send the exact page exposed to the UI through the same AI API, using one loopback request.
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let url = format!("http://{}", listener.local_addr()?);
    let handle = std::thread::spawn(move || -> Result<Value> {
        let start = Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((s, _)) => break s,
                Err(_) => {
                    ensure!(
                        start.elapsed() < Duration::from_secs(8),
                        "AI request timeout"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        };
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut bytes = vec![];
        let mut buf = [0; 8192];
        let end = loop {
            let n = stream.read(&mut buf)?;
            ensure!(n > 0, "missing headers");
            bytes.extend_from_slice(&buf[..n]);
            if let Some(i) = bytes.windows(4).position(|x| x == b"\r\n\r\n") {
                break i + 4;
            }
        };
        let headers = String::from_utf8_lossy(&bytes[..end]);
        let len = headers
            .lines()
            .find_map(|l| {
                l.to_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|v| v.trim().parse::<usize>().ok())
            })
            .unwrap();
        while bytes.len() < end + len {
            let n = stream.read(&mut buf)?;
            ensure!(n > 0, "missing body");
            bytes.extend_from_slice(&buf[..n]);
        }
        let request = serde_json::from_slice(&bytes[end..end + len])?;
        let body = r#"{"choices":[{"message":{"content":"Complete page feedback"},"finish_reason":"stop"}]}"#;
        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes())?;
        Ok(request)
    });
    ai::save_shared(
        &lib,
        ai::Config {
            base_url: url,
            model: "ebook-fixture".into(),
            ..Default::default()
        },
        "",
    )?;
    let reply = ai::chat(&lib, "解释这段", &restored.text(), &[], &cancel, |_| {})?;
    let request = handle.join().unwrap()?;
    let raw = request.to_string();
    ensure!(
        reply == "Complete page feedback"
            && raw.contains("ANCHOR_TARGET")
            && !raw.contains("SECOND_CHAPTER_ONLY"),
        "wrong AI passage"
    );
    fs::write(
        root.join("ai-request.json"),
        serde_json::to_vec_pretty(&request)?,
    )?;
    cases.push(
        json!({"case":"current-reading-page-local-HTTP-AI-material-isolation","passed":true}),
    );
    fs::write(
        root.join("ebook-evidence.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":true,"cases":cases,"library":lib.root,"pages":restored.book.pages.len()}),
        )?,
    )?;
    results.push(json!({"workflow":"EPUB2/3与TXT导入 → 目录/插图/重排分页 → 独立进度重开 → 本机AI → 失败与取消保护","passed":true,"cases":cases.len(),"evidence":root.join("ebook-evidence.json")}));
    Ok(())
}

pub fn external_file(parent: &Library, source: &Path, results: &mut Vec<Value>) -> Result<()> {
    let lib = Library::open(parent.root.join("external-book"))?;
    let cancel = process::cancel_token();
    let mut reading = ebook::open(&lib, source, true, &cancel)?;
    reading.accept_import();
    let mut samples = vec![];
    for page in [
        0,
        reading.book.pages.len() / 2,
        reading.book.pages.len() - 1,
    ] {
        let r = ebook::Reading::load(reading.book.clone(), page, &cancel)?;
        ensure!(!r.text().trim().is_empty(), "external book empty page");
        samples.push(json!({"page":page,"characters":r.text().chars().count(),"decoded_images":r.images.len(),"warnings":r.warnings}));
    }
    let report = json!({"passed":true,"source":source,"imported":reading.book.path,"identity":reading.book.identity,
        "title":reading.book.title,"author":reading.book.author,"pages":reading.book.pages.len(),"toc":reading.book.toc,"samples":samples});
    let path = lib.root.join("external-book-evidence.json");
    fs::write(&path, serde_json::to_vec_pretty(&report)?)?;
    results.push(json!({"workflow":"外部真实 EPUB 导入、完整章节解析与首/中/末页读取","passed":true,"evidence":path}));
    Ok(())
}

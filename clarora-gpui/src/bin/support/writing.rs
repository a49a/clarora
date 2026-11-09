//! Writing drafts → real loopback model → append-only correction history → ZIP restore.
use anyhow::{Context, Result, ensure};
use clarora_gpui::{ai, backup, data::Library, process, writing};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::Command,
    sync::{Arc, Mutex, atomic::Ordering},
    thread,
    time::{Duration, Instant},
};

const FEEDBACK: &str = "合成批改：切题；第二段可补充论据。\n修改建议：保留原意，改善衔接。";

struct Fixture {
    address: String,
    stop: process::Cancel,
    cancel: Arc<Mutex<process::Cancel>>,
    requests: Arc<Mutex<Vec<Value>>>,
    worker: Option<thread::JoinHandle<Result<()>>>,
}
impl Fixture {
    fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?.to_string();
        let stop = process::cancel_token();
        let stopping = stop.clone();
        let cancel = Arc::new(Mutex::new(process::cancel_token()));
        let cancellation = cancel.clone();
        let requests = Arc::new(Mutex::new(vec![]));
        let seen = requests.clone();
        let worker = thread::spawn(move || -> Result<()> {
            while !stopping.load(Ordering::SeqCst) {
                let (mut socket, _) = listener.accept()?;
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
                socket.set_nonblocking(false)?;
                socket.set_read_timeout(Some(Duration::from_secs(3)))?;
                socket.set_write_timeout(Some(Duration::from_secs(3)))?;
                let request = read_request(&mut socket)?;
                let model = request["body"]["model"]
                    .as_str()
                    .context("writing fixture has no model")?
                    .to_string();
                seen.lock().unwrap().push(request);
                if model == "cancel-wait" {
                    cancellation.lock().unwrap().store(true, Ordering::SeqCst);
                    // Wait for the cancelled HTTP client to close, without a timer race.
                    let mut byte = [0];
                    let _ = socket.read(&mut byte);
                    continue;
                }
                let sse = model.starts_with("sse");
                let choice = if model == "length" {
                    json!({"message":{"content":"不应保存的截断反馈"},"finish_reason":"length"})
                } else if sse {
                    json!({"delta":{"content":FEEDBACK},"finish_reason":"stop"})
                } else if model.starts_with("edits-") {
                    let edits = match model.as_str() {
                        "edits-valid" => json!([
                            {"original":"costs also matter","replacement":"costs also deserve consideration","reason":"措辞更明确"},
                            {"original":"learn after work","replacement":"study after work","reason":"搭配更自然"}
                        ]),
                        "edits-empty" => json!([]),
                        "edits-overlap" => json!([
                            {"original":"costs also matter","replacement":"costs are relevant","reason":"措辞"},
                            {"original":"also matter","replacement":"are important","reason":"措辞"}
                        ]),
                        _ => {
                            json!([{"original":"made up quote","replacement":"replacement","reason":"reason"}])
                        }
                    };
                    let content = format!(
                        "```json\n{}\n```",
                        json!({"summary":"合成逐处修改","edits":edits})
                    );
                    json!({"message":{"content":content},"finish_reason":"stop"})
                } else {
                    json!({"message":{"content":FEEDBACK},"finish_reason":"stop"})
                };
                let value = if model == "provider-error" {
                    json!({"error":{"message":"synthetic writing model failure"},"choices":[choice]})
                } else {
                    json!({"choices":[choice]})
                };
                let body = if sse {
                    let done = if model == "sse-disconnect" {
                        ""
                    } else {
                        "data: [DONE]\n\n"
                    };
                    format!("data: {value}\n\n{done}")
                } else {
                    value.to_string()
                };
                let result = response(
                    &mut socket,
                    if sse {
                        "text/event-stream"
                    } else {
                        "application/json"
                    },
                    &body,
                );
                if model != "sse-cancel" {
                    result?;
                }
            }
            Ok(())
        });
        Ok(Self {
            address,
            stop,
            cancel,
            requests,
            worker: Some(worker),
        })
    }
    fn configure(&self, lib: &Library, model: &str) -> Result<()> {
        ai::save_shared(
            lib,
            ai::Config {
                base_url: format!("http://{}", self.address),
                model: model.into(),
                ..Default::default()
            },
            "writing-http-fixture-key",
        )
    }
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
    fn finish(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(&self.address);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("writing HTTP fixture panicked"))??;
        }
        Ok(())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

fn read_request(socket: &mut TcpStream) -> Result<Value> {
    let mut bytes = vec![];
    let mut block = [0; 8192];
    let split = loop {
        let count = socket.read(&mut block)?;
        ensure!(count > 0, "writing HTTP header interrupted");
        bytes.extend_from_slice(&block[..count]);
        ensure!(bytes.len() <= 1024 * 1024, "writing HTTP header too large");
        if let Some(index) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..split]);
    ensure!(
        headers
            .lines()
            .next()
            .unwrap_or("")
            .starts_with("POST /chat/completions "),
        "writing used unexpected endpoint: {headers}"
    );
    ensure!(
        headers
            .lines()
            .any(|s| s.eq_ignore_ascii_case("authorization: Bearer writing-http-fixture-key")),
        "writing did not use the synthetic credential"
    );
    let length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")?
                .trim()
                .parse::<usize>()
                .ok()
        })
        .context("writing HTTP missing Content-Length")?;
    ensure!(length <= 1024 * 1024, "writing request too large");
    while bytes.len() < split + length {
        let count = socket.read(&mut block)?;
        ensure!(count > 0, "writing HTTP body interrupted");
        bytes.extend_from_slice(&block[..count]);
    }
    let body: Value = serde_json::from_slice(&bytes[split..split + length])?;
    Ok(json!({"endpoint":"POST /chat/completions","synthetic_credential":true,"body":body}))
}
fn response(socket: &mut TcpStream, kind: &str, body: &str) -> Result<()> {
    write!(
        socket,
        "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    socket.flush()?;
    Ok(())
}
fn snapshot(lib: &Library, path: &Path) -> Result<()> {
    lib.db()?
        .execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])?;
    Ok(())
}
fn stored(lib: &Library, id: &str) -> Result<Value> {
    Ok(serde_json::to_value(
        writing::get(lib, id)?.context("writing entry disappeared")?,
    )?)
}
fn record(root: &Path, evidence: &mut Vec<Value>, case: &str, details: Value) -> Result<()> {
    evidence.push(json!({"case":case,"passed":true,"details":details}));
    fs::write(
        root.join("writing-evidence.json"),
        serde_json::to_vec_pretty(evidence)?,
    )?;
    Ok(())
}
fn request_contains(fixture: &Fixture, index: usize, prompt: &str, answer: &str) -> Result<()> {
    let requests = fixture.requests.lock().unwrap();
    let messages = requests[index]["body"]["messages"]
        .as_array()
        .context("writing model request messages absent")?;
    let system = messages
        .iter()
        .find(|m| m["role"] == "system")
        .and_then(|m| m["content"].as_str())
        .context("writing model request system material absent")?;
    let material = system
        .split_once("<material>\n")
        .and_then(|(_, s)| s.rsplit_once("\n</material>").map(|(data, _)| data))
        .context("writing material delimiters absent")?;
    let sent: Value = serde_json::from_str(material)?;
    ensure!(
        sent["writing_prompt"] == prompt,
        "writing request omitted or altered full topic"
    );
    ensure!(
        sent["student_answer"] == answer,
        "writing request omitted or altered full answer"
    );
    ensure!(
        sent.get("tags").is_none(),
        "writing tags were sent to the model"
    );
    Ok(())
}

pub fn run(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    let root = lib.root.join("writing");
    let lib = Library::open(root.join("source"))?;
    let mut fixture = Fixture::start()?;
    let mut evidence = vec![];
    let request_path = root.join("http-requests.json");
    // Persist every captured request even when a later assertion fails.
    let outcome = (|| -> Result<()> {
        for (text, expected) in [
            ("  … — 😊\n", 0),
            ("Dear Maria,\nI didn't attend.", 5),
            ("It’s a well-written follow‑up.", 4),
            ("Hello,world — café 1,000.50", 4),
            ("word--word", 2),
        ] {
            ensure!(
                writing::answer_word_count(text) == expected,
                "wrong word count for {text:?}"
            );
        }
        record(
            &root,
            &mut evidence,
            "answer-word-count",
            json!({"cases":5}),
        )?;
        let empty = writing::Entry::draft();
        ensure!(
            writing::save(&lib, &empty).is_err(),
            "empty writing draft inserted"
        );
        ensure!(
            writing::list(&lib)?.is_empty(),
            "empty draft created a writing row"
        );
        let mut incomplete = writing::Entry::draft();
        incomplete.title = "仅标题也可保存".into();
        incomplete = writing::save(&lib, &incomplete)?;
        ensure!(
            writing::list(&lib)?.len() == 1,
            "saved draft absent from list"
        );
        fixture.configure(&lib, "direct")?;
        for (case, prompt, answer) in [
            ("empty-topic", " \n\t", "已写的答案"),
            ("empty-answer", "完整题目", " \r\n\t"),
        ] {
            incomplete.prompt = prompt.into();
            incomplete.answer = answer.into();
            incomplete = writing::save(&lib, &incomplete)?;
            let before = stored(&lib, &incomplete.id)?;
            let requests = fixture.count();
            let rejected = writing::review(&lib, &incomplete.id, &process::cancel_token(), |_| {});
            ensure!(
                rejected.is_err(),
                "{case}: empty writing accepted for correction"
            );
            ensure!(fixture.count() == requests, "{case}: validation sent HTTP");
            ensure!(
                before == stored(&lib, &incomplete.id)?,
                "{case}: changed draft"
            );
            record(
                &root,
                &mut evidence,
                case,
                json!({"requests":0,"error":rejected.err().unwrap().to_string()}),
            )?;
        }
        writing::delete(&lib, &incomplete.id)?;
        ensure!(
            writing::get(&lib, &incomplete.id)?.is_none(),
            "delete retained writing"
        );
        record(
            &root,
            &mut evidence,
            "incomplete-draft-save-delete",
            json!({"all_empty_rejected":true,"title_only_saved":true,"deleted":true}),
        )?;

        let mut entry = writing::Entry::draft();
        entry.title = "议论文 · 合成题目".into();
        entry.prompt = " \nDiscuss whether public libraries should stay open late.\n请给出两方面论证。<topic-end-完整> \n".into();
        entry.answer = " \nI believe that public libraries should stay open late.\n学生 can learn after work; however, costs also matter.\n<answer-end-完整> \t".into();
        entry = writing::save(&lib, &entry)?;
        let reopened = Library::open(lib.root.clone())?;
        let initial = writing::get(&reopened, &entry.id)?.context("writing reopen failed")?;
        ensure!(
            initial.title == entry.title
                && initial.prompt == entry.prompt
                && initial.answer == entry.answer,
            "writing reopen changed exact input"
        );
        ensure!(
            initial.reviews.is_empty(),
            "draft unexpectedly contains corrections"
        );
        ensure!(
            !initial.prompt_markdown,
            "legacy plain text became Markdown"
        );
        let mut legacy = stored(&lib, &entry.id)?;
        legacy.as_object_mut().unwrap().remove("tags");
        lib.db()?.execute(
            "UPDATE ai_records SET payload=? WHERE id=?",
            rusqlite::params![legacy.to_string(), entry.id],
        )?;
        ensure!(writing::get(&reopened, &entry.id)?.unwrap().tags.is_empty());
        let mut tags = vec![];
        writing::add_tag(&mut tags, "  议论文  ")?;
        writing::add_tag(&mut tags, "TOEFL")?;
        ensure!(writing::add_tag(&mut tags, "toefl").is_err());
        ensure!(writing::add_tag(&mut tags, "   ").is_err());
        ensure!(writing::add_tag(&mut tags, "坏\n标签").is_err());
        ensure!(writing::add_tag(&mut tags, &"写".repeat(100)).is_err());
        ensure!(tags == ["议论文", "TOEFL"]);
        entry.tags = tags;
        let tagged = writing::save(&lib, &entry)?;
        ensure!(writing::get(&reopened, &entry.id)?.unwrap() == tagged);
        ensure!(
            tagged.minimum_words.is_none(),
            "legacy draft acquired a goal"
        );
        entry.minimum_words = Some(0);
        ensure!(writing::save(&lib, &entry).is_err());
        entry.minimum_words = Some(writing::MAX_MINIMUM_WORDS + 1);
        ensure!(writing::save(&lib, &entry).is_err());
        entry.minimum_words = Some(150);
        let with_goal = writing::save(&lib, &entry)?;
        ensure!(writing::get(&reopened, &entry.id)?.unwrap() == with_goal);
        entry.minimum_words = None;
        ensure!(writing::save(&lib, &entry)?.minimum_words.is_none());
        entry.minimum_words = Some(150);
        entry.tags.pop();
        entry = writing::save(&lib, &entry)?;
        ensure!(entry.minimum_words == Some(150));
        ensure!(writing::answer_word_count(&entry.answer) < 150);
        record(
            &root,
            &mut evidence,
            "answer-minimum-words",
            json!({"saved":150,"cleared":true,"invalid_rejected":true,"answer_below_minimum":true}),
        )?;
        ensure!(writing::get(&reopened, &entry.id)?.unwrap().tags == ["议论文"]);
        record(
            &root,
            &mut evidence,
            "writing-tags-legacy-add-remove-reopen",
            json!({"legacy_defaults_empty":true,"initial_tags":tagged.tags,"tags_after_removal":entry.tags,"invalid_tag_preserves_labels":true}),
        )?;
        let mut formatted = writing::Entry::draft();
        formatted.prompt = clarora_gpui::writing_format::from_html(
            r#"<h2>写一封邮件</h2><p>Contact <strong>Maria</strong> today.</p><ol start="2"><li>Explain why you were absent.</li><li>Ask for <em>help</em>.</li></ol><script>hidden</script><p><a href="https://example.invalid/">Thank her</a>.<br>Write complete sentences.</p>"#,
        ).context("formatted clipboard failed to convert")?;
        formatted.prompt_markdown = true;
        ensure!(
            formatted.prompt
                == "## 写一封邮件\n\nContact **Maria** today.\n\n2. Explain why you were absent.\n3. Ask for *help*.\n\nThank her.  \nWrite complete sentences.",
            "clipboard structure, emphasis or visible text lost: {:?}",
            formatted.prompt
        );
        writing::save(&lib, &formatted)?;
        let reloaded =
            writing::get(&reopened, &formatted.id)?.context("formatted prompt missing")?;
        ensure!(reloaded.prompt_markdown && reloaded.prompt == formatted.prompt);
        record(
            &root,
            &mut evidence,
            "formatted-prompt-save-reopen",
            json!({"markdown":reloaded.prompt}),
        )?;
        snapshot(&lib, &root.join("before-correction.sqlite"))?;
        record(
            &root,
            &mut evidence,
            "save-and-reopen-exact-input",
            json!({"id":entry.id,"title":entry.title,"prompt":entry.prompt,"answer":entry.answer}),
        )?;

        let mut boundary = writing::Entry::draft();
        boundary.title = "题".repeat(writing::MAX_TITLE_CHARS);
        boundary.prompt = "题".repeat(writing::MAX_PROMPT_CHARS);
        boundary.answer = "答".repeat(writing::MAX_ANSWER_CHARS);
        boundary = writing::save(&lib, &boundary)?;
        for field in ["title", "prompt", "answer"] {
            let mut over = boundary.clone();
            match field {
                "title" => over.title.push('题'),
                "prompt" => over.prompt.push('题'),
                _ => over.answer.push('答'),
            }
            let before = stored(&lib, &boundary.id)?;
            ensure!(
                writing::save(&lib, &over).is_err(),
                "overlimit {field} saved"
            );
            ensure!(
                before == stored(&lib, &boundary.id)?,
                "overlimit {field} overwrote draft"
            );
            // Imported legacy data must pass the same gate before any HTTP request.
            lib.db()?.execute(
                "UPDATE ai_records SET payload=?1 WHERE id=?2",
                [serde_json::to_string(&over)?, boundary.id.clone()],
            )?;
            let requests = fixture.count();
            let rejected = writing::review(&lib, &boundary.id, &process::cancel_token(), |_| {});
            ensure!(
                rejected.is_err() && requests == fixture.count(),
                "overlimit imported {field} reached model"
            );
            lib.db()?.execute(
                "UPDATE ai_records SET payload=?1 WHERE id=?2",
                [serde_json::to_string(&boundary)?, boundary.id.clone()],
            )?;
        }
        writing::delete(&lib, &boundary.id)?;
        record(
            &root,
            &mut evidence,
            "unicode-boundaries-and-overlimit",
            json!({"title_characters":writing::MAX_TITLE_CHARS,"prompt_characters":writing::MAX_PROMPT_CHARS,"answer_characters":writing::MAX_ANSWER_CHARS,"overlimit_http_requests":0}),
        )?;

        lib.set_setting("ai_config", "")?;
        let before = stored(&lib, &entry.id)?;
        let requests = fixture.count();
        let missing = writing::review(&lib, &entry.id, &process::cancel_token(), |_| {});
        ensure!(
            missing.is_err() && fixture.count() == requests,
            "missing model did not fail locally"
        );
        ensure!(
            before == stored(&lib, &entry.id)?,
            "missing model changed writing"
        );
        record(
            &root,
            &mut evidence,
            "missing-model",
            json!({"requests":0,"error":missing.err().unwrap().to_string()}),
        )?;

        for model in ["direct", "sse"] {
            fixture.configure(&lib, model)?;
            let before = writing::get(&lib, &entry.id)?.unwrap();
            let request_index = fixture.count();
            let review = writing::review(&lib, &entry.id, &process::cancel_token(), |_| {})?;
            ensure!(
                fixture.count() == request_index + 1,
                "{model}: duplicate writing model request"
            );
            request_contains(&fixture, request_index, &before.prompt, &before.answer)?;
            let after = writing::get(&Library::open(lib.root.clone())?, &entry.id)?.unwrap();
            let context = review.follow_up_context()?;
            ensure!(
                context.contains(&before.prompt)
                    && context.contains(&before.answer)
                    && context.contains(FEEDBACK)
            );
            ensure!(
                after.title == before.title
                    && after.prompt == before.prompt
                    && after.answer == before.answer,
                "{model}: correction overwrote draft"
            );
            ensure!(
                after.reviews.len() == before.reviews.len() + 1,
                "{model}: correction history was not appended"
            );
            ensure!(
                review.prompt == before.prompt
                    && review.answer == before.answer
                    && review.feedback == FEEDBACK
                    && review.mode() == writing::ReviewMode::Overview
                    && review.model == model
                    && !review.created_at.is_empty(),
                "{model}: correction snapshot lost content or metadata"
            );
            ensure!(
                serde_json::to_value(&after.reviews[..before.reviews.len()])?
                    == serde_json::to_value(&before.reviews)?,
                "{model}: earlier correction history changed"
            );
            record(
                &root,
                &mut evidence,
                &format!("{model}-successful-correction"),
                json!({"request_index":request_index,"review":review,"history_count":after.reviews.len(),"draft_unchanged":true}),
            )?;
        }

        for model in [
            "edits-valid",
            "edits-empty",
            "edits-overlap",
            "edits-unmatched",
        ] {
            fixture.configure(&lib, model)?;
            let before = writing::get(&lib, &entry.id)?.unwrap();
            let request_index = fixture.count();
            let result = writing::review_snapshot_with_mode(
                &lib,
                &before,
                writing::ReviewMode::LineEdits,
                &process::cancel_token(),
                |_| {},
            );
            request_contains(&fixture, request_index, &before.prompt, &before.answer)?;
            ensure!(
                fixture.requests.lock().unwrap()[request_index]["body"]
                    .to_string()
                    .contains("replacement")
            );
            let after = writing::get(&reopened, &entry.id)?.unwrap();
            if matches!(model, "edits-valid" | "edits-empty") {
                let review = result?;
                ensure!(review.mode() == writing::ReviewMode::LineEdits);
                let expected = if model == "edits-valid" {
                    before
                        .answer
                        .replace("costs also matter", "costs also deserve consideration")
                        .replace("learn after work", "study after work")
                } else {
                    before.answer.clone()
                };
                ensure!(review.revision_preview()?.as_deref() == Some(expected.as_str()));
                let annotations = review.revision_annotations()?;
                if model == "edits-valid" {
                    ensure!(
                        annotations.len() == 2 && annotations[0].0.end <= annotations[1].0.start
                    );
                    ensure!(&review.answer[annotations[0].0.clone()] == "learn after work");
                    ensure!(&review.answer[annotations[1].0.clone()] == "costs also matter");
                } else {
                    ensure!(annotations.is_empty());
                }
                ensure!(after.answer == before.answer && after.reviews.last() == Some(&review));
                let context = review.follow_up_context()?;
                ensure!(context.contains(&before.prompt) && context.contains(&before.answer));
                ensure!(context.contains("合成逐处修改"));
                if model == "edits-valid" {
                    ensure!(context.contains("1. 原文：learn after work"));
                    ensure!(context.contains("建议：study after work\n原因：搭配更自然"));
                    ensure!(context.contains("2. 原文：costs also matter"));
                }
            } else {
                ensure!(
                    result.is_err() && after == before,
                    "invalid revision changed the draft or history"
                );
            }
            record(&root, &mut evidence, model, json!({"draft_unchanged":true}))?;
        }

        for (original, revised) in [
            (
                "She has cats and brought food.",
                "She had cats and took food.",
            ),
            ("Dear Maria,\n\n你好。", "Dear Maria,\n\n你好！"),
            ("keep context", "keep this context"),
            ("remove this", "remove"),
            ("erase", ""),
        ] {
            let changes = writing::revision_changes(original, revised);
            let mut cursor = 0;
            let mut actual = String::new();
            for (range, replacement) in &changes {
                actual.push_str(&original[cursor..range.start]);
                actual.push_str(replacement);
                cursor = range.end;
            }
            actual.push_str(&original[cursor..]);
            ensure!(actual == revised);
            if original.starts_with("She has") {
                ensure!(
                    changes
                        .iter()
                        .map(|(range, _)| &original[range.clone()])
                        .eq(["has", "brought"])
                );
            }
        }
        record(
            &root,
            &mut evidence,
            "writing-ink-anchors-and-word-changes",
            json!({"cases":5}),
        )?;

        fixture.configure(&lib, "direct")?;
        let original = writing::get(&lib, &entry.id)?.unwrap();
        let mut revised = original.clone();
        revised.title = "正在批改时的新标题".into();
        revised.prompt = formatted.prompt.clone();
        revised.prompt_markdown = true;
        writing::add_tag(&mut revised.tags, "立场论证")?;
        revised.answer =
            "A newer essay, typed while the first version is being corrected.\n完整新稿。".into();
        let mut edit = None;
        let review = writing::review(&lib, &entry.id, &process::cancel_token(), |_| {
            if edit.is_none() {
                edit = Some(writing::save(&lib, &revised));
            }
        })?;
        edit.context("writing correction emitted no progress")??;
        let after = writing::get(&lib, &entry.id)?.unwrap();
        ensure!(
            after.title == revised.title
                && after.prompt == revised.prompt
                && after.prompt_markdown
                && after.answer == revised.answer
                && after.tags == revised.tags,
            "late correction replaced newer writing draft"
        );
        ensure!(
            review.prompt == original.prompt
                && !review.prompt_markdown
                && review.answer == original.answer,
            "correction history did not retain requested old draft"
        );
        ensure!(
            after.reviews.len() == original.reviews.len() + 1,
            "concurrent edit lost correction history"
        );
        record(
            &root,
            &mut evidence,
            "edit-during-correction",
            json!({"review":review,"current_prompt":after.prompt,"current_answer":after.answer,"old_snapshot_preserved":true}),
        )?;

        // The UI captures this saved draft when the user submits. Autosave can
        // persist newer text before its background worker gets scheduled.
        let submitted = after.clone();
        let mut queued_edit = submitted.clone();
        queued_edit.title = "后台开始之前已经保存的新标题".into();
        queued_edit.prompt = "排队时改成的新题目；本次提交仍应批改旧题。".into();
        queued_edit.prompt_markdown = false;
        queued_edit.answer =
            "A newer draft saved before the correction worker begins.\n排队期间新稿。".into();
        let queued_edit = writing::save(&lib, &queued_edit)?;
        let request_index = fixture.count();
        let review = writing::review_snapshot(&lib, &submitted, &process::cancel_token(), |_| {})?;
        ensure!(
            fixture.count() == request_index + 1,
            "queued correction sent an unexpected number of HTTP requests"
        );
        request_contains(
            &fixture,
            request_index,
            &submitted.prompt,
            &submitted.answer,
        )?;
        let after = writing::get(&lib, &entry.id)?.unwrap();
        ensure!(
            after.title == queued_edit.title
                && after.prompt == queued_edit.prompt
                && !after.prompt_markdown
                && after.answer == queued_edit.answer,
            "queued correction overwrote the newer autosaved draft"
        );
        ensure!(
            review.prompt == submitted.prompt
                && review.prompt_markdown
                && review.answer == submitted.answer,
            "queued correction reviewed a later draft instead of the submitted snapshot"
        );
        ensure!(
            after.reviews.len() == submitted.reviews.len() + 1
                && serde_json::to_value(&after.reviews[..submitted.reviews.len()])?
                    == serde_json::to_value(&submitted.reviews)?,
            "queued correction lost or replaced earlier review history"
        );
        record(
            &root,
            &mut evidence,
            "autosave-before-correction-worker",
            json!({"request_index":request_index,"review":review,"current_prompt":after.prompt,"current_answer":after.answer,"submitted_snapshot_received_by_http":true,"newer_draft_preserved":true}),
        )?;

        let history = serde_json::to_value(&after.reviews)?;
        let mut stale = entry.clone();
        stale.title = "旧编辑快照重新保存".into();
        stale.reviews.clear();
        let saved = writing::save(&lib, &stale)?;
        ensure!(
            serde_json::to_value(&saved.reviews)? == history,
            "stale editor dropped corrections"
        );
        let mut injected = saved.clone();
        let mut forged = saved.reviews[0].clone();
        forged.id = "forged-writing-review".into();
        forged.feedback = "不得注入的伪造反馈".into();
        injected.reviews.push(forged);
        let saved = writing::save(&lib, &injected)?;
        ensure!(
            serde_json::to_value(&saved.reviews)? == history,
            "caller injected forged correction history"
        );
        record(
            &root,
            &mut evidence,
            "stale-save-and-history-injection",
            json!({"preserved_history_count":saved.reviews.len(),"forged_history_rejected":true}),
        )?;

        let mut removed = writing::Entry::draft();
        removed.title = "批改期间删除".into();
        removed.prompt = "Write about an imagined island.".into();
        removed.answer = "The island has a small library.".into();
        removed = writing::save(&lib, &removed)?;
        let mut deletion = None;
        let deleted = writing::review(&lib, &removed.id, &process::cancel_token(), |_| {
            if deletion.is_none() {
                deletion = Some(writing::delete(&lib, &removed.id));
            }
        });
        deletion.context("delete case had no progress")??;
        ensure!(
            deleted.is_err() && writing::get(&lib, &removed.id)?.is_none(),
            "late correction resurrected deleted writing"
        );
        record(
            &root,
            &mut evidence,
            "delete-during-correction",
            json!({"not_resurrected":true,"error":deleted.err().unwrap().to_string()}),
        )?;

        for case in [
            "pre-cancelled",
            "cancel-wait",
            "direct-final-cancel",
            "length",
            "provider-error",
            "sse-disconnect",
            "sse-cancel",
        ] {
            let model = if ["pre-cancelled", "direct-final-cancel"].contains(&case) {
                "direct"
            } else {
                case
            };
            fixture.configure(&lib, model)?;
            let before = stored(&lib, &entry.id)?;
            let requests = fixture.count();
            let cancel = process::cancel_token();
            if case == "pre-cancelled" {
                cancel.store(true, Ordering::SeqCst);
            }
            *fixture.cancel.lock().unwrap() = cancel.clone();
            let started = Instant::now();
            let mut progress = 0;
            let rejected = writing::review(&lib, &entry.id, &cancel, |_| {
                progress += 1;
                if ["direct-final-cancel", "sse-cancel"].contains(&case) {
                    cancel.store(true, Ordering::SeqCst);
                }
            });
            let elapsed = started.elapsed();
            ensure!(
                rejected.is_err(),
                "{case}: failed correction returned success"
            );
            ensure!(
                before == stored(&lib, &entry.id)?,
                "{case}: failed correction changed draft or history"
            );
            let sent = fixture.count() - requests;
            ensure!(
                sent == usize::from(case != "pre-cancelled"),
                "{case}: unexpected request count {sent}"
            );
            let error = rejected.err().unwrap().to_string();
            if case.contains("cancel") {
                ensure!(
                    error.contains("已取消"),
                    "{case}: wrong cancellation error: {error}"
                );
                ensure!(
                    elapsed < Duration::from_millis(1500),
                    "{case}: cancellation stalled for {elapsed:?}"
                );
            }
            record(
                &root,
                &mut evidence,
                case,
                json!({"error":error,"requests":sent,"milliseconds":elapsed.as_millis(),"progress_events":progress,"database_unchanged":true}),
            )?;
        }

        fixture.configure(&lib, "direct")?;
        let before = stored(&lib, &entry.id)?;
        let mut retryable = writing::get(&lib, &entry.id)?.unwrap();
        retryable.answer = "输入仍然保留，保存失败后可以重试。".into();
        let retryable_input = retryable.answer.clone();
        lib.db()?.execute_batch(
            "CREATE TRIGGER writing_fixture_insert_failure BEFORE INSERT ON ai_records WHEN NEW.kind='writing' BEGIN SELECT RAISE(ABORT, 'writing fixture save failure'); END;
             CREATE TRIGGER writing_fixture_update_failure BEFORE UPDATE ON ai_records WHEN OLD.kind='writing' BEGIN SELECT RAISE(ABORT, 'writing fixture save failure'); END;",
        )?;
        let save_failed = writing::save(&lib, &retryable);
        let review_failed = writing::review(&lib, &entry.id, &process::cancel_token(), |_| {});
        lib.db()?.execute_batch("DROP TRIGGER writing_fixture_insert_failure; DROP TRIGGER writing_fixture_update_failure;")?;
        ensure!(
            save_failed.is_err() && review_failed.is_err(),
            "simulated SQLite failure reported success"
        );
        ensure!(
            stored(&lib, &entry.id)? == before && retryable.answer == retryable_input,
            "save failure overwrote persisted or caller content"
        );
        record(
            &root,
            &mut evidence,
            "sqlite-write-failure",
            json!({"save_error":save_failed.err().unwrap().to_string(),"review_error":review_failed.err().unwrap().to_string(),"database_unchanged":true,"input_retained":true}),
        )?;

        let group = writing::create_group(&lib, "  邮件  ")?;
        let empty_group = writing::create_group(&lib, "待练习")?;
        ensure!(group.name == "邮件" && writing::create_group(&lib, "邮件").is_err());
        let original = writing::get(&lib, &entry.id)?.unwrap();
        ensure!(original.group_id.is_none());
        let mut moved = original.clone();
        moved.group_id = Some("unknown-group".into());
        ensure!(writing::save(&lib, &moved).is_err());
        moved.group_id = Some(group.id.clone());
        let saved = writing::save(&lib, &moved)?;
        ensure!(
            saved.group_id == moved.group_id
                && saved.answer == original.answer
                && saved.reviews == original.reviews
        );
        ensure!(writing::get(&reopened, &entry.id)?.as_ref() == Some(&saved));
        ensure!(writing::list_groups(&reopened)?.contains(&empty_group));
        record(
            &root,
            &mut evidence,
            "writing-group-move-and-reopen",
            json!({"legacy_ungrouped":true,"group_id":group.id,"empty_group":empty_group.id,"draft_and_reviews_preserved":true}),
        )?;
        snapshot(&lib, &root.join("after-correction.sqlite"))?;
        let zip = root.join("writing-backup.zip");
        backup::export_local(&lib, &zip)?;
        let restored = Library::open(root.join("restored"))?;
        backup::import_local(&restored, &zip)?;
        ensure!(writing::list_groups(&restored)? == writing::list_groups(&lib)?);
        let source = serde_json::to_value(writing::list(&lib)?)?;
        let first = serde_json::to_value(writing::list(&restored)?)?;
        ensure!(
            source == first,
            "writing ZIP restore changed draft or full correction history"
        );
        backup::import_local(&restored, &zip)?;
        let second = serde_json::to_value(writing::list(&Library::open(restored.root.clone())?)?)?;
        ensure!(
            first == second,
            "repeated writing ZIP restore duplicated or lost records"
        );
        snapshot(&restored, &root.join("restored-writing.sqlite"))?;
        record(
            &root,
            &mut evidence,
            "zip-backup-new-library-repeat-restore",
            json!({"zip":zip,"entries":source.as_array().unwrap().len(),"history_count":source[0]["reviews"].as_array().unwrap().len(),"exact_match":true,"repeat_restore_unchanged":true}),
        )?;
        Ok(())
    })();
    fs::write(
        &request_path,
        serde_json::to_vec_pretty(&*fixture.requests.lock().unwrap())?,
    )?;
    fixture.finish()?;
    outcome?;
    results.push(json!({"workflow":"写作草稿保存/重开 → 完整题目与作答本机 HTTP 批改 → 改稿/删除/取消/失败保护 → 历史与 ZIP 恢复","passed":true,"cases":evidence.len(),"report":root.join("writing-evidence.json"),"http_requests":request_path,"sqlite_before":root.join("before-correction.sqlite"),"sqlite_after":root.join("after-correction.sqlite")}));
    run_native_paste(&root, results)
}

fn run_native_paste(root: &Path, results: &mut Vec<Value>) -> Result<()> {
    let workflow = "原生写作题目：带格式粘贴、标签自动保存和筛选";
    if !cfg!(all(debug_assertions, target_os = "macos"))
        || std::env::var("CLARORA_GUI_MEMORY_CHECK").as_deref() != Ok("1")
    {
        results.push(json!({"workflow":workflow,"passed":true,"skipped":true,"reason":"需要启用CLARORA_GUI_MEMORY_CHECK=1的macOS原生图形会话"}));
        return Ok(());
    }
    let app = std::env::current_exe()?.with_file_name("clarora-gpui");
    let out = root.join("native-paste");
    fs::create_dir_all(&out)?;
    let mut command = Command::new(app);
    command.arg("--writing-paste-check").arg(&out);
    let status = super::native_diagnostic::run(command, &out, Duration::from_secs(30))?;
    let report_path = out.join("diagnostic-run.json");
    let report: Value = serde_json::from_slice(&fs::read(&report_path)?)?;
    ensure!(
        status.success() && report["passed"] == true,
        "原生题目粘贴失败：{}，详见 {}",
        report["error"],
        report_path.display()
    );
    results.push(json!({"workflow":workflow,"passed":true,"report":report_path}));
    Ok(())
}

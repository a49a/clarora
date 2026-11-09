//! Writing drafts and corrections live in their own entity so navigation does not discard edits.
use super::{Action, AppView, FeedbackKind};
use anyhow::{Result, bail};
use clarora_gpui::{
    data::Library,
    process::{self, Cancel},
    writing::{self, Entry, Review, ReviewMode},
    writing_format,
};
use gpui::{ClipboardItem, prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable,
    button::*,
    input::{Input, InputEvent, InputState, Paste},
    menu::{DropdownMenu, PopupMenuItem},
    text::{TextView, TextViewStyle},
    tooltip::Tooltip,
};
use std::{collections::BTreeMap, sync::atomic::Ordering, time::Duration};

pub(super) const BODY_FONT_SIZE: Pixels = px(24.);
pub(super) const BODY_LINE_SPACING: f32 = 1.55;
const EDITOR_HEIGHT: Pixels = px(440.);
const EDITOR_HEADER_HEIGHT: Pixels = px(28.);
const EDITOR_COLUMN_WIDTH: Pixels = px(380.);

#[cfg(all(debug_assertions, target_os = "macos"))]
type ColumnBounds = std::rc::Rc<std::cell::Cell<Option<[Bounds<Pixels>; 2]>>>;

struct CorrectionTask {
    entry_id: String,
    title: String,
    cancel: Cancel,
}

#[derive(Default, Clone, PartialEq, Eq)]
enum GroupFilter {
    #[default]
    All,
    Ungrouped,
    Group(String),
}

#[cfg(all(debug_assertions, target_os = "macos"))]
pub(super) mod paste_probe {
    use super::*;
    use crate::{AppView, diagnostics};
    use anyhow::{Context as _, ensure};
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeHTML};
    use objc2_foundation::NSData;
    use std::path::Path;

    pub(crate) fn prepare(out: &Path) -> Result<Library> {
        let lib = Library::open(out.join("library"))?;
        lib.set_setting("last_page", "writing")?;
        Ok(lib)
    }

    #[derive(Default)]
    pub(crate) struct PasteProbe {
        sent: bool,
        tag_sent: bool,
        tag_removed: bool,
        narrow_requested: bool,
        ink_shown: bool,
        follow_up_opened: bool,
    }

    impl diagnostics::Probe for PasteProbe {
        fn tick(
            &mut self,
            app: &mut AppView,
            window: &mut Window,
            cx: &mut Context<AppView>,
        ) -> Result<bool> {
            let writing = app
                .writing
                .clone()
                .context("writing editor did not mount")?;
            if self.follow_up_opened {
                ensure!(app.assistant_open && app.assistant.expanded);
                let [reference] = app.assistant.references.as_slice() else {
                    anyhow::bail!("expected one pending writing review reference");
                };
                ensure!(
                    reference.text.contains("Earlier submission")
                        && reference.text.contains("原因：Fixture note")
                );
                ensure!(
                    !reference
                        .text
                        .contains("The current draft remains editable.")
                );
                ensure!(app.inputs["question"].read(cx).value().is_empty());
                let persisted: Vec<clarora_gpui::assistant::Reference> =
                    serde_json::from_str(&app.lib.setting("assistant_reference_draft")?)?;
                ensure!(persisted == app.assistant.references);
                writing.update(cx, |view, cx| {
                    let original_id = view.current.id.clone();
                    view.open_group_form(window, cx);
                    view.group_input
                        .update(cx, |input, cx| input.set_value("邮件写作", window, cx));
                    view.create_group(window, cx);
                    let GroupFilter::Group(id) = view.group_filter.clone() else {
                        anyhow::bail!("new writing group was not selected");
                    };
                    ensure!(
                        view.filtered_entries().next().is_none()
                            && view.current.group_id.as_ref() == Some(&id)
                    );
                    view.switch_group(GroupFilter::Ungrouped, window, cx);
                    ensure!(view.current.id == original_id);
                    view.move_to_group(Some(id.clone()), cx);
                    ensure!(
                        view.filtered_entries()
                            .map(|entry| entry.id.as_str())
                            .eq([original_id.as_str()])
                    );
                    let saved =
                        writing::get(&view.lib, &original_id)?.context("moved draft missing")?;
                    ensure!(
                        saved.group_id.as_ref() == Some(&id)
                            && saved.answer == "The current draft remains editable."
                    );
                    view.new_draft(window, cx);
                    ensure!(view.current.group_id.as_ref() == Some(&id));
                    Ok::<_, anyhow::Error>(())
                })?;
                return Ok(true);
            }
            if !self.sent {
                window.resize(size(px(1400.), px(800.)));
                let board = NSPasteboard::pasteboardWithUniqueName();
                board.clearContents();
                let html = b"<h2>Write to Maria</h2><p>Keep <b>these</b> paragraphs.</p><ol><li>Explain</li><li>Ask</li></ol>";
                ensure!(
                    board.setData_forType(Some(&NSData::with_bytes(html)), unsafe {
                        NSPasteboardTypeHTML
                    }),
                    "isolated clipboard rejected HTML"
                );
                writing.update(cx, |view, cx| {
                    view.test_pasteboard = Some(board);
                    view.prompt.update(cx, |input, cx| {
                        input.set_value("replace me 🌱", window, cx);
                        input.focus(window, cx);
                    });
                });
                let handle = window.window_handle();
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        window.draw(cx).clear();
                        for key in ["cmd-a", "cmd-v"] {
                            window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx);
                        }
                    });
                });
                self.sent = true;
                return Ok(false);
            }

            writing.update(cx, |view, cx| {
                let entry = view.edited(cx);
                if self.ink_shown {
                    ensure!(view.show_feedback && view.show_revision_notes);
                    ensure!(entry.answer == "The current draft remains editable.");
                    view.follow_up_review("fixture-review", window, cx);
                    self.follow_up_opened = true;
                    return Ok(false);
                }
                ensure!(
                    entry.prompt
                        == "## Write to Maria\n\nKeep **these** paragraphs.\n\n1. Explain\n2. Ask",
                    "rich paste produced {:?}",
                    entry.prompt
                );
                ensure!(
                    entry.prompt_markdown && view.prompt_preview && entry.answer.is_empty(),
                    "paste did not open the prompt preview or changed the answer"
                );
                ensure!(!view.save_failed, "{}", view.save_status);
                if view.is_dirty(cx) {
                    return Ok(false);
                }
                let reopened = Library::open(view.lib.root.clone())?;
                let saved =
                    writing::get(&reopened, &entry.id)?.context("pasted draft not saved")?;
                if self.tag_removed {
                    ensure!(saved.tags.is_empty() && view.filtered_entries().next().is_none());
                    ensure!(saved.minimum_words == Some(150));
                    if !self.narrow_requested {
                        self.narrow_requested = true;
                        window.resize(size(px(760.), px(760.)));
                        return Ok(false);
                    }
                    let [left, right] = view
                        .test_column_bounds
                        .get()
                        .context("missing editor bounds")?;
                    let [browser, detail] = view
                        .test_workspace_bounds
                        .get()
                        .context("missing workspace bounds")?;
                    ensure!(
                        browser.origin.x == detail.origin.x && detail.origin.y > browser.bottom(),
                        "narrow workspace did not stack: {browser:?}, {detail:?}"
                    );
                    ensure!(
                        left.origin.x == right.origin.x && right.origin.y > left.bottom(),
                        "narrow editor did not stack: {left:?}, {right:?}"
                    );
                    let answer = "The current draft remains editable.";
                    view.answer
                        .update(cx, |input, cx| input.set_value(answer, window, cx));
                    view.finish_correction(
                        &entry.id,
                        Ok(Review {
                            id: "fixture-review".into(),
                            prompt: entry.prompt.clone(),
                            prompt_markdown: true,
                            answer: "Earlier submission".into(),
                            feedback: "Fixture feedback".into(),
                            minimum_words: Some(150),
                            edits: Some(vec![writing::Revision {
                                original: "Earlier submission".into(),
                                replacement: "Earlier draft".into(),
                                reason: "Fixture note".into(),
                            }]),
                            model: "fixture".into(),
                            created_at: entry.updated_at.clone(),
                        }),
                        cx,
                    );
                    ensure!(!view.show_feedback && view.edited(cx).answer == answer);
                    view.show_feedback(true, cx);
                    ensure!(
                        view.show_feedback
                            && view.current.reviews[0].answer == "Earlier submission"
                    );
                    view.show_revision_notes = true;
                    self.ink_shown = true;
                    let handle = window.window_handle();
                    cx.defer(move |cx| {
                        let _ = handle.update(cx, |_, window, cx| window.draw(cx).clear());
                    });
                    return Ok(false);
                }
                view.load_entry(saved, window, cx);
                ensure!(view.prompt_preview && view.edited(cx).prompt == entry.prompt);
                if !self.tag_sent {
                    let [left, right] = view
                        .test_column_bounds
                        .get()
                        .context("missing editor bounds")?;
                    let [browser, detail] = view
                        .test_workspace_bounds
                        .get()
                        .context("missing workspace bounds")?;
                    ensure!(
                        browser.origin.y == detail.origin.y && detail.origin.x > browser.right(),
                        "wide workspace did not split: {browser:?}, {detail:?}"
                    );
                    ensure!(
                        left.origin.y == right.origin.y && right.origin.x > left.right(),
                        "wide editor did not split: {left:?}, {right:?}"
                    );
                    for (label, input) in [("prompt", &view.prompt), ("answer", &view.answer)] {
                        let height = input.update(cx, |input, cx| {
                            input
                                .bounds_for_range(0..0, Bounds::default(), window, cx)
                                .map(|bounds| bounds.size.height)
                        });
                        ensure!(
                            height.is_some_and(|height| (px(36.)..=px(40.)).contains(&height)),
                            "writing {label} editor line height was {height:?}"
                        );
                    }
                    let mut other = Entry::draft();
                    other.prompt = "A different prompt".into();
                    writing::add_tag(&mut other.tags, "TOEFL 阅读")?;
                    writing::save(&reopened, &other)?;
                    view.reload(cx)?;
                    view.minimum_words
                        .update(cx, |input, cx| input.set_value("150", window, cx));
                    view.tag_input.update(cx, |input, cx| {
                        input.set_value(" TOEFL ", window, cx);
                        input.focus(window, cx);
                    });
                    let handle = window.window_handle();
                    cx.defer(move |cx| {
                        let _ = handle.update(cx, |_, window, cx| {
                            window.draw(cx).clear();
                            window.dispatch_keystroke(Keystroke::parse("enter").unwrap(), cx);
                        });
                    });
                    self.tag_sent = true;
                    return Ok(false);
                }
                ensure!(entry.tags == ["TOEFL"], "Enter did not save the tag");
                ensure!(
                    entry.minimum_words == Some(150)
                        && view.minimum_words.read(cx).value().as_ref() == "150"
                );
                ensure!(view.correction.is_none(), "Enter started AI correction");
                view.set_tag_filter(Some("toefl".into()), cx);
                ensure!(
                    view.filtered_entries()
                        .map(|e| e.id.as_str())
                        .eq([entry.id.as_str()])
                );
                view.remove_tag("TOEFL", cx);
                self.tag_removed = true;
                Ok(false)
            })
        }

        fn failed(&mut self, _: String) -> Result<()> {
            Ok(())
        }
    }
}

pub(super) struct WritingView {
    lib: Library,
    page_scroll: ScrollHandle,
    owner: WeakEntity<AppView>,
    entries: Vec<Entry>,
    groups: Vec<writing::Group>,
    group_filter: GroupFilter,
    group_id: Option<String>,
    group_input: Entity<InputState>,
    creating_group: bool,
    current: Entry,
    title: Entity<InputState>,
    prompt: Entity<InputState>,
    prompt_markdown: bool,
    prompt_preview: bool,
    #[cfg(all(debug_assertions, target_os = "macos"))]
    test_pasteboard: Option<objc2::rc::Retained<objc2_app_kit::NSPasteboard>>,
    #[cfg(all(debug_assertions, target_os = "macos"))]
    test_column_bounds: ColumnBounds,
    #[cfg(all(debug_assertions, target_os = "macos"))]
    test_workspace_bounds: ColumnBounds,
    answer: Entity<InputState>,
    minimum_words: Entity<InputState>,
    tags: Vec<String>,
    tag_input: Entity<InputState>,
    tag_error: Option<String>,
    tag_filter: Option<String>,
    save_status: String,
    save_failed: bool,
    load_error: Option<String>,
    selected_review: Option<String>,
    show_feedback: bool,
    show_submission: bool,
    show_revision_notes: bool,
    delete_confirm: Option<String>,
    correction: Option<CorrectionTask>,
    correction_status: Option<(String, bool)>,
    autosave: Option<Task<()>>,
    correction_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl WritingView {
    pub(super) fn new(
        lib: Library,
        page_scroll: ScrollHandle,
        owner: WeakEntity<AppView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (entries, load_error) = match writing::list(&lib) {
            Ok(entries) => (
                entries.into_iter().map(ordered_history).collect::<Vec<_>>(),
                None,
            ),
            Err(error) => (vec![], Some(format!("读取写作列表失败：{error:#}"))),
        };
        let current = entries.first().cloned().unwrap_or_else(Entry::draft);
        let (groups, load_error) = match writing::list_groups(&lib) {
            Ok(groups) => (groups, load_error),
            Err(error) => (Vec::new(), Some(format!("读取写作分组失败：{error:#}"))),
        };
        let group_input = cx.new(|cx| InputState::new(window, cx).placeholder("分组名称"));
        let title = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("标题（可选，方便查找）");
            state.set_value(current.title.clone(), window, cx);
            state
        });
        let prompt = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder("保存写作题目、任务要求或评分标准")
                .multi_line(true)
                .soft_wrap(true);
            state.set_value(current.prompt.clone(), window, cx);
            state
        });
        let answer = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder("在这里写下你的作答，支持中文、英文与多行文本")
                .multi_line(true)
                .soft_wrap(true);
            state.set_value(current.answer.clone(), window, cx);
            state
        });
        let tag_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("例如：邮件写作、托福"));
        let minimum_words = cx.new(|cx| {
            let mut input = InputState::new(window, cx)
                .placeholder("不设目标")
                .validate(|value, _| {
                    value.bytes().all(|byte| byte.is_ascii_digit())
                        && value
                            .parse::<usize>()
                            .is_ok_and(|count| (1..=writing::MAX_MINIMUM_WORDS).contains(&count))
                });
            input.set_value(
                current
                    .minimum_words
                    .map_or_else(String::new, |count| count.to_string()),
                window,
                cx,
            );
            input
        });
        let mut subscriptions = vec![];
        for input in [&title, &prompt, &answer, &minimum_words] {
            subscriptions.push(cx.subscribe(input, |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.changed(cx);
                }
            }));
        }
        subscriptions.push(cx.subscribe_in(
            &group_input,
            window,
            |this, input, event, window, cx| {
                if this.creating_group
                    && matches!(event, InputEvent::PressEnter { secondary: false })
                    && !input.update(cx, |input, cx| {
                        input.marked_text_range(window, cx).is_some()
                    })
                {
                    this.create_group(window, cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &tag_input,
            window,
            |this, input, event, window, cx| match event {
                InputEvent::Change => {
                    this.tag_error = None;
                    cx.notify();
                }
                InputEvent::PressEnter { secondary: false }
                    if !input.update(cx, |input, cx| {
                        input.marked_text_range(window, cx).is_some()
                    }) =>
                {
                    this.add_tag(window, cx);
                }
                _ => {}
            },
        ));
        let save_status = if entries.is_empty() {
            "输入后自动保存".into()
        } else {
            format!("已保存 · {}", display_time(&current.updated_at))
        };
        Self {
            lib,
            page_scroll,
            owner,
            entries,
            groups,
            group_filter: GroupFilter::All,
            group_id: current.group_id.clone(),
            group_input,
            creating_group: false,
            selected_review: current.reviews.first().map(|review| review.id.clone()),
            show_feedback: false,
            show_submission: false,
            show_revision_notes: false,
            prompt_markdown: current.prompt_markdown,
            prompt_preview: current.prompt_markdown,
            #[cfg(all(debug_assertions, target_os = "macos"))]
            test_pasteboard: None,
            #[cfg(all(debug_assertions, target_os = "macos"))]
            test_column_bounds: ColumnBounds::default(),
            #[cfg(all(debug_assertions, target_os = "macos"))]
            test_workspace_bounds: ColumnBounds::default(),
            tags: current.tags.clone(),
            current,
            title,
            prompt,
            answer,
            minimum_words,
            tag_input,
            tag_error: None,
            tag_filter: None,
            save_status,
            save_failed: false,
            load_error,
            delete_confirm: None,
            correction: None,
            correction_status: None,
            autosave: None,
            correction_task: None,
            _subscriptions: subscriptions,
        }
    }

    fn edited(&self, cx: &App) -> Entry {
        let mut entry = self.current.clone();
        entry.title = self.title.read(cx).value().to_string();
        entry.prompt = self.prompt.read(cx).value().to_string();
        entry.prompt_markdown = self.prompt_markdown;
        entry.answer = self.answer.read(cx).value().to_string();
        entry.minimum_words = self.minimum_words_value(cx);
        entry.tags.clone_from(&self.tags);
        entry.group_id.clone_from(&self.group_id);
        entry
    }

    fn is_dirty(&self, cx: &App) -> bool {
        self.title.read(cx).value().as_ref() != self.current.title
            || self.prompt.read(cx).value().as_ref() != self.current.prompt
            || self.prompt_markdown != self.current.prompt_markdown
            || self.answer.read(cx).value().as_ref() != self.current.answer
            || self.minimum_words_value(cx) != self.current.minimum_words
            || self.tags != self.current.tags
            || self.group_id != self.current.group_id
    }

    fn minimum_words_value(&self, cx: &App) -> Option<usize> {
        self.minimum_words.read(cx).value().parse().ok()
    }

    fn add_tag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.tag_input.read(cx).value();
        if name.trim().is_empty() {
            return;
        }
        if !self.edited(cx).has_content() {
            self.tag_error = Some("请先填写题目、标题或作答，再添加标签".into());
            cx.notify();
            return;
        }
        if let Err(error) = writing::add_tag(&mut self.tags, name.as_ref()) {
            self.tag_error = Some(error.to_string());
            cx.notify();
            return;
        }
        self.tag_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.changed(cx);
    }

    fn remove_tag(&mut self, tag: &str, cx: &mut Context<Self>) {
        self.tags.retain(|name| name != tag);
        self.tag_error = None;
        self.changed(cx);
    }

    fn set_tag_filter(&mut self, filter: Option<String>, cx: &mut Context<Self>) {
        self.tag_filter = filter;
        cx.notify();
    }

    fn filtered_entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries
            .iter()
            .filter(|entry| self.in_group(entry))
            .filter(|entry| {
                self.tag_filter.as_ref().is_none_or(|filter| {
                    entry
                        .tags
                        .iter()
                        .any(|tag| tag.eq_ignore_ascii_case(filter))
                })
            })
    }

    fn in_group(&self, entry: &Entry) -> bool {
        match &self.group_filter {
            GroupFilter::All => true,
            GroupFilter::Ungrouped => entry
                .group_id
                .as_ref()
                .is_none_or(|id| !self.groups.iter().any(|group| &group.id == id)),
            GroupFilter::Group(id) => entry.group_id.as_ref() == Some(id),
        }
    }

    fn draft_in_group(&self) -> Entry {
        let mut draft = Entry::draft();
        if let GroupFilter::Group(id) = &self.group_filter {
            draft.group_id = Some(id.clone());
        }
        draft
    }

    fn switch_group(&mut self, filter: GroupFilter, window: &mut Window, cx: &mut Context<Self>) {
        if self.group_filter == filter || self.flush(cx).is_err() {
            return;
        }
        self.group_filter = filter;
        self.tag_filter = None;
        if !self.in_group(&self.current) {
            let entry = self
                .filtered_entries()
                .next()
                .cloned()
                .unwrap_or_else(|| self.draft_in_group());
            self.load_entry(entry, window, cx);
        }
        cx.notify();
    }

    fn move_to_group(&mut self, group: Option<String>, cx: &mut Context<Self>) {
        if self.group_id == group {
            return;
        }
        let previous = std::mem::replace(&mut self.group_id, group);
        if self.flush(cx).is_err() {
            self.group_id = previous;
            return;
        }
        if self.group_filter != GroupFilter::All {
            self.group_filter = self
                .group_id
                .clone()
                .map_or(GroupFilter::Ungrouped, GroupFilter::Group);
            self.tag_filter = None;
        }
        cx.notify();
    }

    fn open_group_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.creating_group = true;
        let date = chrono::Local::now().format("%Y-%-m-%-d").to_string();
        let mut name = date.clone();
        for index in 2.. {
            if !self.groups.iter().any(|group| group.name == name) {
                break;
            }
            name = format!("{date} ({index})");
        }
        self.group_input.update(cx, |input, cx| {
            input.set_value(name, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn create_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.flush(cx).is_err() {
            return;
        }
        match writing::create_group(&self.lib, self.group_input.read(cx).value().as_ref()) {
            Ok(group) => {
                let filter = GroupFilter::Group(group.id.clone());
                self.groups.insert(0, group);
                self.creating_group = false;
                self.load_error = None;
                self.switch_group(filter, window, cx);
            }
            Err(error) => {
                self.load_error = Some(format!("新建写作分组失败：{error:#}"));
                cx.notify();
            }
        }
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.delete_confirm = None;
        self.autosave.take();
        if self.is_dirty(cx) {
            self.save_status = "尚未保存 · 停止输入后自动保存".into();
            self.save_failed = false;
            self.autosave = Some(cx.spawn(async move |this, cx| {
                Timer::after(Duration::from_millis(650)).await;
                let _ = this.update(cx, |this, cx| {
                    let _ = this.flush(cx);
                });
            }));
        } else {
            self.save_status = if self.entries.iter().any(|entry| entry.id == self.current.id) {
                format!("已保存 · {}", display_time(&self.current.updated_at))
            } else {
                "输入后自动保存".into()
            };
            self.save_failed = false;
        }
        cx.notify();
    }

    /// The caller can prevent navigation/close when this fails; editor contents stay intact.
    pub(super) fn flush(&mut self, cx: &mut Context<Self>) -> Result<()> {
        self.autosave.take();
        if !self.is_dirty(cx) {
            return Ok(());
        }
        let entry = self.edited(cx);
        if !entry.has_content() && !self.entries.iter().any(|saved| saved.id == entry.id) {
            self.save_status = "输入后自动保存".into();
            self.save_failed = false;
            cx.notify();
            return Ok(());
        }
        match writing::save(&self.lib, &entry) {
            Ok(saved) => {
                let saved = ordered_history(saved);
                self.upsert(saved.clone());
                self.current = saved;
                self.save_status = format!("已保存 · {}", display_time(&self.current.updated_at));
                self.save_failed = false;
                cx.notify();
                Ok(())
            }
            Err(error) => {
                self.save_status = format!("保存失败，内容仍保留在编辑框：{error:#}");
                self.save_failed = true;
                cx.notify();
                Err(error)
            }
        }
    }

    pub(super) fn context_key(&self) -> String {
        self.current.id.clone()
    }

    pub(super) fn reference(&self, cx: &App) -> String {
        let entry = self.edited(cx);
        format!(
            "写作题目：\n{}\n\n我的作答：\n{}",
            entry.prompt, entry.answer
        )
    }

    /// Refresh records and feedback after background writes or a backup restore, without
    /// replacing text the user may still be editing.
    pub(super) fn reload(&mut self, cx: &mut Context<Self>) -> Result<()> {
        match writing::list(&self.lib)
            .and_then(|entries| Ok((entries, writing::list_groups(&self.lib)?)))
        {
            Ok((entries, groups)) => {
                let entries = entries.into_iter().map(ordered_history).collect::<Vec<_>>();
                if let Some(saved) = entries.iter().find(|entry| entry.id == self.current.id) {
                    if saved.title != self.current.title
                        || saved.prompt != self.current.prompt
                        || saved.prompt_markdown != self.current.prompt_markdown
                        || saved.answer != self.current.answer
                        || saved.minimum_words != self.current.minimum_words
                        || (self.group_id != self.current.group_id
                            && saved.group_id != self.current.group_id)
                        || (self.tags != self.current.tags && saved.tags != self.current.tags)
                    {
                        self.save_status =
                            "本地记录已更新；当前编辑内容仍保留，修改并保存会写入当前内容".into();
                    }
                    self.current.reviews = saved.reviews.clone();
                    if self.tags == self.current.tags {
                        self.tags.clone_from(&saved.tags);
                        self.current.tags.clone_from(&saved.tags);
                    }
                    if self.group_id == self.current.group_id {
                        self.group_id.clone_from(&saved.group_id);
                        self.current.group_id.clone_from(&saved.group_id);
                    }
                } else if self.entries.iter().any(|entry| entry.id == self.current.id) {
                    self.save_status =
                        "本地列表已更新；当前内容仍保留在编辑框，修改后可重新保存".into();
                }
                self.entries = entries;
                self.groups = groups;
                if matches!(&self.group_filter, GroupFilter::Group(id) if !self.groups.iter().any(|group| &group.id == id))
                {
                    self.group_filter = GroupFilter::All;
                    self.tag_filter = None;
                }
                self.load_error = None;
                cx.notify();
                Ok(())
            }
            Err(error) => {
                self.load_error = Some(format!("刷新写作列表失败：{error:#}"));
                cx.notify();
                Err(error)
            }
        }
    }

    fn upsert(&mut self, saved: Entry) {
        let saved = ordered_history(saved);
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == saved.id) {
            *entry = saved;
        } else {
            self.entries.push(saved);
        }
        self.entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    }

    fn load_entry(&mut self, entry: Entry, window: &mut Window, cx: &mut Context<Self>) {
        let entry = ordered_history(entry);
        self.autosave.take();
        self.prompt_markdown = entry.prompt_markdown;
        self.prompt_preview = entry.prompt_markdown;
        self.tags.clone_from(&entry.tags);
        self.group_id.clone_from(&entry.group_id);
        self.tag_error = None;
        self.tag_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.title.update(cx, |input, cx| {
            input.set_value(entry.title.clone(), window, cx)
        });
        self.prompt.update(cx, |input, cx| {
            input.set_value(entry.prompt.clone(), window, cx)
        });
        self.answer.update(cx, |input, cx| {
            input.set_value(entry.answer.clone(), window, cx)
        });
        self.minimum_words.update(cx, |input, cx| {
            input.set_value(
                entry
                    .minimum_words
                    .map_or_else(String::new, |count| count.to_string()),
                window,
                cx,
            )
        });
        self.selected_review = entry.reviews.first().map(|review| review.id.clone());
        self.show_feedback = false;
        self.show_submission = false;
        self.show_revision_notes = false;
        self.delete_confirm = None;
        self.save_status = if self.entries.iter().any(|saved| saved.id == entry.id) {
            format!("已保存 · {}", display_time(&entry.updated_at))
        } else {
            "输入后自动保存".into()
        };
        self.save_failed = false;
        self.current = entry;
        cx.notify();
    }

    fn select(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.flush(cx).is_err() {
            return;
        }
        match writing::get(&self.lib, id) {
            Ok(Some(entry)) => self.load_entry(entry, window, cx),
            Ok(None) => {
                self.load_error = Some("这篇写作已不存在，请刷新列表".into());
                cx.notify();
            }
            Err(error) => {
                self.load_error = Some(format!("读取写作失败：{error:#}"));
                cx.notify();
            }
        }
    }

    fn new_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.flush(cx).is_ok() {
            self.load_entry(self.draft_in_group(), window, cx);
            self.prompt.update(cx, |input, cx| input.focus(window, cx));
        }
    }

    fn delete_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .correction
            .as_ref()
            .is_some_and(|task| task.entry_id == self.current.id)
        {
            return;
        }
        if self.delete_confirm.as_deref() != Some(self.current.id.as_str()) {
            self.delete_confirm = Some(self.current.id.clone());
            cx.notify();
            return;
        }
        match writing::delete(&self.lib, &self.current.id) {
            Ok(()) => {
                self.autosave.take();
                self.entries.retain(|entry| entry.id != self.current.id);
                let next = self
                    .filtered_entries()
                    .next()
                    .cloned()
                    .unwrap_or_else(|| self.draft_in_group());
                self.load_entry(next, window, cx);
            }
            Err(error) => {
                self.load_error = Some(format!("删除失败，写作内容仍保留：{error:#}"));
                cx.notify();
            }
        }
    }

    fn begin_correction(&mut self, mode: ReviewMode, cx: &mut Context<Self>) {
        if self.correction.is_some() || self.flush(cx).is_err() {
            return;
        }
        if let Err(error) = self.check_correction(cx) {
            self.correction_status = Some((error.to_string(), true));
            cx.notify();
            return;
        }
        let entry_id = self.current.id.clone();
        let title = display_title(&self.edited(cx));
        let cancel = process::cancel_token();
        let lib = self.lib.clone();
        // A queued worker must use the content captured by this click, even after autosave.
        let snapshot = self.edited(cx);
        let task_cancel = cancel.clone();
        let worker = cx.background_executor().spawn(async move {
            writing::review_snapshot_with_mode(&lib, &snapshot, mode, &task_cancel, |_| {})
        });
        self.correction = Some(CorrectionTask {
            entry_id: entry_id.clone(),
            title,
            cancel,
        });
        self.correction_status = None;
        self.correction_task = Some(cx.spawn(async move |this, cx| {
            let result = worker.await;
            let _ = this.update(cx, |this, cx| {
                this.finish_correction(&entry_id, result, cx);
            });
        }));
        cx.notify();
    }

    fn check_correction(&self, cx: &App) -> Result<()> {
        let entry = self.edited(cx);
        if entry.prompt.trim().is_empty() || entry.answer.trim().is_empty() {
            bail!("请先填写写作题目和作答，再提交 AI 批改");
        }
        Ok(())
    }

    fn finish_correction(&mut self, id: &str, result: Result<Review>, cx: &mut Context<Self>) {
        let cancelled = self
            .correction
            .as_ref()
            .is_some_and(|task| task.cancel.load(Ordering::SeqCst));
        let title = self
            .correction
            .as_ref()
            .map(|task| task.title.clone())
            .unwrap_or_default();
        self.correction = None;
        match result {
            Ok(review) => {
                if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) {
                    entry.reviews.retain(|saved| saved.id != review.id);
                    entry.reviews.insert(0, review.clone());
                    entry.updated_at = review.created_at.clone();
                }
                if self.current.id == id {
                    self.current.reviews.retain(|saved| saved.id != review.id);
                    self.current.reviews.insert(0, review.clone());
                    self.current.updated_at = review.created_at.clone();
                    self.selected_review = Some(review.id);
                    self.show_submission = false;
                    self.show_revision_notes = false;
                }
                self.entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
                self.correction_status = Some((format!("“{title}”批改完成，原稿已保留"), false));
            }
            Err(_) if cancelled => {
                self.correction_status = Some((format!("“{title}”的 AI 批改已取消"), false));
            }
            Err(error) => {
                self.correction_status = Some((format!("“{title}”批改失败：{error:#}"), true));
            }
        }
        cx.notify();
    }

    fn cancel_correction(&mut self, cx: &mut Context<Self>) {
        if let Some(task) = &self.correction {
            task.cancel.store(true, Ordering::SeqCst);
        }
        cx.notify();
    }

    fn show_feedback(&mut self, show: bool, cx: &mut Context<Self>) {
        self.show_feedback = show;
        self.show_submission = false;
        self.show_revision_notes = false;
        self.page_scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    fn clipboard_html(&self) -> Option<String> {
        #[cfg(all(debug_assertions, target_os = "macos"))]
        if let Some(board) = &self.test_pasteboard {
            return super::writing_clipboard::html_from_board(board);
        }
        super::writing_clipboard::read_html()
    }

    fn paste_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(markdown) = self
            .clipboard_html()
            .and_then(|html| writing_format::from_html(&html))
        else {
            // The Input handles the normal paste action when there is no usable HTML.
            cx.propagate();
            return;
        };
        let already_markdown = self.prompt_markdown;
        self.prompt.update(cx, |input, cx| {
            let len = input.value().encode_utf16().count();
            let selection = input
                .selected_text_range(false, window, cx)
                .map_or(len..len, |selection| selection.range);
            if selection == (0..len) {
                input.replace(markdown, window, cx);
            } else if already_markdown {
                input.replace(format!("\n\n{markdown}\n\n"), window, cx);
            } else {
                // Keep the untouched parts literal when converting a plain-text draft.
                let before = input
                    .text_for_range(0..selection.start, &mut None, window, cx)
                    .unwrap_or_default();
                let after = input
                    .text_for_range(selection.end..len, &mut None, window, cx)
                    .unwrap_or_default();
                let body = [
                    writing_format::escape_plain(&before),
                    markdown,
                    writing_format::escape_plain(&after),
                ]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
                input.replace_text_in_range(Some(0..len), &body, window, cx);
            }
        });
        self.prompt_markdown = true;
        self.prompt_preview = true;
        cx.stop_propagation();
        cx.notify();
    }

    fn tag_editor(&self, cx: &Context<Self>) -> Div {
        let mut tags = div().flex().flex_wrap().items_center().gap_2();
        for tag in &self.tags {
            let name = tag.clone();
            tags = tags.child(
                Button::new(SharedString::from(format!("writing-remove-tag-{tag}")))
                    .small()
                    .outline()
                    .label(format!("{tag} ×"))
                    .tooltip(format!("移除标签「{tag}」"))
                    .on_click(cx.listener(move |this, _, _, cx| this.remove_tag(&name, cx))),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("题目标签"),
            )
            .when(!self.tags.is_empty(), |field| field.child(tags))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(240.))
                            .max_w_full()
                            .child(Input::new(&self.tag_input)),
                    )
                    .child(
                        Button::new("writing-add-tag")
                            .label("添加标签")
                            .outline()
                            .disabled(self.tag_input.read(cx).value().trim().is_empty())
                            .on_click(cx.listener(|this, _, window, cx| this.add_tag(window, cx))),
                    ),
            )
            .when_some(self.tag_error.clone(), |field, message| {
                field.child(div().text_sm().text_color(cx.theme().danger).child(message))
            })
    }

    fn writing_list_heading(&self, cx: &Context<Self>) -> Div {
        let tags = self
            .entries
            .iter()
            .filter(|entry| self.in_group(entry))
            .flat_map(|entry| &entry.tags)
            .map(|tag| (tag.to_ascii_lowercase(), tag.clone()))
            .collect::<BTreeMap<_, _>>();
        let heading = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("我的写作"),
                    )
                    .child(
                        Button::new("writing-new-group")
                            .small()
                            .outline()
                            .label("＋ 新建组")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_group_form(window, cx)),
                            ),
                    ),
            )
            .child(self.group_picker(false, cx));
        if tags.is_empty() && self.tag_filter.is_none() {
            return heading;
        }
        let filter = self.tag_filter.clone();
        let view = cx.entity().downgrade();
        let label = filter
            .as_ref()
            .map_or_else(|| "按标签筛选".into(), |tag| format!("标签：{tag}"));
        heading.child(
            Button::new("writing-tag-filter")
                .small()
                .outline()
                .w_full()
                .min_w_0()
                .child(div().flex_1().min_w_0().truncate().child(label))
                .dropdown_menu(move |mut menu, _, _| {
                    for option in std::iter::once(None).chain(tags.values().cloned().map(Some)) {
                        let name = option.as_deref().unwrap_or("全部标签");
                        let checked = match (&filter, &option) {
                            (Some(active), Some(tag)) => active.eq_ignore_ascii_case(tag),
                            (None, None) => true,
                            _ => false,
                        };
                        let view = view.clone();
                        menu = menu.item(
                            PopupMenuItem::new(name.to_owned())
                                .checked(checked)
                                .on_click(move |_, _, cx| {
                                    let _ = view.update(cx, |this, cx| {
                                        this.set_tag_filter(option.clone(), cx)
                                    });
                                }),
                        );
                    }
                    menu.max_h(px(300.)).scrollable(true)
                }),
        )
    }

    fn group_picker(&self, moving: bool, cx: &Context<Self>) -> impl IntoElement {
        let current = if moving {
            self.group_id
                .clone()
                .map_or(GroupFilter::Ungrouped, GroupFilter::Group)
        } else {
            self.group_filter.clone()
        };
        let group_name = match &current {
            GroupFilter::All => "全部写作",
            GroupFilter::Ungrouped => "未分组",
            GroupFilter::Group(id) => self
                .groups
                .iter()
                .find(|group| &group.id == id)
                .map_or("未分组", |group| group.name.as_str()),
        };
        let owner = cx.entity().downgrade();
        let groups = self.groups.clone();
        Button::new(if moving {
            "writing-move-group"
        } else {
            "writing-select-group"
        })
        .small()
        .outline()
        .when_else(
            moving,
            |button| {
                button.max_w_full().min_w_0().child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(format!("移动分组 · {group_name}")),
                )
            },
            |button| {
                button.w_full().min_w_0().child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(group_name.to_owned()),
                )
            },
        )
        .dropdown_menu(move |mut menu, _, _| {
            let options = (!moving)
                .then_some((GroupFilter::All, "全部写作"))
                .into_iter()
                .chain([(GroupFilter::Ungrouped, "未分组")])
                .chain(
                    groups
                        .iter()
                        .map(|group| (GroupFilter::Group(group.id.clone()), group.name.as_str())),
                );
            for (filter, name) in options {
                let owner = owner.clone();
                menu = menu.item(
                    PopupMenuItem::new(name.to_owned())
                        .checked(current == filter)
                        .on_click(move |_, window, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                if moving {
                                    let id = match &filter {
                                        GroupFilter::Group(id) => Some(id.clone()),
                                        _ => None,
                                    };
                                    this.move_to_group(id, cx);
                                } else {
                                    this.switch_group(filter.clone(), window, cx);
                                }
                            });
                        }),
                );
            }
            menu.max_h(px(300.)).scrollable(true)
        })
    }

    fn writing_browser(&self, stacked: bool, cx: &Context<Self>) -> Div {
        let mut entries = div()
            .id("writing-saved-list")
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .max_h(px(if stacked { 150. } else { 390. }))
            .overflow_y_scroll();
        let mut count = 0;
        for entry in self.filtered_entries() {
            count += 1;
            let id = entry.id.clone();
            let title = display_title(entry);
            let tags = entry
                .tags
                .iter()
                .map(|tag| format!("#{tag}"))
                .collect::<Vec<_>>()
                .join("  ");
            let status = if entry.reviews.is_empty() {
                "草稿".to_owned()
            } else {
                format!("{} 次批改", entry.reviews.len())
            };
            entries = entries.child(
                Button::new(SharedString::from(format!("writing-entry-{id}")))
                    .ghost()
                    .w_full()
                    .min_w_0()
                    .h(px(if tags.is_empty() { 36. } else { 52. }))
                    .selected(entry.id == self.current.id)
                    .tooltip(title.clone())
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .child(div().truncate().child(title))
                                    .when(!tags.is_empty(), |text| {
                                        text.child(
                                            div()
                                                .truncate()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(tags),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(status),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.select(&id, window, cx))),
            );
        }
        if count == 0 {
            entries = entries.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(if self.tag_filter.is_some() {
                        "这个标签下暂无写作"
                    } else {
                        "当前分组暂无写作"
                    }),
            );
        }
        card(cx)
            .p_3()
            .when(!stacked, |browser| browser.w(px(252.)).flex_shrink_0())
            .child(self.writing_list_heading(cx))
            .when(self.creating_group, |panel| {
                panel.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .w(px(220.))
                                .max_w_full()
                                .child(Input::new(&self.group_input)),
                        )
                        .child(
                            Button::new("writing-create-group")
                                .small()
                                .primary()
                                .label("创建分组")
                                .on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.create_group(window, cx)
                                    }),
                                ),
                        )
                        .child(
                            Button::new("writing-cancel-group")
                                .small()
                                .outline()
                                .label("取消")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.creating_group = false;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when_some(self.load_error.clone(), |browser, error| {
                browser.child(
                    div()
                        .text_sm()
                        .whitespace_normal()
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
            .child(
                div()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .pt_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("文章 · {count} 篇")),
                    )
                    .child(entries)
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                Button::new("writing-new")
                                    .small()
                                    .label("＋ 新建写作")
                                    .primary()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.new_draft(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("writing-reload")
                                    .small()
                                    .label("刷新列表")
                                    .outline()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let _ = this.reload(cx);
                                    })),
                            ),
                    ),
            )
    }

    fn writing_workspace(&self, browser: Div, detail: Div, stacked: bool) -> Div {
        let layout = div()
            .w_full()
            .min_w_0()
            .flex()
            .items_start()
            .gap_4()
            .when(stacked, |layout| layout.flex_col())
            .child(browser)
            .child(detail.when(!stacked, |detail| detail.flex_1()));
        #[cfg(all(debug_assertions, target_os = "macos"))]
        let layout = if self.test_pasteboard.is_some() {
            let bounds = self.test_workspace_bounds.clone();
            layout
                .on_children_prepainted(move |children, _, _| bounds.set(children.try_into().ok()))
        } else {
            layout
        };
        layout
    }

    fn prompt_field(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let caption = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .min_h(EDITOR_HEADER_HEIGHT)
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("写作题目"),
            )
            .when(self.prompt_markdown, |caption| {
                caption.child(
                    Button::new("writing-prompt-mode")
                        .small()
                        .outline()
                        .label(if self.prompt_preview {
                            "编辑题目"
                        } else {
                            "预览格式"
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.prompt_preview = !this.prompt_preview;
                            if !this.prompt_preview {
                                this.prompt.update(cx, |input, cx| input.focus(window, cx));
                            }
                            cx.notify();
                        })),
                )
            });
        let body = if self.prompt_preview {
            div()
                .id("writing-prompt-preview")
                .h(EDITOR_HEIGHT)
                .overflow_y_scroll()
                .p_3()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .child(
                    TextView::markdown(
                        "writing-prompt-markdown",
                        self.prompt.read(cx).value().to_string(),
                        window,
                        cx,
                    )
                    .style(
                        TextViewStyle {
                            heading_base_font_size: BODY_FONT_SIZE,
                            ..Default::default()
                        }
                        .heading_font_size(|level, base| {
                            base + px(match level {
                                1 => 8.,
                                2 => 4.,
                                _ => 0.,
                            })
                        }),
                    )
                    .text_size(BODY_FONT_SIZE)
                    .line_height(relative(BODY_LINE_SPACING))
                    .selectable(true),
                )
                .into_any_element()
        } else {
            div()
                .capture_action(cx.listener(|this, _: &Paste, window, cx| {
                    this.paste_prompt(window, cx);
                }))
                .child(body_input(&self.prompt))
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(caption)
            .child(body)
            .when(!self.prompt_preview, |field| {
                field.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("支持从网页带格式粘贴，保留段落、列表及常用强调样式。"),
                )
            })
    }

    fn review_action(&mut self, action: ReviewAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            ReviewAction::Select(id) => {
                self.selected_review = Some(id);
                self.show_submission = false;
                self.show_revision_notes = false;
            }
            ReviewAction::ToggleSubmission => self.show_submission = !self.show_submission,
            ReviewAction::ToggleNotes => self.show_revision_notes = !self.show_revision_notes,
            ReviewAction::FollowUp(id) => self.follow_up_review(&id, window, cx),
        }
        cx.notify();
    }

    fn follow_up_review(&self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(review) = self.current.reviews.iter().find(|review| review.id == id) else {
            return;
        };
        let context = review.follow_up_context();
        let owner = self.owner.clone();
        window.defer(cx, move |window, cx| {
            let _ = owner.update(cx, |app, cx| quote_review(app, context, window, cx));
        });
    }

    fn correction_history(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        correction_history(
            ReviewHistory {
                reviews: &self.current.reviews,
                selected: self.selected_review.as_deref(),
                show_submission: self.show_submission,
                show_notes: self.show_revision_notes,
            },
            Self::review_action,
            window,
            cx,
        )
    }
}

impl Drop for WritingView {
    fn drop(&mut self) {
        if let Some(task) = &self.correction {
            task.cancel.store(true, Ordering::SeqCst);
        }
    }
}

impl Render for WritingView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dirty = self.is_dirty(cx);
        let saved = self.entries.iter().any(|entry| entry.id == self.current.id);
        let answer = self.answer.read(cx).value();
        let character_count = answer.chars().count();
        let word_count = writing::answer_word_count(answer.as_ref());
        let word_status = match self.minimum_words_value(cx) {
            Some(minimum) if word_count < minimum => {
                format!(
                    "{word_count} / {minimum} 词 · 还差 {} 词",
                    minimum - word_count
                )
            }
            Some(minimum) => format!("{word_count} / {minimum} 词 · 已达标"),
            None => format!("{word_count} 词"),
        };
        let prompt_count = self.prompt.read(cx).value().chars().count();
        let title_count = self.title.read(cx).value().chars().count();
        let running_here = self
            .correction
            .as_ref()
            .is_some_and(|task| task.entry_id == self.current.id);
        let navigation_width = self.owner.upgrade().map_or(px(0.), |owner| {
            px(224. * owner.read(cx).navigation_progress())
        });
        let stacked = window.viewport_size().width - navigation_width - px(64.) < px(800.);
        let browser = self.writing_browser(stacked, cx);
        let mut content = div().w_full().min_w_0().flex().flex_col().gap_4();
        if let Some(task) = &self.correction {
            let cancelling = task.cancel.load(Ordering::SeqCst);
            content = content.child(card(cx).bg(cx.theme().accent)
                .child(div().flex().flex_wrap().items_center().justify_between().gap_3()
                    .child(div().flex_1().min_w(px(160.)).flex().flex_col().gap_1()
                        .child(format!("{}“{}”", if cancelling { "正在取消批改：" } else { "正在批改：" }, task.title))
                        .child(div().text_sm().text_color(cx.theme().muted_foreground)
                            .child("可继续编辑或切换学习模块，反馈将保存到原篇；批改以提交时内容为准。")))
                    .child(Button::new("writing-cancel-review").label(if cancelling { "正在取消…" } else { "取消批改" }).outline().disabled(cancelling)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_correction(cx))))));
        } else if let Some((message, failed)) = &self.correction_status {
            let message = message.clone();
            let failed = *failed;
            let copy_message = message.clone();
            content = content.child(
                card(cx)
                    .text_color(if failed {
                        cx.theme().danger
                    } else {
                        cx.theme().primary
                    })
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap_3()
                            .child(div().flex_1().min_w_0().child(message))
                            .when(failed, |row| {
                                row.child(
                                    Button::new("writing-copy-correction-error")
                                        .small()
                                        .outline()
                                        .label("复制错误")
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                copy_message.clone(),
                                            ));
                                        })),
                                )
                            }),
                    ),
            );
        }
        if !self.current.reviews.is_empty() {
            content = content.child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("writing-show-draft")
                            .label("原稿")
                            .outline()
                            .selected(!self.show_feedback)
                            .on_click(cx.listener(|this, _, _, cx| this.show_feedback(false, cx))),
                    )
                    .child(
                        Button::new("writing-show-feedback")
                            .label("批改结果")
                            .outline()
                            .selected(self.show_feedback)
                            .on_click(cx.listener(|this, _, _, cx| this.show_feedback(true, cx))),
                    ),
            );
            if self.show_feedback {
                return self.writing_workspace(
                    browser,
                    content.child(self.correction_history(window, cx)),
                    stacked,
                );
            }
        }
        // Flex wrapping uses the editor's available width, including when a sidebar is open.
        let editors = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_wrap()
            .items_start()
            .gap_4()
            .child(
                editor_column().child(self.prompt_field(window, cx)).child(
                    div()
                        .text_xs()
                        .text_color(if prompt_count > writing::MAX_PROMPT_CHARS {
                            cx.theme().danger
                        } else {
                            cx.theme().muted_foreground
                        })
                        .child(format!(
                            "题目 {prompt_count} / {} 字符",
                            writing::MAX_PROMPT_CHARS
                        )),
                ),
            )
            .child(
                editor_column()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .min_h(EDITOR_HEADER_HEIGHT)
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("我的作答")
                            .child(
                                div()
                                    .id("writing-answer-word-count")
                                    .rounded_md()
                                    .bg(cx.theme().accent)
                                    .px_2()
                                    .py_0p5()
                                    .child(word_status),
                            ),
                    )
                    .child(body_input(&self.answer))
                    .child(
                        div()
                            .text_xs()
                            .text_color(if character_count > writing::MAX_ANSWER_CHARS {
                                cx.theme().danger
                            } else {
                                cx.theme().muted_foreground
                            })
                            .child(format!(
                                "作答 {character_count} / {} 字符",
                                writing::MAX_ANSWER_CHARS
                            )),
                    ),
            );
        #[cfg(all(debug_assertions, target_os = "macos"))]
        let editors = if self.test_pasteboard.is_some() {
            let bounds = self.test_column_bounds.clone();
            editors
                .on_children_prepainted(move |children, _, _| bounds.set(children.try_into().ok()))
        } else {
            editors
        };
        let mut editor = card(cx)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(if saved { "编辑写作" } else { "新草稿" }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .when(saved, |controls| {
                                controls.child(self.group_picker(true, cx))
                            })
                            .child(
                                Button::new("writing-delete")
                                    .label("删除这篇")
                                    .ghost()
                                    .disabled(!saved || running_here)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.delete_current(window, cx)
                                    })),
                            ),
                    ),
            )
            .child(field("标题（可选）", Input::new(&self.title)))
            .child(
                div()
                    .text_xs()
                    .text_color(if title_count > writing::MAX_TITLE_CHARS {
                        cx.theme().danger
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(format!(
                        "标题 {title_count} / {} 字符",
                        writing::MAX_TITLE_CHARS
                    )),
            )
            .child(self.tag_editor(cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("最低词数"),
                    )
                    .child(div().w(px(120.)).child(Input::new(&self.minimum_words)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("留空表示不设目标；低于目标时，AI 批改会说明差额并教你如何扩充"),
                    ),
            )
            .child(editors)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new("writing-save")
                            .label("保存写作")
                            .outline()
                            .on_click(cx.listener(|this, _, _, cx| {
                                let _ = this.flush(cx);
                            })),
                    )
                    .child(
                        Button::new("writing-review")
                            .label("综合批改")
                            .primary()
                            .disabled(self.correction.is_some())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.begin_correction(ReviewMode::Overview, cx)
                            })),
                    )
                    .child(
                        Button::new("writing-review-edits")
                            .label("逐处修改")
                            .outline()
                            .disabled(self.correction.is_some())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.begin_correction(ReviewMode::LineEdits, cx)
                            })),
                    )
                    .when(!self.current.reviews.is_empty(), |bar| {
                        bar.child(
                            Button::new("writing-open-feedback")
                                .label("查看批改结果")
                                .outline()
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.show_feedback(true, cx)),
                                ),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(140.))
                            .text_sm()
                            .text_color(if self.save_failed {
                                cx.theme().danger
                            } else if dirty {
                                cx.theme().muted_foreground
                            } else {
                                cx.theme().primary
                            })
                            .child(self.save_status.clone()),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("点击 AI 批改会把本次题目和作答发送至「设置」中配置的大模型服务；批改结果可用“追问 AI”继续提问。"),
            );
        if self.delete_confirm.as_deref() == Some(self.current.id.as_str()) {
            editor = editor.child(
                div()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().danger)
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(format!(
                        "确认永久删除“{}”及其 {} 条批改记录？当前未保存的编辑也会丢弃。",
                        display_title(&self.edited(cx)),
                        self.current.reviews.len()
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                Button::new("writing-confirm-delete")
                                    .label("确认删除这篇及批改历史")
                                    .danger()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.delete_current(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("writing-keep-entry")
                                    .label("保留写作")
                                    .outline()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.delete_confirm = None;
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        }
        self.writing_workspace(browser, content.child(editor), stacked)
    }
}

pub(super) enum ReviewAction {
    Select(String),
    FollowUp(String),
    ToggleSubmission,
    ToggleNotes,
}

pub(super) struct ReviewHistory<'a> {
    pub reviews: &'a [Review],
    pub selected: Option<&'a str>,
    pub show_submission: bool,
    pub show_notes: bool,
}

pub(super) fn quote_review(
    app: &mut AppView,
    context: Result<String>,
    window: &mut Window,
    cx: &mut Context<AppView>,
) {
    match context {
        Ok(text) => app.handle(
            Action::AssistantQuote(clarora_gpui::assistant::Reference {
                text,
                ..Default::default()
            }),
            window,
            cx,
        ),
        Err(error) => {
            app.report_status(FeedbackKind::Error, format!("无法引用本次批改：{error}"));
            cx.notify();
        }
    }
}

enum MarkdownChunk {
    Text(String),
    Table {
        header: Vec<String>,
        rows: Vec<Vec<String>>,
    },
}

fn is_table_separator(line: &str) -> bool {
    let line = line.trim();
    line.contains('-')
        && line.contains('|')
        && line.chars().all(|ch| matches!(ch, '|' | '-' | ':' | ' '))
}

fn table_cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_owned())
        .collect()
}

fn markdown_chunks(text: &str) -> Vec<MarkdownChunk> {
    let lines = text.lines().collect::<Vec<_>>();
    let mut chunks = Vec::new();
    let mut text_start = 0;
    let mut index = 0;
    let mut fenced = false;
    while index < lines.len() {
        let trimmed = lines[index].trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            index += 1;
            continue;
        }
        if !fenced
            && index + 1 < lines.len()
            && lines[index].contains('|')
            && is_table_separator(lines[index + 1])
        {
            if text_start < index {
                chunks.push(MarkdownChunk::Text(lines[text_start..index].join("\n")));
            }
            let mut header = table_cells(lines[index]);
            let mut rows = Vec::new();
            let mut row = index + 2;
            while row < lines.len() && !lines[row].trim().is_empty() && lines[row].contains('|') {
                rows.push(table_cells(lines[row]));
                row += 1;
            }
            let columns = header
                .len()
                .max(rows.iter().map(Vec::len).max().unwrap_or(0));
            header.resize(columns, String::new());
            for row in &mut rows {
                row.resize(columns, String::new());
            }
            if columns > 0 {
                chunks.push(MarkdownChunk::Table { header, rows });
            }
            index = row;
            text_start = index;
            continue;
        }
        index += 1;
    }
    if text_start < lines.len() {
        chunks.push(MarkdownChunk::Text(lines[text_start..].join("\n")));
    }
    chunks
}

fn markdown_table_row<T>(cells: &[String], header: bool, last: bool, cx: &Context<T>) -> Div {
    let columns = cells.len();
    let mut row = div()
        .w_full()
        .min_w_0()
        .flex()
        .when(header, |row| row.bg(cx.theme().accent))
        .when(!last, |row| {
            row.border_b_1().border_color(cx.theme().border)
        });
    for (index, cell) in cells.iter().enumerate() {
        row = row.child(
            div()
                .flex_1()
                .min_w_0()
                .px_3()
                .py_2()
                .text_sm()
                .whitespace_normal()
                .when(header, |cell| cell.font_weight(FontWeight::SEMIBOLD))
                .when(index + 1 < columns, |cell| {
                    cell.border_r_1().border_color(cx.theme().border)
                })
                .child(cell.clone()),
        );
    }
    row
}

/// Render Markdown while replacing tables with wrapping flex rows. The upstream
/// Markdown table cell widths overflow narrow review columns and clip content.
fn markdown_with_tables<T>(
    id_prefix: &str,
    text: &str,
    window: &mut Window,
    cx: &mut Context<T>,
) -> AnyElement {
    let mut content = div().w_full().min_w_0().flex().flex_col().gap_3();
    for (index, chunk) in markdown_chunks(text).into_iter().enumerate() {
        match chunk {
            MarkdownChunk::Text(text) => {
                if text.trim().is_empty() {
                    continue;
                }
                content = content.child(
                    TextView::markdown(
                        SharedString::from(format!("{id_prefix}-text-{index}")),
                        text,
                        window,
                        cx,
                    )
                    .selectable(true),
                );
            }
            MarkdownChunk::Table { header, rows } => {
                let mut table = div()
                    .w_full()
                    .min_w_0()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .overflow_hidden();
                let has_rows = !rows.is_empty();
                if !header.is_empty() {
                    table = table.child(markdown_table_row(&header, true, !has_rows, cx));
                }
                let count = rows.len();
                for (row_index, row) in rows.into_iter().enumerate() {
                    table =
                        table.child(markdown_table_row(&row, false, row_index + 1 == count, cx));
                }
                content = content.child(table);
            }
        }
    }
    content.into_any_element()
}

pub(super) fn correction_history<T: 'static>(
    history: ReviewHistory<'_>,
    on_action: fn(&mut T, ReviewAction, &mut Window, &mut Context<T>),
    window: &mut Window,
    cx: &mut Context<T>,
) -> AnyElement {
    let mut panel = card(cx).child(
        div()
            .text_lg()
            .font_weight(FontWeight::SEMIBOLD)
            .child("批改历史"),
    );
    if history.reviews.is_empty() {
        return panel
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("还没有批改记录。保存题目与作答后，可提交给设置中的大模型。"),
            )
            .into_any_element();
    }
    let mut tabs = div().flex().flex_wrap().gap_2();
    for (index, review) in history.reviews.iter().enumerate() {
        let id = review.id.clone();
        tabs = tabs.child(
            Button::new(SharedString::from(format!("writing-review-{id}")))
                .small()
                .outline()
                .selected(history.selected == Some(review.id.as_str()))
                .label(format!(
                    "{} · {} · {}",
                    history.reviews.len() - index,
                    review.mode().label(),
                    display_time(&review.created_at)
                ))
                .on_click(cx.listener(move |this, _, window, cx| {
                    on_action(this, ReviewAction::Select(id.clone()), window, cx);
                })),
        );
    }
    panel = panel.child(tabs);
    if let Some(review) = history
        .reviews
        .iter()
        .find(|review| history.selected == Some(review.id.as_str()))
        .or_else(|| history.reviews.first())
    {
        let review_id = review.id.clone();
        panel = panel
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{} · 提交时模型设置：{}",
                        display_time(&review.created_at),
                        review.model
                    )),
            )
            .child(
                Button::new("writing-review-follow-up")
                    .primary()
                    .small()
                    .label("追问 AI")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        on_action(this, ReviewAction::FollowUp(review_id.clone()), window, cx)
                    })),
            )
            .child(
                Button::new("writing-review-source")
                    .outline()
                    .small()
                    .label(if history.show_submission {
                        "收起提交原文"
                    } else {
                        "查看本次提交原文"
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        on_action(this, ReviewAction::ToggleSubmission, window, cx);
                    })),
            )
            .when(history.show_submission, |panel| {
                panel.child(
                    card(cx)
                        .bg(cx.theme().muted)
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("本次批改的题目与作答 · 历史快照"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("反馈对应以下提交内容，当前编辑框的内容可能已经更新。"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("提交的题目"),
                        )
                        .child(if review.prompt_markdown {
                            TextView::markdown(
                                SharedString::from(format!("writing-review-prompt-{}", review.id)),
                                review.prompt.clone(),
                                window,
                                cx,
                            )
                            .selectable(true)
                            .into_any_element()
                        } else {
                            div()
                                .text_sm()
                                .whitespace_normal()
                                .child(review.prompt.clone())
                                .into_any_element()
                        })
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("提交的作答"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .whitespace_normal()
                                .child(review.answer.clone()),
                        ),
                )
            })
            .child(if review.edits.is_some() {
                revision_result(review, history.show_notes, on_action, cx).into_any_element()
            } else {
                markdown_with_tables(
                    &format!("writing-feedback-{}", review.id),
                    &review.feedback,
                    window,
                    cx,
                )
            });
    }
    panel.into_any_element()
}
fn revision_result<T: 'static>(
    review: &Review,
    show_notes: bool,
    on_action: fn(&mut T, ReviewAction, &mut Window, &mut Context<T>),
    cx: &Context<T>,
) -> Div {
    let result = div()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_3()
        .child(div().whitespace_normal().child(review.feedback.clone()));
    let (annotations, preview) = match (review.revision_annotations(), review.revision_preview()) {
        (Ok(annotations), Ok(Some(preview))) => (annotations, preview),
        (Err(error), _) | (_, Err(error)) => {
            return result.child(div().text_color(cx.theme().danger).child(error.to_string()));
        }
        _ => return result,
    };
    let ink = cx.theme().danger;
    let accent = cx.theme().primary;
    let deletion = HighlightStyle {
        color: Some(ink),
        strikethrough: Some(StrikethroughStyle {
            thickness: px(1.5),
            color: Some(ink),
        }),
        ..Default::default()
    };
    let insertion = HighlightStyle {
        color: Some(accent),
        underline: Some(UnderlineStyle {
            thickness: px(1.),
            color: Some(accent),
            wavy: false,
        }),
        ..Default::default()
    };
    let mut paper = String::new();
    let mut marks = Vec::new();
    let mut notes = Vec::new();
    let mut cursor = 0;
    for (index, (range, edit)) in annotations.iter().enumerate() {
        paper.push_str(&review.answer[cursor..range.start]);
        let mut local = 0;
        for (removed, inserted) in writing::revision_changes(&edit.original, &edit.replacement) {
            paper.push_str(&edit.original[local..removed.start]);
            if !removed.is_empty() {
                let start = paper.len();
                paper.push_str(&edit.original[removed.clone()]);
                marks.push((start..paper.len(), deletion));
            }
            if !inserted.is_empty() {
                paper.push('\u{2009}');
                let start = paper.len();
                paper.push_str(inserted);
                marks.push((start..paper.len(), insertion));
                paper.push('\u{2009}');
            }
            local = removed.end;
        }
        paper.push_str(&edit.original[local..]);
        paper.push('\u{202f}');
        let start = paper.len();
        paper.push_str(&format!("[{}]", index + 1));
        let marker = start..paper.len();
        marks.push((marker.clone(), insertion));
        notes.push((marker, format!("{} · {}", index + 1, edit.reason)));
        cursor = range.end;
    }
    paper.push_str(&review.answer[cursor..]);
    let text = StyledText::new(paper).with_highlights(marks);
    let paper = InteractiveText::new(
        SharedString::from(format!("writing-ink-{}", review.id)),
        text,
    )
    .tooltip(move |index, window, cx| {
        let (_, note) = notes.iter().find(|(range, _)| range.contains(&index))?;
        Some(Tooltip::new(note.clone()).max_w(px(360.)).build(window, cx))
    });
    let paper = card(cx)
        .child(div().font_weight(FontWeight::SEMIBOLD).child("原文批改"))
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(if annotations.is_empty() {
                    "本次没有建议修改的位置"
                } else {
                    "删除线为删改原文，下划线为建议文字。悬停编号查看批注。"
                }),
        )
        .child(
            div()
                .text_size(px(20.))
                .line_height(relative(1.65))
                .whitespace_normal()
                .child(paper),
        );
    result.child(paper).when(!annotations.is_empty(), |result| {
        result
            .child(
                Button::new("writing-revision-notes")
                    .outline()
                    .small()
                    .label(if show_notes {
                        "收起批注"
                    } else {
                        "展开批注"
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        on_action(this, ReviewAction::ToggleNotes, window, cx);
                    })),
            )
            .when(show_notes, |result| {
                result.child(card(cx).children(annotations.iter().enumerate().map(
                    |(index, (_, edit))| {
                        div()
                            .whitespace_normal()
                            .child(format!("{} · {}", index + 1, edit.reason))
                    },
                )))
            })
            .child(revision_text("完整建议稿 · 预览", &preview, cx))
    })
}

fn revision_text(label: &'static str, text: &str, cx: &App) -> Div {
    div()
        .min_w_0()
        .max_w_full()
        .flex()
        .flex_col()
        .gap_1()
        .p_3()
        .rounded_md()
        .bg(cx.theme().muted)
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(label),
        )
        .child(div().whitespace_normal().child(text.to_owned()))
}

fn card(cx: &App) -> Div {
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_3()
        .p_5()
        .rounded_2xl()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
}

fn editor_column() -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .min_w_0()
        .max_w_full()
        .flex_basis(EDITOR_COLUMN_WIDTH)
        .flex_grow()
        .flex_shrink_0()
}

fn body_input(state: &Entity<InputState>) -> Input {
    Input::new(state)
        .h(EDITOR_HEIGHT)
        .w_full()
        .flex_shrink_0()
        .text_size(BODY_FONT_SIZE)
        .line_height(relative(BODY_LINE_SPACING))
}

fn field(label: &'static str, input: Input) -> Div {
    div()
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(label),
        )
        .child(input)
}

fn display_title(entry: &Entry) -> String {
    let text = if entry.title.trim().is_empty() {
        entry
            .prompt
            .trim()
            .lines()
            .next()
            .map(|line| {
                if entry.prompt_markdown {
                    let heading = line.trim_start_matches('#');
                    heading.strip_prefix(' ').unwrap_or(line)
                } else {
                    line
                }
            })
            .filter(|line| !line.is_empty())
            .unwrap_or("未命名写作")
    } else {
        entry.title.trim()
    };
    let shortened = text.chars().take(30).collect::<String>();
    if text.chars().count() > 30 {
        format!("{shortened}…")
    } else {
        shortened
    }
}

fn display_time(value: &str) -> String {
    chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f")
        .map(|time| {
            time.and_utc()
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|_| value.replace('T', " ").chars().take(19).collect())
}

fn ordered_history(mut entry: Entry) -> Entry {
    entry
        .reviews
        .sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
    let mut seen = std::collections::BTreeSet::new();
    entry
        .reviews
        .retain(|review| seen.insert(review.id.clone()));
    entry
}

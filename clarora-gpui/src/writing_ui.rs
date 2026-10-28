//! Writing drafts and corrections live in their own entity so navigation does not discard edits.
use anyhow::{Result, bail};
use clarora_gpui::{
    data::Library,
    process::{self, Cancel},
    writing::{self, Entry, Review},
};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, Sizable,
    button::*,
    input::{Input, InputEvent, InputState},
    text::TextView,
};
use std::{sync::atomic::Ordering, time::Duration};

struct CorrectionTask {
    entry_id: String,
    title: String,
    cancel: Cancel,
}

pub(super) struct WritingView {
    lib: Library,
    entries: Vec<Entry>,
    current: Entry,
    title: Entity<InputState>,
    prompt: Entity<InputState>,
    answer: Entity<InputState>,
    save_status: String,
    save_failed: bool,
    load_error: Option<String>,
    selected_review: Option<String>,
    delete_confirm: Option<String>,
    correction: Option<CorrectionTask>,
    correction_status: Option<(String, bool)>,
    autosave: Option<Task<()>>,
    correction_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl WritingView {
    pub(super) fn new(lib: Library, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (entries, load_error) = match writing::list(&lib) {
            Ok(entries) => (
                entries.into_iter().map(ordered_history).collect::<Vec<_>>(),
                None,
            ),
            Err(error) => (vec![], Some(format!("读取写作列表失败：{error:#}"))),
        };
        let current = entries.first().cloned().unwrap_or_else(Entry::draft);
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
        let mut subscriptions = vec![];
        for input in [&title, &prompt, &answer] {
            subscriptions.push(cx.subscribe(input, |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.changed(cx);
                }
            }));
        }
        let save_status = if entries.is_empty() {
            "输入后自动保存".into()
        } else {
            format!("已保存 · {}", display_time(&current.updated_at))
        };
        Self {
            lib,
            entries,
            selected_review: current.reviews.first().map(|review| review.id.clone()),
            current,
            title,
            prompt,
            answer,
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
        entry.answer = self.answer.read(cx).value().to_string();
        entry
    }

    fn is_dirty(&self, cx: &App) -> bool {
        self.title.read(cx).value().as_ref() != self.current.title
            || self.prompt.read(cx).value().as_ref() != self.current.prompt
            || self.answer.read(cx).value().as_ref() != self.current.answer
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
        if entry.title.trim().is_empty()
            && entry.prompt.trim().is_empty()
            && entry.answer.trim().is_empty()
            && !self.entries.iter().any(|saved| saved.id == entry.id)
        {
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
        match writing::list(&self.lib) {
            Ok(entries) => {
                let entries = entries.into_iter().map(ordered_history).collect::<Vec<_>>();
                if let Some(saved) = entries.iter().find(|entry| entry.id == self.current.id) {
                    if saved.title != self.current.title
                        || saved.prompt != self.current.prompt
                        || saved.answer != self.current.answer
                    {
                        self.save_status =
                            "本地记录已更新；当前编辑内容仍保留，修改并保存会写入当前内容".into();
                    }
                    self.current.reviews = saved.reviews.clone();
                } else if self.entries.iter().any(|entry| entry.id == self.current.id) {
                    self.save_status =
                        "本地列表已更新；当前内容仍保留在编辑框，修改后可重新保存".into();
                }
                self.entries = entries;
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
        self.title.update(cx, |input, cx| {
            input.set_value(entry.title.clone(), window, cx)
        });
        self.prompt.update(cx, |input, cx| {
            input.set_value(entry.prompt.clone(), window, cx)
        });
        self.answer.update(cx, |input, cx| {
            input.set_value(entry.answer.clone(), window, cx)
        });
        self.selected_review = entry.reviews.first().map(|review| review.id.clone());
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
            self.load_entry(Entry::draft(), window, cx);
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
                let next = self.entries.first().cloned().unwrap_or_else(Entry::draft);
                self.load_entry(next, window, cx);
            }
            Err(error) => {
                self.load_error = Some(format!("删除失败，写作内容仍保留：{error:#}"));
                cx.notify();
            }
        }
    }

    fn begin_correction(&mut self, cx: &mut Context<Self>) {
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
        let worker = cx
            .background_executor()
            .spawn(async move { writing::review_snapshot(&lib, &snapshot, &task_cancel, |_| {}) });
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
                }
                self.entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
                self.correction_status =
                    Some((format!("“{title}”批改完成，反馈已保存至原篇历史"), false));
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

    fn correction_history(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = card(cx).child(
            div()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .child("批改历史"),
        );
        if self.current.reviews.is_empty() {
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
        for (index, review) in self.current.reviews.iter().enumerate() {
            let id = review.id.clone();
            tabs = tabs.child(
                Button::new(SharedString::from(format!("writing-review-{id}")))
                    .small()
                    .outline()
                    .selected(self.selected_review.as_deref() == Some(review.id.as_str()))
                    .label(format!(
                        "{} · {}",
                        self.current.reviews.len() - index,
                        display_time(&review.created_at)
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected_review = Some(id.clone());
                        cx.notify();
                    })),
            );
        }
        panel = panel.child(tabs);
        if let Some(review) = self
            .current
            .reviews
            .iter()
            .find(|review| self.selected_review.as_deref() == Some(review.id.as_str()))
            .or_else(|| self.current.reviews.first())
        {
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
                    TextView::markdown(
                        SharedString::from(format!("writing-feedback-{}", review.id)),
                        review.feedback.clone(),
                        window,
                        cx,
                    )
                    .selectable(true),
                )
                .child(
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
                        .child(
                            div()
                                .text_sm()
                                .whitespace_normal()
                                .child(review.prompt.clone()),
                        )
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
                );
        }
        panel.into_any_element()
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
        let word_count = answer.split_whitespace().count();
        let prompt_count = self.prompt.read(cx).value().chars().count();
        let title_count = self.title.read(cx).value().chars().count();
        let running_here = self
            .correction
            .as_ref()
            .is_some_and(|task| task.entry_id == self.current.id);
        let mut content = div().w_full().min_w_0().flex().flex_col().gap_4()
            .child(card(cx)
                .child(div().flex().flex_wrap().items_center().justify_between().gap_3()
                    .child(div().flex().flex_col().gap_1()
                        .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child("写作练习"))
                        .child(div().text_sm().text_color(cx.theme().muted_foreground).child("保存题目与作答，再交给大模型批改；每次批改保留提交快照。")))
                    .child(div().flex().flex_wrap().gap_2()
                        .child(Button::new("writing-new").label("新建写作").primary()
                            .on_click(cx.listener(|this, _, window, cx| this.new_draft(window, cx))))
                        .child(Button::new("writing-reload").label("刷新列表").outline()
                            .on_click(cx.listener(|this, _, _, cx| { let _ = this.reload(cx); })))))
                .child(div().text_sm().text_color(cx.theme().muted_foreground)
                    .child(format!("已保存 {} 篇 · 草稿自动保存在本机", self.entries.len()))));
        if let Some(error) = &self.load_error {
            content = content.child(
                card(cx)
                    .border_color(cx.theme().danger)
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }
        if !self.entries.is_empty() {
            let mut entries = div()
                .id("writing-saved-list")
                .flex()
                .flex_col()
                .gap_2()
                .max_h(px(200.))
                .overflow_y_scroll();
            for entry in &self.entries {
                let id = entry.id.clone();
                entries = entries.child(
                    Button::new(SharedString::from(format!("writing-entry-{id}")))
                        .w_full()
                        .justify_start()
                        .overflow_hidden()
                        .child(div().min_w_0().w_full().truncate().child(format!(
                            "{}{}",
                            display_title(entry),
                            if entry.reviews.is_empty() {
                                String::new()
                            } else {
                                format!(" · {} 次批改", entry.reviews.len())
                            }
                        )))
                        .small()
                        .outline()
                        .selected(entry.id == self.current.id)
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.select(&id, window, cx)),
                        ),
                );
            }
            content = content.child(
                card(cx)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("我的写作"),
                    )
                    .child(entries),
            );
        }
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
            content = content.child(
                card(cx)
                    .text_color(if *failed {
                        cx.theme().danger
                    } else {
                        cx.theme().primary
                    })
                    .child(message.clone()),
            );
        }
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
                        Button::new("writing-delete")
                            .label("删除这篇")
                            .ghost()
                            .disabled(!saved || running_here)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.delete_current(window, cx)),
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
            // In fixed multi-line mode, Input owns the viewport height; rows()
            // only sets a minimum height for AutoGrow mode in gpui-component.
            .child(field(
                "写作题目",
                Input::new(&self.prompt)
                    .h(px(220.))
                    .w_full()
                    .flex_shrink_0(),
            ))
            .child(
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
            )
            .child(field(
                "我的作答",
                Input::new(&self.answer)
                    .h(px(440.))
                    .w_full()
                    .flex_shrink_0(),
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(if character_count > writing::MAX_ANSWER_CHARS {
                        cx.theme().danger
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(format!(
                        "作答 {character_count} / {} 字符 · 空格分词 {word_count} 词",
                        writing::MAX_ANSWER_CHARS
                    )),
            )
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
                            .label("AI 批改")
                            .primary()
                            .disabled(self.correction.is_some())
                            .on_click(cx.listener(|this, _, _, cx| this.begin_correction(cx))),
                    )
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
                    .child("点击 AI 批改会把本次题目和作答发送至「设置」中配置的大模型服务。"),
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
        content
            .child(editor)
            .child(self.correction_history(window, cx))
    }
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

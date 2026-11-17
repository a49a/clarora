use super::*;

impl AppView {
    pub(super) fn open_video_import_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        ensure!(self.page == "video", "请先进入视频学习");
        ensure!(
            !self.tasks.foreground.is_running(),
            "请完成或取消当前任务后再导入视频"
        );
        let draft = self
            .video
            .import_edit
            .take()
            .or_else(|| self.video.import_recovery.clone());
        let group = draft
            .as_ref()
            .map(|draft| match draft.group.as_str() {
                "" | video_library::UNGROUPED => "未分组".into(),
                group => group.to_owned(),
            })
            .unwrap_or_else(|| match self.video.group.as_str() {
                "" | video_library::UNGROUPED => "未分组".into(),
                group => group.to_owned(),
            });
        let owner = cx.entity().downgrade();
        let import = cx.new(|cx| VideoImport::new(owner, group, draft, window, cx));
        let input = import.read(cx).input.clone();
        window.open_dialog(cx, move |dialog, window, _| {
            let confirm = import.clone();
            let viewport = window.viewport_size();
            let top = ((viewport.height - px(360.)) / 2.).max(px(8.));
            dialog
                .title(div().text_lg().child("导入视频"))
                .w((viewport.width - px(32.)).clamp(px(1.), px(560.)))
                .margin_top(top)
                .max_h((viewport.height - top - px(42.)).max(px(96.)))
                .overlay_closable(false)
                .child(import.clone())
                .on_ok(move |_, window, cx| {
                    confirm.update(cx, |this, cx| {
                        if !this.input.read(cx).focus_handle(cx).is_focused(window) {
                            // A focused button handles Enter on key-up.
                            return false;
                        }
                        this.submit(window, cx)
                    })
                })
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        Ok(())
    }
}

struct VideoImport {
    owner: WeakEntity<AppView>,
    group: String,
    input: Entity<InputState>,
    error: Option<String>,
    parts: Option<bilibili::Series>,
    parts_cache: BTreeMap<String, bilibili::Series>,
    parts_loading: bool,
    parts_error: Option<String>,
    selected: Option<usize>,
    selected_for: Option<String>,
    selection_touched: bool,
    request: u64,
    cancel: Option<Cancel>,
    parts_task: Option<Task<()>>,
    _edit: Subscription,
}

impl VideoImport {
    fn new(
        owner: WeakEntity<AppView>,
        group: String,
        draft: Option<app_state::VideoImportDraft>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let link = draft
            .as_ref()
            .map(|draft| draft.raw.clone())
            .unwrap_or_default();
        let selected = draft.as_ref().and_then(|draft| draft.part);
        let selected_for = draft
            .as_ref()
            .and_then(|draft| draft.normalized.as_deref())
            .and_then(bilibili_base);
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("粘贴视频链接；多个视频可每行一个，也支持 B 站 BV / av 编号")
                .auto_grow(3, 8)
        });
        if !link.is_empty() {
            input.update(cx, |state, cx| state.set_value(&link, window, cx));
        }
        let edit = cx.subscribe(&input, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.error = None;
                this.parts_error = None;
                this.selection_touched = false;
                this.invalidate_parts();
                this.sync_cached(cx);
                cx.notify();
            }
        });
        Self {
            owner,
            group,
            input,
            error: None,
            parts: None,
            parts_cache: BTreeMap::new(),
            parts_loading: false,
            parts_error: None,
            selected,
            selected_for,
            selection_touched: selected.is_some(),
            request: 0,
            cancel: None,
            parts_task: None,
            _edit: edit,
        }
    }

    fn target_group(&self) -> String {
        if self.group == "未分组" {
            String::new()
        } else {
            self.group.clone()
        }
    }

    fn links(&self, cx: &Context<Self>) -> Vec<String> {
        self.input
            .read(cx)
            .value()
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }

    fn invalidate_parts(&mut self) {
        self.request = self.request.wrapping_add(1);
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        self.parts_task.take();
        self.parts_loading = false;
    }

    fn sync_cached(&mut self, cx: &mut Context<Self>) {
        let link = self.input.read(cx).value();
        let current = process::normalize_video_link(&link)
            .ok()
            .and_then(|link| match link {
                process::VideoLink::Bilibili(url) => {
                    Some((bilibili_base(&url)?, bilibili_requested(&url)))
                }
                process::VideoLink::External(..) => None,
            });
        match current {
            Some((base, requested)) => {
                if self
                    .parts
                    .as_ref()
                    .is_some_and(|series| series.base == base)
                {
                    if !self.selection_touched {
                        let count = self
                            .parts
                            .as_ref()
                            .map_or(0, |series| series.parts.len().saturating_sub(1));
                        self.selected = Some(requested.unwrap_or(0).min(count));
                    }
                    self.selected_for = Some(base);
                    return;
                }
                if let Some(series) = self.parts_cache.get(&base).cloned() {
                    let count = series.parts.len();
                    self.show_parts(series);
                    if !self.selection_touched {
                        self.selected = requested
                            .or(self.selected)
                            .map(|index| index.min(count.saturating_sub(1)));
                    }
                } else if !self.parts_loading {
                    self.parts = None;
                    self.selected = requested;
                    self.selected_for = Some(base);
                }
            }
            None => {
                if !self.parts_loading {
                    self.parts = None;
                    self.selected = None;
                    self.selected_for = None;
                }
            }
        }
    }

    fn show_parts(&mut self, series: bilibili::Series) {
        self.selected_for = Some(series.base.clone());
        if !self.selection_touched || self.selected.is_none() {
            self.selected = Some(series.requested.min(series.parts.len().saturating_sub(1)));
        }
        self.parts = Some(series);
    }

    fn read_parts(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        if self.links(cx).len() > 1 {
            self.parts = None;
            self.parts_loading = false;
            self.parts_error =
                Some("多个链接请在每个 B 站链接里用 ?p= 指定分P；此处只读取单条链接的选集".into());
            cx.notify();
            return;
        }
        let link = self.input.read(cx).value();
        let url = match process::normalize_video_link(&link) {
            Ok(process::VideoLink::Bilibili(url)) => url,
            Ok(process::VideoLink::External(source, _)) => {
                self.parts_error =
                    Some(format!("{} 视频不支持读取选集，请直接下载", source.label()));
                self.parts = None;
                self.parts_loading = false;
                cx.notify();
                return;
            }
            Err(error) => {
                self.parts_error = Some(error.to_string());
                self.parts = None;
                self.parts_loading = false;
                cx.notify();
                return;
            }
        };
        let Some(base) = bilibili_base(&url) else {
            self.parts_error = Some("无法识别 bilibili 稿件".into());
            return;
        };
        if let Some(series) = self.parts_cache.get(&base).cloned() {
            let count = series.parts.len();
            self.parts_error = None;
            self.parts_loading = false;
            self.show_parts(series);
            if !self.selection_touched {
                self.selected = Some(
                    bilibili_requested(&url)
                        .unwrap_or(0)
                        .min(count.saturating_sub(1)),
                );
            }
            cx.notify();
            return;
        }
        self.request = self.request.wrapping_add(1);
        let request = self.request;
        self.parts_loading = true;
        self.parts_error = None;
        self.parts = None;
        self.selected = None;
        self.selected_for = None;
        let cancel = process::cancel_token();
        self.cancel = Some(cancel.clone());
        let fetch_url = url.clone();
        let worker = cx
            .background_executor()
            .spawn(async move { bilibili::fetch(&fetch_url, &cancel) });
        self.parts_task = Some(cx.spawn(async move |this, cx| {
            let result = worker.await;
            let _ = this.update(cx, |this, cx| {
                this.apply_parts(request, base, result, cx);
            });
        }));
        cx.notify();
    }

    fn apply_parts(
        &mut self,
        request: u64,
        base: String,
        result: Result<bilibili::Series>,
        cx: &mut Context<Self>,
    ) {
        if let Ok(series) = &result {
            self.parts_cache.insert(base.clone(), series.clone());
        }
        if request != self.request {
            // A superseded callback may fill the cache but must not replace
            // content for the newer link the user is editing.
            if !self.parts_loading {
                self.sync_cached(cx);
            }
            return;
        }
        self.parts_loading = false;
        match result {
            Ok(series) => {
                self.parts_error = None;
                self.show_parts(series);
            }
            Err(error) if error.to_string() == "已取消" => {}
            Err(error) => {
                self.parts_error = Some(error.to_string());
                self.parts = None;
            }
        }
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.input.update(cx, |input, cx| {
            input.marked_text_range(window, cx).is_some()
        }) {
            return false;
        }
        let links = self.links(cx);
        if links.is_empty() {
            self.error = Some("请先粘贴视频链接或输入 BV / av 编号".into());
            self.input.update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
            return false;
        }
        if links.len() > 1 {
            let group = self.target_group();
            return match self.owner.update(cx, |app, cx| {
                app.feedback.retry_candidate = None;
                let result = app.begin_video_links_import(&links, group);
                if result.is_ok() {
                    app.video.import_recovery = None;
                    app.video.import_edit = None;
                }
                cx.notify();
                result
            }) {
                Ok(Err(error)) => {
                    self.error = Some(error.to_string());
                    self.input.update(cx, |input, cx| input.focus(window, cx));
                    cx.notify();
                    false
                }
                Err(error) => {
                    self.error = Some(error.to_string());
                    cx.notify();
                    false
                }
                Ok(Ok(())) => true,
            };
        }
        let link = links[0].clone();
        let current_base = process::normalize_video_link(&link)
            .ok()
            .and_then(|link| match link {
                process::VideoLink::Bilibili(url) => bilibili_base(&url),
                process::VideoLink::External(..) => None,
            });
        if let Some(index) = self.selected {
            if current_base != self.selected_for {
                self.error = Some("链接已改变，请重新读取选集后再选择分P".into());
                self.input.update(cx, |input, cx| input.focus(window, cx));
                cx.notify();
                return false;
            }
            if let Some(series) = &self.parts
                && index >= series.parts.len()
            {
                self.error = Some("所选分P不存在，请重新读取选集".into());
                cx.notify();
                return false;
            }
        }
        let selected = self.selected;
        match self.owner.update(cx, |app, cx| {
            app.feedback.retry_candidate = None;
            let result = app.begin_video_link_import(&link, false, self.target_group(), selected);
            if result.is_ok() {
                app.video.import_recovery = None;
                app.video.import_edit = None;
            }
            cx.notify();
            result
        }) {
            Ok(Err(error)) => {
                self.error = Some(error.to_string());
                self.input.update(cx, |input, cx| input.focus(window, cx));
                cx.notify();
                false
            }
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                false
            }
            Ok(Ok(())) => true,
        }
    }

    fn local_picker_draft(&self, cx: &Context<Self>) -> app_state::VideoImportDraft {
        let raw = self.input.read(cx).value();
        let normalized = process::normalize_video_link(&raw)
            .ok()
            .and_then(|link| match link {
                process::VideoLink::Bilibili(url) => match self.selected {
                    Some(index) => bilibili_url_for(&url, self.parts.as_ref(), index).or(Some(url)),
                    None => Some(url),
                },
                process::VideoLink::External(_, url) => Some(url),
            });
        app_state::VideoImportDraft {
            raw: raw.to_string(),
            normalized,
            part: self.selected,
            group: self.target_group(),
        }
    }

    fn parts_section(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(error) = &self.parts_error {
            return div()
                .id("video-import-parts-error")
                .flex()
                .flex_col()
                .gap_2()
                .rounded_md()
                .border_1()
                .border_color(cx.theme().danger.opacity(0.5))
                .p_3()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(error.clone()),
                )
                .child(
                    Button::new("video-import-parts-retry")
                        .label("重试读取")
                        .small()
                        .outline()
                        .on_click(cx.listener(|this, _, _, cx| this.read_parts(cx))),
                )
                .into_any_element();
        }
        if self.parts_loading {
            return div()
                .id("video-import-parts-loading")
                .p_3()
                .rounded_md()
                .border_1()
                .border_color(cx.theme().border)
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("正在读取选集…")
                .into_any_element();
        }
        let Some(series) = &self.parts else {
            return div().into_any_element();
        };
        let mut list = div()
            .id("video-import-parts-list")
            .max_h(px(190.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1();
        for (index, part) in series.parts.iter().enumerate() {
            let selected = self.selected == Some(index);
            list = list.child(
                Button::new(SharedString::from(format!("video-import-part-{index}")))
                    .label(format!(
                        "P{} · {}{}",
                        part.number,
                        part.title,
                        if part.duration > 0. {
                            format!(" · {}", duration_label(part.duration))
                        } else {
                            String::new()
                        }
                    ))
                    .small()
                    .outline()
                    .selected(selected)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = Some(index);
                        this.selected_for = this.parts.as_ref().map(|series| series.base.clone());
                        this.selection_touched = true;
                        this.error = None;
                        cx.notify();
                    })),
            );
        }
        div()
            .id("video-import-parts")
            .flex()
            .flex_col()
            .gap_2()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .p_3()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("{} · {} P", series.title, series.parts.len())),
            )
            .child(list)
            .into_any_element()
    }
}

fn bilibili_base(url: &str) -> Option<String> {
    let mut parsed = url::Url::parse(url).ok()?;
    parsed.set_query(None);
    Some(parsed.to_string())
}

fn bilibili_requested(url: &str) -> Option<usize> {
    let parsed = url::Url::parse(url).ok()?;
    parsed
        .query_pairs()
        .find(|(key, _)| key == "p")
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .map(|value| value - 1)
}

fn bilibili_url_for(url: &str, series: Option<&bilibili::Series>, index: usize) -> Option<String> {
    let mut parsed = url::Url::parse(url).ok()?;
    let number = series?.parts.get(index)?.number;
    parsed
        .query_pairs_mut()
        .clear()
        .append_pair("p", &number.to_string());
    Some(parsed.to_string())
}

fn duration_label(seconds: f64) -> String {
    let seconds = seconds.max(0.) as u64;
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            (seconds / 60) % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

impl Drop for VideoImport {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        self.parts_task.take();
    }
}

impl Render for VideoImport {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let browser = self
            .owner
            .upgrade()
            .and_then(|app| app.read(cx).video.youtube_browser);
        let (groups, aliases) = self
            .owner
            .upgrade()
            .map(|app| {
                let app = app.read(cx);
                (
                    app.video.collection.groups.clone(),
                    app.video.group_aliases.clone(),
                )
            })
            .unwrap_or_default();
        if self.group != "未分组" && !groups.iter().any(|group| group == &self.group) {
            self.group = aliases
                .get(&self.group)
                .filter(|group| !group.is_empty() && groups.contains(group))
                .cloned()
                .unwrap_or_else(|| "未分组".into());
        }
        let mut group_row = div()
            .id("video-import-groups")
            .max_h(px(92.))
            .overflow_y_scroll()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(div().text_sm().child("保存到"));
        for group in std::iter::once(String::new()).chain(groups) {
            let label = if group.is_empty() {
                "未分组".to_string()
            } else {
                group.clone()
            };
            let selected = self.group == label;
            group_row = group_row.child(
                Button::new(SharedString::from(format!("video-import-group-{label}")))
                    .label(label)
                    .small()
                    .outline()
                    .selected(selected)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.group = group.clone();
                        cx.notify();
                    })),
            );
        }
        let links = self.links(cx);
        let primary_label = if links.len() > 1 {
            format!("依次下载并导入 {} 个视频", links.len())
        } else {
            self.selected
                .and_then(|index| {
                    self.parts
                        .as_ref()
                        .and_then(|series| series.parts.get(index))
                })
                .map(|part| format!("下载 P{} 并导入", part.number))
                .unwrap_or_else(|| "下载并导入".into())
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("视频链接"),
                    )
                    .child(Input::new(&self.input).cleanable(true))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("支持 B 站、YouTube、抖音、微博和 X（Twitter）。多个链接请每行一个，会依次下载导入。"),
                    )
                    .when_some(self.error.clone(), |form, error| {
                        form.child(div().text_sm().text_color(cx.theme().danger).child(error))
                    }),
            )
            .child(group_row)
            .child(self.parts_section(cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("video-import-parts-read")
                            .label(if self.parts_loading {
                                "正在读取…"
                            } else {
                                "读取 B 站选集"
                            })
                            .outline()
                            .disabled(self.parts_loading || links.len() > 1)
                            .on_click(cx.listener(|this, _, _, cx| this.read_parts(cx))),
                    )
                    .child(
                        Button::new("video-import-download")
                            .label(primary_label)
                            .primary()
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.submit(window, cx) {
                                    window.close_dialog(cx);
                                }
                            })),
                    ),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .pt_3()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        Button::new("video-import-local")
                            .label("选择本地文件…")
                            .outline()
                            .on_click(cx.listener(|this, _, window, cx| {
                                let draft = this.local_picker_draft(cx);
                                let group = this.target_group();
                                window.close_dialog(cx);
                                let _ = this.owner.update(cx, |app, cx| {
                                    app.video.import_group = Some(group);
                                    app.video.import_edit = Some(draft);
                                    app.handle(Action::Pick("video".into()), window, cx);
                                });
                            })),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .max_w_full()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .truncate()
                            .child(format!("当前分组 · {}", self.group)),
                    ),
            )
            .when_some(browser, |form, browser| {
                form.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("YouTube 登录浏览器 · {}", browser.label())),
                        )
                        .child(
                            Button::new("video-import-clear-youtube")
                                .label("清除登录")
                                .small()
                                .outline()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Ok(result) = this.owner.update(cx, |app, cx| {
                                        app.lib.set_setting("youtube_cookie_browser", "")?;
                                        app.video.youtube_browser = None;
                                        cx.notify();
                                        Ok::<_, anyhow::Error>(())
                                    }) {
                                        this.error = result.err().map(|error| error.to_string());
                                        cx.notify();
                                    }
                                })),
                        ),
                )
            })
    }
}

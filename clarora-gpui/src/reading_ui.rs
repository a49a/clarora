use super::*;

fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

impl AppView {
    pub(super) fn document_view(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.tasks.foreground.is_running();
        let reading = self.ebook_reading.as_ref();
        let selected = reading.is_some() || self.pdf_path.is_some();
        let mut view = div().relative().w_full().flex().flex_col().gap_4();
        if !selected {
            view = view.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .flex_wrap()
                    .child(
                        self.button(
                            "import-document",
                            "导入 PDF / EPUB / TXT",
                            Action::Pick("pdf".into()),
                            cx,
                        )
                        .primary()
                        .disabled(busy),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("书籍保存在本机，自动记住阅读进度"),
                    ),
            );
            view = view.child(self.document_library(busy, cx));
        } else {
            let content = if let Some(reading) = reading {
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(self.reading_heading(reading, busy, cx))
                    .child(self.reading_body(reading, window, cx))
            } else {
                self.pdf_reading_body(busy, window, cx)
            };
            view = view.child(content);
            if self.reading_library_open || self.reading_toc_open {
                let panel = if self.reading_toc_open {
                    self.reading_toc_panel(reading, busy, cx)
                } else {
                    self.reading_library_panel(cx)
                };
                view = view.child(self.reading_panel_backdrop(cx)).child(panel);
            }
        }
        view.into_any_element()
    }

    pub(super) fn reading_header_actions(&self, cx: &Context<Self>) -> AnyElement {
        let busy = self.tasks.foreground.is_running();
        div()
            .flex()
            .items_center()
            .gap_2()
            .flex_wrap()
            .child(
                Button::new("reading-library-toggle")
                    .label("书库")
                    .small()
                    .outline()
                    .selected(self.reading_library_open)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.reading_library_open = !this.reading_library_open;
                        this.reading_toc_open = false;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("reading-toc-toggle")
                    .label("目录")
                    .small()
                    .outline()
                    .selected(self.reading_toc_open)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.reading_toc_open = !this.reading_toc_open;
                        this.reading_library_open = false;
                        cx.notify();
                    })),
            )
            .child(
                self.button("reading-import", "＋ 导入", Action::Pick("pdf".into()), cx)
                    .small()
                    .outline()
                    .disabled(busy),
            )
            .into_any_element()
    }

    fn reading_panel_backdrop(&self, cx: &Context<Self>) -> AnyElement {
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .id("reading-panel-backdrop")
            .occlude()
            .on_click(cx.listener(|this, _, _, cx| {
                this.reading_library_open = false;
                this.reading_toc_open = false;
                cx.notify();
            }))
            .into_any_element()
    }

    fn reading_toc_panel(
        &self,
        reading: Option<&ebook::Reading>,
        busy: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let mut list = div().flex().flex_col().gap_1();
        let mut count = 0;
        if let Some(reading) = reading {
            for (i, entry) in reading.book.toc.iter().enumerate() {
                count += 1;
                let delta = entry.page as i32 - reading.page as i32;
                list = list.child(
                    Button::new(ElementId::Name(format!("ebook-chapter-{i}").into()))
                        .small()
                        .outline()
                        .w_full()
                        .justify_start()
                        .label(format!("{}  ·  {}", entry.label, entry.page + 1))
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.reading_toc_open = false;
                            this.reading_library_open = false;
                            this.handle(Action::PdfPage(delta), window, cx);
                        })),
                );
            }
        } else {
            for (i, (title, page)) in self.pdf_bookmarks.iter().enumerate() {
                count += 1;
                let delta = i32::from(*page) - i32::from(self.pdf_page);
                list = list.child(
                    Button::new(ElementId::Name(format!("pdf-bookmark-{i}").into()))
                        .small()
                        .outline()
                        .w_full()
                        .justify_start()
                        .label(format!("{}  ·  {}", title, *page + 1))
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.reading_toc_open = false;
                            this.reading_library_open = false;
                            this.handle(Action::PdfPage(delta), window, cx);
                        })),
                );
            }
        }
        let mut panel = panel(cx)
            .w(px(320.))
            .max_h(px(560.))
            .p_3()
            .shadow_lg()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(if count == 0 {
                        "目录".to_string()
                    } else {
                        format!("目录 · {count}")
                    }),
            );
        if count == 0 {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("本书暂无目录"),
            );
        } else {
            panel = panel.child(
                div()
                    .id("reading-toc-list")
                    .max_h(px(500.))
                    .overflow_y_scroll()
                    .child(list),
            );
        }
        panel
            .absolute()
            .top(px(58.))
            .left(px(0.))
            .id("reading-toc-panel")
            .into_any_element()
    }

    fn reading_library_panel(&self, cx: &Context<Self>) -> AnyElement {
        let selected = self.pdf_path.as_deref();
        let mut books = div().flex().flex_col().gap_1();
        for (i, path) in self.data.document_files.iter().enumerate() {
            books = books.child(self.document_button(
                format!("reading-library-{i}"),
                ebook::ui_label(path),
                path.clone(),
                selected == Some(path.as_path()),
                cx,
            ));
        }
        let mut panel = panel(cx)
            .w(px(340.))
            .max_h(px(560.))
            .p_3()
            .shadow_lg()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("书库"),
            );
        if self.data.document_files.is_empty() {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("导入 PDF、EPUB 或 TXT 开始阅读"),
            );
        } else {
            panel = panel.child(
                div()
                    .id("reading-library-list")
                    .max_h(px(500.))
                    .overflow_y_scroll()
                    .child(books),
            );
        }
        panel
            .absolute()
            .top(px(58.))
            .left(px(0.))
            .id("reading-library-panel")
            .into_any_element()
    }

    fn pdf_reading_body(&self, busy: bool, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let mut body = div().w_full().flex().flex_col().gap_4();
        body = body.child(
            div().flex().items_center().gap_3().flex_wrap().child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xl()
                    .font_weight(FontWeight::BOLD)
                    .child(
                        self.pdf_path
                            .as_deref()
                            .map(ebook::ui_label)
                            .unwrap_or_else(|| "文档".into()),
                    ),
            ),
        );
        body = body.child(
            div()
                .flex()
                .gap_2()
                .flex_wrap()
                .items_center()
                .child(
                    self.button("pdf-prev", "上一页", Action::PdfPage(-1), cx)
                        .disabled(busy || self.pdf_page == 0),
                )
                .child(
                    self.button("pdf-next", "下一页", Action::PdfPage(1), cx)
                        .disabled(busy || self.pdf_page + 1 >= self.pdf_pages),
                )
                .child(
                    self.button("pdf-smaller", "缩小", Action::PdfZoom(-200), cx)
                        .disabled(busy || self.pdf_image.is_none()),
                )
                .child(
                    self.button("pdf-larger", "放大", Action::PdfZoom(200), cx)
                        .disabled(busy || self.pdf_image.is_none()),
                )
                .child(format!(
                    "{}% · {} / {}",
                    self.pdf_width / 10,
                    if self.pdf_pages == 0 {
                        0
                    } else {
                        self.pdf_page + 1
                    },
                    self.pdf_pages
                )),
        );
        if let Some(image) = &self.pdf_image {
            body = body.child(
                img(image.clone())
                    .w(relative(self.pdf_width as f32 / 1000.))
                    .flex_shrink_0()
                    .object_fit(ObjectFit::Contain),
            );
        }
        if !self.document_text.trim().is_empty() {
            body = body.child(
                panel(cx)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("本页文字 · 可选取后问 AI"),
                    )
                    .child(
                        TextView::html(
                            "pdf-selectable-text",
                            format!(
                                "<p>{}</p>",
                                escaped(&self.document_text).replace('\n', "<br/>")
                            ),
                            window,
                            cx,
                        )
                        .selectable(true),
                    ),
            );
        }
        body
    }

    fn document_library(&self, busy: bool, cx: &Context<Self>) -> Div {
        let mut shelf = div().flex().flex_col().gap_2();
        for (i, path) in self.data.document_files.iter().enumerate() {
            let label = ebook::ui_label(path);
            shelf = shelf.child(
                self.document_button(
                    format!("document-library-{i}"),
                    label,
                    path.clone(),
                    false,
                    cx,
                )
                .disabled(busy),
            );
        }
        let mut panel = panel(cx)
            .gap_2()
            .child(div().font_weight(FontWeight::SEMIBOLD).child("书库"));
        if self.data.document_files.is_empty() {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("导入 PDF、EPUB 或 TXT 开始阅读。EPUB 支持目录、文字重排和常见插图；暂不支持加密书籍与 MOBI/AZW3。"),
            );
        } else {
            panel = panel.child(
                div()
                    .id("document-library-list")
                    .max_h(px(240.))
                    .overflow_y_scroll()
                    .child(shelf),
            );
        }
        panel
    }

    fn document_button(
        &self,
        key: impl Into<SharedString>,
        label: String,
        path: PathBuf,
        selected: bool,
        cx: &Context<Self>,
    ) -> Button {
        Button::new(ElementId::Name(key.into()))
            .small()
            .outline()
            .w_full()
            .justify_start()
            .flex_shrink_0()
            .selected(selected)
            .child(div().min_w_0().w_full().truncate().child(label.clone()))
            .tooltip(label)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.reading_library_open = false;
                this.reading_toc_open = false;
                this.handle(Action::PdfOpen(path.clone()), window, cx);
            }))
    }

    fn reading_heading(&self, reading: &ebook::Reading, busy: bool, cx: &Context<Self>) -> Div {
        let count = reading.book.pages.len();
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div().flex().items_center().gap_2().flex_wrap().child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xl()
                                .font_weight(FontWeight::BOLD)
                                .child(reading.book.title.clone()),
                        )
                        .when(!reading.book.author.is_empty(), |v| {
                            v.child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(reading.book.author.clone()),
                            )
                        }),
                ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .flex_wrap()
                    .items_center()
                    .child(
                        self.button("ebook-prev", "上一段", Action::PdfPage(-1), cx)
                            .disabled(busy || reading.page == 0),
                    )
                    .child(
                        self.button("ebook-next", "下一段", Action::PdfPage(1), cx)
                            .disabled(busy || reading.page + 1 >= count),
                    )
                    .child(
                        self.button("ebook-smaller", "A−", Action::PdfZoom(-200), cx)
                            .disabled(busy || reading.font_size <= 14),
                    )
                    .child(
                        self.button("ebook-larger", "A+", Action::PdfZoom(200), cx)
                            .disabled(busy || reading.font_size >= 32),
                    )
                    .child(format!(
                        "{} / {} · {}px",
                        reading.page + 1,
                        count,
                        reading.font_size
                    ))
                    .child(self.button("ebook-copy", "复制当前文字", Action::DocumentCopy, cx)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("文字重排阅读 · AI 提问引用当前片段 · Cmd/Ctrl + ←/→ 翻页"),
            )
    }

    fn reading_body(
        &self,
        reading: &ebook::Reading,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let page = &reading.book.pages[reading.page];
        let mut body = panel(cx).w_full().gap_4().p_6();
        for (i, block) in page.blocks.iter().enumerate() {
            if block.image.is_some() {
                if let Some(image) = reading.images.get(&i) {
                    body = body.child(
                        img(image.clone())
                            .w_full()
                            .h(px(360.))
                            .object_fit(ObjectFit::Contain),
                    );
                }
                body = body.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(block.text.clone()),
                );
            } else {
                let font = if block.heading > 0 {
                    reading.font_size.saturating_add(8 - block.heading).min(38)
                } else {
                    reading.font_size
                };
                // Only escaped text enters the HTML renderer; images are supplied as decoded pixels.
                body = body.child(
                    TextView::html(
                        SharedString::from(format!(
                            "ebook-{}-{}-{i}",
                            reading.book.identity, reading.page
                        )),
                        format!("<p>{}</p>", escaped(&block.text).replace('\n', "<br/>")),
                        window,
                        cx,
                    )
                    .selectable(true)
                    .text_size(px(f32::from(font))),
                );
            }
        }
        if !reading.warnings.is_empty() {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("部分插图暂不支持，已保留图片说明。"),
            );
        }
        body
    }
}

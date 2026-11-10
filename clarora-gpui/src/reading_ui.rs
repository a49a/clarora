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
        let toggle_label = if self.reading_sidebar_open {
            "⟨ 收起目录"
        } else {
            "☰ 目录"
        };
        let mut view = div().w_full().flex().flex_col().gap_4().child(
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
                .when(reading.is_some(), |header| {
                    header.child(
                        Button::new("reading-sidebar-toggle")
                            .small()
                            .outline()
                            .h(px(30.))
                            .label(toggle_label)
                            .tooltip(if self.reading_sidebar_open {
                                "收起书库与目录"
                            } else {
                                "展开书库与目录"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.reading_sidebar_open = !this.reading_sidebar_open;
                                cx.notify();
                            })),
                    )
                })
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("书籍保存在本机，自动记住阅读进度"),
                ),
        );
        if let Some(reading) = reading {
            let sidebar = self.reading_sidebar(reading, busy, cx);
            let mut content = div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_4()
                .child(self.reading_heading(reading, busy, cx));
            content = content.child(self.reading_body(reading, window, cx));
            view = view.child(
                div()
                    .w_full()
                    .flex()
                    .items_start()
                    .gap_4()
                    .when(self.reading_sidebar_open, |row| row.child(sidebar))
                    .child(content),
            );
        } else {
            view = view.child(self.document_library(busy, cx));
            view = view.child(
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
                view = view.child(
                    img(image.clone())
                        .w(relative(self.pdf_width as f32 / 1000.))
                        .flex_shrink_0()
                        .object_fit(ObjectFit::Contain),
                );
            }
            if !self.document_text.trim().is_empty() {
                view = view.child(
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
            for (i, (title, page)) in self.pdf_bookmarks.iter().enumerate() {
                view = view.child(
                    self.button(
                        format!("bookmark-{i}"),
                        title.clone(),
                        Action::PdfPage(i32::from(*page) - i32::from(self.pdf_page)),
                        cx,
                    )
                    .disabled(busy),
                );
            }
        }
        view.into_any_element()
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

    fn reading_sidebar(&self, reading: &ebook::Reading, busy: bool, cx: &Context<Self>) -> Div {
        let mut books = div().flex().flex_col().gap_2();
        for (i, path) in self.data.document_files.iter().enumerate() {
            books = books.child(
                self.document_button(
                    format!("reading-library-{i}"),
                    ebook::ui_label(path),
                    path.clone(),
                    path == &reading.book.path,
                    cx,
                )
                .disabled(busy),
            );
        }
        let mut toc = div().flex().flex_col().gap_1();
        for (i, entry) in reading.book.toc.iter().enumerate() {
            toc = toc.child(
                self.button(
                    format!("ebook-chapter-{i}"),
                    entry.label.clone(),
                    Action::PdfPage(entry.page as i32 - reading.page as i32),
                    cx,
                )
                .small()
                .outline()
                .w_full()
                .justify_start()
                .tooltip(entry.label.clone())
                .disabled(busy),
            );
        }
        div()
            .w(px(260.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                panel(cx)
                    .gap_2()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("书库"))
                    .child(
                        div()
                            .id("reading-library-list")
                            .max_h(px(240.))
                            .overflow_y_scroll()
                            .child(books),
                    ),
            )
            .child(
                panel(cx)
                    .gap_2()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("目录"))
                    .when(reading.book.toc.is_empty(), |panel| {
                        panel.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("本书暂无目录"),
                        )
                    })
                    .when(!reading.book.toc.is_empty(), |panel| {
                        panel.child(
                            div()
                                .id("reading-toc-list")
                                .max_h(px(420.))
                                .overflow_y_scroll()
                                .child(toc),
                        )
                    }),
            )
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
                this.handle(Action::PdfOpen(path.clone()), window, cx);
            }))
    }

    fn reading_heading(&self, reading: &ebook::Reading, busy: bool, cx: &Context<Self>) -> Div {
        let count = reading.book.pages.len();
        panel(cx)
            .gap_3()
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::BOLD)
                    .child(reading.book.title.clone()),
            )
            .when(!reading.book.author.is_empty(), |v| {
                v.child(reading.book.author.clone())
            })
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

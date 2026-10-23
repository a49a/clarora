use super::*;

fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

impl AppView {
    pub(super) fn document_view(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.tasks.cancel.is_some();
        let mut shelf = div().flex().gap_2().flex_wrap();
        for (i, path) in self.data.document_files.iter().enumerate() {
            let label = ebook::display_name(path);
            shelf = shelf.child(
                self.button(
                    format!("document-library-{i}"),
                    label.to_string(),
                    Action::PdfOpen(path.clone()),
                    cx,
                )
                .disabled(busy),
            );
        }
        let mut view = div()
            .w_full()
            .flex()
            .flex_col()
            .gap_4()
            .child(
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
            )
            .child(shelf);
        if let Some(reading) = &self.ebook_reading {
            let count = reading.book.pages.len();
            view = view.child(
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
                            .child(self.button(
                                "ebook-copy",
                                "复制当前文字",
                                Action::DocumentCopy,
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("文字重排阅读 · AI 提问引用当前片段 · Cmd/Ctrl + ←/→ 翻页"),
                    ),
            );
            if !reading.book.toc.is_empty() {
                view = view.child(
                    panel(cx)
                        .gap_2()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("目录"))
                        .child(
                            uniform_list(
                                "ebook-directory",
                                reading.book.toc.len(),
                                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                    range
                                        .filter_map(|i| {
                                            let reading = this.ebook_reading.as_ref()?;
                                            let entry = reading.book.toc.get(i)?;
                                            Some(
                                                div().h(px(36.)).w_full().child(
                                                    this.button(
                                                        format!("ebook-chapter-{i}"),
                                                        entry.label.clone(),
                                                        Action::PdfPage(
                                                            entry.page as i32 - reading.page as i32,
                                                        ),
                                                        cx,
                                                    )
                                                    .disabled(this.tasks.cancel.is_some()),
                                                ),
                                            )
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .h(px((reading.book.toc.len().min(5) * 36) as f32))
                            .w_full(),
                        ),
                );
            }
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
            view = view.child(body);
        } else {
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
            } else if self.pdf_path.is_none() {
                view=view.child(panel(cx).child("导入 PDF、EPUB 或 TXT 开始阅读。EPUB 支持目录、文字重排和常见插图；暂不支持加密书籍与 MOBI/AZW3。"));
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
}

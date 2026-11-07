//! Reading commands own their action effects and execution.
use super::*;
pub(super) enum Command {
    PdfOpen(PathBuf),
    PdfPage(i32),
    PdfZoom(i32),
    DocumentCopy,
}
impl Command {
    pub(super) fn effects(&self, _app: &AppView) -> Effects {
        Effects::default()
    }
    pub(super) fn execute(
        self,
        app: &mut AppView,
        _window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<()> {
        match self {
            Command::PdfOpen(path) => {
                ensure!(
                    !app.tasks.foreground.is_running(),
                    "已有任务正在执行，请完成或取消后重试"
                );
                if ebook::is_ebook(&path) {
                    app.begin("正在读取电子书…", move |lib, cancel, _| {
                        Ok(Outcome::Ebook(ebook::open(&lib, &path, false, &cancel)?))
                    });
                } else {
                    app.load_pdf(path, 0, app.pdf_width);
                }
            }

            Command::DocumentCopy => {
                cx.write_to_clipboard(ClipboardItem::new_string(app.document_text.clone()));
                app.report_status(FeedbackKind::Info, "已复制当前阅读文字".into());
            }

            Command::PdfPage(delta) => {
                ensure!(
                    !app.tasks.foreground.is_running(),
                    "文档仍在载入，请稍后翻页"
                );
                if let Some(reading) = &app.ebook_reading {
                    let page = (reading.page as i64 + i64::from(delta))
                        .clamp(0, reading.book.pages.len() as i64 - 1)
                        as usize;
                    let book = reading.book.clone();
                    let font = reading.font_size;
                    app.begin("正在翻页…", move |_, cancel, _| {
                        let mut reading = ebook::Reading::load(book, page, &cancel)?;
                        reading.font_size = font;
                        Ok(Outcome::Ebook(reading))
                    });
                } else {
                    let page = (app.pdf_page as i32 + delta)
                        .clamp(0, app.pdf_pages.saturating_sub(1) as i32)
                        as u16;
                    app.load_pdf(
                        app.pdf_path.clone().context("请先导入文档")?,
                        page,
                        app.pdf_width,
                    );
                }
            }

            Command::PdfZoom(delta) => {
                ensure!(
                    !app.tasks.foreground.is_running(),
                    "文档仍在载入，请稍后缩放"
                );
                if let Some(reading) = app.ebook_reading.as_mut() {
                    reading.font_size =
                        (i32::from(reading.font_size) + delta.signum() * 2).clamp(14, 32) as u8;
                    ebook::save_progress(&app.lib, reading)?;
                } else {
                    let width = (app.pdf_width + delta).clamp(300, 2400);
                    app.load_pdf(
                        app.pdf_path.clone().context("请先导入文档")?,
                        app.pdf_page,
                        width,
                    );
                }
            }
        }
        Ok(())
    }
}

// Completion policy belongs to this domain: cancellation may race a durable commit.
pub(super) enum Completed {
    Pdf(PathBuf, i32, documents::PdfPage),
    Ebook(ebook::Reading),
}
impl Completed {
    pub(super) fn apply(
        self,
        app: &mut AppView,
        context: &mut Completion,
        _window: &mut Window,
        _cx: &mut Context<AppView>,
    ) {
        match self {
            Completed::Ebook(mut reading) => {
                if context.cancelled {
                    drop(reading);
                    app.report_status(FeedbackKind::Info, "已取消，保留原阅读内容".into());
                } else {
                    reading.accept_import();
                    app.document_text = reading.text();
                    app.pdf_path = Some(reading.book.path.clone());
                    app.pdf_image = None;
                    app.pdf_bookmarks.clear();
                    if let Err(error) = ebook::save_progress(&app.lib, &reading) {
                        app.report_status(
                            FeedbackKind::Error,
                            format!("已载入，但进度保存失败：{error:#}"),
                        );
                    } else {
                        app.report_status(
                            FeedbackKind::Info,
                            format!("已载入《{}》", reading.book.title),
                        );
                    }
                    let _ = app.lib.set_setting("pdf_current", &json!({"path":reading.book.path,"page":reading.page,"width":app.pdf_width}).to_string());
                    app.ebook_reading = Some(reading);
                    app.page_scroll.set_offset(point(px(0.), px(0.)));
                }
            }

            Completed::Pdf(path, width, page) if !context.cancelled => {
                app.pdf_path = Some(path);
                app.pdf_width = width;
                app.ebook_reading = None;
                app.document_text = page.text;
                app.pdf_image = Some(page.image);
                app.pdf_page = page.page;
                app.pdf_pages = page.count;
                app.pdf_bookmarks = page.bookmarks;
                let _ = app.lib.set_setting(
                    "pdf_current",
                    &json!({"path":app.pdf_path,"page":app.pdf_page,"width":app.pdf_width})
                        .to_string(),
                );
                app.report_status(FeedbackKind::Info, "PDF 已载入".into());
            }

            _ => app.report_status(FeedbackKind::Info, "已取消".into()),
        }
    }
}

pub(super) fn enter(app: &mut AppView) {
    if app.page == "pdf"
        && app.pdf_image.is_none()
        && app.ebook_reading.is_none()
        && app.pdf_path.is_some()
        && !app.tasks.foreground.is_running()
    {
        app.render_pdf();
    }
}

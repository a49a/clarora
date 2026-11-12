//! Review history and Markdown/table rendering for writing feedback.
use super::*;

pub struct ReviewHistory<'a> {
    pub reviews: &'a [Review],
    pub selected: Option<&'a str>,
    pub show_submission: bool,
    pub show_notes: bool,
}

pub fn quote_review(
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

pub fn correction_history<T: 'static>(
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

pub(super) fn card(cx: &App) -> Div {
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

pub(super) fn editor_column() -> Div {
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

pub(super) fn body_input(state: &Entity<InputState>) -> Input {
    Input::new(state)
        .h(EDITOR_HEIGHT)
        .w_full()
        .flex_shrink_0()
        .text_size(BODY_FONT_SIZE)
        .line_height(relative(BODY_LINE_SPACING))
}

pub(super) fn field(label: &'static str, input: Input) -> Div {
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

pub(super) fn display_title(entry: &Entry) -> String {
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

pub(super) fn display_time(value: &str) -> String {
    chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f")
        .map(|time| {
            time.and_utc()
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|_| value.replace('T', " ").chars().take(19).collect())
}

pub(super) fn ordered_history(mut entry: Entry) -> Entry {
    entry
        .reviews
        .sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
    let mut seen = std::collections::BTreeSet::new();
    entry
        .reviews
        .retain(|review| seen.insert(review.id.clone()));
    entry
}

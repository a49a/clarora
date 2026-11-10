//! Search and playback state live outside render; scanning runs on a cancellable worker.
use super::*;
use clarora_gpui::sentence_search::{self, Audition, MatchMode, Query, Results, Scope};
const PAGE_SIZE: usize = 20;
#[derive(Clone)]
pub(super) enum Command {
    Search,
    Preset(String),
    Mode(MatchMode),
    Scope(Scope),
    Cancel,
    Play(usize),
    Toggle,
    Step(bool),
    Stop,
    Autoplay,
    Repeats(u8),
    Page(i32),
}
pub(super) struct State {
    pub mode: MatchMode,
    pub scope: Scope,
    pub results: Option<Results>,
    pub phrase: String,
    pub error: String,
    controller: sentence_search::Controller,
    pub audition: Option<Audition>,
    playback: clarora_gpui::media::Playback,
    pub page: usize,
    pub subscription: Option<Subscription>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            mode: MatchMode::Prefix,
            scope: Scope::All,
            results: None,
            phrase: String::new(),
            error: String::new(),
            controller: Default::default(),
            audition: None,
            playback: Default::default(),
            page: 0,
            subscription: None,
        }
    }
}
impl State {
    pub fn cancel(&mut self) {
        self.controller.cancel();
    }
    pub fn stop(&mut self) {
        if let Some(queue) = &mut self.audition {
            queue.stop();
        }
    }
    pub fn suspend(&mut self) {
        if let Some(queue) = &mut self.audition {
            let index = queue.index;
            queue.stop();
            queue.index = index;
            queue.finished = true;
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl AppView {
    fn search_sentences(&mut self, cx: &App) -> Result<()> {
        let phrase = self.value("sentence_query", cx);
        ensure!(
            !phrase.trim().is_empty(),
            "请输入句型，例如 When is 或 Why did"
        );
        ensure!(phrase.chars().count() <= 120, "句型最多 120 个字符");
        self.sentences.cancel();
        self.sentences.stop();
        self.sentences.results = None;
        self.sentences.page = 0;
        self.sentences.error.clear();
        self.sentences.phrase = phrase.trim().to_string();
        let query = Query {
            phrase,
            mode: self.sentences.mode,
            scope: self.sentences.scope,
        };
        self.sentences.controller.request(&self.lib, query);
        Ok(())
    }
    pub(super) fn sentence_command(
        &mut self,
        command: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        match command {
            Command::Search => self.search_sentences(cx)?,
            Command::Preset(phrase) => {
                self.set("sentence_query", phrase, window, cx);
                self.search_sentences(cx)?;
            }
            Command::Mode(mode) => {
                self.sentences.mode = mode;
                if !self.value("sentence_query", cx).trim().is_empty() {
                    self.search_sentences(cx)?;
                }
            }
            Command::Scope(scope) => {
                self.sentences.scope = scope;
                if !self.value("sentence_query", cx).trim().is_empty() {
                    self.search_sentences(cx)?;
                }
            }
            Command::Cancel => {
                self.sentences.cancel();
                self.sentences.error = "搜索已取消".into();
            }
            Command::Page(delta) => {
                let len = self.sentences.results.as_ref().map_or(0, |r| r.hits.len());
                self.sentences.page = (self.sentences.page as i32 + delta)
                    .clamp(0, (len.saturating_sub(1) / PAGE_SIZE) as i32)
                    as usize;
                self.page_scroll.set_offset(Point::default());
            }
            other => {
                let results = self.sentences.results.as_ref().context("请先搜索句型")?;
                ensure!(!results.hits.is_empty(), "没有可试听的匹配句子");
                let queue = self
                    .sentences
                    .audition
                    .get_or_insert_with(|| Audition::new(clarora_gpui::media::Player::new()));
                match other {
                    Command::Play(index) => queue.select(&results.hits, index)?,
                    Command::Toggle => queue.toggle(&results.hits)?,
                    Command::Step(next) => queue.step(&results.hits, next)?,
                    Command::Stop => queue.stop(),
                    Command::Autoplay => queue.autoplay = !queue.autoplay,
                    Command::Repeats(count) => {
                        ensure!([0, 1, 3, 5].contains(&count), "重复方式无效");
                        queue.repeats = count;
                        if let Some(i) = queue.index {
                            queue.select(&results.hits, i)?;
                        }
                    }
                    _ => unreachable!(),
                }
                if let Some(index) = queue.index {
                    self.sentences.page = index / PAGE_SIZE;
                }
                self.sentences.error = queue.error.clone();
                window.focus(&self.focus);
            }
        }
        Ok(())
    }
    pub(super) fn poll_sentence_search(&mut self) -> bool {
        let mut changed = false;
        if let Some(result) = self.sentences.controller.poll() {
            match result {
                Ok(results) => self.sentences.results = Some(results),
                Err(error) => self.sentences.error = error,
            };
            changed = true;
        }
        if self.page == "sentences" {
            if let (Some(results), Some(queue)) =
                (&self.sentences.results, &mut self.sentences.audition)
            {
                match queue.poll(&results.hits) {
                    Ok(updated) => changed |= updated,
                    Err(error) => {
                        let message = format!("{error:#}");
                        if self.sentences.error != message {
                            self.sentences.error = message;
                            changed = true;
                        }
                    }
                }
                if changed {
                    if let Some(index) = queue.index {
                        self.sentences.page = index / PAGE_SIZE;
                    }
                }
                // Paused and completed queues must not force continuous UI rebuilds.
                let playback = queue.snapshot();
                changed |=
                    clarora_gpui::media::controls_changed(&self.sentences.playback, &playback);
                self.sentences.playback = playback;
            }
        }
        changed
    }
    pub(super) fn handle_sentence_search_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let m = event.keystroke.modifiers;
        if self.page != "sentences"
            || m.platform
            || m.control
            || m.alt
            || m.shift
            || window.has_active_dialog(cx)
            || window.focused(cx) != Some(self.focus.clone())
        {
            return false;
        }
        let command = match event.keystroke.key.as_str() {
            "left" => Command::Step(false),
            "right" => Command::Step(true),
            "space" => Command::Toggle,
            "escape" => Command::Stop,
            _ => return false,
        };
        self.handle(Action::Sentence(command), window, cx);
        true
    }
    pub(super) fn sentence_search_view(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let action = |command| Action::Sentence(command);
        let searching = self.sentences.controller.is_loading();
        let mut filters = div().flex().items_center().gap_2().flex_wrap();
        for (label, mode) in [
            ("句首匹配", MatchMode::Prefix),
            ("包含短语", MatchMode::Contains),
        ] {
            filters = filters.child(
                self.button(
                    format!("sentence-mode-{mode:?}"),
                    label,
                    action(Command::Mode(mode)),
                    cx,
                )
                .selected(self.sentences.mode == mode),
            );
        }
        filters = filters.child(div().w(px(12.)));
        for (label, scope) in [
            ("全部音视频", Scope::All),
            ("仅音频", Scope::Audio),
            ("仅视频", Scope::Video),
        ] {
            filters = filters.child(
                self.button(
                    format!("sentence-scope-{scope:?}"),
                    label,
                    action(Command::Scope(scope)),
                    cx,
                )
                .selected(self.sentences.scope == scope),
            );
        }
        let mut presets = div().flex().items_center().gap_2().flex_wrap().child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("试着搜索"),
        );
        for phrase in ["When", "When is", "Why did", "How do", "What if"] {
            presets = presets.child(self.button(
                format!("sentence-preset-{phrase}"),
                phrase,
                action(Command::Preset(phrase.into())),
                cx,
            ));
        }
        let mut view=div().flex().flex_col().gap_4().w_full().child(panel(cx).gap_3()
            .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("找到同一种表达，换着语境听"))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child("检索所有音频组和视频各 P 的已有英文字幕，不区分大小写，按完整单词匹配。"))
            .child(div().flex().gap_2().items_end().flex_wrap().child(div().flex_1().min_w(px(200.)).child(self.field("sentence_query","英文单词或短语")))
                .child(self.button("sentence-submit","搜索全库",action(Command::Search),cx).primary().with_size(gpui_component::Size::Medium))
                .when(searching,|v|v.child(self.button("sentence-cancel","取消搜索",action(Command::Cancel),cx))))
            .child(filters).child(presets));
        if searching {
            view = view.child(
                panel(cx).child("正在扫描字幕库… 可以继续修改查询，再次搜索会取消上一项任务。"),
            );
        }
        if !self.sentences.error.is_empty() {
            view = view.child(
                panel(cx)
                    .text_color(cx.theme().danger)
                    .child(self.sentences.error.clone()),
            );
        }
        let Some(results) = &self.sentences.results else {
            return view.child(panel(cx).child("例如搜索 When is，可以对照不同材料里的问句，一条一条听。尚未生成或导入字幕的材料不会出现在结果中。")).into_any_element();
        };
        let queue = self.sentences.audition.as_ref();
        let selected = queue.and_then(|q| q.index);
        let playing = queue.is_some_and(|q| !q.snapshot().paused && !q.finished);
        let mut summary = format!(
            "“{}” · 找到 {} 句 · 已扫描 {} 份音视频",
            self.sentences.phrase, results.total, results.scanned
        );
        if results.total > results.hits.len() {
            summary.push_str(&format!(" · 载入前 {} 句，请细化搜索", results.hits.len()));
        }
        view = view.child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(summary),
        );
        if results.without_subtitles > 0 || results.unavailable > 0 || !results.problems.is_empty()
        {
            view =
                view.child(
                    panel(cx)
                        .gap_2()
                        .text_sm()
                        .child(format!(
                            "{} 份没有可搜索字幕 · {} 份媒体不可用／未下载",
                            results.without_subtitles, results.unavailable
                        ))
                        .children(results.problems.iter().take(3).map(|message| {
                            div().text_color(cx.theme().danger).child(message.clone())
                        })),
                );
        }
        if results.hits.is_empty() {
            return view.child(panel(cx).child("没有找到匹配句。试试更短的句首，或切换为“包含短语”；也可以先给材料生成英文字幕。")).into_any_element();
        }
        let mut transport = div()
            .flex()
            .gap_2()
            .items_center()
            .flex_wrap()
            .child(
                self.button("sentence-first", "从第一条听", action(Command::Play(0)), cx)
                    .primary(),
            )
            .child(
                self.button(
                    "sentence-prev",
                    "← 上一条",
                    action(Command::Step(false)),
                    cx,
                )
                .disabled(selected == Some(0)),
            )
            .child(
                self.button(
                    "sentence-toggle",
                    if playing {
                        "暂停 · 空格"
                    } else {
                        "播放 · 空格"
                    },
                    action(Command::Toggle),
                    cx,
                )
                .disabled(queue.is_some_and(|q| q.is_loading())),
            )
            .child(
                self.button("sentence-next", "下一条 →", action(Command::Step(true)), cx)
                    .disabled(selected == Some(results.hits.len() - 1)),
            )
            .child(
                self.button("sentence-stop", "停止", action(Command::Stop), cx)
                    .disabled(selected.is_none()),
            );
        transport = transport.child(
            self.button(
                "sentence-autoplay",
                if queue.is_none_or(|q| q.autoplay) {
                    "连续播放 ✓"
                } else {
                    "连续播放"
                },
                action(Command::Autoplay),
                cx,
            )
            .selected(queue.is_none_or(|q| q.autoplay)),
        );
        for (label, count) in [
            ("听一遍", 1),
            ("每句三遍", 3),
            ("每句五遍", 5),
            ("单句循环", 0),
        ] {
            transport = transport.child(
                self.button(
                    format!("sentence-repeats-{count}"),
                    label,
                    action(Command::Repeats(count)),
                    cx,
                )
                .selected(queue.map_or(1, |q| q.repeats) == count),
            );
        }
        let mut now = panel(cx)
            .gap_3()
            .border_color(cx.theme().primary)
            .child(transport);
        if let Some((index, hit)) = selected.and_then(|i| results.hits.get(i).map(|h| (i, h))) {
            let state = queue.unwrap().snapshot();
            now = now
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().primary)
                        .child(format!(
                            "当前 {}/{} · {} · {} – {}{}",
                            index + 1,
                            results.hits.len(),
                            hit.label,
                            clock(hit.cue.start),
                            clock(hit.cue.end),
                            if queue.unwrap().is_loading() {
                                " · 正在载入"
                            } else if queue.unwrap().finished {
                                " · 已播完"
                            } else {
                                ""
                            }
                        )),
                )
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(hit.english.clone()),
                )
                .when(!hit.translation.is_empty(), |v| {
                    v.child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child(hit.translation.clone()),
                    )
                })
                .when_some(state.sentence_repeat, |v, r| {
                    v.child(div().text_sm().child(match r.total_plays {
                        Some(total) => format!("第 {}/{} 遍", r.current_play, total),
                        None => "本句持续循环".into(),
                    }))
                });
        } else {
            now = now.child("点击结果的“试听”，或从第一条开始；左右键切换结果，空格播放／暂停。");
        }
        view = view.child(now);
        let pages = results.hits.len().div_ceil(PAGE_SIZE);
        view = view.child(
            div()
                .flex()
                .gap_3()
                .items_center()
                .child(format!(
                    "结果列表 · 第 {}/{} 页",
                    self.sentences.page + 1,
                    pages
                ))
                .child(
                    self.button(
                        "sentence-page-prev",
                        "上一页",
                        action(Command::Page(-1)),
                        cx,
                    )
                    .disabled(self.sentences.page == 0),
                )
                .child(
                    self.button("sentence-page-next", "下一页", action(Command::Page(1)), cx)
                        .disabled(self.sentences.page + 1 >= pages),
                ),
        );
        for (i, hit) in results
            .hits
            .iter()
            .enumerate()
            .skip(self.sentences.page * PAGE_SIZE)
            .take(PAGE_SIZE)
        {
            view = view.child(
                panel(cx)
                    .gap_2()
                    .when(selected == Some(i), |v| {
                        v.bg(cx.theme().accent).border_color(cx.theme().primary)
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                div()
                                    .flex_1()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{} · {}{} · {} – {}",
                                        if hit.kind == Scope::Audio {
                                            "音频"
                                        } else {
                                            "视频"
                                        },
                                        if hit.group.is_empty() {
                                            String::new()
                                        } else {
                                            format!("{} / ", hit.group)
                                        },
                                        hit.label,
                                        clock(hit.cue.start),
                                        clock(hit.cue.end)
                                    )),
                            )
                            .child(
                                self.button(
                                    format!("sentence-play-{i}"),
                                    if selected == Some(i) {
                                        "重听本句"
                                    } else {
                                        "试听"
                                    },
                                    action(Command::Play(i)),
                                    cx,
                                )
                                .primary(),
                            )
                            .child(self.button(
                                format!("sentence-copy-{i}"),
                                "复制原句",
                                Action::AssistantCopy(hit.english.clone()),
                                cx,
                            )),
                    )
                    .child(div().text_lg().child(hit.english.clone()))
                    .when(!hit.translation.is_empty(), |v| {
                        v.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(hit.translation.clone()),
                        )
                    }),
            );
        }
        view.into_any_element()
    }
}

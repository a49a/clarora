//! Question library for the speaking workspace.
use super::*;
use clarora_gpui::speaking::{self, AudioEdit, Group, Question};

const SPEAKING_RATES: [f64; 8] = [0.6, 0.7, 0.8, 0.9, 1., 1.25, 1.5, 2.];

pub(super) enum PromptText {
    Transcript(String),
    Translation { source: String, text: String },
}

#[cfg(debug_assertions)]
type SpeakingLayoutBounds = std::rc::Rc<std::cell::Cell<Option<[Bounds<Pixels>; 2]>>>;

#[derive(Default)]
struct PromptWaveform {
    path: Option<PathBuf>,
    worker: Option<(Cancel, mpsc::Receiver<Result<Waveform>>)>,
    result: Option<Result<Waveform>>,
    bounds: std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>,
    start_at: f64,
}

impl Drop for PromptWaveform {
    fn drop(&mut self) {
        if let Some((cancel, _)) = &self.worker {
            cancel.store(true, Ordering::SeqCst);
        }
    }
}

#[derive(Default)]
pub(super) struct SpeakingLibrary {
    groups: Vec<Group>,
    questions: Vec<Question>,
    current: Option<Question>,
    audio_edit: AudioEdit,
    rate: Option<f64>,
    waveform: PromptWaveform,
    translation: Option<(String, String)>,
    creating_group: bool,
    editing_group_tags: bool,
    editing_title: bool,
    tag_filter: Option<String>,
    saving: bool,
    #[cfg(debug_assertions)]
    layout_probe: Option<SpeakingLayoutBounds>,
}

impl AppView {
    pub(super) fn reload_speaking_questions(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let groups = speaking::list_groups(&self.lib)?;
        let questions = speaking::list(&self.lib)?;
        if self.speaking.rate.is_none() {
            self.speaking.rate = Some(
                self.lib
                    .setting("speaking_prompt_rate")
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .filter(|rate| SPEAKING_RATES.contains(rate))
                    .unwrap_or(1.),
            );
        }
        let initial = self.speaking.current.is_none() && self.value("reference", cx).is_empty();
        self.speaking.groups = groups;
        self.speaking.questions = questions;
        if initial {
            if let Some(group) = self.speaking.groups.first() {
                let id = group.id.clone();
                self.open_speaking_group(&id, window, cx);
            }
        }
        Ok(())
    }

    pub(super) fn speaking_groups(&self) -> &[Group] {
        &self.speaking.groups
    }

    pub(super) fn refresh_speaking_groups(&mut self) -> Result<()> {
        self.speaking.groups = speaking::list_groups(&self.lib)?;
        Ok(())
    }

    fn speaking_locked(&self) -> bool {
        self.speaking.saving || self.recording.is_some() || self.tasks.foreground.is_running()
    }

    fn speaking_dirty(&self, cx: &App) -> bool {
        self.speaking.current.as_ref().is_some_and(|question| {
            self.value("speaking_title", cx).trim() != question.title
                || self.value("speaking_answer_draft", cx) != question.answer_draft
                || self.value("reference", cx) != question.text
                || self.speaking_translation(cx) != question.translation.as_deref()
                || !matches!(self.speaking.audio_edit, AudioEdit::Keep)
        })
    }

    fn may_switch_speaking_question(&mut self, cx: &App) -> bool {
        if self.speaking_locked() {
            return false;
        }
        if self.speaking_dirty(cx) {
            self.report_status(
                FeedbackKind::Info,
                "请先保存当前题目，或点击「撤销编辑」".into(),
            );
            return false;
        }
        true
    }

    fn set_speaking_question(
        &mut self,
        question: Question,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop_speaking_prompt();
        self.speaking.waveform.start_at = 0.;
        self.speaking.editing_title = false;
        self.set("speaking_title", question.title.clone(), window, cx);
        self.set("reference", question.text.clone(), window, cx);
        self.set(
            "speaking_answer_draft",
            question.answer_draft.clone(),
            window,
            cx,
        );
        self.speaking.translation = question
            .translation
            .clone()
            .map(|text| (question.text.clone(), text));
        self.speaking.current = Some(question);
        self.speaking.audio_edit = AudioEdit::Keep;
        self.recorded = None;
        self.output.clear();
        cx.notify();
    }

    fn open_speaking_group(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let saved = self
            .speaking
            .questions
            .iter()
            .find(|question| question.group_id == id)
            .cloned();
        self.speaking.editing_group_tags = false;
        self.set("speaking_group_tag", "", window, cx);
        self.set_speaking_question(
            saved.unwrap_or_else(|| Question::draft(id.to_owned())),
            window,
            cx,
        );
    }

    fn begin_speaking_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let date = chrono::Local::now().format("%Y-%-m-%-d").to_string();
        let mut name = date.clone();
        for index in 2.. {
            if !self.speaking.groups.iter().any(|group| group.name == name) {
                break;
            }
            name = format!("{date} ({index})");
        }
        self.speaking.creating_group = true;
        self.set("speaking_group", name, window, cx);
        self.inputs["speaking_group"].update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn create_speaking_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.may_switch_speaking_question(cx) {
            cx.notify();
            return;
        }
        match speaking::create_group(&self.lib, &self.value("speaking_group", cx)) {
            Ok(group) => {
                let id = group.id.clone();
                self.speaking.groups.insert(0, group);
                self.speaking.creating_group = false;
                self.speaking.tag_filter = None;
                self.open_speaking_group(&id, window, cx);
            }
            Err(error) => {
                self.report_status(FeedbackKind::Error, format!("新建口语分组失败：{error:#}"))
            }
        }
        cx.notify();
    }

    fn pick_speaking_prompt(&mut self, cx: &mut Context<Self>) {
        if self.speaking_locked() || self.speaking_prompt_audio().is_some() {
            return;
        }
        let Some(question) = &self.speaking.current else {
            return;
        };
        let question_id = question.id.clone();
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picker.await {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update(cx, |this, cx| {
                        if this.page == "speaking"
                            && !this.speaking_locked()
                            && this.speaking_prompt_audio().is_none()
                            && this
                                .speaking
                                .current
                                .as_ref()
                                .is_some_and(|question| question.id == question_id)
                        {
                            this.stop_speaking_prompt();
                            this.speaking.audio_edit = AudioEdit::Import(path);
                            cx.notify();
                        }
                    });
                }
            }
        })
        .detach();
    }

    fn start_speaking_command_import(
        &mut self,
        attach: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.speaking_locked() || (attach && self.speaking_prompt_audio().is_some()) {
            return;
        }
        if self.import_command.trim().is_empty() {
            self.handle(Action::ConfigureCommandImport, window, cx);
            return;
        }
        let Some(current) = &self.speaking.current else {
            return;
        };
        let question = attach.then(|| {
            let mut question = current.clone();
            question.title = self.value("speaking_title", cx);
            question.answer_draft = self.value("speaking_answer_draft", cx);
            question.text = self.value("reference", cx);
            question.translation = self.speaking_translation(cx).map(str::to_owned);
            question
        });
        self.start_speaking_command_import_to(current.group_id.clone(), question, window, cx);
    }

    pub(super) fn start_speaking_command_import_to(
        &mut self,
        group: String,
        question: Option<Question>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.speaking_locked() {
            self.report_status(
                FeedbackKind::Info,
                "请先结束当前任务或录音，再导入口语题目".into(),
            );
            cx.notify();
            return;
        }
        if self.import_command.trim().is_empty() {
            self.handle(Action::ConfigureCommandImport, window, cx);
            return;
        }
        match clarora_gpui::command_import::SpeakingRequest::new(
            &self.lib,
            &group,
            &self.import_command,
            question,
        ) {
            Ok(request) => {
                let label = format!("正在导入口语分组「{}」…", request.group_name());
                self.begin_scoped(TaskScope::Application, &label, move |lib, cancel, _| {
                    Ok(Outcome::SpeakingCommandImported(
                        request.run(&lib, &cancel)?,
                    ))
                });
                if self.tasks.foreground.is_running() {
                    self.tasks.notice = Some(TaskNotice::new("口语命令导入", &label));
                }
            }
            Err(error) => self.report_status(FeedbackKind::Error, error.to_string()),
        }
        cx.notify();
    }

    pub(super) fn finish_speaking_command_import(
        &mut self,
        report: clarora_gpui::command_import::SpeakingReport,
        cancelled: bool,
    ) {
        let count = report.questions.len();
        for question in report.questions {
            if report.attached
                && self
                    .speaking
                    .current
                    .as_ref()
                    .is_some_and(|item| item.id == question.id)
            {
                self.stop_speaking_prompt();
                self.speaking.current = Some(question.clone());
                self.speaking.audio_edit = AudioEdit::Keep;
            }
            if let Some(saved) = self
                .speaking
                .questions
                .iter_mut()
                .find(|saved| saved.id == question.id)
            {
                *saved = question;
            } else {
                self.speaking.questions.insert(0, question);
            }
        }
        let mut message = if report.attached {
            "题目音频已导入".to_owned()
        } else {
            format!("已在「{}」导入 {count} 道口语题", report.group_name)
        };
        if report.failed > 0 {
            message.push_str(&format!("，{} 个文件未导入", report.failed));
        }
        if cancelled {
            message.push_str("，任务已停止，已导入内容已保留");
        }
        self.report_status(FeedbackKind::Info, message);
    }

    fn save_speaking_question(&mut self, cx: &mut Context<Self>) {
        if self.speaking.saving || self.tasks.foreground.is_running() {
            return;
        }
        let Some(mut question) = self.speaking.current.clone() else {
            return;
        };
        question.title = self.value("speaking_title", cx);
        question.answer_draft = self.value("speaking_answer_draft", cx);
        question.text = self.value("reference", cx);
        question.translation = self.speaking_translation(cx).map(str::to_owned);
        let lib = self.lib.clone();
        let audio = self.speaking.audio_edit.clone();
        self.speaking.saving = true;
        let save = cx
            .background_executor()
            .spawn(async move { speaking::save(&lib, question, audio) });
        cx.spawn(async move |this, cx| {
            let result = save.await;
            let _ = this.update(cx, |this, cx| {
                this.speaking.saving = false;
                match result {
                    Ok(question) => {
                        let saved_path = question.audio.as_ref().map(|audio| audio.path.as_path());
                        if this.media.session.owner() == Some(&PlaybackOwner::Speaking)
                            && this.media.session.path.as_deref() != saved_path
                        {
                            this.stop_speaking_prompt();
                        }
                        if let Some(saved) = this
                            .speaking
                            .questions
                            .iter_mut()
                            .find(|saved| saved.id == question.id)
                        {
                            *saved = question.clone();
                        } else {
                            this.speaking.questions.insert(0, question.clone());
                        }
                        this.speaking.current = Some(question);
                        this.speaking.editing_title = false;
                        this.speaking.audio_edit = AudioEdit::Keep;
                        this.report_status(FeedbackKind::Info, "口语题目已保存".into());
                    }
                    Err(error) => this
                        .report_status(FeedbackKind::Error, format!("保存口语题目失败：{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn remove_speaking_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.speaking_locked() || self.speaking_prompt_audio().is_none() {
            return;
        }
        self.stop_speaking_prompt();
        self.speaking.audio_edit = if self
            .speaking
            .current
            .as_ref()
            .is_some_and(|question| question.audio.is_some())
        {
            AudioEdit::Remove
        } else {
            AudioEdit::Keep
        };
        if matches!(self.speaking.audio_edit, AudioEdit::Remove) {
            self.set("reference", "", window, cx);
            self.speaking.translation = None;
        }
        cx.notify();
    }

    fn speaking_prompt_audio(&self) -> Option<(&Path, String)> {
        match &self.speaking.audio_edit {
            AudioEdit::Keep => self
                .speaking
                .current
                .as_ref()?
                .audio
                .as_ref()
                .map(|audio| (audio.path.as_path(), audio.name.clone())),
            AudioEdit::Import(path) => Some((
                path,
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            )),
            AudioEdit::Remove => None,
        }
    }

    pub(super) fn poll_speaking_waveform(&mut self) -> bool {
        let path = (self.page == "speaking")
            .then(|| {
                self.speaking_prompt_audio()
                    .map(|(path, _)| path.to_owned())
            })
            .flatten();
        let waveform = &mut self.speaking.waveform;
        if waveform.path != path {
            *waveform = PromptWaveform::default();
            if let Some(path) = path {
                waveform.path = Some(path.clone());
                let lib = self.lib.clone();
                let cancel = process::cancel_token();
                let worker_cancel = cancel.clone();
                let (sender, receiver) = mpsc::channel();
                waveform.worker = Some((cancel, receiver));
                std::thread::spawn(move || {
                    let _ = sender.send(waveform::extract(&lib, &path, &worker_cancel));
                });
            }
            return true;
        }
        let Some((_, receiver)) = &waveform.worker else {
            return false;
        };
        waveform.result = Some(match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => Err(anyhow::anyhow!("波形读取提前结束")),
        });
        waveform.worker = None;
        true
    }

    fn speaking_playback_matches(&self, path: &Path, snapshot: &Playback) -> bool {
        self.media.session.owner() == Some(&PlaybackOwner::Speaking)
            && self.media.session.path.as_deref() == Some(path)
            && self.media.matches_snapshot(snapshot)
    }

    fn seek_speaking_prompt(&mut self, path: &Path, time: f64) {
        if self.recording.is_some()
            || self
                .speaking_prompt_audio()
                .is_none_or(|(current, _)| current != path)
        {
            return;
        }
        let playback = self.media.player.snapshot();
        if self.speaking_playback_matches(path, &playback) && !playback.eof {
            self.media
                .player
                .command(["seek".into(), time.to_string(), "absolute+exact".into()]);
        } else {
            self.stop_speaking_prompt();
            self.speaking.waveform.start_at = time;
        }
    }

    pub(super) fn stop_speaking_prompt(&mut self) {
        if self.media.session.owner() == Some(&PlaybackOwner::Speaking) {
            self.media.player.set("pause", "yes");
            self.media.release();
            self.media.player.set("speed", self.media.rate);
        }
    }

    fn play_speaking_prompt(&mut self) -> Result<()> {
        ensure!(self.recording.is_none(), "请先停止作答录音");
        let (path, _) = self.speaking_prompt_audio().context("请先添加题目音频")?;
        ensure!(path.is_file(), "题目音频文件不存在，请重新添加");
        let playback = self.media.player.snapshot();
        if self.speaking_playback_matches(path, &playback) && !playback.eof {
            self.media.player.set("pause", "no");
            return Ok(());
        }
        let path = path.to_owned();
        let start = if self.speaking.waveform.path.as_deref() == Some(path.as_path()) {
            std::mem::take(&mut self.speaking.waveform.start_at)
        } else {
            0.
        };
        self.media.open(
            PlaybackOwner::Speaking,
            path,
            start,
            0.,
            false,
            PlaybackMode::Normal,
        );
        self.media
            .player
            .set("speed", self.speaking.rate.unwrap_or(1.));
        Ok(())
    }

    fn set_speaking_rate(&mut self, rate: f64, cx: &mut Context<Self>) {
        self.speaking.rate = Some(rate);
        if self.media.session.owner() == Some(&PlaybackOwner::Speaking) {
            self.media.player.set("speed", rate);
        }
        if let Err(error) = self
            .lib
            .set_setting("speaking_prompt_rate", &rate.to_string())
        {
            self.report_status(FeedbackKind::Error, format!("保存口语倍速失败：{error:#}"));
        }
        cx.notify();
    }

    fn speaking_rate_picker(&self, cx: &Context<Self>) -> impl IntoElement {
        let current = self.speaking.rate.unwrap_or(1.);
        let owner = cx.entity().downgrade();
        Button::new("speaking-prompt-rate")
            .small()
            .outline()
            .label(format!("{current}× ▾"))
            .tooltip("题目音频播放速度")
            .dropdown_menu(move |mut menu, _, _| {
                for rate in SPEAKING_RATES {
                    let owner = owner.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format!("{rate}×"))
                            .checked(rate == current)
                            .on_click(move |_, _, cx| {
                                let _ =
                                    owner.update(cx, |this, cx| this.set_speaking_rate(rate, cx));
                            }),
                    );
                }
                menu
            })
    }

    fn speaking_translation(&self, cx: &App) -> Option<&str> {
        let (source, text) = self.speaking.translation.as_ref()?;
        (self.inputs["reference"].read(cx).value().as_ref() == source).then_some(text.as_str())
    }

    fn start_speaking_text_task(&mut self, translate: bool, cx: &mut Context<Self>) {
        if self.speaking_locked() {
            return;
        }
        let Some(id) = self
            .speaking
            .current
            .as_ref()
            .map(|question| question.id.clone())
        else {
            return;
        };
        let title;
        if translate {
            let source = self.value("reference", cx);
            if source.trim().is_empty() {
                return;
            }
            title = "翻译口语题目";
            self.begin_scoped(
                TaskScope::Application,
                "正在翻译口语题目…",
                move |lib, cancel, _| {
                    let text = speaking::translate_prompt(&lib, &source, &cancel)?;
                    Ok(Outcome::SpeakingTextReady(
                        id,
                        PromptText::Translation { source, text },
                    ))
                },
            );
        } else {
            let Some((path, _)) = self.speaking_prompt_audio() else {
                return;
            };
            let path = path.to_owned();
            self.stop_speaking_prompt();
            title = "转写题目音频";
            self.begin_scoped(
                TaskScope::Application,
                "正在转写题目音频…",
                move |lib, cancel, _| {
                    let text = speaking::transcribe_prompt(&lib, &path, &cancel)?;
                    Ok(Outcome::SpeakingTextReady(id, PromptText::Transcript(text)))
                },
            );
        }
        if self.tasks.foreground.is_running() {
            self.tasks.notice = Some(TaskNotice::new(title, "结果将显示在对应题目中"));
        }
        cx.notify();
    }

    pub(super) fn finish_speaking_text(
        &mut self,
        id: String,
        result: PromptText,
        cancelled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if cancelled {
            self.report_status(FeedbackKind::Info, "已取消".into());
            return;
        }
        if !self
            .speaking
            .current
            .as_ref()
            .is_some_and(|question| question.id == id)
        {
            self.report_status(
                FeedbackKind::Info,
                "题目已切换，请在当前题目重新操作".into(),
            );
            return;
        }
        match result {
            PromptText::Transcript(text) => {
                if self.value("reference", cx) != text {
                    self.speaking.translation = None;
                    self.set("reference", text, window, cx);
                }
            }
            PromptText::Translation { source, text } => {
                if self.value("reference", cx) != source {
                    return;
                }
                self.speaking.translation = Some((source, text));
            }
        }
        if self.speaking_dirty(cx) {
            self.save_speaking_question(cx);
        }
        cx.notify();
    }

    fn start_speaking_group_tags(&mut self, group_id: String, cx: &mut Context<Self>) {
        if self.speaking_locked() {
            return;
        }
        self.begin_scoped(
            TaskScope::Application,
            "正在生成口语分组标签…",
            move |lib, cancel, _| {
                let tags = speaking::suggest_group_tags(&lib, &group_id, &cancel)?;
                Ok(Outcome::SpeakingGroupTagsReady(group_id, tags))
            },
        );
        if self.tasks.foreground.is_running() {
            self.tasks.notice = Some(TaskNotice::new(
                "AI 生成分组标签",
                "正在归纳本组已转写的音频主题…",
            ));
        }
        cx.notify();
    }

    pub(super) fn finish_speaking_group_tags(
        &mut self,
        group_id: &str,
        tags: &[String],
        cancelled: bool,
    ) {
        if cancelled {
            self.report_status(FeedbackKind::Info, "已取消生成分组标签".into());
            return;
        }
        match speaking::merge_group_tags(&self.lib, group_id, tags) {
            Ok((group, added)) => {
                let name = group.name.clone();
                self.update_speaking_group(group);
                let message = if added > 0 {
                    format!("已为「{name}」添加 {added} 个标签")
                } else {
                    format!("「{name}」没有可添加的新标签")
                };
                self.report_status(FeedbackKind::Info, message);
            }
            Err(error) => {
                self.report_status(FeedbackKind::Error, format!("保存分组标签失败：{error:#}"))
            }
        }
    }

    fn update_speaking_group(&mut self, group: Group) {
        if let Some(current) = self
            .speaking
            .groups
            .iter_mut()
            .find(|item| item.id == group.id)
        {
            *current = group;
        }
    }

    fn edit_speaking_group_tag(
        &mut self,
        group_id: &str,
        tag: &str,
        add: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.speaking_locked()
            || self
                .speaking
                .current
                .as_ref()
                .is_none_or(|question| question.group_id != group_id)
        {
            return;
        }
        match speaking::edit_group_tag(&self.lib, group_id, tag, add) {
            Ok(group) => {
                self.update_speaking_group(group);
                if add {
                    self.set("speaking_group_tag", "", window, cx);
                }
                if self.speaking_tag_filter().is_none() {
                    self.speaking.tag_filter = None;
                }
            }
            Err(error) => self.report_status(FeedbackKind::Error, error.to_string()),
        }
        cx.notify();
    }

    fn speaking_tag_filter(&self) -> Option<&str> {
        let filter = self.speaking.tag_filter.as_deref()?;
        self.speaking
            .groups
            .iter()
            .any(|group| {
                group
                    .tags
                    .iter()
                    .any(|tag| tag.eq_ignore_ascii_case(filter))
            })
            .then_some(filter)
    }

    fn speaking_group_tags(
        &self,
        group: &Group,
        locked: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let ai_group = group.id.clone();
        let has_transcripts = self
            .speaking
            .questions
            .iter()
            .any(|question| question.group_id == group.id && question.has_audio_transcript());
        let group_id = group.id.clone();
        let mut tags = div().flex().flex_wrap().items_center().gap_2().child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child("分组标签"),
        );
        for (index, tag) in group.tags.iter().enumerate() {
            let group_id = group.id.clone();
            let tag = tag.clone();
            tags = tags.child(
                Button::new(SharedString::from(format!(
                    "speaking-remove-group-tag-{index}"
                )))
                .small()
                .outline()
                .label(format!("#{tag}  ×"))
                .tooltip("移除这个标签")
                .disabled(locked)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.edit_speaking_group_tag(&group_id, &tag, false, window, cx);
                })),
            );
        }
        tags.child(
            div()
                .w(px(164.))
                .max_w_full()
                .child(Input::new(&self.inputs["speaking_group_tag"]).disabled(locked)),
        )
        .child(
            Button::new("speaking-add-group-tag")
                .small()
                .outline()
                .label("添加标签")
                .disabled(locked)
                .on_click(cx.listener(move |this, _, window, cx| {
                    let tag = this.value("speaking_group_tag", cx);
                    if !tag.trim().is_empty() {
                        this.edit_speaking_group_tag(&group_id, &tag, true, window, cx);
                    }
                })),
        )
        .child(
            Button::new("speaking-generate-group-tags")
                .small()
                .outline()
                .label("✦ AI 生成标签")
                .tooltip(if has_transcripts {
                    "发送本组已转写的音频文字，归纳主题并追加新标签"
                } else {
                    "请先转写本组的题目音频"
                })
                .disabled(locked || !has_transcripts)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.start_speaking_group_tags(ai_group.clone(), cx);
                })),
        )
    }

    pub(super) fn speaking_library_view(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let available =
            f32::from(window.viewport_size().width) - 224. * self.navigation_progress() - 64.;
        let stacked = available < 730.;
        let locked = self.speaking_locked();
        let browser = self.speaking_question_browser(stacked, cx);
        let layout = div()
            .w_full()
            .min_w_0()
            .flex()
            .items_start()
            .gap_4()
            .when(stacked, |layout| layout.flex_col())
            .child(browser)
            .child(
                panel(cx)
                    .min_w_0()
                    .when_else(stacked, |detail| detail.w_full(), |detail| detail.flex_1())
                    .when_else(
                        self.speaking.current.is_some(),
                        |detail| {
                            let question = self.speaking.current.as_ref().unwrap();
                            detail
                                .child(self.speaking_question_heading(question, locked, cx))
                                .children(
                                    (self.speaking_dirty(cx) || self.speaking.saving).then(|| {
                                        self.speaking_question_actions(question, locked, cx)
                                    }),
                                )
                                .child(self.speaking_prompt_card(locked, cx))
                                .child(self.speaking_answer_draft_view(locked, cx))
                        },
                        |detail| {
                            detail
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("题目详情"),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("创建一个分组，即可添加音频题目。"),
                                )
                        },
                    ),
            );
        #[cfg(debug_assertions)]
        let layout = if let Some(bounds) = self.speaking.layout_probe.clone() {
            layout
                .on_children_prepainted(move |children, _, _| bounds.set(children.try_into().ok()))
        } else {
            layout
        };
        layout
    }

    fn speaking_question_heading(
        &self,
        question: &Question,
        locked: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("题目详情"),
            )
            .when_else(
                self.speaking.editing_title,
                |heading| {
                    let saved_title = question.title.clone();
                    heading.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div().flex_1().min_w_0().child(
                                    Input::new(&self.inputs["speaking_title"]).disabled(locked),
                                ),
                            )
                            .child(
                                Button::new("speaking-cancel-rename")
                                    .small()
                                    .outline()
                                    .label("取消改名")
                                    .disabled(locked)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.set("speaking_title", saved_title.clone(), window, cx);
                                        this.speaking.editing_title = false;
                                        cx.notify();
                                    })),
                            ),
                    )
                },
                |heading| {
                    heading.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .whitespace_normal()
                                    .text_xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(question.label()),
                            )
                            .child(
                                Button::new("speaking-rename-question")
                                    .small()
                                    .outline()
                                    .label("改名")
                                    .disabled(locked)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.speaking.editing_title = true;
                                        this.inputs["speaking_title"]
                                            .update(cx, |input, cx| input.focus(window, cx));
                                        cx.notify();
                                    })),
                            ),
                    )
                },
            )
    }

    fn speaking_question_browser(&self, stacked: bool, cx: &Context<Self>) -> impl IntoElement {
        let locked = self.speaking_locked();
        let question = self.speaking.current.as_ref();
        let group_id = question.map(|question| question.group_id.as_str());
        let active_group = self
            .speaking
            .groups
            .iter()
            .find(|group| Some(group.id.as_str()) == group_id);
        let filter = self.speaking_tag_filter().map(str::to_owned);
        let groups: Vec<_> = self
            .speaking
            .groups
            .iter()
            .filter(|group| {
                filter.as_ref().is_none_or(|filter| {
                    group
                        .tags
                        .iter()
                        .any(|tag| tag.eq_ignore_ascii_case(filter))
                })
            })
            .cloned()
            .collect();
        let mut available_tags = std::collections::BTreeMap::new();
        for tag in self.speaking.groups.iter().flat_map(|group| &group.tags) {
            available_tags
                .entry(tag.to_ascii_lowercase())
                .or_insert_with(|| tag.clone());
        }
        let filter_owner = cx.entity().downgrade();
        let owner = cx.entity().downgrade();
        let active_id = group_id.map(str::to_owned);
        let mut view = panel(cx)
            .min_w_0()
            .when_else(
                stacked,
                |list| list.w_full(),
                |list| list.w(px(270.)).flex_shrink_0(),
            )
            .p_3()
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
                            .child("口语题库"),
                    )
                    .child(
                        Button::new("speaking-new-group")
                            .outline()
                            .small()
                            .disabled(locked)
                            .label("＋ 新建组")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.begin_speaking_group(window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("speaking-select-group")
                            .outline()
                            .small()
                            .disabled(locked || groups.is_empty())
                            .w_full()
                            .min_w_0()
                            .child(div().flex_1().min_w_0().truncate().child(
                                active_group.map_or_else(
                                    || "选择分组".to_owned(),
                                    |group| group.name.clone(),
                                ),
                            ))
                            .dropdown_menu(move |mut menu, _, _| {
                                for group in &groups {
                                    let id = group.id.clone();
                                    let owner = owner.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(group.name.clone())
                                            .checked(active_id.as_deref() == Some(id.as_str()))
                                            .on_click(move |_, window, cx| {
                                                let _ = owner.update(cx, |this, cx| {
                                                    if this.may_switch_speaking_question(cx) {
                                                        this.open_speaking_group(&id, window, cx);
                                                    }
                                                    cx.notify();
                                                });
                                            }),
                                    );
                                }
                                menu.max_h(px(300.)).scrollable(true)
                            }),
                    )
                    .when(!available_tags.is_empty(), |bar| {
                        bar.child(
                            Button::new("speaking-filter-groups-by-tag")
                                .small()
                                .outline()
                                .w_full()
                                .min_w_0()
                                .child(div().flex_1().min_w_0().truncate().child(
                                    filter.as_ref().map_or_else(
                                        || "按标签筛选".to_owned(),
                                        |tag| format!("筛选 · #{tag}"),
                                    ),
                                ))
                                .dropdown_menu(move |mut menu, _, _| {
                                    for tag in std::iter::once(None)
                                        .chain(available_tags.values().cloned().map(Some))
                                    {
                                        let owner = filter_owner.clone();
                                        let checked = match (&tag, &filter) {
                                            (Some(tag), Some(filter)) => {
                                                tag.eq_ignore_ascii_case(filter)
                                            }
                                            _ => tag == filter,
                                        };
                                        let label = tag.as_ref().map_or_else(
                                            || "全部分组".to_owned(),
                                            |tag| format!("#{tag}"),
                                        );
                                        menu = menu.item(
                                            PopupMenuItem::new(label).checked(checked).on_click(
                                                move |_, _, cx| {
                                                    let _ = owner.update(cx, |this, cx| {
                                                        this.speaking.tag_filter = tag.clone();
                                                        cx.notify();
                                                    });
                                                },
                                            ),
                                        );
                                    }
                                    menu.max_h(px(300.)).scrollable(true)
                                }),
                        )
                    }),
            )
            .when(self.speaking.creating_group, |panel| {
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
                                .child(Input::new(&self.inputs["speaking_group"]).disabled(locked)),
                        )
                        .child(
                            Button::new("speaking-create-group")
                                .small()
                                .primary()
                                .label("创建分组")
                                .disabled(locked)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.create_speaking_group(window, cx)
                                })),
                        )
                        .child(
                            Button::new("speaking-cancel-group")
                                .small()
                                .outline()
                                .label("取消")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.speaking.creating_group = false;
                                    cx.notify();
                                })),
                        ),
                )
            });
        if let Some(group) = active_group {
            let questions: Vec<_> = self
                .speaking
                .questions
                .iter()
                .filter(|q| q.group_id == group.id)
                .collect();
            let count = questions.len();
            let mut list = div()
                .id("speaking-question-list")
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .max_h(px(if stacked { 142. } else { 390. }))
                .overflow_y_scroll();
            for item in questions {
                let selected = question.is_some_and(|question| question.id == item.id);
                let question = item.clone();
                let kind = match (item.audio.is_some(), item.text.trim().is_empty()) {
                    (true, false) => "已转写",
                    (true, true) => "待转写",
                    (false, false) => "已存文本",
                    (false, true) => "待添加音频",
                };
                list = list.child(
                    Button::new(SharedString::from(format!("speaking-question-{}", item.id)))
                        .ghost()
                        .w_full()
                        .min_w_0()
                        .h(px(36.))
                        .selected(selected)
                        .disabled(locked)
                        .tooltip(item.label())
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(div().flex_1().min_w_0().truncate().child(item.label()))
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(kind),
                                ),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if this
                                .speaking
                                .current
                                .as_ref()
                                .is_none_or(|current| current.id != question.id)
                                && this.may_switch_speaking_question(cx)
                            {
                                this.set_speaking_question(question.clone(), window, cx);
                            }
                            cx.notify();
                        })),
                );
            }
            view = view
                .child(
                    div()
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
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(if group.tags.is_empty() {
                                            "暂无分组标签".into()
                                        } else {
                                            group
                                                .tags
                                                .iter()
                                                .map(|tag| format!("#{tag}"))
                                                .collect::<Vec<_>>()
                                                .join("  ")
                                        }),
                                )
                                .child(
                                    Button::new("speaking-toggle-group-tags")
                                        .small()
                                        .outline()
                                        .label(if self.speaking.editing_group_tags {
                                            "收起标签"
                                        } else {
                                            "管理标签"
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.speaking.editing_group_tags =
                                                !this.speaking.editing_group_tags;
                                            cx.notify();
                                        })),
                                ),
                        )
                        .when(self.speaking.editing_group_tags, |tags| {
                            tags.child(self.speaking_group_tags(group, locked, cx))
                        }),
                )
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
                                .child(format!("题单 · {count} 题")),
                        )
                        .child(if count == 0 {
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("本组还没有题目")
                                .into_any_element()
                        } else {
                            list.into_any_element()
                        })
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_2()
                                .child(
                                    Button::new("speaking-new-question")
                                        .small()
                                        .primary()
                                        .disabled(locked)
                                        .label("＋ 新建题目")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            if this.may_switch_speaking_question(cx) {
                                                if let Some(group_id) = this
                                                    .speaking
                                                    .current
                                                    .as_ref()
                                                    .map(|q| q.group_id.clone())
                                                {
                                                    this.set_speaking_question(
                                                        Question::draft(group_id),
                                                        window,
                                                        cx,
                                                    );
                                                }
                                            }
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("speaking-command-import-group")
                                        .small()
                                        .outline()
                                        .disabled(locked)
                                        .label("批量导入")
                                        .tooltip("使用设置中的本机导入命令，每个音频建一道新题")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.start_speaking_command_import(false, window, cx);
                                        })),
                                ),
                        ),
                );
        }
        view
    }

    fn pick_speaking_answer_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.speaking_locked() {
            return;
        }
        let Some(question) = &self.speaking.current else {
            return;
        };
        let question_id = question.id.clone();
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("导入作答草稿（.txt 或 .md）".into()),
        });
        cx.spawn_in(window, async move |this, window| {
            let path = match picker.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                _ => None,
            };
            let Some(path) = path else { return };
            let text = window
                .background_executor()
                .spawn(async move { speaking::read_answer_draft(&path) })
                .await;
            let _ = this.update_in(window, |this, window, cx| {
                if this.page != "speaking"
                    || this.speaking_locked()
                    || this
                        .speaking
                        .current
                        .as_ref()
                        .is_none_or(|question| question.id != question_id)
                {
                    return;
                }
                if let Err(error) =
                    text.and_then(|text| this.append_speaking_answer_draft(&text, window, cx))
                {
                    this.report_status(FeedbackKind::Error, format!("导入作答草稿失败：{error:#}"));
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn append_speaking_answer_draft(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let mut draft = self.value("speaking_answer_draft", cx);
        if !draft.is_empty() {
            draft.push_str("\n\n");
        }
        draft.push_str(text);
        speaking::validate_answer_draft(&draft)?;
        self.set("speaking_answer_draft", draft, window, cx);
        Ok(())
    }

    fn speaking_answer_draft_view(&self, locked: bool, cx: &Context<Self>) -> impl IntoElement {
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("作答草稿"))
                    .child(
                        Button::new("speaking-import-answer-draft")
                            .small()
                            .outline()
                            .label("导入文本")
                            .tooltip("支持 UTF-8 的 .txt 和 .md，有内容时追加到末尾")
                            .disabled(locked)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.pick_speaking_answer_draft(window, cx)
                            })),
                    ),
            )
            .child(
                Input::new(&self.inputs["speaking_answer_draft"])
                    .w_full()
                    .h(px(160.))
                    .disabled(locked),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("点击上方「保存题目」保留草稿"),
            )
    }

    fn speaking_waveform_view(&self, path: &Path, cx: &Context<Self>) -> impl IntoElement {
        let state = &self.speaking.waveform;
        let result = (state.path.as_deref() == Some(path))
            .then_some(state.result.as_ref())
            .flatten();
        let waveform = result.and_then(|result| result.as_ref().ok());
        let playback = &self.media.snapshot;
        let loaded = self.speaking_playback_matches(path, playback);
        let position = if loaded {
            playback.position
        } else if state.path.as_deref() == Some(path) {
            state.start_at
        } else {
            0.
        };
        let duration = if loaded && playback.duration > 0. {
            playback.duration
        } else {
            waveform.map_or(0., |waveform| waveform.duration)
        };
        let peaks = waveform
            .map(|waveform| waveform.peaks.clone())
            .unwrap_or_default();
        let cursor = if duration > 0. {
            (position / duration).clamp(0., 1.) as f32
        } else {
            0.
        };
        let bounds_cell = state.bounds.clone();
        let primary = cx.theme().primary;
        let path = path.to_owned();
        let can_seek = duration > 0. && self.recording.is_none();
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .id("speaking-waveform")
                    .w_full()
                    .h(px(44.))
                    .rounded_md()
                    .overflow_hidden()
                    .bg(cx.theme().muted)
                    .when(can_seek, |track| track.cursor_pointer())
                    .child(
                        canvas(
                            move |bounds, _, _| bounds_cell.set(bounds),
                            move |bounds, _, window, _| {
                                audio_player_ui::paint_waveform_peaks(
                                    window, bounds, &peaks, primary,
                                );
                                if duration > 0. {
                                    let offset = (bounds.size.width - px(2.)).max(px(0.)) * cursor;
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(bounds.left() + offset, bounds.top()),
                                            size(px(2.), bounds.size.height),
                                        ),
                                        rgb(0xe87240),
                                    ));
                                }
                            },
                        )
                        .size_full(),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            let bounds = this.speaking.waveform.bounds.get();
                            if can_seek && bounds.size.width > px(0.) {
                                let fraction = ((event.position.x - bounds.left())
                                    / bounds.size.width)
                                    .clamp(0., 1.);
                                let time =
                                    (f64::from(fraction) * duration).min((duration - 0.01).max(0.));
                                this.seek_speaking_prompt(&path, time);
                                cx.notify();
                            }
                        }),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if duration > 0. {
                        format!("{} / {} · 点击音轨定位", clock(position), clock(duration))
                    } else if result.is_some() {
                        "暂时无法读取波形".into()
                    } else {
                        "正在读取波形…".into()
                    }),
            )
    }

    fn speaking_prompt_card(&self, locked: bool, cx: &Context<Self>) -> impl IntoElement {
        let audio = self.speaking_prompt_audio();
        let transcript = self.inputs["reference"].read(cx).value();
        let has_text = !transcript.trim().is_empty();
        let translation = self.speaking_translation(cx);
        let playing = audio.as_ref().is_some_and(|(path, _)| {
            self.speaking_playback_matches(path, &self.media.snapshot)
                && !self.media.snapshot.paused
                && !self.media.snapshot.eof
        });
        let mut card = div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .child(div().font_weight(FontWeight::SEMIBOLD).child("题目音频"));

        if let Some((path, name)) = &audio {
            card = card
                .child(
                    div()
                        .w_full()
                        .truncate()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(name.clone()),
                )
                .child(self.speaking_waveform_view(path, cx))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("speaking-play-prompt")
                                .small()
                                .primary()
                                .disabled(self.recording.is_some())
                                .label(if playing {
                                    "暂停播放"
                                } else {
                                    "▶ 播放题目"
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if playing {
                                        this.media.player.set("pause", "yes");
                                    } else if let Err(error) = this.play_speaking_prompt() {
                                        this.report_status(FeedbackKind::Error, error.to_string());
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(self.speaking_rate_picker(cx))
                        .child(
                            Button::new("speaking-remove-prompt")
                                .small()
                                .outline()
                                .label("移除音频")
                                .disabled(locked)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.remove_speaking_prompt(window, cx);
                                })),
                        ),
                );
        } else {
            card = card
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("尚未添加音频"),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            Button::new("speaking-pick-prompt")
                                .small()
                                .primary()
                                .disabled(locked)
                                .label("添加音频")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.pick_speaking_prompt(cx)),
                                ),
                        )
                        .child(
                            Button::new("speaking-command-import-question")
                                .small()
                                .outline()
                                .disabled(locked)
                                .label("用命令导入")
                                .tooltip("使用设置中的本机导入命令，命令需返回一个音频")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.start_speaking_command_import(true, window, cx);
                                })),
                        ),
                );
        }

        if audio.is_some() || has_text {
            card = card.child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .rounded_lg()
                    .bg(cx.theme().muted)
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(div().flex_1().font_weight(FontWeight::SEMIBOLD).child(
                                if audio.is_some() {
                                    "题目转写"
                                } else {
                                    "已保存的题目文字"
                                },
                            ))
                            .when(audio.is_some(), |bar| {
                                bar.child(
                                    Button::new("speaking-transcribe-prompt")
                                        .small()
                                        .outline()
                                        .label(if has_text {
                                            "重新转写"
                                        } else {
                                            "音频转文字"
                                        })
                                        .disabled(locked)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.start_speaking_text_task(false, cx);
                                        })),
                                )
                            })
                            .when(has_text, |bar| {
                                bar.child(
                                    Button::new("speaking-translate-prompt")
                                        .small()
                                        .outline()
                                        .label(if translation.is_some() {
                                            "重新翻译"
                                        } else {
                                            "翻译成中文"
                                        })
                                        .disabled(locked)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.start_speaking_text_task(true, cx);
                                        })),
                                )
                            }),
                    )
                    .when_else(
                        has_text,
                        |panel| {
                            panel.child(speaking_result_body("speaking-transcript", &transcript))
                        },
                        |panel| {
                            panel.child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("转写后，原文会显示在这里，也可用于跟读评分。"),
                            )
                        },
                    )
                    .when_some(translation, |panel, text| {
                        panel.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .border_t_1()
                                .border_color(cx.theme().border)
                                .pt_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("中文译文"),
                                )
                                .child(speaking_result_body("speaking-translation", text)),
                        )
                    }),
            );
        }

        card
    }

    fn speaking_question_actions(
        &self,
        question: &Question,
        locked: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let can_save = self.speaking_prompt_audio().is_some()
            || !self.value("reference", cx).trim().is_empty()
            || self
                .speaking
                .questions
                .iter()
                .any(|saved| saved.id == question.id);
        let question = question.clone();
        div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(
                Button::new("speaking-save-question")
                    .small()
                    .primary()
                    .label(if self.speaking.saving {
                        "正在保存…"
                    } else {
                        "保存题目"
                    })
                    .disabled(locked || !can_save)
                    .on_click(cx.listener(|this, _, _, cx| this.save_speaking_question(cx))),
            )
            .child(
                Button::new("speaking-revert-question")
                    .small()
                    .outline()
                    .label("撤销编辑")
                    .disabled(locked)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.stop_speaking_prompt();
                        this.set("reference", question.text.clone(), window, cx);
                        this.set("speaking_title", question.title.clone(), window, cx);
                        this.set(
                            "speaking_answer_draft",
                            question.answer_draft.clone(),
                            window,
                            cx,
                        );
                        this.speaking.translation = question
                            .translation
                            .clone()
                            .map(|text| (question.text.clone(), text));
                        this.speaking.audio_edit = AudioEdit::Keep;
                        this.speaking.editing_title = false;
                        cx.notify();
                    })),
            )
    }
}

fn speaking_result_body(id: &'static str, text: &str) -> impl IntoElement {
    div()
        .id(id)
        .min_w_0()
        .max_h(px(216.))
        .overflow_y_scroll()
        .whitespace_normal()
        .child(text.to_owned())
}

#[cfg(debug_assertions)]
pub(super) mod probe {
    use super::*;

    pub(crate) fn prepare(out: &Path) -> Result<Library> {
        let lib = Library::open(out.join("library"))?;
        lib.set_setting("last_page", "speaking")?;
        let mut wav = hound::WavWriter::create(
            lib.root.join("prompt.wav"),
            hound::WavSpec {
                channels: 1,
                sample_rate: 8000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )?;
        for sample in 0..16000 {
            let value = (f32::sin(sample as f32 * 0.2) * 8000.) as i16;
            wav.write_sample(value)?;
        }
        wav.finalize()?;
        fs::write(lib.root.join("draft.md"), "# Notes\r\n第二行")?;
        Ok(lib)
    }

    fn draw_next_frame(window: &Window, cx: &mut Context<AppView>) {
        cx.notify();
        let handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| window.draw(cx).clear());
        });
    }

    #[derive(Default)]
    pub(crate) struct Probe {
        stage: u8,
        seek_phase: u8,
    }

    impl diagnostics::Probe for Probe {
        fn tick(
            &mut self,
            app: &mut AppView,
            window: &mut Window,
            cx: &mut Context<AppView>,
        ) -> Result<bool> {
            match self.stage {
                0 => {
                    ensure!(app.page == "speaking");
                    app.media.player = Player::without_audio_device();
                    app.begin_speaking_group(window, cx);
                    app.set("speaking_group", "原生验收", window, cx);
                    app.create_speaking_group(window, cx);
                    let current = app
                        .speaking
                        .current
                        .as_ref()
                        .context("未选中口语分组")?
                        .clone();
                    app.set("speaking_group_tag", "跟读", window, cx);
                    app.edit_speaking_group_tag(&current.group_id, "跟读", true, window, cx);
                    ensure!(app.value("speaking_group_tag", cx).is_empty());
                    let groups = speaking::list_groups(&app.lib)?;
                    ensure!(
                        groups
                            .iter()
                            .any(|group| group.id == current.group_id && group.tags == ["跟读"])
                    );
                    ensure!(app.speaking.current.as_ref() == Some(&current));
                    app.speaking.tag_filter = Some("跟读".into());
                    ensure!(app.speaking_tag_filter() == Some("跟读"));
                    app.edit_speaking_group_tag(&current.group_id, "跟读", false, window, cx);
                    ensure!(app.speaking.tag_filter.is_none());

                    app.speaking.editing_title = true;
                    app.set("speaking_title", "练习题名称", window, cx);
                    app.set("speaking_answer_draft", "Existing draft", window, cx);
                    let imported = speaking::read_answer_draft(&app.lib.root.join("draft.md"))?;
                    app.append_speaking_answer_draft(&imported, window, cx)?;
                    ensure!(app.speaking_dirty(cx) && !app.may_switch_speaking_question(cx));

                    app.set("reference", "Listen and explain.", window, cx);
                    app.speaking.audio_edit = AudioEdit::Import(app.lib.root.join("prompt.wav"));
                    app.save_speaking_question(cx);
                }
                1 => {
                    if app.speaking.saving {
                        return Ok(false);
                    }
                    let Some(waveform) = &app.speaking.waveform.result else {
                        return Ok(false);
                    };
                    let waveform = waveform
                        .as_ref()
                        .map_err(|error| anyhow::anyhow!("{error:#}"))?;
                    ensure!(
                        (waveform.duration - 2.).abs() < 0.01
                            && waveform.peaks.iter().any(|peak| *peak > 0.)
                    );
                    if app.speaking.waveform.bounds.get().size.width <= px(0.) {
                        draw_next_frame(window, cx);
                        return Ok(false);
                    }
                    let saved = speaking::list(&app.lib)?;
                    ensure!(saved.len() == 1 && saved[0].text == "Listen and explain.");
                    ensure!(saved[0].title == "练习题名称" && !app.speaking.editing_title);
                    ensure!(saved[0].answer_draft == "Existing draft\n\n# Notes\n第二行");

                    ensure!(
                        saved[0].audio.as_ref().is_some_and(|audio| audio
                            .path
                            .starts_with(app.lib.root.join("media")))
                    );
                    app.recorded = Some(app.lib.root.join("student.wav"));
                    let shared_rate = app.media.rate;
                    app.set_speaking_rate(0.7, cx);
                    ensure!(
                        app.lib.setting("speaking_prompt_rate")? == "0.7"
                            && app.media.rate == shared_rate
                    );
                    let path = app.speaking_prompt_audio().unwrap().0.to_owned();
                    app.seek_speaking_prompt(&path, 0.8);
                    ensure!(
                        app.media.session.owner().is_none()
                            && app.speaking.waveform.start_at == 0.8
                    );
                    app.play_speaking_prompt()?;
                    ensure!(app.recorded.as_ref() == Some(&app.lib.root.join("student.wav")));
                }
                2 => {
                    let playback = app.media.player.snapshot();
                    ensure!(
                        playback.error.is_empty(),
                        "题目音频播放失败：{}",
                        playback.error
                    );
                    if !app.media.matches_snapshot(&playback) || playback.duration < 1.9 {
                        return Ok(false);
                    }
                    let path = app.speaking_prompt_audio().unwrap().0.to_owned();
                    match self.seek_phase {
                        0 => {
                            if playback.position < 0.7 {
                                return Ok(false);
                            }
                            app.media.player.set("pause", "yes");
                            self.seek_phase = 1;
                            return Ok(false);
                        }
                        1 => {
                            if !playback.paused {
                                return Ok(false);
                            }
                            app.seek_speaking_prompt(&path, 0.35);
                            self.seek_phase = 2;
                            return Ok(false);
                        }
                        2 => {
                            if (playback.position - 0.35).abs() > 0.08 {
                                return Ok(false);
                            }
                            ensure!(playback.paused, "点音轨时应保持暂停");
                            app.play_speaking_prompt()?;
                            self.seek_phase = 3;
                            return Ok(false);
                        }
                        _ => {
                            if playback.paused || playback.position <= 0.4 {
                                return Ok(false);
                            }
                        }
                    }
                    let group = app.speaking.current.as_ref().unwrap().group_id.clone();
                    app.set_speaking_question(Question::draft(group), window, cx);
                    ensure!(
                        app.media.session.owner().is_none()
                            && app.value("reference", cx).is_empty()
                            && app.value("speaking_answer_draft", cx).is_empty()
                            && app.recorded.is_none()
                    );
                    app.poll_speaking_waveform();
                    ensure!(
                        app.speaking.waveform.result.is_none()
                            && app.speaking.waveform.path.is_none()
                    );
                    let path = app
                        .lib
                        .root
                        .join("prompt.wav")
                        .to_string_lossy()
                        .into_owned();
                    let command = if cfg!(windows) {
                        format!("Write-Output '{}'", path.replace('\'', "''"))
                    } else {
                        format!("printf '%s\\n' '{}'", path.replace('\'', "'\\''"))
                    };
                    app.set("command", command, window, cx);
                    app.handle(Action::SaveCommandImport, window, cx);
                    app.set("reference", "Keep this question text.", window, cx);
                    app.set(
                        "speaking_answer_draft",
                        "Keep this answer draft.",
                        window,
                        cx,
                    );
                    app.start_speaking_command_import(true, window, cx);
                    ensure!(app.tasks.foreground.is_running());
                }
                3 => {
                    app.poll_tasks(window, cx);
                    if app.tasks.foreground.is_running() {
                        return Ok(false);
                    }
                    let question = app
                        .speaking
                        .current
                        .as_ref()
                        .context("命令导入后当前题目丢失")?;
                    ensure!(
                        question.text == "Keep this question text." && question.audio.is_some(),
                        "命令未附加到原题：{}",
                        app.status
                    );
                    ensure!(app.value("reference", cx) == question.text && app.recorded.is_none());
                    ensure!(question.answer_draft == "Keep this answer draft.");
                    ensure!(speaking::list(&app.lib)?.contains(question));
                    let id = question.id.clone();
                    // A completed transcription becomes the reference and is saved directly.
                    app.finish_speaking_text(
                        id,
                        PromptText::Transcript("New transcript".into()),
                        false,
                        window,
                        cx,
                    );
                    ensure!(app.value("reference", cx) == "New transcript");
                }
                4 => {
                    if app.speaking.saving {
                        return Ok(false);
                    }
                    let question = app.speaking.current.as_ref().unwrap();
                    ensure!(
                        question.text == "New transcript"
                            && speaking::list(&app.lib)?.contains(question)
                    );
                    app.finish_speaking_text(
                        question.id.clone(),
                        PromptText::Translation {
                            source: "New transcript".into(),
                            text: "新的转写。".into(),
                        },
                        false,
                        window,
                        cx,
                    );
                }
                5 => {
                    if app.speaking.saving {
                        return Ok(false);
                    }
                    let question = app.speaking.current.as_ref().unwrap().clone();
                    ensure!(question.translation.as_deref() == Some("新的转写。"));
                    ensure!(speaking::list(&app.lib)?.contains(&question));
                    app.start_speaking_command_import(true, window, cx);
                    ensure!(!app.tasks.foreground.is_running(), "已有音频时不能命令覆盖");
                    app.remove_speaking_prompt(window, cx);
                    ensure!(app.value("reference", cx).is_empty());
                    ensure!(
                        app.speaking_translation(cx).is_none()
                            && app.speaking_prompt_audio().is_none()
                    );
                    app.save_speaking_question(cx);
                }
                6 => {
                    if app.speaking.saving {
                        return Ok(false);
                    }
                    let question = app.speaking.current.as_ref().unwrap();
                    ensure!(
                        question.audio.is_none()
                            && question.text.is_empty()
                            && question.translation.is_none()
                    );
                    ensure!(speaking::list(&app.lib)?.contains(question));
                    app.start_speaking_command_import(true, window, cx);
                    ensure!(app.tasks.foreground.is_running());
                }
                7 => {
                    app.poll_tasks(window, cx);
                    if app.tasks.foreground.is_running() {
                        return Ok(false);
                    }
                    let question = app.speaking.current.as_ref().unwrap();
                    ensure!(question.audio.is_some() && question.text.is_empty());
                    ensure!(question.answer_draft == "Keep this answer draft.");
                    ensure!(speaking::list(&app.lib)?.contains(question));
                    let group = speaking::create_group(&app.lib, "悬浮窗目标")?;
                    app.refresh_speaking_groups()?;
                    app.handle(Action::Page("flashcard".into()), window, cx);
                    ensure!(app.page == "flashcard");
                    app.handle(
                        super::super::floating_ui::speaking_probe_action(app, &group.id)?,
                        window,
                        cx,
                    );
                    ensure!(app.tasks.foreground.is_running());
                }
                8 => {
                    app.poll_tasks(window, cx);
                    if app.tasks.foreground.is_running() {
                        return Ok(false);
                    }
                    let group = speaking::list_groups(&app.lib)?
                        .into_iter()
                        .find(|group| group.name == "悬浮窗目标")
                        .context("未创建目标组")?;
                    let saved = speaking::list(&app.lib)?;
                    let [imported] = saved
                        .iter()
                        .filter(|question| question.group_id == group.id)
                        .collect::<Vec<_>>()[..]
                    else {
                        anyhow::bail!("悬浮窗命令未导入指定组：{}", app.status);
                    };
                    ensure!(imported.audio.is_some() && imported.text.is_empty());
                    ensure!(
                        app.speaking
                            .current
                            .as_ref()
                            .is_some_and(|question| question.group_id != group.id)
                    );
                    ensure!(
                        app.value("reference", cx).is_empty() && app.command_imported.is_none()
                    );
                    app.handle(Action::Page("speaking".into()), window, cx);
                    app.speaking.layout_probe = Some(SpeakingLayoutBounds::default());
                    window.resize(size(px(1400.), px(800.)));
                    draw_next_frame(window, cx);
                }
                9 => {
                    let bounds = app.speaking.layout_probe.as_ref().unwrap();
                    let Some([list, detail]) = bounds.get() else {
                        return Ok(false);
                    };
                    ensure!(
                        list.origin.y == detail.origin.y && list.right() < detail.origin.x,
                        "宽窗口未左右分栏：{list:?}，{detail:?}"
                    );
                    bounds.set(None);
                    window.resize(size(px(760.), px(760.)));
                    draw_next_frame(window, cx);
                }
                _ => {
                    let Some([list, detail]) = app.speaking.layout_probe.as_ref().unwrap().get()
                    else {
                        return Ok(false);
                    };
                    ensure!(
                        list.origin.x == detail.origin.x && list.bottom() < detail.origin.y,
                        "窄窗口未上下排列：{list:?}，{detail:?}"
                    );
                    return Ok(true);
                }
            }
            self.stage += 1;
            Ok(false)
        }

        fn failed(&mut self, _: String) -> Result<()> {
            Ok(())
        }
    }
}

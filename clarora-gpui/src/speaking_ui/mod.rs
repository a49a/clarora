//! Question library for the speaking workspace.
use super::*;
use clarora_gpui::{
    speaking::{self, AudioEdit, Group, Question},
    writing::{Review, ReviewMode, ReviewSubject},
};
use writing_ui::{ReviewAction, ReviewHistory};

const SPEAKING_RATES: [f64; 8] = [0.6, 0.7, 0.8, 0.9, 1., 1.25, 1.5, 2.];

pub(super) enum PromptText {
    Transcript(String),
    Translation { source: String, text: String },
}

#[cfg(debug_assertions)]
type SpeakingLayoutBounds = std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>>;

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
    draft_autosave: Option<Task<()>>,
    draft_save_error: Option<String>,
    show_reviews: bool,
    selected_review: Option<String>,
    show_review_source: bool,
    show_review_notes: bool,
    review_scroll: ScrollHandle,
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
        if initial && let Some(group) = self.speaking.groups.first() {
            let id = group.id.clone();
            self.open_speaking_group(&id, window, cx);
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
        self.speaking_draft_dirty(cx) || self.speaking_question_dirty(cx)
    }

    fn speaking_question_dirty(&self, cx: &App) -> bool {
        self.speaking.current.as_ref().is_some_and(|question| {
            self.value("speaking_title", cx).trim() != question.title
                || self.value("reference", cx) != question.text
                || self.speaking_translation(cx) != question.translation.as_deref()
                || !matches!(self.speaking.audio_edit, AudioEdit::Keep)
        })
    }

    fn speaking_draft_dirty(&self, cx: &App) -> bool {
        self.speaking.current.as_ref().is_some_and(|question| {
            self.inputs["speaking_answer_draft"]
                .read(cx)
                .value()
                .as_ref()
                != question.answer_draft
                || self.speaking_target_words(cx) != question.target_words
        })
    }

    fn speaking_target_words(&self, cx: &App) -> Option<usize> {
        self.inputs["speaking_target_words"]
            .read(cx)
            .value()
            .as_ref()
            .trim()
            .parse()
            .ok()
    }

    pub(super) fn schedule_speaking_draft_save(&mut self, cx: &mut Context<Self>) {
        self.speaking.draft_autosave.take();
        self.speaking.draft_save_error = None;
        if self.speaking_draft_dirty(cx) {
            self.speaking.draft_autosave = Some(cx.spawn(async move |this, cx| {
                Timer::after(Duration::from_millis(650)).await;
                let _ = this.update(cx, |this, cx| {
                    if !this.speaking.saving {
                        let _ = this.flush_speaking_draft(cx);
                    }
                });
            }));
        }
        cx.notify();
    }

    pub(super) fn flush_speaking_draft(&mut self, cx: &mut Context<Self>) -> Result<()> {
        self.speaking.draft_autosave.take();
        if !self.speaking_draft_dirty(cx) {
            return Ok(());
        }
        ensure!(!self.speaking.saving, "口语题目正在保存，请稍候");
        let answer = self.value("speaking_answer_draft", cx);
        let target_words = self.speaking_target_words(cx);
        speaking::validate_target_words(target_words)?;
        let question = self.speaking.current.as_mut().unwrap();
        let result = speaking::save_answer_draft(&self.lib, question, &answer, target_words);
        match &result {
            Ok(()) => {
                question.answer_draft = answer.clone();
                question.target_words = target_words;
                if let Some(saved) = self
                    .speaking
                    .questions
                    .iter_mut()
                    .find(|saved| saved.id == question.id)
                {
                    saved.answer_draft = answer;
                    saved.target_words = target_words;
                } else {
                    self.speaking.questions.insert(0, question.clone());
                }
                self.speaking.draft_save_error = None;
            }
            Err(error) => self.speaking.draft_save_error = Some(error.to_string()),
        }
        cx.notify();
        result
    }

    fn may_switch_speaking_question(&mut self, cx: &mut Context<Self>) -> bool {
        if self.speaking_locked() {
            return false;
        }
        if let Err(error) = self.flush_speaking_draft(cx) {
            self.report_status(FeedbackKind::Error, format!("作答草稿尚未保存：{error:#}"));
            cx.notify();
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
        self.speaking.draft_autosave.take();
        self.speaking.draft_save_error = None;
        self.stop_speaking_prompt();
        self.speaking.waveform.start_at = 0.;
        self.speaking.show_reviews = false;
        self.speaking.selected_review = None;
        self.speaking.show_review_source = false;
        self.speaking.show_review_notes = false;
        self.speaking
            .review_scroll
            .set_offset(point(px(0.), px(0.)));
        self.speaking.editing_title = false;
        self.set("speaking_title", question.title.clone(), window, cx);
        self.set("reference", question.text.clone(), window, cx);
        self.set(
            "speaking_answer_draft",
            question.answer_draft.clone(),
            window,
            cx,
        );
        self.set(
            "speaking_target_words",
            question
                .target_words
                .map_or_else(String::new, |count| count.to_string()),
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

    pub(super) fn open_speaking_delete_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        ensure!(self.page == "speaking", "请先进入口语练习");
        ensure!(!self.speaking_locked(), "请先等待当前任务完成");
        let question = self.speaking.current.clone().context("请先选择题目")?;
        let saved = self
            .speaking
            .questions
            .iter()
            .any(|item| item.id == question.id);
        let id = question.id.clone();
        let group_id = question.group_id.clone();
        let label = question.label();
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, window, cx| {
            let confirm = view.clone();
            let id = id.clone();
            let group_id = group_id.clone();
            dialog
                .title(if saved {
                    "删除这道题？"
                } else {
                    "放弃未保存的题目？"
                })
                .confirm()
                .w((window.viewport_size().width - px(32.)).min(px(520.)))
                .button_props(
                    DialogButtonProps::default()
                        .cancel_text("取消")
                        .ok_text(if saved { "确认删除" } else { "放弃题目" })
                        .ok_variant(ButtonVariant::Danger),
                )
                .child(div().text_sm().child(format!("题目：{label}")))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(if saved {
                            "题目、作答草稿和批改历史会从本机删除；原始音频文件保留。此操作不能撤销。"
                        } else {
                            "这道题还没有保存到题库；放弃后不会保留当前编辑内容。"
                        }),
                )
                .on_ok(move |_, window, cx| {
                    confirm
                        .update(cx, |this, cx| {
                            let result = if saved {
                                clarora_gpui::speaking::delete(&this.lib, &id)
                            } else {
                                Ok(())
                            };
                            match result {
                                Ok(()) => {
                                    this.speaking.questions.retain(|item| item.id != id);
                                    let next = this
                                        .speaking
                                        .questions
                                        .iter()
                                        .find(|item| item.group_id == group_id)
                                        .cloned()
                                        .unwrap_or_else(|| Question::draft(group_id.clone()));
                                    this.set_speaking_question(next, window, cx);
                                    this.report_status(
                                        FeedbackKind::Info,
                                        if saved {
                                            "题目已删除；原始音频文件保留".to_string()
                                        } else {
                                            "已放弃未保存的题目".to_string()
                                        },
                                    );
                                    true
                                }
                                Err(error) => {
                                    this.report_status(
                                        FeedbackKind::Error,
                                        format!("删除题目失败：{error:#}"),
                                    );
                                    cx.notify();
                                    false
                                }
                            }
                        })
                        .unwrap_or(true)
                })
        });
        Ok(())
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
            if let Ok(Ok(Some(paths))) = picker.await
                && let Some(path) = paths.into_iter().next()
            {
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
        })
        .detach();
    }

    fn start_speaking_command_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.speaking_locked() || self.speaking_prompt_audio().is_some() {
            return;
        }
        if self.import_command.trim().is_empty() {
            self.handle(Action::ConfigureCommandImport, window, cx);
            return;
        }
        if let Err(error) = self.flush_speaking_draft(cx) {
            self.report_status(FeedbackKind::Error, format!("作答草稿尚未保存：{error:#}"));
            cx.notify();
            return;
        }
        let Some(current) = &self.speaking.current else {
            return;
        };
        let mut question = current.clone();
        question.title = self.value("speaking_title", cx);
        question.answer_draft = self.value("speaking_answer_draft", cx);
        question.target_words = self.speaking_target_words(cx);
        question.text = self.value("reference", cx);
        question.translation = self.speaking_translation(cx).map(str::to_owned);
        self.start_speaking_command_import_to(current.group_id.clone(), Some(question), window, cx);
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
        for mut question in report.questions {
            if report.attached
                && let Some(current) = self
                    .speaking
                    .current
                    .as_ref()
                    .filter(|item| item.id == question.id)
            {
                // The draft may have autosaved after the import worker committed.
                question.answer_draft.clone_from(&current.answer_draft);
                question.target_words = current.target_words;
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
        self.persist_speaking_question(None, cx);
    }

    fn persist_speaking_question(
        &mut self,
        review_mode: Option<ReviewMode>,
        cx: &mut Context<Self>,
    ) {
        if self.speaking.saving || self.tasks.foreground.is_running() {
            return;
        }
        let Some(mut question) = self.speaking.current.clone() else {
            return;
        };
        question.title = self.value("speaking_title", cx);
        question.answer_draft = self.value("speaking_answer_draft", cx);
        question.target_words = self.speaking_target_words(cx);
        question.text = self.value("reference", cx);
        question.translation = self.speaking_translation(cx).map(str::to_owned);
        let lib = self.lib.clone();
        let audio = self.speaking.audio_edit.clone();
        self.speaking.draft_autosave.take();
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
                        this.speaking.draft_save_error = None;
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
                        if let Some(mode) = review_mode {
                            this.request_speaking_review(mode);
                        } else {
                            this.report_status(FeedbackKind::Info, "口语题目已保存".into());
                        }
                    }
                    Err(error) => {
                        if this.speaking_draft_dirty(cx) {
                            this.speaking.draft_save_error = Some(error.to_string());
                        }
                        this.report_status(
                            FeedbackKind::Error,
                            format!("保存口语题目失败：{error:#}"),
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn start_speaking_review(&mut self, mode: ReviewMode, cx: &mut Context<Self>) {
        if self.speaking_locked() || self.speaking.current.is_none() {
            return;
        }
        let draft = self.value("speaking_answer_draft", cx);
        let validation = if self.value("reference", cx).trim().is_empty() {
            Err(anyhow::anyhow!("请先转写题目音频"))
        } else if draft.trim().is_empty() {
            Err(anyhow::anyhow!("请先填写作答草稿"))
        } else {
            speaking::validate_answer_draft(&draft)
        };
        if let Err(error) = validation {
            self.report_status(FeedbackKind::Error, error.to_string());
        } else if self.speaking_dirty(cx) {
            self.persist_speaking_question(Some(mode), cx);
        } else {
            self.request_speaking_review(mode);
        }
        cx.notify();
    }

    fn request_speaking_review(&mut self, mode: ReviewMode) {
        if self.tasks.foreground.is_running() {
            self.report_status(
                FeedbackKind::Info,
                "草稿已保存，当前任务结束后可重新提交批改".into(),
            );
            return;
        }
        let Some(snapshot) = self.speaking.current.clone() else {
            return;
        };
        let label = format!("正在批改口语草稿「{}」…", snapshot.label());
        self.begin_scoped(TaskScope::Application, &label, move |lib, cancel, _| {
            let review = speaking::review_draft(&lib, &snapshot, mode, &cancel)?;
            Ok(Outcome::SpeakingReviewed(snapshot.id, review))
        });
        if self.tasks.foreground.is_running() {
            self.tasks.notice = Some(TaskNotice::new("口语草稿批改", &label));
        }
    }

    pub(super) fn finish_speaking_review(&mut self, id: &str, review: Review) {
        if let Some(question) = self
            .speaking
            .questions
            .iter_mut()
            .find(|question| question.id == id)
        {
            question.reviews.retain(|saved| saved.id != review.id);
            question.reviews.insert(0, review.clone());
        }
        if let Some(question) = self
            .speaking
            .current
            .as_mut()
            .filter(|question| question.id == id)
        {
            question.reviews.retain(|saved| saved.id != review.id);
            self.speaking.selected_review = Some(review.id.clone());
            question.reviews.insert(0, review);
            self.speaking.show_reviews = true;
            self.speaking.show_review_source = false;
            self.speaking.show_review_notes = false;
            self.speaking
                .review_scroll
                .set_offset(point(px(0.), px(0.)));
        }
        self.report_status(FeedbackKind::Info, "口语批改已保存".into());
    }

    fn speaking_review_action(
        &mut self,
        action: ReviewAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            ReviewAction::Select(id) => {
                self.speaking.selected_review = Some(id);
                self.speaking.show_review_source = false;
                self.speaking.show_review_notes = false;
                self.speaking
                    .review_scroll
                    .set_offset(point(px(0.), px(0.)));
            }
            ReviewAction::ToggleSubmission => {
                self.speaking.show_review_source = !self.speaking.show_review_source
            }
            ReviewAction::ToggleNotes => {
                self.speaking.show_review_notes = !self.speaking.show_review_notes
            }
            ReviewAction::FollowUp(id) => {
                if let Some(review) = self
                    .speaking
                    .current
                    .as_ref()
                    .and_then(|question| question.reviews.iter().find(|review| review.id == id))
                {
                    let context = review.follow_up_context_for(ReviewSubject::Speaking);
                    writing_ui::quote_review(self, context, window, cx);
                }
            }
        }
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

    fn start_speaking_draft_audio(&mut self, cx: &mut Context<Self>) -> Result<()> {
        ensure!(self.data.tts_ready, "请先在音频页下载 Kokoro 模型");
        ensure!(
            !self.tasks.foreground.is_running(),
            "请先完成或取消当前任务再生成读音"
        );
        let text = self.inputs["speaking_answer_draft"]
            .read(cx)
            .value()
            .to_string();
        ensure!(!text.trim().is_empty(), "请先填写作答草稿");
        let voice = self.tts_voice;
        let speed = self.tts_speed;
        if let Some(path) = tts::cached_preview(&self.lib, &text, voice, speed) {
            self.play_speaking_draft_audio(path)?;
            self.report_status(FeedbackKind::Info, "正在播放草稿读音".into());
            return Ok(());
        }
        self.begin_scoped(
            TaskScope::Application,
            "正在生成草稿读音…",
            move |lib, cancel, _| {
                let path = tts::preview(&lib, &tts::model_dir(&lib), &text, voice, speed, &cancel)?;
                Ok(Outcome::SpeakingDraftAudio(path))
            },
        );
        if self.tasks.foreground.is_running() {
            self.tasks.notice = Some(TaskNotice::new(
                "口语草稿读音",
                "正在本机生成发音，首次加载模型需稍候…",
            ));
        }
        cx.notify();
        Ok(())
    }

    pub(super) fn finish_speaking_draft_audio(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let current = self.inputs["speaking_answer_draft"]
            .read(cx)
            .value()
            .to_string();
        let expected = tts::preview_path(&self.lib, &current, self.tts_voice, self.tts_speed);
        if expected != path {
            self.report_status(
                FeedbackKind::Info,
                "草稿读音已生成，但作答或音色已变化，可重新生成".into(),
            );
            return;
        }
        self.report_status(FeedbackKind::Info, "草稿读音已生成".into());
        if let Err(error) = self.play_speaking_draft_audio(path) {
            self.report_status(FeedbackKind::Error, format!("播放草稿读音失败：{error:#}"));
        }
    }

    fn play_speaking_draft_audio(&mut self, path: PathBuf) -> Result<()> {
        ensure!(self.recording.is_none(), "请先停止作答录音");
        ensure!(path.is_file(), "草稿读音文件不存在，请重新生成");
        let playback = self.media.player.snapshot();
        if self.speaking_playback_matches(&path, &playback) {
            if playback.eof {
                self.media.open(
                    PlaybackOwner::Speaking,
                    path,
                    0.,
                    0.,
                    false,
                    PlaybackMode::Normal,
                );
            } else if playback.paused {
                self.media.player.set("pause", "no");
            } else {
                self.media.player.set("pause", "yes");
            }
        } else {
            self.media.open(
                PlaybackOwner::Speaking,
                path,
                0.,
                0.,
                false,
                PlaybackMode::Normal,
            );
        }
        self.media.player.set("speed", 1.);
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
        if self
            .speaking
            .current
            .as_ref()
            .is_none_or(|question| question.id != id)
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

    pub(super) fn prepare_speaking_group_check(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let group_id = self
            .speaking
            .current
            .as_ref()
            .context("请先选择题目的分组")?
            .group_id
            .clone();
        let group = self
            .speaking
            .groups
            .iter()
            .find(|group| group.id == group_id)
            .cloned()
            .context("口语分组不存在，请刷新后重试")?;
        let text = speaking::group_review_context(&group, &self.speaking.questions)?;
        self.add_assistant_reference(
            clarora_gpui::assistant::Reference {
                text,
                ..Default::default()
            },
            window,
            cx,
        )?;
        self.set("question", speaking::GROUP_REVIEW_PROMPT, window, cx);
        self.inputs["question"].update(cx, |state, cx| state.focus(window, cx));
        self.report_status(
            FeedbackKind::Info,
            "已加入整组作答引用并填好批改要求，确认后可发送".into(),
        );
        Ok(())
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
}

mod views;

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
#[path = "../speaking_ui_probe.rs"]
pub(super) mod probe;

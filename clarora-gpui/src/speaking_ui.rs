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
        if attach && let Err(error) = self.flush_speaking_draft(cx) {
            self.report_status(FeedbackKind::Error, format!("作答草稿尚未保存：{error:#}"));
            cx.notify();
            return;
        }
        let Some(current) = &self.speaking.current else {
            return;
        };
        let question = attach.then(|| {
            let mut question = current.clone();
            question.title = self.value("speaking_title", cx);
            question.answer_draft = self.value("speaking_answer_draft", cx);
            question.target_words = self.speaking_target_words(cx);
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

    pub(super) fn speaking_library_view(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let available =
            f32::from(window.viewport_size().width) - 224. * self.navigation_progress() - 64.;
        let detail_width = (available - 42.).max(0.);
        let compare_reviews = detail_width >= 760.;
        let review_height = (f32::from(window.viewport_size().height) - 320.).clamp(360., 720.);
        let locked = self.speaking_locked();
        let muted = cx.theme().muted_foreground;
        let layout = div()
            .w_full()
            .min_w_0()
            .child(panel(cx).w_full().min_w_0().when_else(
                self.speaking.current.is_some(),
                |detail| {
                    let question = self.speaking.current.as_ref().unwrap();
                    detail
                        .child(self.speaking_question_heading(question, locked, cx))
                        .children(
                            (self.speaking_question_dirty(cx) || self.speaking.saving)
                                .then(|| self.speaking_question_actions(question, locked, cx)),
                        )
                        .child(self.speaking_prompt_card(locked, cx))
                        .child({
                            let history = (self.speaking.show_reviews
                                && !question.reviews.is_empty())
                            .then(|| self.speaking_review_history(question, window, cx));
                            match history {
                                Some(history) if compare_reviews => {
                                    let left_width = (detail_width * 0.5).clamp(400., 800.);
                                    let review_width = (detail_width - left_width - 16.).max(360.);
                                    let draft_height = (review_height - 150.).max(200.);
                                    let answer = self.speaking_answer_draft_view(
                                        question,
                                        locked,
                                        Some(px(draft_height)),
                                        cx,
                                    );
                                    div()
                                        .w_full()
                                        .flex()
                                        .items_start()
                                        .gap_4()
                                        .child(
                                            div().w(px(left_width)).flex_shrink_0().child(answer),
                                        )
                                        .child(
                                            div()
                                                .relative()
                                                .w(px(review_width))
                                                .h(px(review_height))
                                                .overflow_hidden()
                                                .child(
                                                    div()
                                                        .id("speaking-review-panel")
                                                        .size_full()
                                                        .overflow_y_scroll()
                                                        .track_scroll(&self.speaking.review_scroll)
                                                        .on_scroll_wheel(|_, _, cx| {
                                                            cx.stop_propagation()
                                                        })
                                                        .child(history),
                                                )
                                                .child(
                                                    div()
                                                        .absolute()
                                                        .top_0()
                                                        .left_0()
                                                        .size_full()
                                                        .child(
                                                            Scrollbar::vertical(
                                                                &self.speaking.review_scroll,
                                                            )
                                                            .scrollbar_show(ScrollbarShow::Always),
                                                        ),
                                                ),
                                        )
                                        .into_any_element()
                                }
                                Some(history) => div()
                                    .w_full()
                                    .flex()
                                    .flex_col()
                                    .gap_4()
                                    .child(
                                        self.speaking_answer_draft_view(question, locked, None, cx),
                                    )
                                    .child(
                                        div()
                                            .relative()
                                            .w_full()
                                            .h(px(420.))
                                            .overflow_hidden()
                                            .child(
                                                div()
                                                    .id("speaking-review-panel")
                                                    .size_full()
                                                    .overflow_y_scroll()
                                                    .track_scroll(&self.speaking.review_scroll)
                                                    .on_scroll_wheel(|_, _, cx| {
                                                        cx.stop_propagation()
                                                    })
                                                    .child(history),
                                            )
                                            .child(
                                                div()
                                                    .absolute()
                                                    .top_0()
                                                    .left_0()
                                                    .size_full()
                                                    .child(
                                                        Scrollbar::vertical(
                                                            &self.speaking.review_scroll,
                                                        )
                                                        .scrollbar_show(ScrollbarShow::Always),
                                                    ),
                                            ),
                                    )
                                    .into_any_element(),
                                None => self
                                    .speaking_answer_draft_view(question, locked, None, cx)
                                    .into_any_element(),
                            }
                        })
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
                                .text_color(muted)
                                .child("创建一个分组，即可添加音频题目。"),
                        )
                },
            ));
        #[cfg(debug_assertions)]
        let layout = if let Some(bounds) = self.speaking.layout_probe.clone() {
            layout
                .on_children_prepainted(move |children, _, _| bounds.set(children.first().copied()))
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

    pub(super) fn speaking_header_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
        let owner = cx.entity().downgrade();
        let filter_owner = cx.entity().downgrade();
        let active_id = group_id.map(str::to_owned);
        let questions: Vec<_> = active_group
            .map(|group| {
                self.speaking
                    .questions
                    .iter()
                    .filter(|question| question.group_id == group.id)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let count = questions.len();
        let answered = questions
            .iter()
            .filter(|question| !question.answer_draft.trim().is_empty())
            .count();
        let mut controls = div()
            .min_w_0()
            .flex_1()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .rounded_xl()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("分组"),
                    )
                    .child(
                        Button::new("speaking-select-group")
                            .outline()
                            .small()
                            .h(px(30.))
                            .disabled(locked || groups.is_empty())
                            .w(px(190.))
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
                    .when(!available_tags.is_empty(), |row| {
                        row.child(
                            Button::new("speaking-filter-groups-by-tag")
                                .small()
                                .outline()
                                .h(px(30.))
                                .w(px(150.))
                                .child(div().flex_1().min_w_0().truncate().child(
                                    filter.as_ref().map_or_else(
                                        || "标签筛选".to_owned(),
                                        |tag| format!("#{tag} · 筛选"),
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
                    })
                    .child(
                        Button::new("speaking-new-group")
                            .ghost()
                            .small()
                            .h(px(30.))
                            .disabled(locked)
                            .label("＋ 新建组")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.begin_speaking_group(window, cx);
                            })),
                    )
                    .when(active_group.is_some(), |row| {
                        row.child(
                            Button::new("speaking-toggle-group-tags")
                                .ghost()
                                .small()
                                .h(px(30.))
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
                        )
                    })
                    .when_some(
                        active_group.filter(|group| !group.tags.is_empty()),
                        |row, group| {
                            row.child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(
                                        group
                                            .tags
                                            .iter()
                                            .map(|tag| format!("#{tag}"))
                                            .collect::<Vec<_>>()
                                            .join("  "),
                                    ),
                            )
                        },
                    ),
            )
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("题目 · {count}")),
                    )
                    .child(
                        div()
                            .id("speaking-question-strip")
                            .flex_1()
                            .min_w(px(180.))
                            .flex()
                            .items_center()
                            .overflow_x_scroll()
                            .when(count == 0, |strip| {
                                strip.child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("本组还没有题目"),
                                )
                            })
                            .when(count > 0, |strip| {
                                strip.child(
                                    div()
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .p_1()
                                        .rounded_full()
                                        .bg(cx.theme().muted)
                                        .children(questions.into_iter().map(|item| {
                                            let selected = question
                                                .is_some_and(|question| question.id == item.id);
                                            let question_id = item.id.clone();
                                            let kind = match (
                                                item.audio.is_some(),
                                                item.text.trim().is_empty(),
                                            ) {
                                                (true, false) => "已转写",
                                                (true, true) => "待转写",
                                                (false, false) => "已存文本",
                                                (false, true) => "待添加音频",
                                            };
                                            let button = Button::new(SharedString::from(format!(
                                                "speaking-question-{}",
                                                item.id
                                            )))
                                            .small()
                                            .h(px(30.))
                                            .px_3()
                                            .rounded_full();
                                            let button = if selected {
                                                button.primary()
                                            } else {
                                                button.ghost()
                                            };
                                            button
                                                .disabled(locked)
                                                .label(item.label())
                                                .tooltip(format!("{} · {}", item.label(), kind))
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        if this
                                                            .speaking
                                                            .current
                                                            .as_ref()
                                                            .is_none_or(|current| {
                                                                current.id != question_id
                                                            })
                                                            && this.may_switch_speaking_question(cx)
                                                            && let Some(question) = this
                                                                .speaking
                                                                .questions
                                                                .iter()
                                                                .find(|question| {
                                                                    question.id == question_id
                                                                })
                                                                .cloned()
                                                        {
                                                            this.set_speaking_question(
                                                                question, window, cx,
                                                            );
                                                        }
                                                        cx.notify();
                                                    },
                                                ))
                                        })),
                                )
                            }),
                    )
                    .when(active_group.is_some(), |row| {
                        row.child(
                            div()
                                .flex_shrink_0()
                                .w(px(1.))
                                .h(px(20.))
                                .mx_1()
                                .bg(cx.theme().border),
                        )
                        .child(
                            Button::new("speaking-group-check")
                                .small()
                                .outline()
                                .h(px(30.))
                                .label("整组批改")
                                .tooltip(format!(
                                    "把本组 {answered} 道已写草稿的题目一起加入引用，检查前后答案是否重复"
                                ))
                                .disabled(locked || answered < 2)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.handle(Action::SpeakingGroupCheck, window, cx);
                                })),
                        )
                        .child(
                            Button::new("speaking-new-question")
                                .small()
                                .primary()
                                .h(px(30.))
                                .disabled(locked)
                                .label("＋ 题目")
                                .tooltip("新建题目")
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
                                .ghost()
                                .h(px(30.))
                                .disabled(locked)
                                .label("批量导入")
                                .tooltip("使用设置中的本机导入命令，每个音频建一道新题")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.start_speaking_command_import(false, window, cx);
                                })),
                        )
                    }),
            );
        if self.speaking.creating_group {
            controls = controls.child(
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
            );
        }
        if let Some(group) = active_group.filter(|_| self.speaking.editing_group_tags) {
            controls = controls.child(self.speaking_group_tags(group, locked, cx));
        }
        controls
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

    fn speaking_answer_draft_view(
        &self,
        question: &Question,
        locked: bool,
        editor_height: Option<Pixels>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let editor_height = editor_height.unwrap_or(px(160.));
        let draft = self.inputs["speaking_answer_draft"].read(cx).value();
        let words = clarora_gpui::writing::answer_word_count(&draft);
        let chars = draft.chars().count();
        let limit = speaking::MAX_ANSWER_DRAFT_CHARS;
        let target_words = self.speaking_target_words(cx);
        let word_status = match target_words {
            Some(target) if words < target => {
                format!("{words} / {target} 词 · 还差 {} 词", target - words)
            }
            Some(target) => format!("{words} / {target} 词 · 已达到目标"),
            None => format!("{words} 词"),
        };
        let has_draft = !draft.trim().is_empty();
        let has_prompt = !self.inputs["reference"].read(cx).value().trim().is_empty();
        let draft_dirty =
            draft.as_ref() != question.answer_draft || target_words != question.target_words;
        let save_error = self.speaking.draft_save_error.clone();
        let save_status = if let Some(error) = &save_error {
            format!("保存失败 · {error}")
        } else if self.speaking.saving {
            "正在保存…".into()
        } else if draft_dirty {
            "等待自动保存…".into()
        } else if question.answer_draft.is_empty() && question.target_words.is_none() {
            "输入后自动保存".into()
        } else {
            "草稿已保存".into()
        };
        let owner = cx.entity().downgrade();
        let draft_audio = tts::cached_preview(&self.lib, &draft, self.tts_voice, self.tts_speed);
        let playback = self.media.player.snapshot();
        let draft_audio_loaded = draft_audio
            .as_deref()
            .is_some_and(|path| self.speaking_playback_matches(path, &playback));
        let draft_audio_playing = draft_audio_loaded && !playback.paused && !playback.eof;
        let draft_audio_eof = draft_audio_loaded && playback.eof;
        let draft_audio_fraction = if draft_audio_loaded && playback.duration > 0. {
            (playback.position / playback.duration).clamp(0., 1.) as f32
        } else {
            0.
        };
        let draft_audio_time = if draft_audio_loaded {
            format!(
                "{} / {}",
                clock(playback.position),
                clock(playback.duration)
            )
        } else {
            String::new()
        };
        let tts_ready = self.data.tts_ready;
        let tts_busy = self.tasks.foreground.is_running();
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
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("目标词数"),
                            )
                            .child(
                                div().w(px(88.)).child(
                                    Input::new(&self.inputs["speaking_target_words"])
                                        .disabled(self.speaking.saving),
                                ),
                            ),
                    )
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
                    .h(editor_height)
                    .text_size(writing_ui::BODY_FONT_SIZE)
                    .line_height(relative(writing_ui::BODY_LINE_SPACING))
                    .disabled(self.speaking.saving),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .whitespace_normal()
                            .when(save_error.is_some(), |status| {
                                status.text_color(cx.theme().danger)
                            })
                            .child(save_status),
                    )
                    .when_some(save_error, |row, error| {
                        row.child(
                            Button::new("speaking-copy-draft-error")
                                .small()
                                .outline()
                                .label("复制错误")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                        error.clone(),
                                    ));
                                })),
                        )
                    })
                    .child(
                        div()
                            .flex_shrink_0()
                            .when(chars > limit, |count| count.text_color(cx.theme().danger))
                            .child(format!("{word_status} · {chars} / {limit} 字符")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("speaking-save-draft")
                            .small()
                            .outline()
                            .label("保存草稿")
                            .disabled(!draft_dirty || self.speaking.saving || chars > limit)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let _ = this.flush_speaking_draft(cx);
                            })),
                    )
                    .when(has_draft, |bar| {
                        bar.child(
                            Button::new("speaking-review-draft")
                                .small()
                                .primary()
                                .label("AI 批改 ▾")
                                .tooltip(if has_prompt {
                                    "先保存草稿，再发送题目文字和草稿给设置中的聊天模型"
                                } else {
                                    "请先转写题目音频"
                                })
                                .disabled(locked || !has_prompt || chars > limit)
                                .dropdown_menu(move |mut menu, _, _| {
                                    for mode in [ReviewMode::Overview, ReviewMode::LineEdits] {
                                        let owner = owner.clone();
                                        menu =
                                            menu.item(PopupMenuItem::new(mode.label()).on_click(
                                                move |_, _, cx| {
                                                    let _ = owner.update(cx, |app, cx| {
                                                        app.start_speaking_review(mode, cx)
                                                    });
                                                },
                                            ));
                                    }
                                    menu
                                }),
                        )
                    })
                    .when(!question.reviews.is_empty(), |bar| {
                        bar.child(
                            Button::new("speaking-show-reviews")
                                .small()
                                .outline()
                                .label(if self.speaking.show_reviews {
                                    "收起批改结果".to_owned()
                                } else {
                                    format!("批改结果 · {}", question.reviews.len())
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.speaking.show_reviews = !this.speaking.show_reviews;
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .when_some(draft_audio.clone(), |row, path| {
                        row.child(
                            Button::new("speaking-draft-audio-play")
                                .small()
                                .outline()
                                .label(if draft_audio_playing {
                                    "暂停读音"
                                } else if draft_audio_eof {
                                    "重新播放"
                                } else {
                                    "播放读音"
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Err(error) = this.play_speaking_draft_audio(path.clone())
                                    {
                                        this.report_status(
                                            FeedbackKind::Error,
                                            format!("播放草稿读音失败：{error:#}"),
                                        );
                                    }
                                    cx.notify();
                                })),
                        )
                        .when(draft_audio_loaded, |row| {
                            row.child(
                                div().flex_1().min_w(px(120.)).child(
                                    Progress::new()
                                        .value(draft_audio_fraction)
                                        .bg(cx.theme().primary),
                                ),
                            )
                            .child(div().flex_shrink_0().child(draft_audio_time.clone()))
                        })
                    })
                    .when(draft_audio.is_none(), |row| {
                        row.child(
                            Button::new("speaking-draft-audio-generate")
                                .small()
                                .outline()
                                .label("生成读音")
                                .tooltip(if tts_ready {
                                    "使用音频页选择的 Kokoro 音色和语速，生成本稿读音并播放"
                                } else {
                                    "请先在音频页下载 Kokoro 模型"
                                })
                                .disabled(
                                    !tts_ready
                                        || !has_draft
                                        || chars > 10_000
                                        || tts_busy
                                        || locked,
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Err(error) = this.start_speaking_draft_audio(cx) {
                                        this.report_status(
                                            FeedbackKind::Error,
                                            format!("生成草稿读音失败：{error:#}"),
                                        );
                                    }
                                    cx.notify();
                                })),
                        )
                    })
                    .when(!tts_ready && draft_audio.is_none(), |row| {
                        row.child(div().child("需先在音频页下载 Kokoro 模型"))
                    }),
            )
    }

    fn speaking_review_history(
        &self,
        question: &Question,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        writing_ui::correction_history(
            ReviewHistory {
                reviews: &question.reviews,
                selected: self.speaking.selected_review.as_deref(),
                show_submission: self.speaking.show_review_source,
                show_notes: self.speaking.show_review_notes,
            },
            Self::speaking_review_action,
            window,
            cx,
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
        let cursor = if duration > 0. {
            (position / duration).clamp(0., 1.) as f32
        } else {
            0.
        };
        let bounds_cell = state.bounds.clone();
        let primary = cx.theme().primary;
        let rail_color = cx.theme().border;
        let peaks = waveform
            .map(|waveform| waveform.peaks.clone())
            .unwrap_or_default();
        let path = path.to_owned();
        let can_seek = duration > 0. && self.recording.is_none();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .id("speaking-waveform")
                    .relative()
                    .h(px(52.))
                    .flex_shrink_0()
                    .rounded_md()
                    .bg(cx.theme().muted)
                    .when(can_seek, |track| track.cursor_pointer())
                    .child(
                        // Anchor to the laid-out track. Relative-sized children can be
                        // measured at their intrinsic width inside the scroll panel.
                        canvas(
                            move |bounds, _, _| bounds_cell.set(bounds),
                            move |bounds, _, window, _| {
                                audio_player_ui::paint_waveform_peaks(
                                    window,
                                    Bounds::new(bounds.origin, size(bounds.size.width, px(38.))),
                                    &peaks,
                                    primary,
                                );
                                let rail = Bounds::new(
                                    point(bounds.left(), bounds.bottom() - px(8.)),
                                    size(bounds.size.width, px(3.)),
                                );
                                window.paint_quad(fill(rail, rail_color));
                                if duration > 0. {
                                    let offset = rail.size.width * cursor;
                                    window.paint_quad(fill(
                                        Bounds::new(rail.origin, size(offset, rail.size.height)),
                                        primary,
                                    ));
                                    let marker = (offset - px(2.))
                                        .clamp(px(0.), (rail.size.width - px(4.)).max(px(0.)));
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(rail.left() + marker, rail.top() - px(3.)),
                                            size(px(4.), px(9.)),
                                        ),
                                        primary,
                                    ));
                                }
                            },
                        )
                        .absolute()
                        .inset_0(),
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
                    .on_click(cx.listener(|this, _, window, cx| {
                        let Some(question) = &this.speaking.current else {
                            return;
                        };
                        let (text, title, draft, translation) = (
                            question.text.clone(),
                            question.title.clone(),
                            question.answer_draft.clone(),
                            question.translation.clone(),
                        );
                        this.stop_speaking_prompt();
                        this.set("reference", text.clone(), window, cx);
                        this.set("speaking_title", title, window, cx);
                        this.set("speaking_answer_draft", draft, window, cx);
                        this.speaking.translation =
                            translation.map(|translation| (text, translation));
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

    #[cfg(target_os = "macos")]
    fn click_track(
        window: &Window,
        bounds: Bounds<Pixels>,
        fraction: f32,
        cx: &mut Context<AppView>,
    ) -> Result<()> {
        let position = point(
            bounds.left() + bounds.size.width * fraction,
            bounds.center().y,
        );
        native_probe_input::send(
            window,
            cx,
            vec![
                native_probe_input::MouseEvent::Down(position),
                native_probe_input::MouseEvent::Up(position),
            ],
        )
    }

    #[derive(Default)]
    pub(crate) struct Probe {
        stage: u8,
        seek_phase: u8,
        clicked_track: bool,
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
                    app.speaking.layout_probe = Some(SpeakingLayoutBounds::default());
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
                    let track = app.speaking.waveform.bounds.get();
                    if track.size.width <= px(0.) {
                        draw_next_frame(window, cx);
                        return Ok(false);
                    }
                    let Some(detail) = app.speaking.layout_probe.as_ref().unwrap().get() else {
                        draw_next_frame(window, cx);
                        return Ok(false);
                    };
                    let left_inset = track.left() - detail.left();
                    let right_inset = detail.right() - track.right();
                    ensure!(
                        track.size.height == px(52.)
                            && left_inset >= px(0.)
                            && left_inset <= px(40.)
                            && (left_inset - right_inset).abs() <= px(2.),
                        "音轨没有铺满题目卡片：{track:?}，详情列 {detail:?}"
                    );
                    if !self.clicked_track {
                        self.clicked_track = true;
                        #[cfg(target_os = "macos")]
                        click_track(window, track, 0.78, cx)?;
                        #[cfg(not(target_os = "macos"))]
                        app.seek_speaking_prompt(
                            &app.speaking_prompt_audio().unwrap().0.to_owned(),
                            1.56,
                        );
                        return Ok(false);
                    }
                    ensure!(
                        (app.speaking.waveform.start_at - 1.56).abs() < 0.02,
                        "音轨点击未定位：{:?}，开始位置 {}",
                        track,
                        app.speaking.waveform.start_at
                    );
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
                    match self.seek_phase {
                        0 => {
                            if playback.position < 1.46 {
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
                            #[cfg(target_os = "macos")]
                            click_track(window, app.speaking.waveform.bounds.get(), 0.12, cx)?;
                            #[cfg(not(target_os = "macos"))]
                            app.seek_speaking_prompt(
                                &app.speaking_prompt_audio().unwrap().0.to_owned(),
                                0.24,
                            );
                            self.seek_phase = 2;
                            return Ok(false);
                        }
                        2 => {
                            if (playback.position - 0.24).abs() > 0.08 {
                                return Ok(false);
                            }
                            ensure!(playback.paused, "点音轨时应保持暂停");
                            app.play_speaking_prompt()?;
                            self.seek_phase = 3;
                            return Ok(false);
                        }
                        _ => {
                            if playback.paused || playback.position <= 0.3 {
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
                    app.set(
                        "speaking_answer_draft",
                        "Draft edited during import.",
                        window,
                        cx,
                    );
                    app.flush_speaking_draft(cx)?;
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
                    ensure!(question.answer_draft == "Draft edited during import.");
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
                    ensure!(question.answer_draft == "Draft edited during import.");
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
                    if let Ok(base_url) = std::env::var("CLARORA_SPEAKING_REVIEW_FIXTURE") {
                        ai::save_shared(
                            &app.lib,
                            ai::Config {
                                base_url,
                                model: "shared-fixture".into(),
                                ..Default::default()
                            },
                            "sqlite-http-fixture",
                        )?;
                        app.set("reference", "Respond to the recording.", window, cx);
                        app.set("speaking_answer_draft", "Opening native draft.", window, cx);
                        app.start_speaking_review(ReviewMode::LineEdits, cx);
                        ensure!(app.speaking.saving);
                    }
                }
                9 => {
                    app.poll_tasks(window, cx);
                    if app.speaking.saving || app.tasks.foreground.is_running() {
                        return Ok(false);
                    }
                    if std::env::var_os("CLARORA_SPEAKING_REVIEW_FIXTURE").is_some() {
                        let question = app.speaking.current.as_ref().unwrap();
                        let review = question.reviews.first().context(app.status.clone())?;
                        ensure!(
                            review.answer == "Opening native draft."
                                && review.mode() == ReviewMode::LineEdits
                        );
                        ensure!(
                            app.value("speaking_answer_draft", cx) == review.answer
                                && app.speaking.show_reviews
                        );
                        ensure!(speaking::list(&app.lib)?.contains(question));
                        let id = review.id.clone();
                        app.speaking_review_action(ReviewAction::ToggleSubmission, window, cx);
                        app.speaking_review_action(ReviewAction::ToggleNotes, window, cx);
                        ensure!(app.speaking.show_review_source && app.speaking.show_review_notes);
                        app.speaking_review_action(ReviewAction::FollowUp(id), window, cx);
                        ensure!(app.assistant_open);
                    }
                    app.set(
                        "speaking_answer_draft",
                        "Autosave while editing.",
                        window,
                        cx,
                    );
                }
                10 => {
                    if app.speaking_draft_dirty(cx) {
                        return Ok(false);
                    }
                    let question = app.speaking.current.as_ref().unwrap().clone();
                    ensure!(question.answer_draft == "Autosave while editing.");
                    ensure!(speaking::list(&app.lib)?.contains(&question));

                    app.set(
                        "speaking_answer_draft",
                        "Flush before switching question.",
                        window,
                        cx,
                    );
                    ensure!(app.may_switch_speaking_question(cx));
                    ensure!(speaking::list(&app.lib)?.iter().any(|saved| {
                        saved.id == question.id
                            && saved.answer_draft == "Flush before switching question."
                    }));

                    app.set(
                        "speaking_answer_draft",
                        "Flush before navigating.",
                        window,
                        cx,
                    );
                    app.handle(Action::Page("flashcard".into()), window, cx);
                    ensure!(app.page == "flashcard");
                    ensure!(speaking::list(&app.lib)?.iter().any(|saved| {
                        saved.id == question.id && saved.answer_draft == "Flush before navigating."
                    }));
                    app.handle(Action::Page("speaking".into()), window, cx);
                    ensure!(app.value("speaking_answer_draft", cx) == "Flush before navigating.");

                    app.set(
                        "speaking_answer_draft",
                        "Explicitly saved draft.",
                        window,
                        cx,
                    );
                    app.flush_speaking_draft(cx)?;
                    ensure!(speaking::list(&app.lib)?.iter().any(|saved| {
                        saved.id == question.id
                            && saved.answer_draft == "Explicitly saved draft."
                            && saved.reviews == question.reviews
                    }));
                    app.speaking.layout_probe = Some(SpeakingLayoutBounds::default());
                    window.resize(size(px(1400.), px(800.)));
                    draw_next_frame(window, cx);
                }
                11 => {
                    let bounds = app.speaking.layout_probe.as_ref().unwrap();
                    let Some(detail) = bounds.get() else {
                        return Ok(false);
                    };
                    ensure!(detail.size.width > px(900.), "宽窗口内容区过窄：{detail:?}");
                    bounds.set(None);
                    window.resize(size(px(760.), px(760.)));
                    draw_next_frame(window, cx);
                }
                _ => {
                    let Some(detail) = app.speaking.layout_probe.as_ref().unwrap().get() else {
                        return Ok(false);
                    };
                    ensure!(detail.size.width > px(400.), "窄窗口内容区过窄：{detail:?}");
                    app.set("speaking_answer_draft", "Flush on close.", window, cx);
                    app.prepare_close(cx);
                    let current = app.speaking.current.as_ref().unwrap();
                    ensure!(
                        current.answer_draft == "Flush on close."
                            && speaking::list(&app.lib)?.contains(current)
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

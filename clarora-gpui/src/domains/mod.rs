//! Typed application commands. Views use Action; domains own effects and execution.
use super::*;
mod assistant;
mod audio;
mod cards;
mod cloud;
mod focus;
mod library;
mod media;
mod reading;
mod sentences;
mod settings;
mod shell;
mod speech;
mod tasks;
mod video;
#[derive(Clone)]
pub(super) enum Action {
    ChatGpt(chatgpt_ui::Command),
    Sentence(sentence_search_ui::Command),
    Page(String),
    Pick(String),
    Audio(String),
    Practice(String),
    AudioPractice(String),
    AudioBlind(bool),
    AudioBlindChinese(bool),
    AudioBlindReveal,
    AudioBlindRevealCue(String, Cue),
    AudioBlindHide,
    AudioBlindResume(bool),
    AudioBlindSentence(i8),
    AudioBlindRow(String, Cue, Option<usize>),
    AudioClipMode,
    AudioReview,
    AudioReviewStep(i8),
    AudioReviewStop,
    AudioTools,
    AudioFont(u8),
    AudioCurrent,
    AudioFollow,
    CopySelectedSubtitle,
    CopySelection(String),
    AudioCopySubtitle(String, Option<Cue>),
    SavedClips(Vec<clip_marks::Saved>),
    AudioRepeat(usize),
    AudioSentence(i8),
    LibrarySelect { id: String, reveal: bool },
    LibraryRenameAudio(String),
    LibraryCommitRename,
    LibraryCancelRename,
    LibraryTool(String),
    LibrarySaveName,
    LibraryFilter(u8),
    LibrarySort(u8),
    LibraryBatch,
    LibraryShowSources,
    LibrarySelectAll,
    LibraryReset,
    LibraryMove(String),
    LibraryDelete(Vec<String>),
    LibraryConfirmDelete(String),
    LibraryStudy(bool),
    Flip,
    ReplayCard,
    Next,
    Previous,
    Grade(String),
    ReviewMode(bool),
    Saved(bool),
    Filter(String),
    SaveSentence,
    EditCard,
    Composer,
    SaveEditedCard,
    Play,
    Seek(f64),
    SeekTo(f64),
    VideoTimelineSeek(f64),
    Boundary(f64, bool),
    Word(f64, f64, String, bool),
    Score,
    Assistant,
    Navigation,
    Rate(f64),
    Volume(f64),
    Mark(bool),
    Repeat,
    VideoFullscreen,
    VideoExitFullscreen,
    VideoPrecision,
    VideoSubtitleSize(u8),
    VideoCaptions,
    VideoTranslate,
    VideoClearLoop,
    VideoRepeatCue,
    VideoSentenceRepeat(Option<usize>, bool),
    VideoStopSentenceRepeat,
    VideoSentence(bool),
    VideoArrow(bool, bool),
    VideoCancelPart,
    VideoPlayCue(usize),
    VideoCopySubtitle(Option<usize>),
    LoopMode(usize),
    Mute,
    Subtitle(i64, bool),
    SubtitleImport(bool),
    Download,
    ReadParts,
    SelectPart(usize),
    SelectPartAtEnd(usize),
    AutoNextPart,
    Cancel,
    DismissTaskNotice,
    CancelModel,
    CancelAudioSubtitles,
    DismissAudioSubtitles,
    DismissModelNotice,
    DismissFeedback(bool),
    RetryFeedback,
    Clip,
    Chat,
    AssistantQuote(clarora_gpui::assistant::Reference),
    AssistantCurrent,
    AssistantRemove(usize),
    AssistantReferenceDetails,
    AssistantTurnReference(usize),
    AssistantSessions,
    AssistantWide,
    AssistantLegacy,
    AssistantCopy(String),
    AssistantPrompt(String),
    SaveAnswer,
    ChatSession(String),
    ToggleReasoning,
    PdfOpen(PathBuf),
    Favorite,
    Transcribe,
    VideoTranscribe,
    VideoTranscribeAll,
    VideoTranslateAll,
    CancelVideoSubtitles,
    DismissVideoSubtitles,
    VideoAsk(video_qa::Scope),
    VideoFollow,
    VideoPanel(usize),
    VideoTools,
    VideoTranscriptMode(bool),
    VideoCopyTranscript,
    VideoAskTranscript,
    VideoSelect(String),
    VideoEpisodes(String),
    VideoRemove(String),
    VideoConfirmRemove,
    VideoSubtitleExport,
    Translate,
    Record,
    StopRecord,
    Timer(u64),
    TimerPause,
    TimerReset,
    FocusStart(u64),
    FocusResume,
    FocusEmergency,
    FocusKeep,
    FocusFinish,
    FocusDistraction,
    FocusReview(String, Option<u8>),
    SaveAi,
    SaveAiProfile,
    SelectAiProfile(String),
    SelectAiProvider(String),
    CredentialMode(bool),
    ImportLegacyCredentials,
    SaveStorage,
    Models(String),
    SelectModel(String),
    CloudList,
    CloudBackup,
    CloudRestore(String),
    CloudSync,
    CloudAutoSync,
    CloudRetention(u8, usize),
    CloudCleanupPreview,
    CloudCleanup,
    PdfPage(i32),
    PdfZoom(i32),
    DocumentCopy,
    Theme,
    Highlight,
    ClearHighlights,
    TtsDownload,
    TtsGenerate,
    TtsVoice(i32),
    TtsSpeed(f32),
    TtsPlay,
    SpeakingGroupCheck,
    SpeakingDelete,
    SpeakingBatchImport,
    MergeAudios,
    CheckAudio(String),
    Suspend(bool),
    CommandImport,
    CommandImportSpeaking(String),
    AudioGroupSubtitles(clarora_gpui::audio_batch::Mode),
    ConfigureCommandImport,
    SaveCommandImport,
    FloatingImport,
    NameImportedAudio,
    CloseFloatingNames,
    Refresh,
    GraphSelect(String),
    GraphList(i64),
}

enum Routed {
    Assistant(assistant::Command),
    Audio(audio::Command),
    Cards(cards::Command),
    Cloud(cloud::Command),
    Focus(focus::Command),
    Library(library::Command),
    Media(media::Command),
    Reading(reading::Command),
    Sentences(sentences::Command),
    Settings(settings::Command),
    Shell(shell::Command),
    Speech(speech::Command),
    Tasks(tasks::Command),
    Video(video::Command),
}
pub(super) struct Command(Routed);
impl From<Action> for Command {
    fn from(action: Action) -> Self {
        Self(match action {
            Action::ChatGpt(v0) => Routed::Settings(settings::Command::ChatGpt(v0)),
            Action::Sentence(v0) => Routed::Sentences(sentences::Command::Sentence(v0)),
            Action::Page(v0) => Routed::Shell(shell::Command::Page(v0)),
            Action::Pick(v0) => Routed::Shell(shell::Command::Pick(v0)),
            Action::Audio(v0) => Routed::Audio(audio::Command::Audio(v0)),
            Action::Practice(v0) => Routed::Library(library::Command::Practice(v0)),
            Action::AudioPractice(v0) => Routed::Audio(audio::Command::AudioPractice(v0)),
            Action::AudioBlind(v0) => Routed::Audio(audio::Command::AudioBlind(v0)),
            Action::AudioBlindChinese(v0) => Routed::Audio(audio::Command::AudioBlindChinese(v0)),
            Action::AudioBlindReveal => Routed::Audio(audio::Command::AudioBlindReveal),
            Action::AudioBlindRevealCue(v0, v1) => {
                Routed::Audio(audio::Command::AudioBlindRevealCue(v0, v1))
            }
            Action::AudioBlindHide => Routed::Audio(audio::Command::AudioBlindHide),
            Action::AudioBlindResume(v0) => Routed::Audio(audio::Command::AudioBlindResume(v0)),
            Action::AudioBlindSentence(v0) => Routed::Audio(audio::Command::AudioBlindSentence(v0)),
            Action::AudioBlindRow(v0, v1, v2) => {
                Routed::Audio(audio::Command::AudioBlindRow(v0, v1, v2))
            }
            Action::AudioClipMode => Routed::Audio(audio::Command::AudioClipMode),
            Action::AudioReview => Routed::Audio(audio::Command::AudioReview),
            Action::AudioReviewStep(v0) => Routed::Audio(audio::Command::AudioReviewStep(v0)),
            Action::AudioReviewStop => Routed::Audio(audio::Command::AudioReviewStop),
            Action::AudioTools => Routed::Audio(audio::Command::AudioTools),
            Action::AudioFont(v0) => Routed::Audio(audio::Command::AudioFont(v0)),
            Action::AudioCurrent => Routed::Audio(audio::Command::AudioCurrent),
            Action::AudioFollow => Routed::Audio(audio::Command::AudioFollow),
            Action::CopySelectedSubtitle => Routed::Media(media::Command::CopySelectedSubtitle),
            Action::CopySelection(v0) => Routed::Media(media::Command::CopySelection(v0)),
            Action::AudioCopySubtitle(v0, v1) => {
                Routed::Audio(audio::Command::AudioCopySubtitle(v0, v1))
            }
            Action::SavedClips(v0) => Routed::Media(media::Command::SavedClips(v0)),
            Action::AudioRepeat(v0) => Routed::Audio(audio::Command::AudioRepeat(v0)),
            Action::AudioSentence(v0) => Routed::Audio(audio::Command::AudioSentence(v0)),
            Action::LibrarySelect { id, reveal } => {
                Routed::Library(library::Command::LibrarySelect { id, reveal })
            }
            Action::LibraryRenameAudio(v0) => {
                Routed::Library(library::Command::LibraryRenameAudio(v0))
            }
            Action::LibraryCommitRename => Routed::Library(library::Command::LibraryCommitRename),
            Action::LibraryCancelRename => Routed::Library(library::Command::LibraryCancelRename),
            Action::LibraryTool(v0) => Routed::Library(library::Command::LibraryTool(v0)),
            Action::LibrarySaveName => Routed::Library(library::Command::LibrarySaveName),
            Action::LibraryFilter(v0) => Routed::Library(library::Command::LibraryFilter(v0)),
            Action::LibrarySort(v0) => Routed::Library(library::Command::LibrarySort(v0)),
            Action::LibraryBatch => Routed::Library(library::Command::LibraryBatch),
            Action::LibraryShowSources => Routed::Library(library::Command::LibraryShowSources),
            Action::LibrarySelectAll => Routed::Library(library::Command::LibrarySelectAll),
            Action::LibraryReset => Routed::Library(library::Command::LibraryReset),
            Action::LibraryMove(v0) => Routed::Library(library::Command::LibraryMove(v0)),
            Action::LibraryDelete(v0) => Routed::Library(library::Command::LibraryDelete(v0)),
            Action::LibraryConfirmDelete(v0) => {
                Routed::Library(library::Command::LibraryConfirmDelete(v0))
            }
            Action::LibraryStudy(v0) => Routed::Library(library::Command::LibraryStudy(v0)),
            Action::Flip => Routed::Cards(cards::Command::Flip),
            Action::ReplayCard => Routed::Cards(cards::Command::ReplayCard),
            Action::Next => Routed::Cards(cards::Command::Next),
            Action::Previous => Routed::Cards(cards::Command::Previous),
            Action::Grade(v0) => Routed::Cards(cards::Command::Grade(v0)),
            Action::ReviewMode(v0) => Routed::Cards(cards::Command::ReviewMode(v0)),
            Action::Saved(v0) => Routed::Cards(cards::Command::Saved(v0)),
            Action::Filter(v0) => Routed::Cards(cards::Command::Filter(v0)),
            Action::SaveSentence => Routed::Cards(cards::Command::SaveSentence),
            Action::EditCard => Routed::Cards(cards::Command::EditCard),
            Action::Composer => Routed::Cards(cards::Command::Composer),
            Action::SaveEditedCard => Routed::Cards(cards::Command::SaveEditedCard),
            Action::Play => Routed::Media(media::Command::Play),
            Action::Seek(v0) => Routed::Media(media::Command::Seek(v0)),
            Action::SeekTo(v0) => Routed::Media(media::Command::SeekTo(v0)),
            Action::VideoTimelineSeek(v0) => Routed::Video(video::Command::VideoTimelineSeek(v0)),
            Action::Boundary(v0, v1) => Routed::Media(media::Command::Boundary(v0, v1)),
            Action::Word(v0, v1, v2, v3) => Routed::Media(media::Command::Word(v0, v1, v2, v3)),
            Action::Score => Routed::Speech(speech::Command::Score),
            Action::Assistant => Routed::Assistant(assistant::Command::Assistant),
            Action::Navigation => Routed::Shell(shell::Command::Navigation),
            Action::Rate(v0) => Routed::Media(media::Command::Rate(v0)),
            Action::Volume(v0) => Routed::Media(media::Command::Volume(v0)),
            Action::Mark(v0) => Routed::Media(media::Command::Mark(v0)),
            Action::Repeat => Routed::Media(media::Command::Repeat),
            Action::VideoFullscreen => Routed::Video(video::Command::VideoFullscreen),
            Action::VideoExitFullscreen => Routed::Video(video::Command::VideoExitFullscreen),
            Action::VideoPrecision => Routed::Video(video::Command::VideoPrecision),
            Action::VideoSubtitleSize(v0) => Routed::Video(video::Command::VideoSubtitleSize(v0)),
            Action::VideoCaptions => Routed::Video(video::Command::VideoCaptions),
            Action::VideoTranslate => Routed::Video(video::Command::VideoTranslate),
            Action::VideoClearLoop => Routed::Video(video::Command::VideoClearLoop),
            Action::VideoRepeatCue => Routed::Video(video::Command::VideoRepeatCue),
            Action::VideoSentenceRepeat(v0, v1) => {
                Routed::Video(video::Command::VideoSentenceRepeat(v0, v1))
            }
            Action::VideoStopSentenceRepeat => {
                Routed::Video(video::Command::VideoStopSentenceRepeat)
            }
            Action::VideoSentence(v0) => Routed::Video(video::Command::VideoSentence(v0)),
            Action::VideoArrow(v0, v1) => Routed::Video(video::Command::VideoArrow(v0, v1)),
            Action::VideoCancelPart => Routed::Video(video::Command::VideoCancelPart),
            Action::VideoPlayCue(v0) => Routed::Video(video::Command::VideoPlayCue(v0)),
            Action::VideoCopySubtitle(v0) => Routed::Video(video::Command::VideoCopySubtitle(v0)),
            Action::LoopMode(v0) => Routed::Media(media::Command::LoopMode(v0)),
            Action::Mute => Routed::Media(media::Command::Mute),
            Action::Subtitle(v0, v1) => Routed::Media(media::Command::Subtitle(v0, v1)),
            Action::SubtitleImport(v0) => Routed::Media(media::Command::SubtitleImport(v0)),
            Action::Download => Routed::Video(video::Command::Download),
            Action::ReadParts => Routed::Video(video::Command::ReadParts),
            Action::SelectPart(v0) => Routed::Video(video::Command::SelectPart(v0)),
            Action::SelectPartAtEnd(v0) => Routed::Video(video::Command::SelectPartAtEnd(v0)),
            Action::AutoNextPart => Routed::Video(video::Command::AutoNextPart),
            Action::Cancel => Routed::Tasks(tasks::Command::Cancel),
            Action::DismissTaskNotice => Routed::Tasks(tasks::Command::DismissTaskNotice),
            Action::CancelModel => Routed::Tasks(tasks::Command::CancelModel),
            Action::CancelAudioSubtitles => Routed::Tasks(tasks::Command::CancelAudioSubtitles),
            Action::DismissAudioSubtitles => Routed::Tasks(tasks::Command::DismissAudioSubtitles),
            Action::DismissModelNotice => Routed::Tasks(tasks::Command::DismissModelNotice),
            Action::DismissFeedback(_) | Action::RetryFeedback => {
                Routed::Tasks(tasks::Command::FeedbackHandled)
            }
            Action::Clip => Routed::Media(media::Command::Clip),
            Action::Chat => Routed::Assistant(assistant::Command::Chat),
            Action::AssistantQuote(v0) => Routed::Assistant(assistant::Command::AssistantQuote(v0)),
            Action::AssistantCurrent => Routed::Assistant(assistant::Command::AssistantCurrent),
            Action::AssistantRemove(v0) => {
                Routed::Assistant(assistant::Command::AssistantRemove(v0))
            }
            Action::AssistantReferenceDetails => {
                Routed::Assistant(assistant::Command::AssistantReferenceDetails)
            }
            Action::AssistantTurnReference(v0) => {
                Routed::Assistant(assistant::Command::AssistantTurnReference(v0))
            }
            Action::AssistantSessions => Routed::Assistant(assistant::Command::AssistantSessions),
            Action::AssistantWide => Routed::Assistant(assistant::Command::AssistantWide),
            Action::AssistantLegacy => Routed::Assistant(assistant::Command::AssistantLegacy),
            Action::AssistantCopy(v0) => Routed::Assistant(assistant::Command::AssistantCopy(v0)),
            Action::AssistantPrompt(v0) => {
                Routed::Assistant(assistant::Command::AssistantPrompt(v0))
            }
            Action::SaveAnswer => Routed::Assistant(assistant::Command::SaveAnswer),
            Action::ChatSession(v0) => Routed::Assistant(assistant::Command::ChatSession(v0)),
            Action::ToggleReasoning => Routed::Assistant(assistant::Command::ToggleReasoning),
            Action::PdfOpen(v0) => Routed::Reading(reading::Command::PdfOpen(v0)),
            Action::Favorite => Routed::Cards(cards::Command::Favorite),
            Action::Transcribe => Routed::Audio(audio::Command::Transcribe),
            Action::VideoTranscribe => Routed::Video(video::Command::VideoTranscribe),
            Action::VideoTranscribeAll => Routed::Video(video::Command::VideoTranscribeAll),
            Action::VideoTranslateAll => Routed::Video(video::Command::VideoTranslateAll),
            Action::CancelVideoSubtitles => Routed::Video(video::Command::CancelVideoSubtitles),
            Action::DismissVideoSubtitles => Routed::Video(video::Command::DismissVideoSubtitles),
            Action::VideoAsk(v0) => Routed::Video(video::Command::VideoAsk(v0)),
            Action::VideoFollow => Routed::Video(video::Command::VideoFollow),
            Action::VideoPanel(v0) => Routed::Video(video::Command::VideoPanel(v0)),
            Action::VideoTools => Routed::Video(video::Command::VideoTools),
            Action::VideoTranscriptMode(v0) => {
                Routed::Video(video::Command::VideoTranscriptMode(v0))
            }
            Action::VideoCopyTranscript => Routed::Video(video::Command::VideoCopyTranscript),
            Action::VideoAskTranscript => Routed::Video(video::Command::VideoAskTranscript),
            Action::VideoSelect(v0) => Routed::Video(video::Command::VideoSelect(v0)),
            Action::VideoEpisodes(v0) => Routed::Video(video::Command::VideoEpisodes(v0)),
            Action::VideoRemove(v0) => Routed::Video(video::Command::VideoRemove(v0)),
            Action::VideoConfirmRemove => Routed::Video(video::Command::VideoConfirmRemove),
            Action::VideoSubtitleExport => Routed::Video(video::Command::VideoSubtitleExport),
            Action::Translate => Routed::Audio(audio::Command::Translate),
            Action::Record => Routed::Speech(speech::Command::Record),
            Action::StopRecord => Routed::Speech(speech::Command::StopRecord),
            Action::Timer(v0) => Routed::Focus(focus::Command::Timer(v0)),
            Action::TimerPause => Routed::Focus(focus::Command::TimerPause),
            Action::TimerReset => Routed::Focus(focus::Command::TimerReset),
            Action::FocusStart(v0) => Routed::Focus(focus::Command::FocusStart(v0)),
            Action::FocusResume => Routed::Focus(focus::Command::FocusResume),
            Action::FocusEmergency => Routed::Focus(focus::Command::FocusEmergency),
            Action::FocusKeep => Routed::Focus(focus::Command::FocusKeep),
            Action::FocusFinish => Routed::Focus(focus::Command::FocusFinish),
            Action::FocusDistraction => Routed::Focus(focus::Command::FocusDistraction),
            Action::FocusReview(id, fatigue) => {
                Routed::Focus(focus::Command::FocusReview(id, fatigue))
            }
            Action::SaveAi => Routed::Settings(settings::Command::SaveAi),
            Action::SaveAiProfile => Routed::Settings(settings::Command::SaveAiProfile),
            Action::SelectAiProfile(v0) => Routed::Settings(settings::Command::SelectAiProfile(v0)),
            Action::SelectAiProvider(v0) => {
                Routed::Settings(settings::Command::SelectAiProvider(v0))
            }
            Action::CredentialMode(v0) => Routed::Settings(settings::Command::CredentialMode(v0)),
            Action::ImportLegacyCredentials => {
                Routed::Settings(settings::Command::ImportLegacyCredentials)
            }
            Action::SaveStorage => Routed::Settings(settings::Command::SaveStorage),
            Action::Models(v0) => Routed::Settings(settings::Command::Models(v0)),
            Action::SelectModel(v0) => Routed::Settings(settings::Command::SelectModel(v0)),
            Action::CloudList => Routed::Cloud(cloud::Command::CloudList),
            Action::CloudBackup => Routed::Cloud(cloud::Command::CloudBackup),
            Action::CloudRestore(v0) => Routed::Cloud(cloud::Command::CloudRestore(v0)),
            Action::CloudSync => Routed::Cloud(cloud::Command::CloudSync),
            Action::CloudAutoSync => Routed::Cloud(cloud::Command::CloudAutoSync),
            Action::CloudRetention(v0, v1) => Routed::Cloud(cloud::Command::CloudRetention(v0, v1)),
            Action::CloudCleanupPreview => Routed::Cloud(cloud::Command::CloudCleanupPreview),
            Action::CloudCleanup => Routed::Cloud(cloud::Command::CloudCleanup),
            Action::PdfPage(v0) => Routed::Reading(reading::Command::PdfPage(v0)),
            Action::PdfZoom(v0) => Routed::Reading(reading::Command::PdfZoom(v0)),
            Action::DocumentCopy => Routed::Reading(reading::Command::DocumentCopy),
            Action::Theme => Routed::Settings(settings::Command::Theme),
            Action::Highlight => Routed::Audio(audio::Command::Highlight),
            Action::ClearHighlights => Routed::Audio(audio::Command::ClearHighlights),
            Action::TtsDownload => Routed::Speech(speech::Command::TtsDownload),
            Action::TtsGenerate => Routed::Speech(speech::Command::TtsGenerate),
            Action::TtsVoice(v0) => Routed::Speech(speech::Command::TtsVoice(v0)),
            Action::TtsSpeed(v0) => Routed::Speech(speech::Command::TtsSpeed(v0)),
            Action::TtsPlay => Routed::Speech(speech::Command::TtsPlay),
            Action::SpeakingGroupCheck => Routed::Speech(speech::Command::GroupCheck),
            Action::SpeakingDelete => Routed::Speech(speech::Command::Delete),
            Action::SpeakingBatchImport => Routed::Speech(speech::Command::BatchImport),
            Action::MergeAudios => Routed::Library(library::Command::MergeAudios),
            Action::CheckAudio(v0) => Routed::Library(library::Command::CheckAudio(v0)),
            Action::Suspend(v0) => Routed::Cards(cards::Command::Suspend(v0)),
            Action::CommandImport => Routed::Library(library::Command::CommandImport),
            Action::CommandImportSpeaking(group) => {
                Routed::Speech(speech::Command::ImportSpeakingGroup(group))
            }
            Action::AudioGroupSubtitles(v0) => {
                Routed::Audio(audio::Command::AudioGroupSubtitles(v0))
            }
            Action::ConfigureCommandImport => {
                Routed::Library(library::Command::ConfigureCommandImport)
            }
            Action::SaveCommandImport => Routed::Library(library::Command::SaveCommandImport),
            Action::FloatingImport => Routed::Shell(shell::Command::FloatingImport),
            Action::NameImportedAudio => Routed::Library(library::Command::NameImportedAudio),
            Action::CloseFloatingNames => Routed::Library(library::Command::CloseFloatingNames),
            Action::Refresh => Routed::Shell(shell::Command::Refresh),
            Action::GraphSelect(v0) => Routed::Shell(shell::Command::GraphSelect(v0)),
            Action::GraphList(v0) => Routed::Shell(shell::Command::GraphList(v0)),
        })
    }
}
impl Command {
    pub(super) fn effects(&self, app: &AppView) -> Effects {
        match &self.0 {
            Routed::Assistant(command) => command.effects(app),
            Routed::Audio(command) => command.effects(app),
            Routed::Cards(command) => command.effects(app),
            Routed::Cloud(command) => command.effects(app),
            Routed::Focus(command) => command.effects(app),
            Routed::Library(command) => command.effects(app),
            Routed::Media(command) => command.effects(app),
            Routed::Reading(command) => command.effects(app),
            Routed::Sentences(command) => command.effects(app),
            Routed::Settings(command) => command.effects(app),
            Routed::Shell(command) => command.effects(app),
            Routed::Speech(command) => command.effects(app),
            Routed::Tasks(command) => command.effects(app),
            Routed::Video(command) => command.effects(app),
        }
    }
    pub(super) fn execute(
        self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<()> {
        match self.0 {
            Routed::Assistant(command) => command.execute(app, window, cx),
            Routed::Audio(command) => command.execute(app, window, cx),
            Routed::Cards(command) => command.execute(app, window, cx),
            Routed::Cloud(command) => command.execute(app, window, cx),
            Routed::Focus(command) => command.execute(app, window, cx),
            Routed::Library(command) => command.execute(app, window, cx),
            Routed::Media(command) => command.execute(app, window, cx),
            Routed::Reading(command) => command.execute(app, window, cx),
            Routed::Sentences(command) => command.execute(app, window, cx),
            Routed::Settings(command) => command.execute(app, window, cx),
            Routed::Shell(command) => command.execute(app, window, cx),
            Routed::Speech(command) => command.execute(app, window, cx),
            Routed::Tasks(command) => command.execute(app, window, cx),
            Routed::Video(command) => command.execute(app, window, cx),
        }
    }
}
#[derive(Default)]
pub(super) struct Effects {
    restore_focus: bool,
    sync_audio_mode: bool,
    leave_video_sentence: bool,
    refresh_data: bool,
    refresh_chat: bool,
}
impl Effects {
    pub(super) fn before(&self, app: &mut AppView) {
        if self.refresh_chat {
            app.chat_requested_context.clear();
        }
        if self.leave_video_sentence
            && app.page == "video"
            && (app.media.session.video_sentence().is_some()
                || app.video.repeat_three
                || app.video.pending_start.is_some())
        {
            app.clear_video_sentence_repeat();
        }
    }
    pub(super) fn after(
        &self,
        app: &mut AppView,
        success: bool,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) {
        if self.sync_audio_mode && success && app.media.session.is_audio(&app.audio.current_id) {
            app.audio.repeat_mode = app.media.session.mode;
        }
        if success && app.page == "video" && app.media.session.is_video() {
            // File-bound A/B is never a page preference.
            app.video.repeat_mode = if app.media.session.mode == PlaybackMode::Track {
                PlaybackMode::Track
            } else {
                PlaybackMode::Normal
            };
        }
        if self.refresh_data {
            app.refresh_view_data();
        }
        if self.restore_focus && !app.subtitle_text.borrow().focus.is_focused(window) {
            window.focus(&app.focus);
        }
        if self.refresh_chat {
            if let Err(error) = app.chat.refresh() {
                app.report_status(FeedbackKind::Error, format!("刷新会话失败：{error:#}"));
            }
        }
        app.sync_assistant(window, cx);
    }
}

/// Dispatch-only context; domains own whether a completed commit overrides cancel.
pub(super) struct Completion {
    pub(super) cancelled: bool,
    pub(super) chat: bool,
}
pub(super) fn complete(
    outcome: Outcome,
    app: &mut AppView,
    context: &mut Completion,
    window: &mut Window,
    cx: &mut Context<AppView>,
) {
    match outcome {
        Outcome::CredentialMode(v0) => {
            settings::Completed::CredentialMode(v0).apply(app, context, window, cx)
        }
        Outcome::LibraryRestored(v0, v1) => {
            cloud::Completed::LibraryRestored(v0, v1).apply(app, context, window, cx)
        }
        Outcome::Message(v0) => tasks::Completed::Message(v0).apply(app, context, window, cx),
        Outcome::Issue(v0) => tasks::Completed::Issue(v0).apply(app, context, window, cx),
        Outcome::Parts(v0) => video::Completed::Parts(v0).apply(app, context, window, cx),
        Outcome::PartVideo(v0, v1, v2, v3, v4, v5) => {
            video::Completed::PartVideo(v0, v1, v2, v3, v4, v5).apply(app, context, window, cx)
        }
        Outcome::Output(v0) => assistant::Completed::Output(v0).apply(app, context, window, cx),
        Outcome::VideoTranslated(v0, v1, v2, v3) => {
            video::Completed::VideoTranslated(v0, v1, v2, v3).apply(app, context, window, cx)
        }
        Outcome::Pdf(v0, v1, v2) => {
            reading::Completed::Pdf(v0, v1, v2).apply(app, context, window, cx)
        }
        Outcome::Ebook(v0) => reading::Completed::Ebook(v0).apply(app, context, window, cx),
        Outcome::Ocr(v0) => speech::Completed::Ocr(v0).apply(app, context, window, cx),
        Outcome::KeyPoints(v0, v1) => {
            audio::Completed::KeyPoints(v0, v1).apply(app, context, window, cx)
        }
        Outcome::BackupCreated(v0, v1) => {
            cloud::Completed::BackupCreated(v0, v1).apply(app, context, window, cx)
        }
        Outcome::BackupFailed(v0) => {
            cloud::Completed::BackupFailed(v0).apply(app, context, window, cx)
        }
        Outcome::CloudVersions(v0) => {
            cloud::Completed::CloudVersions(v0).apply(app, context, window, cx)
        }
        Outcome::CloudCleanupPlan(v0) => {
            cloud::Completed::CloudCleanupPlan(v0).apply(app, context, window, cx)
        }
        Outcome::CloudCleaned(v0, v1) => {
            cloud::Completed::CloudCleaned(v0, v1).apply(app, context, window, cx)
        }
        Outcome::CloudSynced(v0, v1) => {
            cloud::Completed::CloudSynced(v0, v1).apply(app, context, window, cx)
        }
        Outcome::Recording(v0) => speech::Completed::Recording(v0).apply(app, context, window, cx),
        Outcome::TtsReady(v0) => audio::Completed::TtsReady(v0).apply(app, context, window, cx),
        Outcome::CommandImported(v0) => {
            library::Completed::CommandImported(v0).apply(app, context, window, cx)
        }
        Outcome::SpeakingCommandImported(report) => {
            app.finish_speaking_command_import(report, context.cancelled)
        }
        Outcome::SpeakingTextReady(id, result) => {
            app.finish_speaking_text(id, result, context.cancelled, window, cx)
        }
        Outcome::SpeakingGroupTagsReady(group_id, tags) => {
            app.finish_speaking_group_tags(&group_id, &tags, context.cancelled)
        }
        Outcome::SpeakingReviewed(id, review) => {
            context.cancelled = false;
            app.finish_speaking_review(&id, review);
        }
        Outcome::SpeakingDraftAudio(path) => app.finish_speaking_draft_audio(path, cx),
        Outcome::AudioMerged(v0) => {
            audio::Completed::AudioMerged(v0).apply(app, context, window, cx)
        }
    }
}

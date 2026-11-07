//! Application actions coordinate domain controllers.
use super::*;

impl AppView {
    pub(super) fn handle(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        let action = if self.page == "sentences" {
            match action {
                Action::Play => Action::Sentence(sentence_search_ui::Command::Toggle),
                Action::Next => Action::Sentence(sentence_search_ui::Command::Step(true)),
                Action::Previous => Action::Sentence(sentence_search_ui::Command::Step(false)),
                other => other,
            }
        } else {
            action
        };
        if self.focus_mode.active()
            && matches!(
                &action,
                Action::Timer(_)
                    | Action::TimerPause
                    | Action::TimerReset
                    | Action::FocusStart(_)
                    | Action::FocusResume
            )
        {
            self.report_status(
                FeedbackKind::Info,
                "电脑专注期间不能暂停、重置或改变时长；如需提前结束，请使用顶部「紧急结束」".into(),
            );
            cx.notify();
            return;
        }
        self.video.picture_click = None;
        // Modal confirmation owns interaction until it is confirmed or dismissed.
        if window.has_active_dialog(cx)
            && !matches!(
                action,
                Action::VideoConfirmRemove | Action::LibraryConfirmDelete(_)
            )
        {
            return;
        }
        if !matches!(action, Action::VideoArrow(..)) {
            if matches!(action, Action::SelectPart(_) | Action::SelectPartAtEnd(_)) {
                self.video.part_confirmation = None;
            } else {
                self.dismiss_video_part_confirmation();
            }
        }
        if matches!(
            &action,
            Action::Flip
                | Action::ReplayCard
                | Action::Next
                | Action::Previous
                | Action::Grade(_)
                | Action::EditCard
                | Action::Favorite
                | Action::Play
        ) && matches!(self.page.as_str(), "discover" | "flashcard")
            && !self.cards_ready()
        {
            self.report_status(FeedbackKind::Info, "正在读取学习卡片，请稍候".into());
            cx.notify();
            return;
        }
        if self.begin_feedback_action(&action, window, cx) {
            return;
        }
        let action_retry = self.feedback.retry_candidate.clone();
        let command = domains::Command::from(action);
        let effects = command.effects(self);
        effects.before(self);
        let result = command.execute(self, window, cx);
        let success = result.is_ok();
        if let Err(error) = result {
            self.report_status(FeedbackKind::Error, format!("{error:#}"));
            self.feedback.attach_retry(action_retry);
        }
        effects.after(self, success, window, cx);
        cx.notify();
    }
}

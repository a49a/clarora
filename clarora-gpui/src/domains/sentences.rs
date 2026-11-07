//! Sentences commands own their action effects and execution.
use super::*;
pub(super) enum Command {
    Sentence(sentence_search_ui::Command),
}
impl Command {
    pub(super) fn effects(&self, _app: &AppView) -> Effects {
        Effects::default()
    }
    pub(super) fn execute(
        self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<()> {
        match self {
            Command::Sentence(command) => app.sentence_command(command, window, cx)?,
        }
        Ok(())
    }
}

pub(super) fn leave(app: &mut AppView, next: &str) {
    if app.page == "sentences" && next != "sentences" {
        app.sentences.suspend();
    }
}

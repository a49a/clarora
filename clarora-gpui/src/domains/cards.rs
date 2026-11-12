//! Cards commands own their action effects and execution.
use super::*;
pub(super) enum Command {
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
    Favorite,
    Suspend(bool),
}
impl Command {
    pub(super) fn effects(&self, app: &AppView) -> Effects {
        Effects {
            restore_focus: (matches!(app.page.as_str(), "discover" | "flashcard")
                && matches!(
                    self,
                    Command::Flip
                        | Command::ReplayCard
                        | Command::Next
                        | Command::Previous
                        | Command::Grade(..)
                        | Command::ReviewMode(..)
                        | Command::Saved(..)
                        | Command::Filter(..)
                        | Command::Favorite
                )),
            ..Effects::default()
        }
    }
    pub(super) fn execute(
        self,
        app: &mut AppView,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Result<()> {
        match self {
            Command::Composer => {
                app.composer_open = !app.composer_open;
                if app.composer_open {
                    app.page_scroll.scroll_to_bottom();
                }
            }

            Command::Saved(saved) => {
                app.saved_only = saved;
                app.refresh();
            }

            Command::ReviewMode(due) => {
                app.due = due;
                app.refresh();
                app.revealed = false;
            }

            Command::Filter(filter) => {
                app.card_filter = filter;
                app.refresh();
            }

            Command::Flip => app.revealed = !app.revealed,

            Command::ReplayCard => {
                ensure!(
                    matches!(app.page.as_str(), "flashcard" | "discover"),
                    "请先打开闪卡"
                );
                let card = app.current_card().context("当前没有闪卡")?;
                if card.media.is_empty() {
                    app.report_status(FeedbackKind::Info, "这张闪卡没有可重播的音频".into());
                    return Ok(());
                }
                // Reload the current card's own range, never swap in a stale
                // preloader or merely resume whichever file the player holds.
                app.media.preloaded_index = None;
                app.play_card();
            }

            Command::Next => {
                if !app.cards.is_empty() {
                    app.history.push(app.index);
                    if app.page == "discover" {
                        app.index = if let Some(index) = app.media.preloaded_index {
                            index
                        } else {
                            let due = &app.loads.discover_due;
                            let favorites = &app.loads.discover_favorites;
                            clarora_gpui::discover::next(
                                &app.cards,
                                &app.history,
                                app.index,
                                due,
                                favorites,
                            )
                            .unwrap_or(0)
                        };
                    } else {
                        app.index = (app.index + 1) % app.cards.len();
                    }
                    app.revealed = false;
                    app.play_card();
                }
            }

            Command::Previous => {
                app.index = app.history.pop().unwrap_or(app.index.saturating_sub(1));
                app.revealed = false;
                app.play_card();
            }

            Command::Grade(grade) => {
                let card = app.current_card().context("没有待复习卡片")?;
                ensure!(app.revealed, "请先翻面再评分");
                let due = app.lib.grade(&card, &grade)?;
                app.report_status(FeedbackKind::Info, format!("下次复习：{due}"));
                app.cards.remove(app.index);
                if grade == "again" {
                    app.cards.push(card);
                }
                if app.index >= app.cards.len() {
                    app.index = 0;
                }
                app.revealed = false;
                app.play_card();
            }

            Command::EditCard => {
                app.page_scroll.scroll_to_bottom();
                let card = app.current_card().context("没有可编辑的卡片")?;
                app.set("front", card.front.clone(), window, cx);
                app.set("back", card.back.clone(), window, cx);
                app.editing = Some(card);
                app.report_status(
                    FeedbackKind::Info,
                    "在下方编辑原文与翻译，然后保存修改".into(),
                );
            }

            Command::SaveEditedCard => {
                let card = app.editing.as_ref().context("请先选择编辑卡片")?;
                app.lib
                    .update_card(card, &app.value("front", cx), &app.value("back", cx))?;
                app.editing = None;
                app.refresh();
                app.report_status(FeedbackKind::Info, "卡片已更新，复习进度保留".into());
            }

            Command::SaveSentence => {
                documents::save_sentence(
                    &app.lib,
                    &app.value("front", cx),
                    &app.value("back", cx),
                )?;
                app.refresh();
                app.report_status(FeedbackKind::Info, "句子卡已保存".into());
            }

            Command::Favorite => {
                let card = app.current_card().context("当前没有内容")?;
                let key = format!("{}:{}", card.kind, card.id);
                let exists = app.lib.rows(
                    "SELECT key FROM discover_favorites WHERE key=?",
                    &[key.clone().into()],
                )?;
                if exists.is_empty() {
                    app.lib.put("discover_favorites",json!({"key":key,"payload":json!({"key":key,"kind":card.kind,"front":card.front,"back":card.back,"source":"GPUI","audio":if card.media.is_empty(){Value::Null}else{json!({"uri":card.media,"startMs":card.start*1000.,"endMs":card.end*1000.})}}).to_string()}))?;
                    app.loads.discover_favorites.insert(key.clone());
                    app.media.preloaded_index = None;
                    if card.id.starts_with("segment:") {
                        let count=app.lib.rows("SELECT id FROM clip_cards WHERE audio_uri=? AND start_ms=? AND end_ms=?",&[card.media.clone().into(),(card.start*1000.).into(),(card.end*1000.).into()])?;
                        if count.is_empty() {
                            app.lib.put("clip_cards",json!({"id":card.id,"en_text":card.front,"zh_text":card.back,"audio_uri":card.media,"start_ms":card.start*1000.,"end_ms":card.end*1000.,"created_at":now()}))?;
                        }
                    }
                    app.report_status(FeedbackKind::Info, "已收藏".into());
                } else {
                    app.lib.delete("discover_favorites", &key)?;
                    app.loads.discover_favorites.remove(&key);
                    app.media.preloaded_index = None;
                    app.report_status(FeedbackKind::Info, "已取消收藏".into());
                }
            }

            Command::Suspend(suspend) => {
                let card = app.current_card().context("当前没有卡片")?;
                app.lib.db()?.execute("INSERT INTO review_schedule(card_kind,card_id,suspended) VALUES(?,?,?) ON CONFLICT(card_kind,card_id) DO UPDATE SET suspended=excluded.suspended",rusqlite::params![card.kind,card.id,i32::from(suspend)])?;
                app.refresh();
                app.report_status(
                    FeedbackKind::Info,
                    if suspend {
                        "已暂停此卡复习"
                    } else {
                        "已恢复此卡复习"
                    }
                    .into(),
                );
            }
        }
        Ok(())
    }
}

pub(super) fn enter(app: &mut AppView) {
    if matches!(app.page.as_str(), "discover" | "flashcard") {
        app.play_card();
    }
}

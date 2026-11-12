//! Native speaking workflow probe, split out of the production UI module.
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
                    saved[0]
                        .audio
                        .as_ref()
                        .is_some_and(|audio| audio.path.starts_with(app.lib.root.join("media")))
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
                    app.speaking.waveform.result.is_none() && app.speaking.waveform.path.is_none()
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
                app.start_speaking_command_import(window, cx);
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
                app.start_speaking_command_import(window, cx);
                ensure!(!app.tasks.foreground.is_running(), "已有音频时不能命令覆盖");
                app.remove_speaking_prompt(window, cx);
                ensure!(app.value("reference", cx).is_empty());
                ensure!(
                    app.speaking_translation(cx).is_none() && app.speaking_prompt_audio().is_none()
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
                app.start_speaking_command_import(window, cx);
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
                ensure!(app.value("reference", cx).is_empty() && app.command_imported.is_none());
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
                app.handle(Action::SpeakingDelete, window, cx);
                ensure!(window.has_active_dialog(cx), "删除题目没有打开确认对话框");
                window.close_dialog(cx);
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

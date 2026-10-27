//! Audio selection, restoration and waveform coordination.
use super::*;

impl AppView {
    pub(super) fn open_audio(&mut self, row: &Row) {
        self.audio.library.cancel_rename();
        self.invalidate_blind();
        self.audio.blind_settle_until = Some(Instant::now() + Duration::from_millis(250));
        let path = PathBuf::from(text(row, "audio_uri"));
        self.media.player.open(&path);
        self.media.path = Some(path);
        self.audio.current_id = text(row, "id");
        let _ = self
            .lib
            .set_setting("last_listening_audio", &self.audio.current_id);
        self.load_waveform();
        self.audio.key_points.clear();
        self.audio.practice_id = text(row, "practice_id");
        if let Err(error) = self
            .lib
            .set_setting("library_group", &self.audio.practice_id)
        {
            self.report_status(
                FeedbackKind::Error,
                format!("保存所选练习组失败：{error:#}"),
            );
        }
        self.audio.cues.clear();
        self.invalidate_blind();
        self.audio.scroll.reset(self.audio.cues.len());
        self.audio.active = None;
        self.audio.current_scroll.set_offset(Point::default());
        self.audio.clip_mode = false;
        self.audio.tools_open = false;
        if self.media.repeat == 2 {
            self.media.repeat = 0;
        }
        self.media.loop_a = None;
        self.media.loop_b = None;
        self.refresh_saved_ranges();
        self.refresh();
    }
    pub(super) fn restore_listening(&mut self) {
        self.loads.restore_pending = true;
        self.refresh();
    }
    pub(super) fn restore_listening_cached(&mut self) {
        let Some((last, position)) = self.loads.restore_audio.clone() else {
            return;
        };
        if let Some(row) = self
            .audio
            .audios
            .iter()
            .find(|r| text(r, "id") == last)
            .cloned()
        {
            self.open_audio(&row);
            self.media.player.set("pause", "yes");
            self.media.player.command([
                "seek".into(),
                position.to_string(),
                "absolute+exact".into(),
            ]);
        }
    }
    pub(super) fn load_waveform(&mut self) {
        if let Some(cancel) = self.audio.waveform_cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        self.audio.waveform = None;
        self.audio.waveform_rx = None;
        if let Some(path) = self.media.path.clone() {
            let lib = self.lib.clone();
            let cancel = process::cancel_token();
            self.audio.waveform_cancel = Some(cancel.clone());
            let (tx, rx) = mpsc::channel();
            self.audio.waveform_rx = Some(rx);
            std::thread::spawn(move || {
                let _ =
                    tx.send(waveform::extract(&lib, &path, &cancel).map_err(|e| format!("{e:#}")));
            });
        }
    }
    pub(super) fn selected(&self) -> Option<Row> {
        self.audio
            .audios
            .iter()
            .find(|r| text(r, "id") == self.audio.current_id)
            .cloned()
    }
    pub(super) fn invalidate_blind(&mut self) {
        self.audio.blind.invalidate();
        self.audio.blind_pending = None;
    }
}

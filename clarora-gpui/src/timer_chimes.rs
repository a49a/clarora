//! Original synthesized bells, generated locally and played independently of learning media.
use crate::media::{Playback, Player};
use anyhow::Result;
use std::{
    f64::consts::TAU,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};
#[derive(Clone, Copy)]
pub enum Cue {
    Start,
    End,
}
fn wav(cue: Cue) -> Result<Vec<u8>> {
    let seconds = match cue {
        Cue::Start => 1.4,
        Cue::End => 2.2,
    };
    let offsets: &[f64] = match cue {
        Cue::Start => &[0.],
        Cue::End => &[0., 0.65],
    };
    let frequency = match cue {
        Cue::Start => 660.,
        Cue::End => 528.,
    };
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(
            &mut cursor,
            hound::WavSpec {
                channels: 1,
                sample_rate: 44100,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )?;
        for i in 0..(seconds * 44100.) as usize {
            let time = i as f64 / 44100.;
            let mut value = 0.;
            for offset in offsets {
                let t = time - offset;
                if t < 0. {
                    continue;
                }
                let attack = (t / 0.018).min(1.);
                let tail = ((seconds - time) / 0.15).clamp(0., 1.);
                value += attack
                    * tail
                    * ((TAU * frequency * t).sin() * (-3. * t).exp() * 0.65
                        + (TAU * frequency * 2.76 * t).sin() * (-5. * t).exp() * 0.23
                        + (TAU * frequency * 4.07 * t).sin() * (-7. * t).exp() * 0.12);
            }
            writer.write_sample((value * 0.28 * f64::from(i16::MAX)) as i16)?;
        }
        writer.finalize()?;
    }
    Ok(cursor.into_inner())
}
fn prepare(directory: &Path, cue: Cue) -> Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let path = directory.join(match cue {
        Cue::Start => "start-v1.wav",
        Cue::End => "end-v1.wav",
    });
    let bytes = wav(cue)?;
    if fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
        fs::write(&path, bytes)?;
    }
    Ok(path)
}
pub struct Chimes {
    directory: PathBuf,
    player: Option<Player>,
    audio_device: bool,
    reported_error: String,
}
impl Chimes {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            player: None,
            audio_device: true,
            reported_error: String::new(),
        }
    }
    pub fn without_audio_device(directory: PathBuf) -> Self {
        Self {
            audio_device: false,
            ..Self::new(directory)
        }
    }
    pub fn play(&mut self, cue: Cue) -> Result<PathBuf> {
        let path = prepare(&self.directory, cue)?;
        if self
            .player
            .as_ref()
            .is_some_and(|player| !player.snapshot().error.is_empty())
        {
            self.player = None;
        }
        let player = self.player.get_or_insert_with(|| {
            if self.audio_device {
                Player::new()
            } else {
                Player::without_audio_device()
            }
        });
        player.set("volume", "70");
        player.open(&path);
        self.reported_error.clear();
        Ok(path)
    }
    pub fn stop(&self) {
        if let Some(player) = &self.player {
            player.command(["stop"]);
        }
    }
    pub fn snapshot(&self) -> Option<Playback> {
        self.player.as_ref().map(Player::snapshot)
    }
    pub fn take_error(&mut self) -> Option<String> {
        let error = self.player.as_ref()?.snapshot().error;
        if error.is_empty() || error == self.reported_error {
            return None;
        }
        self.reported_error = error.clone();
        Some(error)
    }
}

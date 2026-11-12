//! Playback-time accounting for the statistics page.
//!
//! A fixed timer tick is not a reliable clock: frame drops and system sleep
//! make the UI tick late, and a pending fraction used to be lost on pause or
//! quit. This accumulator uses the player position as the source of truth and
//! uses wall time only to reject seeks and sleep gaps.

use std::time::Instant;

/// Persist a partial sample after this many seconds of playback.
pub const FLUSH_SECONDS: f64 = 30.;

#[derive(Default)]
pub struct MediaStats {
    pending: f64,
    position: f64,
    path: String,
    sampled_at: Option<Instant>,
}

impl MediaStats {
    /// Observe one player snapshot. Returns true when the caller should drain
    /// and persist the pending seconds.
    ///
    /// `active` is false while paused, stopped, or on a page that must not
    /// count as media playback. A path change or an implausible position jump
    /// settles the previous sample without counting the jump itself.
    pub fn observe(
        &mut self,
        path: &str,
        position: f64,
        rate: f64,
        active: bool,
        now: Instant,
    ) -> bool {
        if !active || !position.is_finite() || position < 0. {
            self.sampled_at = None;
            return self.pending > 0.;
        }
        let Some(last) = self.sampled_at else {
            self.position = position;
            self.path.clear();
            self.path.push_str(path);
            self.sampled_at = Some(now);
            return false;
        };
        if self.path != path {
            let flush = self.pending > 0.;
            self.position = position;
            self.path.clear();
            self.path.push_str(path);
            self.sampled_at = Some(now);
            return flush;
        }
        let wall = now.saturating_duration_since(last).as_secs_f64();
        let advanced = position - self.position;
        let rate = if rate.is_finite() {
            rate.clamp(0.1, 4.0)
        } else {
            1.
        };
        // A seek is much larger than the media could have advanced at the
        // current speed. A zero/negative jump is buffering, a pause, or sleep.
        if advanced > 0. && advanced <= wall * rate.max(1.) + 0.25 {
            // Count wall time, but cap it by actual progress so a long
            // buffering gap is not credited as playback. At 0.5x/2x normal
            // progress still settles to the full wall interval.
            self.pending += wall.min(advanced / rate + 0.1);
        }
        self.position = position;
        self.sampled_at = Some(now);
        self.pending >= FLUSH_SECONDS
    }

    /// Take the accumulated seconds. The caller persists them atomically.
    pub fn take(&mut self) -> f64 {
        std::mem::take(&mut self.pending)
    }

    /// Return seconds after a failed persistence attempt so the next tick can
    /// retry instead of silently dropping the sample.
    pub fn restore(&mut self, seconds: f64) {
        if seconds.is_finite() && seconds > 0. {
            self.pending += seconds;
        }
    }
}

//! A/B handles share colors and inward flags across text, waveform and timeline.
use super::*;
pub(super) const START_COLOR: u32 = 0x2563eb;
pub(super) const END_COLOR: u32 = 0xc0267d;
pub(super) fn edge_color(start: bool) -> Rgba {
    rgb(if start { START_COLOR } else { END_COLOR })
}
fn flag(start: bool) -> Div {
    div()
        .px_1()
        .rounded_sm()
        .bg(edge_color(start))
        .text_color(rgb(0xffffff))
        .text_size(px(10.))
        .line_height(px(13.))
        .font_weight(FontWeight::BOLD)
        .whitespace_nowrap()
        .child(if start { "A ▶" } else { "◀ B" })
}
/// Text markers are absolute children of a word, never independent flex items.
pub(super) fn text_edge(edge: subtitles::WordEdge, start: bool) -> Div {
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .left(relative(edge.fraction))
        .w(px(0.))
        .child(
            div()
                .absolute()
                .top(px(13.))
                .bottom(px(13.))
                .w(px(3.))
                .bg(edge_color(start)),
        )
        .child(if start {
            flag(true).absolute().top_0().left_0()
        } else {
            flag(false).absolute().bottom_0().right_0()
        })
}
pub(super) fn waveform_edge(time: f64, duration: f64, start: bool) -> Div {
    let fraction = if duration.is_finite() && duration > 0. {
        (time / duration).clamp(0., 1.) as f32
    } else {
        0.
    };
    // Keep the flags within the waveform even at media start/end.
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .left(relative(fraction))
        .w(px(0.))
        .child(if start {
            flag(true)
                .absolute()
                .top_0()
                .when(fraction > 0.9, |v| v.right_0())
                .when(fraction <= 0.9, |v| v.left_0())
        } else {
            flag(false)
                .absolute()
                .bottom_0()
                .when(fraction < 0.1, |v| v.left_0())
                .when(fraction >= 0.1, |v| v.right_0())
        })
}

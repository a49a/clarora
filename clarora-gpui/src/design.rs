//! Shared desktop palette and embedded line icons.
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Theme};
use std::borrow::Cow;

pub fn apply(cx: &mut App) {
    let dark = cx.theme().is_dark();
    let theme = Theme::global_mut(cx);
    theme.font_size = px(14.);
    theme.radius = px(8.);
    theme.radius_lg = px(16.);
    let (surface, foreground, muted, secondary, border, accent, primary) = if dark {
        (
            0x172522, 0xe3ede8, 0x20332d, 0xa2b6ac, 0x30463d, 0x24493d, 0x7dd8b5,
        )
    } else {
        (
            0xffffff, 0x233b34, 0xf0f4ef, 0x74817a, 0xe2e8df, 0xe2f0e7, 0x26765b,
        )
    };
    theme.background = rgb(surface).into();
    theme.foreground = rgb(foreground).into();
    theme.muted = rgb(muted).into();
    theme.muted_foreground = rgb(secondary).into();
    theme.border = rgb(border).into();
    theme.input = rgb(border).into();
    theme.accent = rgb(accent).into();
    theme.accent_foreground = rgb(primary).into();
    theme.primary = rgb(primary).into();
    theme.primary_hover = rgb(if dark { 0x95e3c5 } else { 0x1e634c }).into();
    theme.primary_active = rgb(if dark { 0x65c7a4 } else { 0x174d3c }).into();
    theme.primary_foreground = rgb(if dark { 0x102c21 } else { 0xffffff }).into();
    theme.ring = rgb(primary).into();
    theme.caret = rgb(primary).into();
    theme.link = rgb(primary).into();
    theme.secondary = rgb(muted).into();
    theme.secondary_foreground = rgb(foreground).into();
}
pub fn canvas(cx: &App) -> Hsla {
    rgb(if cx.theme().is_dark() {
        0x101c18
    } else {
        0xf6f7f3
    })
    .into()
}
pub fn rail(cx: &App) -> Hsla {
    rgb(if cx.theme().is_dark() {
        0x14211c
    } else {
        0xeef2eb
    })
    .into()
}
pub fn icon(name: &str, color: Hsla) -> Svg {
    svg()
        .path(format!("ui/{name}.svg"))
        .size(px(18.))
        .text_color(color)
}
fn paths(name: &str) -> Option<&'static str> {
    Some(match name {
        "discover" => "<path d='m12 3 2.4 6.6L21 12l-6.6 2.4L12 21l-2.4-6.6L3 12l6.6-2.4Z'/>",
        "flashcard" => {
            "<rect x='6' y='6' width='15' height='15' rx='3'/><path d='M3 16V5a2 2 0 0 1 2-2h11M10 11h7m-7 5h4'/>"
        }
        "listening" => {
            "<path d='M4 14v-3a8 8 0 0 1 16 0v3'/><rect x='3' y='12' width='4' height='8' rx='2'/><rect x='17' y='12' width='4' height='8' rx='2'/>"
        }
        "video" => "<rect x='3' y='4' width='18' height='16' rx='3'/><path d='m10 8 6 4-6 4Z'/>",
        "speaking" => {
            "<rect x='9' y='3' width='6' height='12' rx='3'/><path d='M5 11v1a7 7 0 0 0 14 0v-1M12 19v3m-4 0h8'/>"
        }
        "library" => {
            "<path d='M3 8V5a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Zm0 0h18'/>"
        }
        "pdf" => {
            "<path d='M12 5c-3-3-7-3-10-1v15c3-2 7-2 10 1 3-3 7-3 10-1V4c-3-2-7-2-10 1Zm0 0v15'/>"
        }
        "ocr" => {
            "<path d='M8 3H4a1 1 0 0 0-1 1v4m13-5h4a1 1 0 0 1 1 1v4M3 16v4a1 1 0 0 0 1 1h4m8 0h4a1 1 0 0 0 1-1v-4M8 16l4-9 4 9m-6.5-3h5'/>"
        }
        "graph" => {
            "<circle cx='12' cy='5' r='3'/><circle cx='5' cy='19' r='3'/><circle cx='19' cy='19' r='3'/><path d='m10.5 8-4 8m7-8 4 8M8 19h8'/>"
        }
        "stats" => "<path d='M4 3v17h17M8 16v-5m5 5V7m5 9V4'/>",
        "meditation" => "<path d='M20 3C10 2 3 6 4 13a7 7 0 0 0 11 6c5-4 4-9 5-16ZM3 22 16 8'/>",
        "settings" => {
            "<path d='M4 6h16M4 12h16M4 18h16'/><circle cx='9' cy='6' r='2' fill='white'/><circle cx='15' cy='12' r='2' fill='white'/><circle cx='8' cy='18' r='2' fill='white'/>"
        }
        "fullscreen" => "<path d='M9 3H3v6m12-6h6v6M3 15v6h6m12-6v6h-6'/>",
        "fullscreen-exit" => "<path d='M3 9h6V3m6 0v6h6M9 21v-6H3m18 0h-6v6'/>",
        "sidebar-open" => "<path d='M3 4v16M7 6l6 6-6 6m7-12 6 6-6 6' stroke-width='2.5'/>",
        "sidebar-close" => "<path d='M21 4v16M17 6l-6 6 6 6M10 6l-6 6 6 6' stroke-width='2.5'/>",
        "play" => "<path d='m9 5 11 7-11 7Z' fill='black' stroke='none'/>",
        "pause" => "<path d='M8 5v14M16 5v14' stroke-width='4'/>",
        "rewind" => "<path d='M4 8a8 8 0 1 1-1 8M4 3v5h5'/>",
        "forward" => "<path d='M20 8a8 8 0 1 0 1 8m-1-13v5h-5'/>",
        "volume" => "<path d='m11 4-5 4H3v8h3l5 4ZM15 8a6 6 0 0 1 0 8m3-11a10 10 0 0 1 0 14'/>",
        "muted" => "<path d='m11 4-5 4H3v8h3l5 4Zm5 5 6 6m0-6-6 6'/>",
        "arrow" => "<path d='M4 12h16m-6-6 6 6-6 6'/>",
        "check" => "<path d='m5 12 4 4L19 6'/>",
        "sun" => {
            "<circle cx='12' cy='12' r='4'/><path d='M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1 1m12 12 1 1M5 19l1-1M18 6l1-1'/>"
        }
        _ => return None,
    })
}
pub struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(path.strip_prefix("ui/").and_then(|p| p.strip_suffix(".svg"))
            .and_then(paths).map(|body| Cow::Owned(format!("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='black' stroke-width='1.6' stroke-linecap='round' stroke-linejoin='round'>{body}</svg>").into_bytes())))
    }
    fn list(&self, _: &str) -> Result<Vec<SharedString>> {
        Ok(vec![])
    }
}

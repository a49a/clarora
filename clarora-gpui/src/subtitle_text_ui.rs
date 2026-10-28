//! Native character hit-testing over existing subtitle word cells; all cells share one selection.
use super::*;
use clarora_gpui::subtitle_selection::{Document, Selection};
use std::{cell::RefCell, ops::Range, rc::Rc, sync::Arc};
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Surface {
    Audio,
    Focus,
    Video,
}
#[derive(Clone)]
struct Fragment {
    surface: Surface,
    range: Range<usize>,
    bounds: Bounds<Pixels>,
    layout: TextLayout,
}
pub(super) struct State {
    pub focus: FocusHandle,
    audio: Arc<Document>,
    video: Arc<Document>,
    selection: Selection,
    pub block_click: bool,
    surface: Surface,
    press: Point<Pixels>,
    layouts: Vec<Fragment>,
}
impl State {
    pub fn new(cx: &mut App) -> Self {
        Self {
            focus: cx.focus_handle(),
            audio: Arc::new(Document::new(String::new(), &[])),
            video: Arc::new(Document::new(String::new(), &[])),
            selection: Default::default(),
            block_click: false,
            surface: Surface::Audio,
            press: Point::default(),
            layouts: vec![],
        }
    }
    pub fn documents(&mut self, audio: Document, video: Document) {
        let changed_audio = *self.audio != audio;
        let changed_video = *self.video != video;
        if (changed_audio && self.surface != Surface::Video)
            || (changed_video && self.surface == Surface::Video)
        {
            self.clear();
        }
        if changed_audio {
            self.audio = Arc::new(audio);
        }
        if changed_video {
            self.video = Arc::new(video);
        }
    }
    pub fn clear(&mut self) {
        self.selection.clear();
        self.block_click = false;
    }
    pub fn has_selection(&self) -> bool {
        self.selection.has_selection()
    }
    pub fn dragged(&self) -> bool {
        self.block_click
    }
    pub fn selection_text(&self) -> String {
        self.selection.text()
    }
    pub fn selection_from_press(&self, press: Option<Point<Pixels>>) -> bool {
        self.selection.has_selection() && press.is_some_and(|p| p == self.press)
    }
    pub fn begin_frame(&mut self) {
        self.layouts.clear();
    }
    fn doc(&self, surface: Surface) -> Arc<Document> {
        if surface == Surface::Video {
            self.video.clone()
        } else {
            self.audio.clone()
        }
    }
    fn nearest(&self, position: Point<Pixels>) -> Option<usize> {
        self.layouts
            .iter()
            .filter(|f| f.surface == self.surface)
            .min_by(|a, b| {
                let distance = |f: &Fragment| {
                    let dx = f32::from(
                        (f.bounds.left() - position.x)
                            .max(px(0.))
                            .max(position.x - f.bounds.right()),
                    );
                    let dy = f32::from(
                        (f.bounds.top() - position.y)
                            .max(px(0.))
                            .max(position.y - f.bounds.bottom()),
                    );
                    dx * dx + dy * dy * 100.
                };
                distance(a).total_cmp(&distance(b))
            })
            .map(|f| {
                let local = f
                    .layout
                    .index_for_position(position)
                    .unwrap_or_else(|i| i)
                    .min(f.range.len());
                f.range.start + local
            })
    }
}
pub(super) struct SelectableText {
    text: StyledText,
    document: Arc<Document>,
    range: Range<usize>,
    surface: Surface,
    state: Rc<RefCell<State>>,
    view: WeakEntity<AppView>,
}
impl IntoElement for SelectableText {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for SelectableText {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let (layout, _) = self.text.request_layout(id, inspector, window, cx);
        (layout, ())
    }
    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        self.text
            .prepaint(id, inspector, bounds, &mut (), window, cx);
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }
    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.text
            .paint(id, inspector, bounds, &mut (), &mut (), window, cx);
        let layout = self.text.layout().clone();
        self.state.borrow_mut().layouts.push(Fragment {
            surface: self.surface,
            range: self.range.clone(),
            bounds,
            layout: layout.clone(),
        });
        window.set_cursor_style(CursorStyle::IBeam, hitbox);
        let state = self.state.clone();
        let view = self.view.clone();
        let range = self.range.clone();
        let document = self.document.clone();
        let surface = self.surface;
        let hitbox = hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if !phase.bubble() || event.button != MouseButton::Left || !hitbox.is_hovered(window) {
                return;
            }
            let allowed = view
                .update(cx, |this, _| match surface {
                    Surface::Video => this.page == "video",
                    _ => this.page == "listening" && !this.audio.blind.enabled,
                })
                .unwrap_or(false);
            if !allowed || !Arc::ptr_eq(&state.borrow().doc(surface), &document) {
                return;
            }
            let index = layout
                .index_for_position(event.position)
                .unwrap_or_else(|i| i)
                .min(range.len());
            let focus = {
                let mut state = state.borrow_mut();
                let doc = state.doc(surface);
                state.surface = surface;
                state.press = event.position;
                state.selection.begin(doc, range.start + index);
                state.focus.clone()
            };
            window.focus(&focus);
            window.prevent_default();
            let _ = view.update(cx, |_, cx| cx.notify());
        });
        let state = self.state.clone();
        let view = self.view.clone();
        // Capture release after a drag so no word/cue click can seek or alter A/B.
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if !phase.capture() || event.button != MouseButton::Left {
                return;
            }
            let mut state = state.borrow_mut();
            if state.selection.selecting {
                let dragged = state.selection.dragged;
                state.selection.finish();
                drop(state);
                if dragged {
                    cx.stop_propagation();
                }
                let _ = view.update(cx, |_, cx| cx.notify());
            }
        });
    }
}
impl AppView {
    pub(super) fn selected_focus_cue(&self) -> Option<usize> {
        let state = self.subtitle_text.borrow();
        if state.surface != Surface::Focus
            || (!state.selection.selecting && !state.selection.has_selection())
        {
            return None;
        }
        let anchor = state.selection.anchor;
        state
            .audio
            .cues
            .iter()
            .position(|cue| cue.original.start <= anchor && anchor <= cue.translation.end)
    }
    pub(super) fn subtitle_fragment(
        &self,
        surface: Surface,
        cue: usize,
        word: Option<usize>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let state = self.subtitle_text.borrow();
        let doc = state.doc(surface);
        let range = doc
            .cues
            .get(cue)
            .and_then(|c| {
                if let Some(word) = word {
                    c.words.get(word).cloned()
                } else {
                    Some(c.translation.clone())
                }
            })
            .unwrap_or(0..0);
        let mut text = StyledText::new(doc.text[range.clone()].to_string());
        if state.surface == surface && state.selection.has_selection() {
            let selected = state.selection.range();
            let start = selected.start.max(range.start);
            let end = selected.end.min(range.end);
            if start < end {
                text = text.with_highlights([(
                    start - range.start..end - range.start,
                    HighlightStyle {
                        background_color: Some(cx.theme().selection),
                        ..Default::default()
                    },
                )]);
            }
        }
        SelectableText {
            text,
            document: doc,
            range,
            surface,
            state: self.subtitle_text.clone(),
            view: cx.entity().downgrade(),
        }
        .into_any_element()
    }
    pub(super) fn subtitle_drag_update(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let mut state = self.subtitle_text.borrow_mut();
        if !event.dragging() || !state.selection.selecting {
            return;
        }
        let moved = (event.position - state.press).magnitude() >= 4.;
        let started = moved && !state.selection.dragged;
        if let Some(index) = state.nearest(event.position) {
            state.selection.extend(index, moved);
            state.block_click |= moved;
        }
        drop(state);
        if started {
            self.media.player.set("pause", "yes");
        }
        cx.notify();
    }
    pub(super) fn subtitle_copy_anchor(&self, cx: &Context<Self>) -> Div {
        let focus = self.subtitle_text.borrow().focus.clone();
        div()
            .absolute()
            .size(px(1.))
            .track_focus(&focus)
            .key_context("SubtitleText")
            .on_action(cx.listener(|this, _: &CopySubtitleSelection, _, cx| {
                let selected = this.subtitle_text.borrow().selection.text();
                if !selected.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(selected));
                }
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &ClearSubtitleSelection, window, cx| {
                this.subtitle_text.borrow_mut().clear();
                window.focus(&this.focus);
                cx.notify();
            }))
    }
    pub(super) fn subtitle_dragged(&self) -> bool {
        self.subtitle_text.borrow().dragged()
    }
    pub(super) fn has_subtitle_selection(&self) -> bool {
        self.subtitle_text.borrow().has_selection()
    }
}

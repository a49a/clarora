//! Center a cue by measuring only that row and the few rows above it.
use super::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Default)]
pub(super) struct Follow {
    pending: Rc<Cell<Option<usize>>>,
    viewport: Rc<Cell<Size<Pixels>>>,
}

impl Follow {
    pub fn request(&self, index: usize) {
        self.pending.set(Some(index));
    }
}

pub(super) fn centered_list(
    follow: &Follow,
    scroll: ListState,
    enabled: bool,
    active: Option<usize>,
    render: impl FnMut(usize, &mut Window, &mut App) -> AnyElement + 'static,
) -> Div {
    if !enabled || follow.pending.get() != active {
        follow.pending.set(None);
    }
    let render = Rc::new(RefCell::new(render));
    let measure = render.clone();
    let pending = follow.pending.clone();
    let viewport = follow.viewport.clone();
    let measured_scroll = scroll.clone();
    div()
        .relative()
        .overflow_hidden()
        // Measure before the list: its state is not borrowed yet, and this
        // frame's width is known (important after resizing or changing font).
        .child(
            canvas(
                move |bounds, window, cx| {
                    if bounds.size.height <= px(0.) || bounds.size.width <= px(0.) {
                        return;
                    }
                    let resized = viewport.replace(bounds.size) != bounds.size;
                    let requested = pending
                        .take()
                        .or_else(|| (enabled && resized).then_some(active).flatten());
                    let Some(index) = requested else { return };
                    if index >= measured_scroll.item_count() {
                        return;
                    }
                    let available = size(
                        AvailableSpace::Definite(bounds.size.width),
                        AvailableSpace::MinContent,
                    );
                    let mut render = measure.borrow_mut();
                    let height = render(index, window, cx)
                        .layout_as_root(available, window, cx)
                        .height;
                    let mut remaining = (bounds.size.height - height).max(px(0.)) / 2.;
                    let mut first = index;
                    let mut offset = px(0.);
                    while remaining > px(0.) && first > 0 {
                        first -= 1;
                        let height = render(first, window, cx)
                            .layout_as_root(available, window, cx)
                            .height;
                        offset = (height - remaining).max(px(0.));
                        remaining -= height;
                    }
                    // Native list layout clamps at the beginning/end. A row
                    // taller than the viewport starts at its first line.
                    measured_scroll.scroll_to(ListOffset {
                        item_ix: first,
                        offset_in_item: offset,
                    });
                },
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .child(
            list(scroll, move |index, window, cx| {
                render.borrow_mut()(index, window, cx)
            })
            .size_full(),
        )
}

// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

use crate::{Decorate, IntoText, TextSource, label};
use day_core::{BuildCx, Flex, PassThrough, Piece, RNode, with_tree};
use day_reactive::{Binding, Scope, Signal, bind};
use day_spec::{Event, GestureKind, kinds, props::ContainerProps};
use std::cell::Cell;
use std::rc::Rc;

/// Show a short, nonblocking message in the current window. The toolkit dismisses it.
pub fn toast<M>(text: impl IntoText<M>) {
    let text = text.into_text().initial();
    if !text.is_empty() {
        let root = day_core::current_window()
            .map(|w| day_core::id_to_rnode(day_core::windows::window_node_id(&w)));
        with_tree(|tree| tree.show_toast(root.unwrap_or_else(|| tree.root_node()), &text));
    }
}

pub(crate) fn install_tooltip(anchor: RNode, text: TextSource) {
    if with_tree(|tree| tree.needs_touch_tooltip()) {
        let help = text.clone();
        let scope = Scope::current();
        day_core::frame::request(move |_| {
            // Context menus and explicit long presses own this gesture, in either modifier order.
            let available = with_tree(|t| {
                t.node_exists(anchor)
                    && t.context_menu_of(anchor).is_none()
                    && t.gesture_ancestor(anchor, GestureKind::LongPress).is_none()
                    && t.gesture_target(anchor, GestureKind::LongPress).is_none()
            });
            if available {
                scope.enter(|| install_touch_help(anchor, help));
            }
        })
        .in_scope();
    }
    bind(
        move || text.resolve(),
        move |help| with_tree(|t| t.set_tooltip(anchor, help)),
    );
}

fn install_touch_help(anchor: RNode, text: TextSource) {
    let open = Signal::new(false);
    let content = text.clone();
    install_popover(anchor, open, move || label(content.clone()).padding(8.0));
    let mut cx = BuildCx::new(anchor);
    with_tree(|t| t.enable_gesture(anchor, GestureKind::LongPress));
    let timer = Rc::new(Cell::new(None::<day_core::TaskHandle>));
    let cleanup = timer.clone();
    Scope::current().on_cleanup(move || {
        if let Some(task) = cleanup.take() {
            task.abort();
        }
    });
    cx.on(anchor, move |event| {
        if matches!(event, Event::LongPress(_)) && !text.initial().is_empty() {
            if let Some(task) = timer.take() {
                task.abort();
            }
            open.set(true);
            timer.set(Some(day_core::task(async move {
                day_core::sleep(3000).await;
                open.set(false);
            })));
        }
    });
}

pub(crate) fn install_popover<P: Piece>(
    anchor: RNode,
    open: impl Binding<bool>,
    content: impl Fn() -> P + 'static,
) {
    let owner = Scope::current();
    let active = Rc::new(Cell::new(None::<(RNode, Scope)>));
    // During construction the native anchor may still be detached. An initially
    // open binding presents once the window has had a layout opportunity.
    let mounted = open.peek().then(|| {
        let ready = Signal::new(false);
        day_core::frame::request(move |_| ready.set(true)).in_scope();
        ready
    });
    let read = open.clone();
    bind(
        move || read.read() && mounted.is_none_or(|ready| ready.get()),
        move |shown| {
            if let Some((panel, scope)) = active.take() {
                scope.dispose();
                with_tree(|t| t.close_popover(anchor, panel));
            }
            if !*shown || !with_tree(|t| t.node_exists(anchor)) {
                return;
            }
            let scope = owner.enter(Scope::child);
            scope.enter(|| {
                let panel = with_tree(|t| {
                    t.create_node(
                        kinds::CONTAINER,
                        &ContainerProps::default(),
                        Rc::new(PassThrough),
                        Flex::default(),
                        true,
                        true,
                        scope,
                    )
                });
                let mut cx = BuildCx::new(panel);
                content().build(&mut cx);
                let state = open.clone();
                cx.on(panel, move |event| {
                    if matches!(event, Event::PopoverDismissed) {
                        state.write(false);
                    }
                });
                active.set(Some((panel, scope)));
                if !with_tree(|t| t.open_popover(anchor, panel)) {
                    day_core::enqueue_event(day_core::rnode_to_id(panel), Event::PopoverDismissed);
                }
            });
        },
    );
}

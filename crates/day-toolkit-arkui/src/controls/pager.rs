use crate::arkui_sys::native_node::{
    ArkUI_NodeAttributeType as Attr, ArkUI_NodeEvent, ArkUI_NodeEventType as Ev, ArkUI_NodeType,
    OH_ArkUI_NodeEvent_GetEventType, OH_ArkUI_NodeEvent_GetInputEvent,
    OH_ArkUI_NodeEvent_GetNodeComponentEvent, OH_ArkUI_NodeEvent_GetNodeHandle,
};
use crate::arkui_sys::ui_input_event::OH_ArkUI_UIInputEvent_GetAction;
use crate::{AHandle, ArkUi, node};
use day_pieces::pager::*;
use day_spec::Event;
use day_spec::NodeId;
use std::{cell::RefCell, collections::HashMap};
struct State {
    node: NodeId,
    slots: Vec<node::Handle>,
    config: PagerState,
    busy: bool,
    animating: bool,
    touching: bool,
    pending: Option<PagerState>,
    serial: u64,
}
day_reactive::tls_slots! {
    pager;
    static STATES: RefCell<HashMap<usize, State>> = RefCell::new(HashMap::new());
}
fn apply(handle: node::Handle, config: PagerState) {
    let pages = STATES.with(|states| {
        let mut states = states.borrow_mut();
        let state = states.get_mut(&(handle as usize))?;
        if state.busy {
            state.pending = Some(config.clone());
            return None;
        }
        if state.slots.is_empty() {
            state.slots = (0..node::child_count(handle))
                .map(|i| node::child_at(handle, i as i32))
                .collect();
        }
        state.config = config.clone();
        Some(state.slots.clone())
    });
    let Some(pages) = pages else {
        return;
    };
    // No registry borrow across ArkUI calls: setters may synchronously invoke receivers.
    for _ in 0..node::child_count(handle) {
        node::remove_child(handle, node::child_at(handle, 0));
    }
    for slot in &config.order {
        if let Some(page) = pages.get(*slot) {
            node::add_child(handle, *page);
        }
    }
    if let Some(index) = config.order.iter().position(|s| *s == config.selected) {
        node::set_i32(handle, Attr::NODE_SWIPER_INDEX, index as i32);
    }
}
fn begin(key: usize, animated: bool) {
    let event = STATES.with(|states| {
        let mut states = states.borrow_mut();
        let state = states.get_mut(&key)?;
        state.animating |= animated;
        if state.busy {
            return None;
        }
        state.busy = true;
        state.serial = state.serial.wrapping_add(1);
        Some((state.node, state.config.epoch))
    });
    if let Some((id, epoch)) = event {
        crate::emit(id, Event::custom("pager", format!("begin {epoch}")));
    }
}

fn settle(key: usize, index: usize) {
    let result = STATES.with(|states| {
        let mut states = states.borrow_mut();
        let state = states.get_mut(&key)?;
        if state.animating || state.touching || !state.busy {
            return None;
        }
        state.busy = false;
        let slot = state
            .config
            .order
            .get(index)
            .copied()
            .unwrap_or(state.config.selected);
        Some((state.node, state.config.epoch, slot, state.pending.take()))
    });
    if let Some((id, epoch, slot, pending)) = result {
        crate::emit(id, Event::custom("pager", format!("settle {epoch} {slot}")));
        if let Some(config) = pending {
            apply(key as node::Handle, config);
        }
    }
}
fn defer_settle(key: usize, index: Option<usize>) {
    let token = STATES.with(|states| states.borrow().get(&key).map(|s| (s.node, s.serial)));
    let Some(token) = token else {
        return;
    };
    crate::post_local(Box::new(move || {
        let live = STATES.with(|states| {
            states.borrow().get(&key).is_some_and(|s| {
                (s.node, s.serial) == token && s.busy && !s.animating && !s.touching
            })
        });
        if !live {
            return;
        }
        let index = index.unwrap_or_else(|| {
            node::get_i32(key as node::Handle, Attr::NODE_SWIPER_INDEX, 0)
                .unwrap_or(0)
                .max(0) as usize
        });
        settle(key, index);
    }));
}
unsafe extern "C" fn on_event(event: *mut ArkUI_NodeEvent) {
    day_spec::ffi_guard::contain((), || {
        // ArkUI owns the event and its payload for the duration of this callback.
        let (handle, kind, index) = unsafe {
            let payload = OH_ArkUI_NodeEvent_GetNodeComponentEvent(event);
            (
                OH_ArkUI_NodeEvent_GetNodeHandle(event),
                OH_ArkUI_NodeEvent_GetEventType(event),
                if payload.is_null() {
                    0
                } else {
                    (*payload).data[0].i32_.max(0) as usize
                },
            )
        };
        let key = handle as usize;
        if kind == Ev::NODE_TOUCH_EVENT {
            let action =
                unsafe { OH_ArkUI_UIInputEvent_GetAction(OH_ArkUI_NodeEvent_GetInputEvent(event)) };
            if action == 1 || action == 0 || action == 3 {
                STATES.with(|states| {
                    if let Some(state) = states.borrow_mut().get_mut(&key) {
                        state.touching = action == 1;
                        if state.touching {
                            state.serial = state.serial.wrapping_add(1);
                        }
                    }
                });
            }
            if action == 1 {
                begin(key, false);
            }
            if action == 0 || action == 3 {
                defer_settle(key, None);
            }
        } else if kind == Ev::NODE_SWIPER_EVENT_ON_ANIMATION_START {
            begin(key, true);
        } else if kind == Ev::NODE_SWIPER_EVENT_ON_GESTURE_SWIPE {
            // This is a per-frame progress event, not a completed gesture. Never
            // select/reorder here: the native animation still owns the page geometry.
            begin(key, false);
        } else if kind == Ev::NODE_SWIPER_EVENT_ON_ANIMATION_END {
            STATES.with(|states| {
                if let Some(state) = states.borrow_mut().get_mut(&key) {
                    state.animating = false;
                }
            });
            defer_settle(key, Some(index));
        }
    });
}
fn make(_: &mut ArkUi, _: &PagerProps, id: NodeId) -> AHandle {
    let handle = node::create(ArkUI_NodeType::ARKUI_NODE_SWIPER);
    node::set_i32(handle, Attr::NODE_SWIPER_LOOP, 0);
    node::set_i32(handle, Attr::NODE_SWIPER_SHOW_INDICATOR, 0);
    node::set_i32(handle, Attr::NODE_SWIPER_CACHED_COUNT, 2);
    STATES.with(|states| {
        states.borrow_mut().insert(
            handle as usize,
            State {
                node: id,
                slots: vec![],
                config: PagerState::default(),
                busy: false,
                animating: false,
                touching: false,
                pending: None,
                serial: 0,
            },
        );
    });
    for event in [
        Ev::NODE_SWIPER_EVENT_ON_ANIMATION_START,
        Ev::NODE_SWIPER_EVENT_ON_ANIMATION_END,
        Ev::NODE_SWIPER_EVENT_ON_GESTURE_SWIPE,
        Ev::NODE_TOUCH_EVENT,
    ] {
        node::register_event(handle, event, id.0);
    }
    node::add_event_receiver(handle, on_event);
    AHandle(handle)
}
fn update(_: &mut ArkUi, h: &AHandle, p: &PagerPatch) {
    apply(h.0, p.0.clone());
}
fn release(_: &mut ArkUi, h: &AHandle) {
    STATES.with(|states| {
        states.borrow_mut().remove(&(h.0 as usize));
    });
}
day_pieces::renderer!(crate::RENDERERS,ArkUi,kind:KIND,props:PagerProps,patch:PagerPatch,make:make,update:update,measure:day_pieces::fill_measure,release:release);

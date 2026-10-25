// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
use day_spec::{Point, transfer::*};
use gtk4::{gdk, glib, prelude::*};

pub fn source(widget: &gtk4::Widget, source: Source) {
    let drag = gtk4::DragSource::new();
    drag.set_actions(gdk::DragAction::COPY);
    drag.connect_prepare(move |ds, x, y| {
        day_spec::ffi_guard::contain(None, || {
            let offer = source(Point::new(x, y))?;
            let packet = offer.encode()?;
            let mut providers = vec![gdk::ContentProvider::for_bytes(
                BUNDLE_MIME,
                &glib::Bytes::from_owned(packet),
            )];
            if let Some(item) = offer.items.first() {
                for r in &item.representations {
                    providers.push(gdk::ContentProvider::for_bytes(
                        &r.mime,
                        &glib::Bytes::from_owned(r.bytes.as_ref().clone()),
                    ));
                }
            }
            if let Some(w) = ds.widget() {
                ds.set_icon(
                    Some(&gtk4::WidgetPaintable::new(Some(&w))),
                    x as i32,
                    y as i32,
                );
            }
            Some(gdk::ContentProvider::new_union(&providers))
        })
    });
    widget.add_controller(drag);
}
fn location(drop: &gdk::Drop, x: f64, y: f64) -> Location {
    Location {
        local: drop.drag().is_some(),
        position: Point::new(x, y),
        types: drop
            .formats()
            .mime_types()
            .iter()
            .map(|s| s.to_string())
            .collect(),
        allowed: if drop.actions().contains(gdk::DragAction::COPY) {
            vec![Operation::Copy]
        } else {
            vec![]
        },
    }
}
fn action(target: &Target, drop: &gdk::Drop, x: f64, y: f64) -> gdk::DragAction {
    if target.proposal(&location(drop, x, y)) == Operation::Copy {
        gdk::DragAction::COPY
    } else {
        gdk::DragAction::empty()
    }
}
pub fn target(widget: &gtk4::Widget, target: Target) {
    let mut types: Vec<&str> = target.types.iter().map(String::as_str).collect();
    types.push(BUNDLE_MIME);
    let controller = gtk4::DropTargetAsync::new(
        Some(gdk::ContentFormats::new(&types)),
        gdk::DragAction::COPY,
    );
    controller.connect_drag_enter({
        let target = target.clone();
        move |_, d, x, y| action(&target, d, x, y)
    });
    controller.connect_drag_motion({
        let target = target.clone();
        move |_, d, x, y| action(&target, d, x, y)
    });
    controller.connect_drop(move |_, drop, x, y| {
        let at = location(drop, x, y);
        if target.proposal(&at) != Operation::Copy {
            return false;
        }
        let drop = drop.clone();
        let target = target.clone();
        glib::MainContext::default().spawn_local(async move {
            let mut types = vec![BUNDLE_MIME];
            types.extend(target.types.iter().map(String::as_str));
            let read = async {
                let (stream, mime) = drop
                    .read_future(&types, glib::Priority::DEFAULT)
                    .await
                    .ok()?;
                let mut bytes = Vec::new();
                loop {
                    let chunk = stream
                        .read_bytes_future(64 * 1024, glib::Priority::DEFAULT)
                        .await
                        .ok()?;
                    if chunk.is_empty() {
                        break;
                    }
                    if bytes.len() + chunk.len() > MAX_BYTES {
                        return None;
                    }
                    bytes.extend_from_slice(&chunk);
                }
                if mime == BUNDLE_MIME {
                    Offer::decode(&bytes)
                } else {
                    Some(Offer {
                        items: vec![Item::new(vec![Representation::new(mime.as_str(), bytes)])],
                    })
                }
            };
            // Dropping the GIO futures cancels their outstanding native reads.
            let offer = glib::future_with_timeout(std::time::Duration::from_secs(30), read)
                .await
                .ok()
                .flatten();
            let accepted = offer.is_some_and(|offer| target.deliver(at, offer));
            drop.finish(if accepted {
                gdk::DragAction::COPY
            } else {
                gdk::DragAction::empty()
            });
        });
        true
    });
    widget.add_controller(controller);
}

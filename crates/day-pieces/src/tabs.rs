// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Independently owned document tabs. Selection and order never identify a document;
//! stable keys do. A close request is separate from removal so apps can ask about edits.

use crate::{Decorate, IntoText, NavStyle, Route, TextSource, item, nav};
use day_core::with_tree;
use day_core::{AnyPiece, BuildCx, Piece, RNode};
use day_reactive::{Signal, batch};
use std::cell::RefCell;
use std::rc::Rc;

/// Ordered document identities and the selected document. Own this above the tab host.
/// Payloads (editor buffers, browser profiles, etc.) remain application-owned.
pub struct TabSet<K: Clone + PartialEq + 'static> {
    order: Signal<Vec<K>>,
    selected: Signal<Option<K>>,
}
impl<K: Clone + PartialEq + 'static> Copy for TabSet<K> {}
impl<K: Clone + PartialEq + 'static> Clone for TabSet<K> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<K: Clone + PartialEq + 'static> Default for TabSet<K> {
    fn default() -> Self {
        Self::new()
    }
}
impl<K: Clone + PartialEq + 'static> TabSet<K> {
    pub fn new() -> Self {
        Self {
            order: Signal::new(Vec::new()),
            selected: Signal::new(None),
        }
    }
    pub fn keys(self) -> Vec<K> {
        self.order.get()
    }
    pub fn selected(self) -> Option<K> {
        self.selected.get()
    }
    pub fn contains(self, key: &K) -> bool {
        self.order.get_untracked().contains(key)
    }
    /// Add a document once. Background insertion preserves the current selection.
    pub fn open(self, key: K, foreground: bool) {
        batch(|| {
            if !self.contains(&key) {
                self.order.update(|keys| keys.push(key.clone()));
            }
            if foreground || self.selected.get_untracked().is_none() {
                self.selected.set(Some(key));
            }
        });
    }
    pub fn select(self, key: &K) -> bool {
        if !self.contains(key) {
            return false;
        }
        self.selected.set(Some(key.clone()));
        true
    }
    /// Remove after the app accepts a close request. Prefer the following neighbor,
    /// or the preceding one when closing the last tab. Closing the final tab selects None.
    pub fn close(self, key: &K) -> bool {
        let mut keys = self.order.get_untracked();
        let Some(index) = keys.iter().position(|k| k == key) else {
            return false;
        };
        keys.remove(index);
        batch(|| {
            if self.selected.get_untracked().as_ref() == Some(key) {
                self.selected
                    .set(keys.get(index).or_else(|| keys.last()).cloned());
            }
            self.order.set(keys);
        });
        true
    }
    /// Move a document to its final index, preserving selected identity.
    pub fn move_to(self, key: &K, index: usize) -> bool {
        let mut keys = self.order.get_untracked();
        let Some(from) = keys.iter().position(|k| k == key) else {
            return false;
        };
        let to = index.min(keys.len() - 1);
        if from == to {
            return true;
        }
        let key = keys.remove(from);
        keys.insert(to, key);
        self.order.set(keys);
        true
    }
    pub fn select_next(self, backwards: bool) {
        let keys = self.order.get_untracked();
        if keys.is_empty() {
            return;
        }
        let current = self.selected.get_untracked();
        let at = keys
            .iter()
            .position(|k| Some(k) == current.as_ref())
            .unwrap_or(0);
        let next = if backwards {
            (at + keys.len() - 1) % keys.len()
        } else {
            (at + 1) % keys.len()
        };
        self.selected.set(Some(keys[next].clone()));
    }
}

type TabTitle<K> = Rc<dyn Fn(&K) -> String>;
type TabContent<K> = Rc<dyn Fn(&K) -> AnyPiece>;
/// A resident, closable document surface. Content is built once per key and disposed
/// on removal. A selected document owns its native focus and command context.
pub struct DocumentTabs<K: Route> {
    tabs: TabSet<K>,
    title: TabTitle<K>,
    content: TabContent<K>,
    new_label: TextSource,
    close_label: TextSource,
    on_new: Option<Rc<dyn Fn()>>,
    on_close: Option<Rc<dyn Fn(K)>>,
    native: Rc<dyn Fn() -> bool>,
    hide_when_empty: bool,
    chrome: Option<Rc<dyn Fn(TabActions<K>) -> AnyPiece>>,
    layout: Option<Box<dyn FnOnce(AnyPiece, AnyPiece) -> AnyPiece>>,
    strip_id: Option<day_core::ElementId>,
}
/// Labels are required rather than supplying untranslated English chrome.
pub fn document_tabs<K: Route, P: Piece, N, C>(
    tabs: TabSet<K>,
    new_label: impl IntoText<N>,
    close_label: impl IntoText<C>,
    title: impl Fn(&K) -> String + 'static,
    content: impl Fn(&K) -> P + 'static,
) -> DocumentTabs<K> {
    DocumentTabs {
        tabs,
        title: Rc::new(title),
        content: Rc::new(move |k| AnyPiece::new(content(k))),
        new_label: new_label.into_text(),
        close_label: close_label.into_text(),
        on_new: None,
        on_close: None,
        native: Rc::new(|| true),
        hide_when_empty: false,
        chrome: None,
        layout: None,
        strip_id: None,
    }
}
impl<K: Route> DocumentTabs<K> {
    /// Name the default strip's elements for scripts and tests: `{prefix}-strip` on the row of
    /// tabs, `{prefix}-tab-{key}` and `{prefix}-close-{key}` per document, `{prefix}-new` on
    /// the add button. Without it the prefix is `day-documents-{n}`, unique per host.
    pub fn strip_id(mut self, id: day_core::ElementId) -> Self {
        self.strip_id = Some(id);
        self
    }
    /// Reactive preference. Switching chrome preserves every resident page and native view.
    pub fn native(mut self, native: impl Fn() -> bool + 'static) -> Self {
        self.native = Rc::new(native);
        self
    }
    /// Keep the owning window hidden when no documents remain. The host and its scope
    /// stay alive, so a later open can present documents again. Intended for a dedicated
    /// document window; the default keeps the empty host visible.
    pub fn hide_when_empty(mut self, hide: bool) -> Self {
        self.hide_when_empty = hide;
        self
    }
    /// Fully app-owned emulated chrome. Use the supplied actions for shared close/reorder semantics.
    pub fn chrome<P: Piece>(mut self, build: impl Fn(TabActions<K>) -> P + 'static) -> Self {
        self.chrome = Some(Rc::new(move |actions| AnyPiece::new(build(actions))));
        self
    }
    /// Place chrome anywhere around the resident content (top, bottom, side, or overlay).
    /// The chrome is empty while native presentation is active; content is never rebuilt.
    pub fn layout<P: Piece>(
        mut self,
        build: impl FnOnce(AnyPiece, AnyPiece) -> P + 'static,
    ) -> Self {
        self.layout = Some(Box::new(move |bar, content| {
            AnyPiece::new(build(bar, content))
        }));
        self
    }

    pub fn on_new(mut self, f: impl Fn() + 'static) -> Self {
        self.on_new = Some(Rc::new(f));
        self
    }
    /// Installing a handler makes it responsible for calling TabSet::close when accepted.
    pub fn on_close(mut self, f: impl Fn(K) + 'static) -> Self {
        self.on_close = Some(Rc::new(f));
        self
    }
}
impl<K: Route> Piece for DocumentTabs<K> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        static NEXT_HOST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let host_id = NEXT_HOST.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let strip_prefix = Rc::new(
            self.strip_id
                .unwrap_or_else(|| day_core::id!(document_strip, host_id)),
        );
        let tabs = self.tabs;
        let preference = self.native;
        let window_tabs = day_core::windows::native_window_tabs();
        let toolkit_tabs = with_tree(|t| t.native_document_tabs());
        let native: Rc<dyn Fn() -> bool> =
            Rc::new(move || preference() && (window_tabs || toolkit_tabs));
        let custom_chrome = self.chrome;
        let layout = self.layout;
        let mounted = Rc::new(RefCell::new(Vec::<MountedTab<K>>::new()));
        let revision = Signal::new(0u64);
        let controller_title = self.title.clone();
        let controller_close = self.on_close.clone();
        let controller_new = self.on_new.clone();
        let native_windows = native.clone();
        let title = self.title;
        let content = self.content;
        let on_new = self.on_new;
        let chrome_new = on_new.clone();
        let chrome_add = on_new.clone();
        let on_close = self.on_close;
        let chrome_close = on_close.clone();
        let chrome_title = title.clone();
        let config = day_spec::props::DocumentTabsConfig {
            new_label: self.new_label.initial(),
            close_label: self.close_label.initial(),
            can_add: on_new.is_some(),
        };
        let actions = TabActions {
            tabs,
            host_id,
            on_new: on_new.clone(),
            on_close: on_close.clone(),
        };
        let pages = mounted.clone();
        let page_native = native.clone();
        let new_label = config.new_label.clone();
        let close_label = config.close_label.clone();
        let host = nav(tabs.selected)
            .style(NavStyle::Tabs)
            .local()
            .items(crate::nav_items(
                move || tabs.keys(),
                move |key| item(Some(key.clone()), title(key)),
            ))
            .destination(move |key: &Option<K>| match key {
                Some(k) => AnyPiece::new(MountTab {
                    key: k.clone(),
                    tabs,
                    native: if window_tabs {
                        page_native.clone()
                    } else {
                        Rc::new(|| false)
                    },
                    content: content(k),
                    pages: pages.clone(),
                    revision,
                }),
                None => AnyPiece::new(crate::spacer()),
            })
            .document_mode(config, move |event| {
                if let day_spec::Event::Custom { tag, num, text } = event {
                    match *tag {
                        "day:tab-new" => {
                            if let Some(f) = &on_new {
                                f();
                            }
                        }
                        "day:tab-close" => {
                            if let Some(key) = K::from_key(text).filter(|k| tabs.contains(k)) {
                                if let Some(f) = &on_close {
                                    f(key);
                                } else {
                                    tabs.close(&key);
                                }
                            }
                        }
                        "day:tab-move" => {
                            if let Some(key) = K::from_key(text).filter(|k| tabs.contains(k))
                                && num.is_finite()
                                && *num >= 0.0
                            {
                                tabs.move_to(&key, *num as usize);
                            }
                        }
                        _ => {}
                    }
                }
            });
        let host = AnyPiece::new(TabChromeHost {
            content: AnyPiece::new(host),
            native: native.clone(),
            window_tabs,
        });
        // The toolkit still owns the resident-page container. Where no closable document
        // strip exists, compose one from native buttons, scrolling, and drag sessions. The
        // current document's button is a selected button, not a disabled one: it keeps its
        // focus stop, re-activating it is harmless, and assistive tech hears "selected".
        let default_chrome = move || {
            let chrome_title = chrome_title.clone();
            let chrome_close = chrome_close.clone();
            let chrome_new = chrome_new.clone();
            let chrome_add = chrome_add.clone();
            let new_label = new_label.clone();
            let close_label = close_label.clone();
            let prefix = strip_prefix.clone();
            let tab_prefix = prefix.clone();
            let strip = crate::scroll(
                crate::row((crate::each(
                    crate::items(move || tabs.keys(), |key: &K| key.key()),
                    move |slot| {
                        let title = chrome_title.clone();
                        let key = slot.get();
                        let select_key = key.clone();
                        let selected_key = key.clone();
                        let announced_key = key.clone();
                        let close_key = key.clone();
                        let drag_key = key.clone();
                        let drop_key = key.clone();
                        let close = chrome_close.clone();
                        let close_menu = close.clone();
                        let menu_key = key.clone();
                        crate::row((
                            crate::button(move || title(&slot.get()))
                                .selected(move || tabs.selected().as_ref() == Some(&selected_key))
                                .action(move || {
                                    tabs.select(&select_key);
                                })
                                .id(day_core::id!(
                                    document_tab,
                                    format!("{}:{}", tab_prefix.as_str(), key.key())
                                ))
                                .a11y(move |a| {
                                    a.role(day_spec::Role::Tab).selected(move || {
                                        tabs.selected().as_ref() == Some(&announced_key)
                                    })
                                }),
                            crate::button(close_label.clone())
                                .icon(day_spec::Symbol::Close)
                                .icon_only()
                                .action(move || {
                                    if let Some(f) = &close {
                                        f(close_key.clone());
                                    } else {
                                        tabs.close(&close_key);
                                    }
                                })
                                .id(day_core::id!(
                                    document_close,
                                    format!("{}:{}", tab_prefix.as_str(), key.key())
                                )),
                        ))
                        .spacing(2.0)
                        .context_menu(vec![crate::menu_item(close_label.clone()).action(
                            move || {
                                if let Some(f) = &close_menu {
                                    f(menu_key.clone());
                                } else {
                                    tabs.close(&menu_key);
                                }
                            },
                        )])
                        .drag_source(move |_| {
                            Some(day_spec::transfer::Offer {
                                items: vec![day_spec::transfer::Item::new(vec![
                                    day_spec::transfer::Representation::new(
                                        "application/vnd.day.document-tab",
                                        format!("{host_id}:{}", drag_key.key()).into_bytes(),
                                    ),
                                ])],
                            })
                        })
                        .drop_target(day_spec::transfer::Target {
                            types: vec!["application/vnd.day.document-tab".into()],
                            accept: Rc::new(|location| {
                                if location.local
                                    && location.has("application/vnd.day.document-tab")
                                {
                                    day_spec::transfer::Operation::Move
                                } else {
                                    day_spec::transfer::Operation::None
                                }
                            }),
                            receive: Rc::new(move |drop| {
                                let Some(key) = drop
                                    .items
                                    .first()
                                    .and_then(|item| item.get("application/vnd.day.document-tab"))
                                    .and_then(|bytes| std::str::from_utf8(bytes).ok())
                                    .and_then(|text| text.strip_prefix(&format!("{host_id}:")))
                                    .and_then(K::from_key)
                                else {
                                    return false;
                                };
                                let Some(index) =
                                    tabs.keys().iter().position(|key| key == &drop_key)
                                else {
                                    return false;
                                };
                                tabs.move_to(&key, index)
                            }),
                        })
                    },
                ),))
                .spacing(4.0)
                .id((*prefix).clone())
                .a11y(|a| a.role(day_spec::Role::TabList)),
            )
            .horizontal()
            .height(44.0);
            let add = crate::when(
                move || chrome_new.is_some(),
                move || {
                    let on_new = chrome_add.clone();
                    crate::button(new_label.clone())
                        .action(move || {
                            if let Some(f) = &on_new {
                                f();
                            }
                        })
                        .id(day_core::id!(document_new, prefix.as_str()))
                },
            );
            AnyPiece::new(crate::row((strip.grow(), add)).spacing(4.0))
        };
        let bar = AnyPiece::new(crate::when(
            move || !native(),
            move || match &custom_chrome {
                Some(build) => build(actions.clone()),
                None => default_chrome(),
            },
        ));
        let root = match layout {
            Some(build) => build(bar, host).build(cx),
            None => crate::column((bar, host.grow())).build(cx),
        };
        if window_tabs {
            install_window_tabs(
                host_id,
                tabs,
                mounted,
                revision,
                native_windows,
                controller_title,
                controller_new,
                controller_close,
                self.hide_when_empty,
            );
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn document_order_and_selection_are_independent() {
        let scope = day_reactive::Scope::detached();
        scope.enter(|| {
            let tabs = TabSet::new();
            tabs.open(10, true);
            tabs.open(20, false);
            tabs.open(30, false);
            assert_eq!(tabs.selected(), Some(10));
            tabs.move_to(&10, 2);
            assert_eq!(tabs.keys(), vec![20, 30, 10]);
            assert_eq!(tabs.selected(), Some(10));
            tabs.close(&10);
            assert_eq!(tabs.selected(), Some(30));
            tabs.select_next(false);
            assert_eq!(tabs.selected(), Some(20));
            tabs.close(&30);
            assert_eq!(tabs.selected(), Some(20));
            tabs.close(&20);
            assert_eq!(tabs.selected(), None);
            assert!(!tabs.select(&40));
            assert!(!tabs.move_to(&40, 0));
        });
        scope.dispose();
    }
}

/// Commands shared by native and application-drawn tab chrome.
#[derive(Clone)]
pub struct TabActions<K: Route> {
    pub tabs: TabSet<K>,
    host_id: u64,
    on_new: Option<Rc<dyn Fn()>>,
    on_close: Option<Rc<dyn Fn(K)>>,
}
impl<K: Route> TabActions<K> {
    pub fn new_tab(&self) {
        if let Some(f) = &self.on_new {
            f();
        }
    }
    pub fn close(&self, key: K) {
        if let Some(f) = &self.on_close {
            f(key);
        } else {
            self.tabs.close(&key);
        }
    }
    /// Local drag data, scoped to this collection so unrelated tab hosts cannot collide.
    pub fn drag(&self, key: &K) -> day_spec::transfer::Offer {
        day_spec::transfer::Offer {
            items: vec![day_spec::transfer::Item::new(vec![
                day_spec::transfer::Representation::new(
                    "application/vnd.day.document-tab",
                    format!("{}:{}", self.host_id, key.key()).into_bytes(),
                ),
            ])],
        }
    }
    /// A drop target that moves a tab before this key, preserving document identity.
    pub fn drop_before(&self, key: K) -> day_spec::transfer::Target {
        let actions = self.clone();
        day_spec::transfer::Target {
            types: vec!["application/vnd.day.document-tab".into()],
            accept: Rc::new(|location| {
                if location.local && location.has("application/vnd.day.document-tab") {
                    day_spec::transfer::Operation::Move
                } else {
                    day_spec::transfer::Operation::None
                }
            }),
            receive: Rc::new(move |drop| {
                let Some(moving) = drop
                    .items
                    .first()
                    .and_then(|item| item.get("application/vnd.day.document-tab"))
                    .and_then(|bytes| std::str::from_utf8(bytes).ok())
                    .and_then(|text| text.strip_prefix(&format!("{}:", actions.host_id)))
                    .and_then(K::from_key)
                else {
                    return false;
                };
                let Some(index) = actions.tabs.keys().iter().position(|k| k == &key) else {
                    return false;
                };
                let from = actions.tabs.keys().iter().position(|k| k == &moving);
                actions.tabs.move_to(
                    &moving,
                    index.saturating_sub(usize::from(from.is_some_and(|from| from < index))),
                )
            }),
        }
    }
}

struct TabChromeHost {
    content: AnyPiece,
    native: Rc<dyn Fn() -> bool>,
    window_tabs: bool,
}
impl Piece for TabChromeHost {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let host = self.content.build(cx);
        day_reactive::bind(
            move || (self.native)() && !self.window_tabs,
            move |native| {
                with_tree(|t| {
                    t.patch(
                        host,
                        Box::new(day_spec::props::DocumentTabsPatch { native: *native }),
                        true,
                    )
                });
            },
        );
        host
    }
}
struct MountedTab<K> {
    scope: day_reactive::Scope,
    key: K,
    node: RNode,
    home: RNode,
    home_window: RNode,
    chrome_window: day_core::toolbar::DocumentWindow,
    window: Option<day_core::windows::WindowHandle>,
}
struct MountTab<K: Route> {
    key: K,
    tabs: TabSet<K>,
    native: Rc<dyn Fn() -> bool>,
    content: AnyPiece,
    pages: Rc<RefCell<Vec<MountedTab<K>>>>,
    revision: Signal<u64>,
}
impl<K: Route> Piece for MountTab<K> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let home = cx.parent();
        let home_window = day_core::current_page_window();
        let chrome_window = day_core::toolbar::DocumentWindow::new(home_window);
        day_reactive::Scope::current().provide(chrome_window.clone());
        let key = self.key.clone();
        let gate: Rc<dyn Fn() -> bool> =
            Rc::new(move || (self.native)() || self.tabs.selected().as_ref() == Some(&key));
        let node = crate::nav::with_document_active(gate.clone(), || {
            day_core::with_page_in(
                home,
                Some(gate),
                day_spec::ToolbarColumn::Detail,
                home_window,
                || self.content.build(cx),
            )
        });
        self.pages.borrow_mut().push(MountedTab {
            scope: day_reactive::Scope::current(),
            key: self.key.clone(),
            node,
            home,
            home_window,
            chrome_window,
            window: None,
        });
        self.revision.update(|n| *n += 1);
        day_reactive::Scope::current().on_cleanup(move || {
            let page = {
                let mut pages = self.pages.borrow_mut();
                pages
                    .iter()
                    .position(|p| p.key == self.key)
                    .map(|i| pages.remove(i))
            };
            if let Some(page) = page {
                with_tree(|t| {
                    t.reparent(page.node, page.home);
                });
                if let Some(window) = page.window {
                    window.close();
                }
            }
        });
        node
    }
}
struct MoveTab(RNode);
impl Piece for MoveTab {
    fn build(self, cx: &mut BuildCx) -> RNode {
        if with_tree(|t| t.reparent(self.0, cx.parent())) {
            self.0
        } else {
            // A queued open may outlive its document. Leave an empty host to close safely.
            crate::spacer().build(cx)
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn install_window_tabs<K: Route>(
    host: u64,
    tabs: TabSet<K>,
    pages: Rc<RefCell<Vec<MountedTab<K>>>>,
    revision: Signal<u64>,
    native: Rc<dyn Fn() -> bool>,
    title: TabTitle<K>,
    on_new: Option<Rc<dyn Fn()>>,
    on_close: Option<Rc<dyn Fn(K)>>,
    hide_when_empty: bool,
) {
    let group = format!("day.documents.{host}");
    if let Some(f) = on_new {
        day_core::windows::register_tab_new(group.clone(), f);
    }
    let initial = day_core::windows::current_window();
    let previous = Rc::new(RefCell::new(Vec::<K>::new()));
    // The window callbacks below outlive the host: closing a document's window on disposal
    // reports back after the host's scope, and its signals, are gone. Once the scope is
    // cleaned up every callback is a no-op.
    let alive = Rc::new(std::cell::Cell::new(true));
    {
        let alive = alive.clone();
        day_reactive::Scope::current().on_cleanup(move || alive.set(false));
    }
    let mode = native.clone();
    let focus_native = native.clone();
    let observed_alive = alive.clone();
    let observed_pages = pages.clone();
    let observed_previous = previous.clone();
    let observe: Rc<dyn Fn()> = Rc::new(move || {
        if !observed_alive.get() || !mode() {
            return;
        }
        let windows = observed_pages
            .borrow()
            .iter()
            .filter_map(|p| p.window.clone().map(|w| (p.key.clone(), w)))
            .collect::<Vec<_>>();
        let mut order = tabs.keys();
        for (_, window) in &windows {
            let group = window.tab_order();
            let keys = group
                .iter()
                .filter_map(|id| {
                    windows
                        .iter()
                        .find(|(_, w)| day_core::windows::window_node_id(w) == *id)
                        .map(|(k, _)| k.clone())
                })
                .collect::<Vec<_>>();
            let positions = order
                .iter()
                .enumerate()
                .filter_map(|(i, k)| keys.contains(k).then_some(i))
                .collect::<Vec<_>>();
            for (i, key) in positions.into_iter().zip(keys) {
                order[i] = key;
            }
        }
        if order != tabs.keys() {
            *observed_previous.borrow_mut() = order.clone();
            batch(|| {
                for (i, key) in order.iter().enumerate() {
                    tabs.move_to(key, i);
                }
            });
        }
    });

    let last_selected = Rc::new(RefCell::new(None::<K>));
    day_reactive::bind(
        move || {
            (
                revision.get(),
                native(),
                tabs.keys()
                    .into_iter()
                    .map(|key| {
                        let text = title(&key);
                        (key, text)
                    })
                    .collect::<Vec<_>>(),
                tabs.selected(),
            )
        },
        move |(_, native, entries, selected)| {
            if !native {
                let windows = {
                    let mut pages = pages.borrow_mut();
                    pages
                        .iter_mut()
                        .filter_map(|page| {
                            let window = page.window.take()?;
                            page.chrome_window.set(page.home_window);
                            with_tree(|t| {
                                t.reparent(page.node, page.home);
                            });
                            Some(window)
                        })
                        .collect::<Vec<_>>()
                };
                for window in windows {
                    window.close();
                }
                previous.borrow_mut().clear();
                *last_selected.borrow_mut() = None;
                if let Some(initial) = &initial {
                    initial.set_visible(!hide_when_empty || !entries.is_empty());
                }
                return;
            }
            let existing = pages
                .borrow()
                .iter()
                .filter_map(|p| p.window.clone().map(|w| (p.key.clone(), w)))
                .collect::<Vec<_>>();
            let known = existing
                .iter()
                .map(|(_, w)| day_core::windows::window_node_id(w))
                .collect::<Vec<_>>();
            let target = existing
                .iter()
                .find(|(key, _)| Some(key) == selected.as_ref())
                .or_else(|| {
                    existing
                        .iter()
                        .find(|(key, _)| Some(key) == last_selected.borrow().as_ref())
                })
                .or_else(|| existing.first());
            let mut target_group = target.map(|(_, w)| w.tab_order()).unwrap_or_default();
            if target_group.is_empty()
                && let Some((_, w)) = target
            {
                target_group.push(day_core::windows::window_node_id(w));
            }
            let mut added = false;
            for (key, title) in entries {
                let node = pages
                    .borrow()
                    .iter()
                    .find(|p| &p.key == key && p.window.is_none())
                    .map(|p| p.node);
                if let Some(node) = node {
                    let size = initial
                        .as_ref()
                        .and_then(|w| w.frame())
                        .map(|r| r.size)
                        .unwrap_or(day_spec::Size::new(1000.0, 740.0));
                    let window = day_core::windows::open_window(
                        day_spec::WindowOptions {
                            title: title.clone(),
                            size,
                            start_hidden: true,
                            tabbing: day_spec::WindowTabbing::Group(group.clone()),
                            key: Some(format!("{group}:{}", key.key())),
                            ..Default::default()
                        },
                        move || MoveTab(node),
                    );
                    let close_observer = observe.clone();
                    window.on_close(move || close_observer());
                    let close_key = key.clone();
                    let close = on_close.clone();
                    let close_observer = observe.clone();
                    let close_alive = alive.clone();
                    window.on_close_request(move || {
                        if !close_alive.get() {
                            return;
                        }
                        close_observer();
                        if let Some(f) = &close {
                            f(close_key.clone());
                        } else {
                            tabs.close(&close_key);
                        }
                    });
                    let focus_key = key.clone();
                    let native_selection = last_selected.clone();
                    let focus_observer = observe.clone();
                    let focus_alive = alive.clone();
                    let focus_mode = focus_native.clone();
                    window.on_focus(move || {
                        // Returning to the composed strip closes the windows one by one, and
                        // the platform makes the next one key as each goes: that focus is
                        // not a choice, so it must not move the selection.
                        if !focus_alive.get() || !focus_mode() {
                            return;
                        }
                        focus_observer();
                        if tabs.contains(&focus_key) {
                            // Native activation already selected this window. Echoing it
                            // back through focus() can oscillate between queued Qt events.
                            *native_selection.borrow_mut() = Some(focus_key.clone());
                            tabs.select(&focus_key);
                        }
                    });
                    if let Some(page) = pages.borrow_mut().iter_mut().find(|p| &p.key == key) {
                        window.set_content_scope(page.scope);
                        let root =
                            day_core::id_to_rnode(day_core::windows::window_node_id(&window));
                        page.chrome_window.set(root);
                        page.window = Some(window);
                    }
                    added = true;
                }
            }
            let windows = entries
                .iter()
                .filter_map(|(key, title)| {
                    let window = pages
                        .borrow()
                        .iter()
                        .find(|p| &p.key == key)?
                        .window
                        .clone()?;
                    window.set_title(title);
                    Some((key.clone(), window))
                })
                .collect::<Vec<_>>();
            let order = windows
                .iter()
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            if added || *previous.borrow() != order {
                if added {
                    let group = windows
                        .iter()
                        .filter(|(_, w)| {
                            let id = day_core::windows::window_node_id(w);
                            !known.contains(&id) || target_group.contains(&id)
                        })
                        .map(|(_, w)| w.clone())
                        .collect::<Vec<_>>();
                    day_core::windows::group_windows(&group);
                } else {
                    let mut seen = Vec::new();
                    for (_, window) in &windows {
                        let id = day_core::windows::window_node_id(window);
                        if seen.contains(&id) {
                            continue;
                        }
                        let group = window.tab_order();
                        seen.extend(group.iter().copied());
                        let group = windows
                            .iter()
                            .filter(|(_, w)| group.contains(&day_core::windows::window_node_id(w)))
                            .map(|(_, w)| w.clone())
                            .collect::<Vec<_>>();
                        if group.len() > 1 {
                            day_core::windows::group_windows(&group);
                        }
                    }
                }
                *previous.borrow_mut() = order;
            }
            if let Some(initial) = &initial {
                initial.set_visible(!hide_when_empty && windows.is_empty());
            }
            if added || *last_selected.borrow() != *selected {
                *last_selected.borrow_mut() = selected.clone();
                if let Some((_, window)) = windows
                    .iter()
                    .find(|(key, _)| Some(key) == selected.as_ref())
                {
                    window.focus();
                }
            }
        },
    );
}

/// The document tabs' cases (docs/testing.md): the composed strip on every toolkit, the model's
/// corner cases, and the OS window groups where a toolkit has them.
#[cfg(feature = "conformance")]
pub(crate) mod conformance {
    use super::*;
    use crate::*;
    use day_core::conformance::{Case, Drive, FrameExpect, NativeExpect, TestResult};
    use day_spec::{Cap, kinds};

    /// The documents a case can open, in the order the controls open them.
    const KEYS: [&str; 3] = ["a", "b", "c"];

    /// How a fixture answers close requests.
    #[derive(Clone, Copy, PartialEq)]
    enum Closing {
        /// No handler: a request removes the key.
        Direct,
        /// A handler that vetoes the first request and accepts every later one.
        VetoFirst,
    }

    /// One host and the controls that drive its model from the app's side: open, close, move,
    /// cycle, rename, and the native-presentation preference. Every document is a label, a
    /// counter and a button that bumps it, so resident content is observable through its
    /// state. The strip takes the `docs` prefix; `selected` and `order` read the model back.
    fn fixture(closing: Closing, with_new: bool, native: bool) -> impl Piece {
        let tabs = TabSet::<String>::new();
        let pref = Signal::new(native);
        let renames = Signal::new(0u32);
        let vetoes = Signal::new(0u32);
        let closes = Signal::new(Vec::<String>::new());
        let next = Signal::new(0u32);
        let mut host = document_tabs(
            tabs,
            "New document",
            "Close document",
            move |key: &String| {
                let n = renames.get();
                if n == 0 {
                    format!("Doc {key}")
                } else {
                    format!("Doc {key} ({n})")
                }
            },
            |key: &String| {
                let count = Signal::new(0u32);
                let key = key.clone();
                column((
                    label(format!("Document {key}")).id(day_core::id!(doc, key.to_owned())),
                    button("Bump")
                        .action(move || count.update(|n| *n += 1))
                        .id(day_core::id!(document_bump, key.to_owned())),
                    label(move || count.get().to_string())
                        .id(day_core::id!(document_count, key.to_owned())),
                ))
                .spacing(4.0)
            },
        )
        .strip_id(day_core::id!(docs))
        .native(move || pref.get());
        if closing == Closing::VetoFirst {
            host = host.on_close(move |key: String| {
                if vetoes.get_untracked() == 0 {
                    vetoes.update(|n| *n += 1);
                } else {
                    closes.update(|c| c.push(key.clone()));
                    tabs.close(&key);
                }
            });
        }
        if with_new {
            host = host.on_new(move || {
                next.update(|n| *n += 1);
                tabs.open(format!("n{}", next.get_untracked()), true);
            });
        }
        let opens = KEYS.map(|key| {
            button(format!("Open {key}"))
                .action(move || tabs.open(key.to_string(), true))
                .id(day_core::id!(document_open, key.to_owned()))
        });
        let [open_a, open_b, open_c] = opens;
        column((
            row((
                open_a,
                open_b,
                open_c,
                button("Open c behind")
                    .action(move || tabs.open("c".to_string(), false))
                    .id(day_core::id!(open_c_bg)),
                button("Close b")
                    .action(move || {
                        tabs.close(&"b".to_string());
                    })
                    .id(day_core::id!(close_b)),
            ))
            .spacing(4.0),
            row((
                button("Next")
                    .action(move || tabs.select_next(false))
                    .id(day_core::id!(next)),
                button("Previous")
                    .action(move || tabs.select_next(true))
                    .id(day_core::id!(prev)),
                button("Last first")
                    .action(move || {
                        if let Some(last) = tabs.keys().last() {
                            tabs.move_to(last, 0);
                        }
                    })
                    .id(day_core::id!(move_last_first)),
                button("A far")
                    .action(move || {
                        tabs.move_to(&"a".to_string(), 99);
                    })
                    .id(day_core::id!(move_a_far)),
                button("Rename")
                    .action(move || renames.update(|n| *n += 1))
                    .id(day_core::id!(rename)),
                toggle(pref).id(day_core::id!(native_pref)),
            ))
            .spacing(4.0),
            row((
                label(move || format!("selected {}", tabs.selected().unwrap_or("none".into())))
                    .id(day_core::id!(selected)),
                label(move || format!("order {}", tabs.keys().join(","))).id(day_core::id!(order)),
                label(move || format!("vetoes {}", vetoes.get())).id(day_core::id!(vetoes)),
                label(move || format!("closes {}", closes.get().join(",")))
                    .id(day_core::id!(closes)),
            ))
            .spacing(8.0),
            host.grow(),
        ))
        .spacing(8.0)
    }

    /// The current document's tab is a selected button, enabled and focusable, not a disabled
    /// one; tapping another tab moves the selection, in Day and natively.
    #[day_macros::test(day_core)]
    fn document_tabs_strip_selects_and_stays_enabled() -> Case {
        Case::new()
            .proves(kinds::NAV)
            .proves_modifier("selected")
            .page(|| fixture(Closing::Direct, false, false))
            .drive(|d: Drive| async move {
                // A foreground open selects what it opens.
                d.tap("document_open:a").await?;
                d.tap("document_open:b").await?;
                d.assert_text("selected", "selected b").await?;
                d.assert_text("doc:b", "Document b").await?;
                d.assert_text("document_tab:docs%3Aa", "Doc a").await?;
                d.assert_enabled("document_tab:docs%3Aa", true).await?;
                d.assert_enabled("document_tab:docs%3Ab", true).await?;
                d.assert_on("document_tab:docs%3Ab", true).await?;
                d.assert_on("document_tab:docs%3Aa", false).await?;
                d.assert_native(
                    "document_tab:docs%3Ab",
                    NativeExpect {
                        checked: Some(true),
                        ..Default::default()
                    },
                )
                .await?;
                d.assert_native(
                    "document_tab:docs%3Aa",
                    NativeExpect {
                        checked: Some(false),
                        ..Default::default()
                    },
                )
                .await?;
                d.shot("two-tabs").await?;
                d.tap("document_tab:docs%3Aa").await?;
                d.assert_text("selected", "selected a").await?;
                d.assert_text("doc:a", "Document a").await?;
                d.assert_on("document_tab:docs%3Ab", false).await?;
                d.assert_on("document_tab:docs%3Aa", true).await?;
                d.assert_native(
                    "document_tab:docs%3Aa",
                    NativeExpect {
                        checked: Some(true),
                        enabled: Some(true),
                        ..Default::default()
                    },
                )
                .await?;
                d.assert_native(
                    "document_tab:docs%3Ab",
                    NativeExpect {
                        checked: Some(false),
                        enabled: Some(true),
                        ..Default::default()
                    },
                )
                .await?;
                // Re-activating the current tab is harmless.
                d.tap("document_tab:docs%3Aa").await?;
                d.assert_text("selected", "selected a").await?;
                d.assert_on("document_tab:docs%3Aa", true).await
            })
    }

    /// Closing the selected document selects the following neighbor, then the preceding last
    /// one, and the final close leaves no selection and no tabs.
    #[day_macros::test(day_core)]
    fn document_tabs_close_selects_neighbor() -> Case {
        Case::new()
            .proves(kinds::NAV)
            .page(|| fixture(Closing::Direct, false, false))
            .drive(|d: Drive| async move {
                d.tap("document_open:a").await?;
                d.tap("document_open:b").await?;
                d.tap("document_open:c").await?;
                d.tap("document_tab:docs%3Ab").await?;
                d.assert_text("order", "order a,b,c").await?;
                d.tap("document_close:docs%3Ab").await?;
                d.assert_text("selected", "selected c").await?;
                d.assert_text("order", "order a,c").await?;
                d.assert_missing("doc:b").await?;
                d.assert_missing("document_tab:docs%3Ab").await?;
                d.tap("document_close:docs%3Ac").await?;
                d.assert_text("selected", "selected a").await?;
                d.assert_text("order", "order a").await?;
                d.tap("document_close:docs%3Aa").await?;
                d.assert_text("selected", "selected none").await?;
                d.assert_text("order", "order ").await?;
                d.assert_missing("document_tab:docs%3Aa").await?;
                d.assert_missing("doc:a").await
            })
    }

    /// A close handler receives a request: keeping the key vetoes it, and only the handler's
    /// own `TabSet::close` removes the document.
    #[day_macros::test(day_core)]
    fn document_tabs_close_request_can_be_vetoed() -> Case {
        Case::new()
            .proves(kinds::NAV)
            .page(|| fixture(Closing::VetoFirst, false, false))
            .drive(|d: Drive| async move {
                d.tap("document_open:a").await?;
                d.tap("document_open:b").await?;
                d.tap("document_close:docs%3Aa").await?;
                d.assert_text("vetoes", "vetoes 1").await?;
                d.assert_text("order", "order a,b").await?;
                d.assert_text("doc:a", "Document a").await?;
                d.tap("document_close:docs%3Aa").await?;
                d.assert_text("closes", "closes a").await?;
                d.assert_text("order", "order b").await?;
                d.assert_text("selected", "selected b").await?;
                d.assert_missing("doc:a").await
            })
    }

    /// Reordering permutes the strip and the pages without rebuilding them: a counter bumped
    /// before the move still shows its count after it, and the first tab sits at the strip's
    /// leading edge. A move past the end lands on the last slot; the selection follows its key.
    #[day_macros::test(day_core)]
    fn document_tabs_reorder_keeps_resident_content() -> Case {
        Case::new()
            .proves(kinds::NAV)
            .page(|| fixture(Closing::Direct, false, false))
            .drive(|d: Drive| async move {
                d.tap("document_open:a").await?;
                d.tap("document_open:b").await?;
                d.tap("document_open:c").await?;
                d.tap("document_tab:docs%3Ab").await?;
                d.tap("document_bump:b").await?;
                d.tap("document_bump:b").await?;
                d.assert_text("document_count:b", "2").await?;
                d.tap("move_last_first").await?;
                d.assert_text("order", "order c,a,b").await?;
                d.assert_text("selected", "selected b").await?;
                d.assert_text("document_count:b", "2").await?;
                d.wait_idle().await?;
                d.assert_frame(
                    "document_tab:docs%3Ac",
                    FrameExpect {
                        x: Some(0.0),
                        relative_to: Some("docs".into()),
                        ..Default::default()
                    },
                )
                .await?;
                d.tap("move_a_far").await?;
                d.assert_text("order", "order c,b,a").await?;
                d.assert_text("selected", "selected b").await?;
                d.assert_text("document_count:b", "2").await?;
                d.tap("document_tab:docs%3Aa").await?;
                d.assert_text("selected", "selected a").await?;
                d.assert_text("doc:a", "Document a").await
            })
    }

    /// A background open adds the document, builds it, and leaves the selection alone; an
    /// open of a key already present changes nothing.
    #[day_macros::test(day_core)]
    fn document_tabs_background_open_keeps_selection() -> Case {
        Case::new()
            .proves(kinds::NAV)
            .page(|| fixture(Closing::Direct, false, false))
            .drive(|d: Drive| async move {
                d.tap("document_open:a").await?;
                d.tap("open_c_bg").await?;
                d.assert_text("order", "order a,c").await?;
                d.assert_text("selected", "selected a").await?;
                d.assert_text("doc:c", "Document c").await?;
                d.assert_on("document_tab:docs%3Ac", false).await?;
                d.tap("document_open:a").await?;
                d.tap("open_c_bg").await?;
                d.assert_text("order", "order a,c").await?;
                d.assert_text("selected", "selected a").await?;
                d.tap("next").await?;
                d.assert_text("selected", "selected c").await?;
                d.assert_on("document_tab:docs%3Ac", true).await
            })
    }

    /// `select_next` cycles through the collection in both directions and wraps at the ends.
    #[day_macros::test(day_core)]
    fn document_tabs_select_next_wraps() -> Case {
        Case::new()
            .proves(kinds::NAV)
            .page(|| fixture(Closing::Direct, false, false))
            .drive(|d: Drive| async move {
                d.tap("next").await?;
                d.assert_text("selected", "selected none").await?;
                d.tap("document_open:a").await?;
                d.tap("document_open:b").await?;
                d.tap("document_open:c").await?;
                d.tap("document_tab:docs%3Aa").await?;
                d.tap("next").await?;
                d.assert_text("selected", "selected b").await?;
                d.tap("next").await?;
                d.assert_text("selected", "selected c").await?;
                d.tap("next").await?;
                d.assert_text("selected", "selected a").await?;
                d.tap("prev").await?;
                d.assert_text("selected", "selected c").await?;
                d.assert_on("document_tab:docs%3Ac", true).await?;
                d.assert_on("document_tab:docs%3Aa", false).await
            })
    }

    /// The strip's add button and the app's `on_new` handler: the app allocates the key and
    /// opens it in front; without a handler the strip shows no add button.
    #[day_macros::test(day_core)]
    fn document_tabs_new_button_delegates_to_app() -> Case {
        Case::new()
            .proves(kinds::NAV)
            .page(|| fixture(Closing::Direct, true, false))
            .drive(|d: Drive| async move {
                d.tap("document_open:a").await?;
                d.assert_text("document_new:docs", "New document").await?;
                d.tap("document_new:docs").await?;
                d.assert_text("order", "order a,n1").await?;
                d.assert_text("selected", "selected n1").await?;
                d.assert_text("doc:n1", "Document n1").await?;
                d.tap("document_new:docs").await?;
                d.assert_text("order", "order a,n1,n2").await?;
                d.assert_text("selected", "selected n2").await?;
                d.tap("document_close:docs%3An2").await?;
                d.assert_text("selected", "selected n1").await
            })
    }

    /// Without an `on_new` handler the strip has no add button, and an empty collection shows
    /// an empty strip and no selection until a document opens.
    #[day_macros::test(day_core)]
    fn document_tabs_empty_host_shows_nothing() -> Case {
        Case::new()
            .proves(kinds::NAV)
            .page(|| fixture(Closing::Direct, false, false))
            .drive(|d: Drive| async move {
                d.assert_text("selected", "selected none").await?;
                d.assert_missing("document_tab:docs%3Aa").await?;
                d.assert_missing("document_new:docs").await?;
                d.tap("document_open:b").await?;
                d.assert_text("selected", "selected b").await?;
                d.assert_text("document_tab:docs%3Ab", "Doc b").await?;
                d.assert_frame(
                    "document_tab:docs%3Ab",
                    FrameExpect {
                        x: Some(0.0),
                        relative_to: Some("docs".into()),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// Titles follow the app's title function: a rename re-labels every tab in place, and the
    /// native widget shows the new title.
    #[day_macros::test(day_core)]
    fn document_tabs_titles_follow_model() -> Case {
        Case::new()
            .proves(kinds::NAV)
            .page(|| fixture(Closing::Direct, false, false))
            .drive(|d: Drive| async move {
                d.tap("document_open:a").await?;
                d.tap("document_open:b").await?;
                d.assert_text("document_tab:docs%3Ab", "Doc b").await?;
                d.tap("rename").await?;
                d.assert_text("document_tab:docs%3Aa", "Doc a (1)").await?;
                d.assert_text("document_tab:docs%3Ab", "Doc b (1)").await?;
                d.assert_native(
                    "document_tab:docs%3Ab",
                    NativeExpect {
                        text: Some("Doc b (1)".into()),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// Switching between native and composed presentation keeps every document's content and
    /// state, repeatedly; where a toolkit has no native presentation the switch is a no-op and
    /// the same invariants hold.
    #[day_macros::test(day_core)]
    fn document_tabs_presentation_switch_keeps_content() -> Case {
        Case::new()
            .proves(kinds::NAV)
            .proves_duty("native_document_tabs")
            .timeout(90.0)
            .page(|| fixture(Closing::Direct, false, false))
            .drive(|d: Drive| async move {
                d.tap("document_open:a").await?;
                d.tap("document_open:b").await?;
                d.tap("document_tab:docs%3Aa").await?;
                d.tap("document_bump:a").await?;
                let mut count = 1;
                d.assert_text("document_count:a", &count.to_string())
                    .await?;
                for _ in 0..2 {
                    d.toggle("native_pref", true).await?;
                    d.wait_idle().await?;
                    d.assert_text("document_count:a", &count.to_string())
                        .await?;
                    d.assert_text("selected", "selected a").await?;
                    d.assert_text("doc:b", "Document b").await?;
                    // A bump while presented natively lands on the same counter.
                    d.tap("document_bump:a").await?;
                    count += 1;
                    d.assert_text("document_count:a", &count.to_string())
                        .await?;
                    d.toggle("native_pref", false).await?;
                    d.wait_idle().await?;
                    d.assert_text("document_count:a", &count.to_string())
                        .await?;
                    d.assert_text("doc:a", "Document a").await?;
                    d.assert_text("document_tab:docs%3Aa", "Doc a").await?;
                    d.assert_on("document_tab:docs%3Aa", true).await?;
                }
                d.assert_text("document_count:a", "3").await?;
                d.assert_text("order", "order a,b").await?;
                d.tap("document_tab:docs%3Ab").await?;
                d.assert_text("selected", "selected b").await
            })
    }

    /// Where the toolkit has OS window tabs, native presentation opens one window per document
    /// in the host's group, a model close closes its window, and composed presentation takes
    /// the documents back and closes every window.
    #[day_macros::test(day_core)]
    fn document_tabs_window_groups_follow_model() -> Case {
        Case::new()
            .proves(kinds::NAV)
            .proves_duty("native_window_tabs")
            .proves_duty("group_windows")
            .proves_duty("window_tab_order")
            .requires(Cap::WindowTabbing)
            .timeout(90.0)
            .page(|| fixture(Closing::Direct, false, true))
            .drive(|d: Drive| async move {
                // The previous case's windows may still be closing: a native close reports
                // back through the platform's own loop. The baseline is the count once it has
                // held still for a few turns.
                let mut base = day_core::windows::open_window_count();
                let mut held = 0;
                for _ in 0..60 {
                    d.pause(0.05).await?;
                    let now = day_core::windows::open_window_count();
                    if now == base {
                        held += 1;
                        if held >= 6 {
                            break;
                        }
                    } else {
                        base = now;
                        held = 0;
                    }
                }
                // Likewise each count below is given a few turns to settle before it is judged.
                async fn windows(d: &Drive, what: &str, base: usize, want: usize) -> TestResult {
                    let mut got = day_core::windows::open_window_count();
                    for _ in 0..40 {
                        if got == want {
                            break;
                        }
                        d.pause(0.05).await?;
                        got = day_core::windows::open_window_count();
                    }
                    d.check(
                        got == want,
                        &format!("{what}: {got} windows open, expected {want} ({base} before)"),
                    )
                }
                d.tap("document_open:a").await?;
                d.tap("document_open:b").await?;
                d.wait_idle().await?;
                d.assert_text("selected", "selected b").await?;
                windows(&d, "after opening a and b", base, base + 2).await?;
                d.tap("document_bump:b").await?;
                d.assert_text("document_count:b", "1").await?;
                d.tap("close_b").await?;
                d.wait_idle().await?;
                d.assert_text("order", "order a").await?;
                d.assert_missing("doc:b").await?;
                windows(&d, "after closing b", base, base + 1).await?;
                d.tap("document_open:c").await?;
                d.wait_idle().await?;
                d.assert_text("selected", "selected c").await?;
                windows(&d, "after opening c", base, base + 2).await?;
                d.toggle("native_pref", false).await?;
                d.wait_idle().await?;
                windows(&d, "after returning to the composed strip", base, base).await?;
                d.assert_text("doc:a", "Document a").await?;
                d.assert_text("doc:c", "Document c").await?;
                d.assert_text("document_tab:docs%3Ac", "Doc c").await?;
                d.assert_on("document_tab:docs%3Ac", true).await
            })
    }

    /// App-owned chrome: `.chrome` replaces the strip and `TabActions` carries close, new and
    /// the model; `.layout` places it after the content.
    #[day_macros::test(day_core)]
    fn document_tabs_custom_chrome_uses_actions() -> Case {
        Case::new()
            .proves(kinds::NAV)
            .page(|| {
                let tabs = TabSet::<String>::new();
                let next = Signal::new(0u32);
                let host = document_tabs(
                    tabs,
                    "New",
                    "Close",
                    |key: &String| format!("Doc {key}"),
                    |key: &String| {
                        label(format!("Document {key}")).id(day_core::id!(doc, key.to_owned()))
                    },
                )
                .native(|| false)
                .on_new(move || {
                    next.update(|n| *n += 1);
                    tabs.open(format!("n{}", next.get_untracked()), true);
                })
                .chrome(|actions: TabActions<String>| {
                    let tabs = actions.tabs;
                    let closer = actions.clone();
                    row((
                        each(
                            items(move || tabs.keys(), |key: &String| key.clone()),
                            move |slot| {
                                let key = slot.get();
                                let select_key = key.clone();
                                let selected_key = key.clone();
                                let close_key = key.clone();
                                let closer = closer.clone();
                                row((
                                    button(format!("Doc {key}"))
                                        .selected(move || {
                                            tabs.selected().as_ref() == Some(&selected_key)
                                        })
                                        .action(move || {
                                            tabs.select(&select_key);
                                        })
                                        .id(day_core::id!(custom_tab, key.to_owned())),
                                    button("x")
                                        .action(move || closer.close(close_key.clone()))
                                        .id(day_core::id!(custom_close, key.to_owned())),
                                ))
                            },
                        ),
                        button("+")
                            .action(move || actions.new_tab())
                            .id(day_core::id!(custom_new)),
                    ))
                    .id(day_core::id!(custom_bar))
                })
                .layout(|bar, content| column((content.grow(), bar)));
                column((
                    label(move || format!("selected {}", tabs.selected().unwrap_or("none".into())))
                        .id(day_core::id!(selected)),
                    host.grow(),
                ))
            })
            .drive(|d: Drive| async move {
                d.assert_missing("docs").await?;
                d.tap("custom_new").await?;
                d.tap("custom_new").await?;
                d.assert_text("selected", "selected n2").await?;
                d.assert_text("doc:n1", "Document n1").await?;
                d.assert_on("custom_tab:n2", true).await?;
                d.assert_on("custom_tab:n1", false).await?;
                d.tap("custom_tab:n1").await?;
                d.assert_text("selected", "selected n1").await?;
                d.assert_on("custom_tab:n1", true).await?;
                d.tap("custom_close:n1").await?;
                d.assert_text("selected", "selected n2").await?;
                d.assert_missing("doc:n1").await?;
                d.wait_idle().await?;
                // The bar sits below the content (`.layout`), at the host's trailing edge.
                d.assert_frame(
                    "doc:n2",
                    FrameExpect {
                        y: Some(0.0),
                        relative_to: Some("selected".into()),
                        tolerance: Some(40.0),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// The model alone: an open of a present key is one tab, unknown keys are refused, a move
    /// past the end clamps, and closing picks the neighbor the contract names.
    #[day_macros::test(day_core)]
    fn document_tabs_model_contract() -> Case {
        Case::headless().run(|t: Drive| async move {
            let scope = day_reactive::Scope::detached();
            let result = scope.enter(|| {
                let tabs = TabSet::<u32>::new();
                t.check_eq(tabs.selected(), None)?;
                t.check_eq(tabs.keys(), Vec::<u32>::new())?;
                tabs.open(1, false);
                t.check_eq(tabs.selected(), Some(1))?;
                tabs.open(2, false);
                tabs.open(1, true);
                t.check_eq(tabs.keys(), vec![1, 2])?;
                t.check_eq(tabs.selected(), Some(1))?;
                t.check(!tabs.select(&9), "selecting an unknown key is refused")?;
                t.check(!tabs.move_to(&9, 0), "moving an unknown key is refused")?;
                t.check(!tabs.close(&9), "closing an unknown key is refused")?;
                t.check(tabs.move_to(&1, 99), "a move past the end is accepted")?;
                t.check_eq(tabs.keys(), vec![2, 1])?;
                t.check_eq(tabs.selected(), Some(1))?;
                tabs.open(3, false);
                t.check_eq(tabs.keys(), vec![2, 1, 3])?;
                t.check(tabs.close(&1), "closing the selected key")?;
                t.check_eq(tabs.selected(), Some(3))?;
                t.check(tabs.close(&3), "closing the selected last key")?;
                t.check_eq(tabs.selected(), Some(2))?;
                tabs.select_next(false);
                t.check_eq(tabs.selected(), Some(2))?;
                t.check(tabs.close(&2), "closing the final key")?;
                t.check_eq(tabs.selected(), None)?;
                tabs.select_next(true);
                t.check_eq(tabs.selected(), None)
            });
            scope.dispose();
            result
        })
    }
}

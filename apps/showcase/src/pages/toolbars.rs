//! Window toolbars (docs/toolbars.md). The demonstration is the MAIN WINDOW'S OWN toolbar —
//! a toolbar is window chrome, so there is nowhere on a page to put one. This page installs
//! that bar, shows what each item is doing live, and drives the whole API from the content:
//! add and remove an item, enable and disable one, and read the two-way bindings.
//!
//! Where the toolkit has no toolbar (`Cap::Toolbar` is `Unsupported` — the phones, the web)
//! nothing installs, and the page says so rather than drawing an imitation.

use day::prelude::*;
use std::cell::OnceCell;

use crate::widgets::page;

thread_local! {
    /// App-global, not page-scoped: the toolbar is installed once for the window and outlives
    /// every visit to this page, so its bindings must not die when the page pops.
    static STATE: OnceCell<ToolbarDemo> = const { OnceCell::new() };
}

/// The signals the main window's toolbar is bound to.
#[derive(Clone, Copy)]
struct ToolbarDemo {
    query: Signal<String>,
    starred: Signal<bool>,
    /// How many times a plain toolbar button has been pressed.
    presses: Signal<i64>,
    /// Whether the optional item is in the bar — the add/remove demonstration.
    extra: Signal<bool>,
    /// Whether the refresh item is enabled.
    refresh_enabled: Signal<bool>,
    /// The last thing the toolbar did, in words.
    last: Signal<String>,
}

/// The toolbar's search text, which also filters the sidebar (`crate::destinations`). Public to
/// the crate because the shell reads it while building the nav, before this page ever opens.
pub(crate) fn search_query() -> Signal<String> {
    state().query
}

fn state() -> ToolbarDemo {
    STATE.with(|c| {
        *c.get_or_init(|| ToolbarDemo {
            query: Signal::global(String::new()),
            starred: Signal::global(false),
            presses: Signal::global(0),
            extra: Signal::global(false),
            refresh_enabled: Signal::global(true),
            last: Signal::global(String::new()),
        })
    })
}

/// Does this toolkit have a real toolbar?
fn available() -> bool {
    capability(Cap::Toolbar) == Support::Native
}

/// Install the main window's toolbar. Called from `root` for the primary window and from the
/// New Window builder for each secondary, so every window gets its own bar.
pub(crate) fn install() {
    if !available() {
        return;
    }
    let s = state();

    // Reactive: the builder reads `extra`, so ticking that switch adds or removes the item —
    // the add/remove API is just a different list. It also re-lowers on a language change,
    // which is why the labels are `res::str` calls rather than captured Strings.
    toolbar_reactive(move || {
        let mut items = vec![
            // A plain command.
            toolbar_button("tb-new", crate::res::str::toolbar_new())
                .icon(Symbol::New)
                .action(move || note(s, crate::res::str::toolbar_last_new())),
            // ...and one that can be disabled from the page, to show the targeted patch: only
            // this item changes, so a search in progress is undisturbed.
            toolbar_button("tb-refresh", crate::res::str::toolbar_refresh())
                .icon(Symbol::Refresh)
                .action(move || note(s, crate::res::str::toolbar_last_refresh()))
                .enabled_when(move || s.refresh_enabled.get()),
            toolbar_separator(),
            // Two-way: the page's switch and the toolbar button show one state.
            toolbar_toggle("tb-star", crate::res::str::toolbar_star(), s.starred)
                .icon(Symbol::Star)
                .action(move || note(s, crate::res::str::toolbar_last_star())),
            // A pull-down, built from the same entries the menu bar takes.
            toolbar_menu(
                "tb-menu",
                crate::res::str::toolbar_menu(),
                vec![
                    menu_item(crate::res::str::toolbar_menu_first().format())
                        .action(move || note(s, crate::res::str::toolbar_last_menu_first())),
                    menu_item(crate::res::str::toolbar_menu_second().format())
                        .action(move || note(s, crate::res::str::toolbar_last_menu_second())),
                    menu_separator(),
                    menu_role(MenuRole::Copy),
                ],
            )
            .icon(Symbol::More),
        ];
        if s.extra.get() {
            items.push(
                toolbar_button("tb-extra", crate::res::str::toolbar_extra())
                    .icon(Symbol::Bookmark)
                    .action(move || note(s, crate::res::str::toolbar_last_extra())),
            );
        }
        items.push(toolbar_flexible_space());
        items.push(
            toolbar_search("tb-search", s.query)
                .placeholder(crate::res::str::toolbar_search_placeholder()),
        );
        items
    });
}

/// Record what the toolbar just did, and count the presses.
fn note(s: ToolbarDemo, what: day::LocalizedText) {
    s.presses.set(s.presses.get_untracked() + 1);
    s.last.set(what.format());
}

pub(crate) fn toolbars_page() -> AnyPiece {
    page(
        crate::res::str::nav_toolbars(),
        "toolbars-title",
        Some(crate::res::str::toolbars_caption()),
        form((readout_section(), controls_section(), vocabulary_section())).any(),
    )
}

/// What the bar is doing right now — the two-way bindings, read from the page.
fn readout_section() -> impl Piece {
    let s = state();
    section((
        // Where there is no toolbar the rest of the page has nothing to report on.
        when(
            || !available(),
            || {
                label(crate::res::str::toolbar_unsupported())
                    .color(crate::palette::SLATE)
                    .id("toolbar-unsupported")
            },
        ),
        labeled(
            crate::res::str::toolbar_query_label(),
            label(move || {
                let q = s.query.get();
                if q.is_empty() {
                    crate::res::str::toolbar_query_empty().format()
                } else {
                    q
                }
            })
            .id("toolbar-query"),
        ),
        labeled(
            crate::res::str::toolbar_star_label(),
            label(move || {
                if s.starred.get() {
                    crate::res::str::toolbar_on().format()
                } else {
                    crate::res::str::toolbar_off().format()
                }
            })
            .id("toolbar-star-state"),
        ),
        labeled(
            crate::res::str::toolbar_presses_label(),
            label(move || crate::res::str::toolbar_presses(s.presses.get() as f64).format())
                .id("toolbar-presses"),
        ),
        labeled(
            crate::res::str::toolbar_last_label(),
            label(move || {
                let l = s.last.get();
                if l.is_empty() {
                    crate::res::str::toolbar_last_none().format()
                } else {
                    l
                }
            })
            .id("toolbar-last"),
        ),
    ))
    .title(crate::res::str::toolbar_readout_title())
}

/// Driving the bar from the content: add/remove an item, disable one, write a bound signal.
fn controls_section() -> impl Piece {
    let s = state();
    section((
        labeled(
            crate::res::str::toolbar_extra_label(),
            toggle(s.extra).id("toolbar-extra-switch"),
        ),
        labeled(
            crate::res::str::toolbar_enabled_label(),
            toggle(s.refresh_enabled).id("toolbar-enabled-switch"),
        ),
        labeled(
            crate::res::str::toolbar_star_label(),
            toggle(s.starred).id("toolbar-star-switch"),
        ),
        row((
            button(crate::res::str::toolbar_clear_search())
                .action(move || s.query.set(String::new()))
                .id("toolbar-clear-search"),
            button(crate::res::str::toolbar_seed_search())
                .action(move || s.query.set(crate::res::str::toolbar_seed_text().format()))
                .id("toolbar-seed-search"),
        ))
        .spacing(8.0),
    ))
    .title(crate::res::str::toolbar_controls_title())
}

/// What is in the bar, so the page names each kind the vocabulary offers.
fn vocabulary_section() -> impl Piece {
    section((
        labeled(
            crate::res::str::toolbar_kind_button(),
            label(crate::res::str::toolbar_kind_button_note()),
        ),
        labeled(
            crate::res::str::toolbar_kind_toggle(),
            label(crate::res::str::toolbar_kind_toggle_note()),
        ),
        labeled(
            crate::res::str::toolbar_kind_menu(),
            label(crate::res::str::toolbar_kind_menu_note()),
        ),
        labeled(
            crate::res::str::toolbar_kind_search(),
            label(crate::res::str::toolbar_kind_search_note()),
        ),
        labeled(
            crate::res::str::toolbar_kind_space(),
            label(crate::res::str::toolbar_kind_space_note()),
        ),
    ))
    .title(crate::res::str::toolbar_vocabulary_title())
}

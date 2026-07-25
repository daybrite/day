//! The ready-made consent surface (feature `ui`): a banner an app embeds to disclose a pending
//! crash report and let the user view, send, or discard it. Everything the user reads is localized
//! (en/fr/ar/zh-CN, [`crate::i18n`]); the destination disclosure comes from the configured
//! [`Reporter`]'s `describe`. Nothing is uploaded until the user taps Send.

use day_pieces::prelude::*;

use crate::ReportMeta;
use crate::i18n::t;

/// Theme-neutral card surface (translucent mid-grey reads on both light and dark themes, so the
/// default label colors stay legible — the showcase/daylite idiom; see day-ui-visual-checks).
const CARD: Color = Color::rgba(0.5, 0.5, 0.55, 0.16);

fn card(content: AnyPiece) -> AnyPiece {
    content.padding(14.0).background(CARD).corner_radius(12.0)
}

/// A banner disclosing the newest pending crash report, with View / Send / Discard. Renders
/// nothing when there is no pending report. Embed it near the top of a screen; it is reactive —
/// it appears when a crash is pending and disappears once the user sends or discards it.
pub fn consent_banner() -> AnyPiece {
    let pending = crate::pending();
    let expanded = Signal::new(false);
    let status = Signal::new(String::new());
    when(
        move || !pending.get().is_empty(),
        move || banner_body(pending, expanded, status),
    )
    .any()
}

fn banner_body(
    pending: Signal<Vec<ReportMeta>>,
    expanded: Signal<bool>,
    status: Signal<String>,
) -> AnyPiece {
    // The newest pending report drives the banner.
    let Some(meta) = pending.get().into_iter().next() else {
        return spacer().any();
    };

    let disclosure = crate::state()
        .and_then(|s| s.reporter.as_ref().map(|r| r.describe()))
        .unwrap_or_default();

    let view_meta = meta.clone();
    let toggle = move || expanded.set(!expanded.get_untracked());

    let send_meta = meta.clone();
    // Localize on the MAIN thread now (i18n reads a thread-local + the live locale signal, neither
    // valid on the transport's worker thread); the completion callback just picks a precomputed
    // string and writes it through a Setter (the cross-thread-safe signal write).
    let (sending, sent, failed) = (t("crash-sending"), t("crash-sent"), t("crash-failed"));
    let status_setter = status.setter();
    let on_send = move || {
        status.set(sending.clone());
        let (sent, failed) = (sent.clone(), failed.clone());
        crate::send(&send_meta, move |res| {
            status_setter.set(if res.is_ok() { sent } else { failed });
        });
    };

    let discard_meta = meta.clone();
    let on_discard = move || crate::discard(&discard_meta);

    let report_body = move || {
        // Read-only display of the full report; a scrolled label so nothing is editable.
        let text = crate::report_text(&view_meta);
        scroll(label(text).font(Font::Caption)).any()
    };

    card(
        column((
            label(t("crash-title"))
                .font(Font::Headline)
                .id("dbreak-title"),
            label(t("crash-body")).font(Font::Footnote),
            when(
                move || !status.get().is_empty(),
                move || label(status.get()).font(Font::Callout).id("dbreak-status"),
            ),
            when(move || expanded.get(), report_body)
                .any()
                .id("dbreak-report"),
            row((
                button(t("crash-view")).action(toggle).id("dbreak-view"),
                button(t("crash-send")).action(on_send).id("dbreak-send"),
                button(t("crash-discard"))
                    .action(on_discard)
                    .id("dbreak-discard"),
            ))
            .spacing(8.0),
            when(
                {
                    let has = !disclosure.is_empty();
                    move || has
                },
                move || label(disclosure.clone()).font(Font::Caption2),
            ),
        ))
        .spacing(8.0)
        .any(),
    )
    .id("dbreak-banner")
    .any()
}

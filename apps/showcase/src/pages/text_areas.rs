use day::prelude::*;

use crate::widgets::page;

// Demo filler for the seed buttons — illustrative content, not localized.
const SHORT: &str =
    "A short note. Edit it, or seed longer or structured text with the buttons below.";
const LONG: &str = "\
Day lays out native widgets from a declarative description — you write the shape of the UI once and \
each platform's real toolkit draws it. There is no webview and no custom renderer; a button is the \
platform's button, a text area is the platform's editor.

State is reactive: a signal drives the tree, and only the widgets that depend on a changed value are \
touched. Text you type here flows back into the bound signal, and programmatic writes flow back out — \
a controlled input, in both directions.

Scroll this text if it outgrows the editor's height band. The band grows with the content between a \
minimum and a maximum number of lines, then the editor scrolls internally.";
const MARKDOWN: &str = "\
# Text areas

A `text_area` is a native multi-line editor with three controllable attributes:

- **editable** — read-only when off
- **selectable** — copyable even when read-only
- **spell-check** — the red squiggles, where the toolkit has them

See the [documentation](https://daybrite.dev/docs/textarea) for the per-toolkit support matrix.

    // a fenced code sample renders as plain text here
    text_area(content).editable(false).spellcheck(false)";

pub(crate) fn text_areas_page() -> AnyPiece {
    // What the running toolkit can actually honor — an unsupported attribute grays out its toggle.
    // `Emulated` counts as honored: the attribute behaves, it just isn't one native property behind
    // the scenes (XAML has no TextBox selection flag, so it collapses selections as they form).
    let cap_editable = capability(Cap::TextEditable) != Support::Unsupported;
    let cap_selectable = capability(Cap::TextSelectable) != Support::Unsupported;
    let cap_spellcheck = capability(Cap::TextSpellCheck) != Support::Unsupported;

    let content = Signal::new(SHORT.to_string());
    // The three attributes, each bound to a toggle. Live: flipping a toggle patches the editor.
    // Spell-check starts off where the toolkit has none (Qt/GTK), so the disabled toggle reads
    // "off" rather than falsely showing an active checker.
    let editable = Signal::new(true);
    let selectable = Signal::new(true);
    let spellcheck = Signal::new(cap_spellcheck);

    // Editing implies selection — no backend can present editable-but-unselectable text (on Android
    // an editable field is always selectable, so read-only is the only way to stop selection). So
    // turning Selectable off also turns Editable off, and the Editable toggle disables while it is.
    Effect::new(move || {
        if !selectable.get() {
            editable.set(false);
        }
    });

    // A fixed five-line editor: it scrolls internally, so seeding short vs. long text never changes
    // its height.
    let editor = section((text_area(content)
        .editable(editable)
        .selectable(selectable)
        .spellcheck(spellcheck)
        .min_lines(5)
        .max_lines(5)
        .id("textareas-editor"),))
    .title(crate::res::str::textareas_editor_section());

    let seed = section((row((
        button(crate::res::str::textareas_seed_short())
            .action(move || content.set(SHORT.into()))
            .id("ta-seed-short"),
        button(crate::res::str::textareas_seed_long())
            .action(move || content.set(LONG.into()))
            .id("ta-seed-long"),
        button(crate::res::str::textareas_seed_markdown())
            .action(move || content.set(MARKDOWN.into()))
            .id("ta-seed-markdown"),
    ))
    .spacing(8.0),))
    .title(crate::res::str::textareas_seed_section());

    // Each toggle is disabled where the running toolkit can't honor the attribute (GTK can't stop
    // selection; GTK/Qt/ArkUI have no spell-check) — the `capability()` gating idiom. Editable also
    // disables whenever Selectable is off, since editing without selection is not a valid state.
    let attrs = section((
        labeled(
            crate::res::str::textareas_editable(),
            toggle(editable)
                .enabled(move || cap_editable && selectable.get())
                .id("ta-editable"),
        ),
        labeled(
            crate::res::str::textareas_selectable(),
            toggle(selectable)
                .enabled(cap_selectable)
                .id("ta-selectable"),
        ),
        labeled(
            crate::res::str::textareas_spellcheck(),
            toggle(spellcheck)
                .enabled(cap_spellcheck)
                .id("ta-spellcheck"),
        ),
    ))
    .title(crate::res::str::textareas_attrs_section());

    page(
        crate::res::str::nav_textareas(),
        "textareas-title",
        Some(crate::res::str::textareas_caption()),
        form((editor, seed, attrs)).any(),
    )
}

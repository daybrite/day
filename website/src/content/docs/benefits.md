---
title: Is Day a good fit?
description: "Decide whether native controls, a shared codebase, and Day’s workflow suit your app."
order: 2
section: Product
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Day is a good candidate when you want to write your app in Rust, share its interface across
platforms, and use the controls supplied by each platform’s toolkit. The tradeoff is that
your interface will vary with the platform, and you will still need to test those differences.

## For tools people use every day

A settings screen, a document window, or a list of records usually benefits from familiar
controls: text fields that select text as expected, menus with keyboard shortcuts, and
scrolling that feels like the rest of the system. Day lets you describe these in one
codebase. The toolkit supplies their native appearance and behavior.

If your design depends on identical controls and elaborate custom animation everywhere,
check [what Day can style](/docs/explanation/styling) before going further. A framework with its own
renderer may give you more control over that kind of interface.

## Try your hardest requirement first

Day’s component library and ecosystem are still developing. Before committing a project,
build a small example of the feature you are least sure about: perhaps a particular input,
a large list, a platform service, or a multi-window workflow.

Check the [support tiers](/docs/reference/platforms#support-tiers) for your targets. A target’s tier
describes testing and maintenance; it does not promise that every API is available. Run the
example on the platforms you need and inspect the result with their accessibility tools.

If a control is missing, you can [compose existing pieces or wrap a native widget](/contributing/extending).
A service without a UI belongs in a [part](/docs/reference/parts). Both approaches keep the integration
in a separate crate, but a native integration still needs implementation and testing on each
target you support.

<span id="what-you-give-up"></span>

## Allow for the development workflow

Day uses an incremental rebuild and relaunch after code changes. It does not preserve a
running app’s state through hot reload. A [dayscript walkthrough](/docs/how-to/dayscript) can take
you back to the screen you are working on, though the app still restarts.

The interface and application logic are Rust. If your team is new to Rust, learning the
language is part of the project. UI state stays on the main thread; background work returns
results through the APIs described in [Reactivity](/docs/explanation/reactivity#threading-and-async-work).

## For small teams serving several platforms

Beyond the controls, Day provides [localization](/docs/how-to/localization),
[accessibility annotations](/docs/how-to/accessibility), and [automated UI walkthroughs](/docs/how-to/dayscript).
The CLI handles project creation, toolchain checks, builds, and [packaging](/docs/how-to/packaging).
These tools share the app’s configuration, so you can reuse a walkthrough for regression
checks and screenshots in different languages.

They do not replace platform testing. Focus order, text layout, system keyboards, permission
dialogs, and signing requirements still need attention on each target.

## Common questions

### How does Day render the interface?

Desktop and mobile backends create controls through their selected native UI toolkits.
You can also embed a web view for web content or build for the experimental browser target.

### Can I keep an existing Rust library?

Yes, when the library and its dependencies support your chosen targets. Keep it in a Cargo
crate and connect its results to UI state. Use the [async guidance](/docs/explanation/reactivity#threading-and-async-work)
for work that would otherwise block interaction.

### Does a native control make the app accessible?

It provides a starting point, not a certification. Labels, grouping, focus, and backend
capabilities still matter. Use Day’s [accessibility annotations](/docs/how-to/accessibility)
and test with the assistive technologies your audience uses.

### Can one catalog serve every build?

Yes. Fluent catalogs generate Rust accessors that the shared UI uses on each target.
Message arguments are checked against the generated API. Translators can express plural forms
in the catalog; [the localization guide](/docs/how-to/localization) includes a complete example.

### Does Day submit the app to the stores?

The CLI builds packages and prepares store-listing metadata. Submission depends on separate
store tools and credentials. See [packaging and distribution](/docs/how-to/packaging) for the
boundary between a built artifact and a distributable release.

## Make the decision with a small app

Follow [Getting started](/docs/tutorials/first-app), then replace the sample screen with one from
your own design. That will tell you more about Day’s fit than a feature checklist: how much
code you share, where the platforms differ, and which integrations you would need to maintain.

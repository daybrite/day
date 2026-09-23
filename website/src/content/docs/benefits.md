---
title: Is Day a good fit?
description: "Decide whether native controls, a shared codebase, and Day’s workflow suit your app."
order: 2
section: Start here
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Day is a good candidate when you want to write your app in Rust, share its interface across
platforms, and use the controls supplied by each platform’s toolkit. The tradeoff is that
your interface will vary with the platform, and you will still need to test those differences.

## Start with the interface you want

A settings screen, a document window, or a list of records usually benefits from familiar
controls: text fields that select text as expected, menus with keyboard shortcuts, and
scrolling that feels like the rest of the system. Day lets you describe these in one
codebase. The toolkit supplies their native appearance and behavior.

If your design depends on identical controls and elaborate custom animation everywhere,
check [what Day can style](/docs/styling) before going further. A framework with its own
renderer may give you more control over that kind of interface.

## Try your hardest requirement first

Day’s component library and ecosystem are still developing. Before committing a project,
build a small example of the feature you are least sure about: perhaps a particular input,
a large list, a platform service, or a multi-window workflow.

Check the [support tiers](/docs/platforms#support-tiers) for your targets. A target’s tier
describes testing and maintenance; it does not promise that every API is available. Run the
example on the platforms you need and inspect the result with their accessibility tools.

If a control is missing, you can [compose existing pieces or wrap a native widget](/docs/extending).
A service without a UI belongs in a [part](/docs/parts). Both approaches keep the integration
in a separate crate, but a native integration still needs implementation and testing on each
target you support.

<span id="what-you-give-up"></span>

## Allow for the development workflow

Day uses an incremental rebuild and relaunch after code changes. It does not preserve a
running app’s state through hot reload. A [dayscript walkthrough](/docs/dayscript) can take
you back to the screen you are working on, though the app still restarts.

The interface and application logic are Rust. If your team is new to Rust, learning the
language is part of the project. UI state stays on the main thread; background work returns
results through the APIs described in [Reactivity](/docs/reactivity#threads).

## The work Day helps with

Beyond the controls, Day provides [localization](/docs/localization),
[accessibility annotations](/docs/accessibility), and [automated UI walkthroughs](/docs/dayscript).
The CLI handles project creation, toolchain checks, builds, and [packaging](/docs/packaging).
These tools share the app’s configuration, so you can reuse a walkthrough for regression
checks and screenshots in different languages.

They do not replace platform testing. Focus order, text layout, system keyboards, permission
dialogs, and signing requirements still need attention on each target.

## Make the decision with a small app

Follow [Getting started](/docs/getting-started), then replace the sample screen with one from
your own design. That will tell you more about Day’s fit than a feature checklist: how much
code you share, where the platforms differ, and which integrations you would need to maintain.

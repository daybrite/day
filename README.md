# Day

Day is a Rust framework for building desktop and mobile apps with a platform-native user interface. Day apps are truly native apps, using the same underlying toolkits as the first-party apps created by the OS vendor. That is to say: on macOS it uses AppKit, UIKit on iOS, GTK or Qt on Linux, WinUI on Windows, and Material views on Android.

The goal of the Day project is to facilitate the creation of apps that are indistinguishable — both in look and in feel — from the platform-native apps that come pre-installed with the operating system. In addition to being familiar to the user and efficient (both in terms of memory, application size, and battery use), Day apps also benefit from automatic platform integration and accessibility support.

Read the [introductionary post](https://daybrite.dev/blog/new-day/) for more details about the motivation behind Day, and where it fits into the broader Rust ecosystem.

[Documentation](https://daybrite.dev/docs/) · [App Gallery](https://daybrite.dev/gallery/) · [Rust API](https://docs.rs/day-app)

## Run your first app

Day requires Rust 1.100 or newer, which can be installed with [`rustup`](https://rustup.rs). The [system requirements](https://daybrite.dev/docs/reference/system-requirements) vary by both host and target platforms.

```sh
cargo install day-cli
day doctor
day new app hello-day
cd hello-day
day launch
```

The starter app opens in a new window with a list you can add to and edit. Its navigation is in `src/lib.rs` and its screens are in `src/pages.rs`. You can also browse the source for a freshly-created app in the [Day-Rise](https://github.com/daybrite/Day-Rise) repository.

## API tour

Day provides a reactive and declarative API atop the underlying imperitive toolkit, which enables the same code to define the user interface across mobile and desktop platforms. It is conceptually similar to SwiftUI for iOS and macOS or Jetpack Compose for Android.

In the following example, a signal holds a "count" value and each press updates the signal and the label displays its new value. For localization, Day generates the text accessors from two messages in `resource/locales/en/app.ftl`.

```fluent
counter_value = Count: { $count }
add_one = Add one
```

```rust
pub fn root() -> impl Piece {
    let count = Signal::new(0i64);

    column((
        label(res::str::counter_value(count)),
        button(res::str::add_one())
            .action(move || count.update(|n| *n = n.saturating_add(1))),
    ))
    .spacing(Space::M)
    .padding(Space::L)
}
```

The [step-by-step tutorial](https://daybrite.dev/docs/tutorials/state/) includes the surrounding code and a walkthrough that presses the button and checks the result.

## Platforms

Day supports a variety of platforms, each with a "Tier" for their current level of support:

| Platform | UI toolkit | Support |
| --- | --- | --- |
| macOS | AppKit | Tier 1 |
| iOS and iPadOS | UIKit | Tier 1 |
| Android | Material Components | Tier 1 |
| Linux | GTK 4 with libadwaita, or Qt 6 Widgets | Tier 2 |
| Windows | WinUI 3 | Tier 2 |
| HarmonyOS | ArkUI | Experimental |
| Web | Browser DOM | Experimental |

Tier 2 targets receive less testing than Tier 1. The [platform reference](https://daybrite.dev/docs/reference/platforms) lists package formats and known limitations for each toolkit.

## Getting involved

Use the [discussions forum](https://github.com/daybrite/day/discussions) for questions. For [bug reports](https://github.com/daybrite/day/issues), include the Day version, target toolkit, and steps to reproduce the problem. Code contributions are currently [by invitation](CONTRIBUTING.md).

Day is licensed under the [Mozilla Public License 2.0](LICENSE).

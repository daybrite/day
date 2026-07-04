# Lottie (external piece, iOS + Android)

> **Status: implemented** as `day-piece-lottie` — an EXTERNAL Day Piece rendering a Lottie animation,
> **iOS + Android only**. It is the reference for a piece that pulls an EXTERNAL native package on each
> platform: the **lottie-ios** SwiftPM package on iOS (via the `[package.metadata.day.ios]` mechanism it
> introduces) and **`com.airbnb.android:lottie`** on Android. Verified rendering + animating on the iOS
> simulator and the Android emulator.

## Authoring

```rust
use day_piece_lottie::lottie;

lottie("hello")                 // renders the bundled hello.json (looping, autoplaying)
    .frame(220.0, 220.0)        // it's a growing leaf — constrain it
    .id("lottie-view")
```

`lottie(name)` loads `name`(.json), **bundled with the app** — the iOS app bundle (`Bundle.main`) and
the Android `assets/`. `.looping(false)` plays once; `.autoplay(false)` starts paused. `Lottie`
implements `Piece`, so `.id()/.a11y()/.frame()` chain via `Decorate`.

The showcase's `lottie` page is `#[cfg(any(target_os = "ios", target_os = "android"))]`, so the nav item
appears only on those builds; the bundled `apps/showcase/assets/hello.json` is a small hand-authored
animation (a rotating, pulsing rounded square).

## Per-platform native realization

| | iOS (UIKit) | Android |
|---|---|---|
| control | `LottieAnimationView` (lottie-ios) | `LottieAnimationView` (lottie-android) |
| dependency | SwiftPM `github.com/airbnb/lottie-ios` | Gradle `com.airbnb.android:lottie` |
| declared in | `[package.metadata.day.ios].swift-packages` | `[package.metadata.day.android].gradle-dependencies` |
| shim | `ios/swift/DayLottie.swift` (`@_cdecl`) | `android/java/…/DayLottie.java` (static method) |

Both shims wrap a `LottieAnimationView` behind a flat interface the piece's Rust calls; the iOS shim
returns a `UIView` Rust wraps via `Retained::from_raw`, and the Android shim returns a `View` through JNI.

## The iOS mechanism this piece introduces

A piece can't drive a Swift library from Rust directly, and it ships as a SwiftPM package — neither of
which day supported before. `[package.metadata.day.ios]` (see [extending.md](extending.md)) adds it: at
build time the CLI generates a **local SwiftPM package** (`build/day/ios/DayPieces`) whose `Package.swift`
depends on every piece's `swift-packages` and compiles every piece's staged Swift shims. The app's
`.xcodeproj` depends on that one local package, so adding an iOS piece is pure `Cargo.toml` data — no
`.xcodeproj` edits. This mirrors the Android `day-pieces.json` → Gradle scaffold flow.

## Notes / gotchas

- **AndroidX** — Lottie's `LottieAnimationView` extends `androidx.appcompat.widget.AppCompatImageView`,
  so the app must set `android.useAndroidX=true` (a non-fatal *warning* is logged about the framework
  theme not being an AppCompat theme; it renders regardless).
- **Gradle configuration cache** — the scaffold reads the generated `day-pieces.json` at configuration
  time, which the config cache can't track, so it ships disabled (else a newly added piece's Gradle
  dependency is silently dropped from the build).

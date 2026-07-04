// The day-piece-lottie crate's OWN iOS backend — a Swift shim over airbnb/lottie-ios. It's staged
// into the generated `DayPieces` SwiftPM package (docs/extending.md), which depends on the lottie-ios
// package declared in this crate's [package.metadata.day.ios]. LottieAnimationView is a Swift class
// with a non-@objc API, so Rust can't drive it directly (like it drives UIKit via objc2); this shim
// exposes a flat C ABI (`@_cdecl`) that lib-uikit.rs calls. It is the iOS twin of android/java/DayLottie.java.

import UIKit
import Lottie

/// Create a LottieAnimationView for the bundled animation `name` (`name.json` in the app bundle) and
/// return it as a +1-retained pointer — the Rust caller takes ownership (wraps it as Retained<UIView>).
@_cdecl("day_lottie_new")
public func day_lottie_new(
    _ namePtr: UnsafePointer<CChar>,
    _ looping: Bool,
    _ autoplay: Bool
) -> UnsafeMutableRawPointer {
    let name = String(cString: namePtr)
    let view = LottieAnimationView(name: name)
    view.contentMode = .scaleAspectFit
    view.loopMode = looping ? .loop : .playOnce
    if autoplay {
        view.play()
    }
    return Unmanaged.passRetained(view).toOpaque()
}

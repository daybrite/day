// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The day-piece-lottie crate's OWN iOS backend — a Swift shim over airbnb/lottie-ios. It's staged
// into the generated `DayPieces` SwiftPM package (docs/extending.md), which depends on the lottie-ios
// package declared in this crate's [package.metadata.day.ios]. LottieAnimationView is a Swift class
// with a non-@objc API, so Rust can't drive it directly (like it drives UIKit via objc2); this shim
// exposes a flat C ABI (`@_cdecl`) that lib-uikit.rs calls. It is the iOS twin of android/java/DayLottie.java.

import UIKit
import Lottie

/// Create a LottieAnimationView for the bundled animation and return it as a +1-retained pointer —
/// the Rust caller takes ownership (wraps it as Retained<UIView>).
///
/// `path` is the animation resolved from the project's `resource/assets/`, which `day build` stages
/// into the bundle's `assets/` subdirectory. It is preferred because `LottieAnimationView(name:)`
/// searches only the bundle ROOT, where Day stages nothing — an app relying on that would have to
/// add a second copy of the JSON to its Xcode project by hand. `name` remains the fallback for the
/// projects that did exactly that.
@_cdecl("day_lottie_new")
public func day_lottie_new(
    _ namePtr: UnsafePointer<CChar>,
    _ pathPtr: UnsafePointer<CChar>,
    _ looping: Bool,
    _ autoplay: Bool,
    _ speed: Double
) -> UnsafeMutableRawPointer {
    let name = String(cString: namePtr)
    let path = String(cString: pathPtr)
    let view: LottieAnimationView
    if !path.isEmpty, let animation = LottieAnimation.filepath(path) {
        view = LottieAnimationView(animation: animation)
    } else {
        view = LottieAnimationView(name: name)
    }
    view.contentMode = .scaleAspectFit
    view.loopMode = looping ? .loop : .playOnce
    view.animationSpeed = CGFloat(speed)
    if autoplay {
        view.play()
    }
    return Unmanaged.passRetained(view).toOpaque()
}

/// Update the playback rate of an existing LottieAnimationView (from a `Speed` patch). The pointer is
/// the same object Rust wraps as a UIView; we take an unretained reference (Rust still owns the +1).
///
/// Assigning `animationSpeed` alone re-adds the in-flight animation, and with the Core Animation
/// rendering engine + loop mode that restarts it at frame 0 (lottie-ios can't retime a running
/// CAAnimation in place). So we snapshot the current progress and, if it was playing, resume from that
/// exact frame with `play()` — which loops the FULL range from the current progress at the new speed
/// (unlike `play(fromProgress:toProgress:)`, which would permanently shrink the loop to `[progress, 1]`).
/// The result: the scrubber changes speed without the animation jumping back to the start.
///
/// NOTE: this must NOT be called faster than ~one display frame — right after a re-add,
/// `presentation()` is briefly nil so `realtimeAnimationProgress` falls back to the model end frame
/// (a bogus 1.0), which would then be written back and corrupt playback. day's `Slider` snaps to its
/// `.step`, so a bound speed only changes at step crossings (never per drag frame), keeping calls
/// comfortably spaced. Keep that invariant if driving speed from anything other than a stepped slider.
@_cdecl("day_lottie_set_speed")
public func day_lottie_set_speed(_ viewPtr: UnsafeMutableRawPointer, _ speed: Double) {
    let view = Unmanaged<LottieAnimationView>.fromOpaque(viewPtr).takeUnretainedValue()
    let newSpeed = CGFloat(speed)
    guard view.animationSpeed != newSpeed else { return }
    let wasPlaying = view.isAnimationPlaying
    let progress = view.realtimeAnimationProgress
    view.animationSpeed = newSpeed
    if wasPlaying {
        view.currentProgress = progress
        view.play()
    }
}

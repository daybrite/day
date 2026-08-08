// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// day-piece-swiftui — the Swift half (docs/swiftui.md). Staged by `day build` into the generated
// DayPieces package on both Apple legs (`#if os(...)` selects the host toolkit), alongside any
// generated provider glue and the app's own Swift sources.
//
// The naming contract is deliberately platform-neutral: a provider is an @objc class named
// `DayView_<name>` (dots in the Rust-side name become underscores), resolved here with
// NSClassFromString — the same string contract a future Jetpack Compose leg can satisfy with
// Class.forName. No registration call, no startup scan.

import SwiftUI
#if os(macOS)
import AppKit
#else
import UIKit
#endif

/// Subclass this and name the class `@objc(DayView_<name>)` to expose a SwiftUI view to Day's
/// `swiftui("<name>")`. Generated glue does exactly that for every public View in a scanned
/// SwiftPM package; hand-written providers are the escape hatch for views needing custom wiring.
open class DaySwiftUIProvider: NSObject {
    public required override init() {}

    /// Called once to create the view and again on every params change. Return the SAME underlying
    /// view type from every call so SwiftUI diffing preserves `@State` across updates.
    open func body(_ params: String?) -> AnyView {
        AnyView(EmptyView())
    }
}

/// Support surface shared by the shim and the generated provider glue.
public enum DaySwiftUI {
    /// The visible stand-in when a provider class is missing or its params fail to decode —
    /// a hosted error marker rather than a crash or a blank, mirroring Day's placeholder leaves.
    public static func errorView(_ name: String) -> AnyView {
        AnyView(Text("⟨\(name)?⟩").foregroundColor(.red).padding(4))
    }
}

// Associated-object keys: the provider (both platforms) and, on iOS, the UIHostingController that
// owns the returned view — dropping the controller would tear the view down under Day's feet.
private var dayProviderKey: UInt8 = 0
#if !os(macOS)
private var dayControllerKey: UInt8 = 0
#endif

// State retention (`.state_key(...)` on the Rust side): hosting views kept alive across
// unmount/remount, so the SwiftUI state graph they own (`@State`, `@StateObject`, scroll
// positions) survives Day disposing the node. One view per key, pinned for the app's lifetime;
// main-thread only, like every make/update call Day issues.
#if os(macOS)
private var dayRetainedViews: [String: NSView] = [:]
#else
private var dayRetainedViews: [String: UIView] = [:]
#endif

/// Resolve `name` to its provider, or a stand-in provider that hosts the error view.
private func dayProvider(named name: String) -> DaySwiftUIProvider {
    let className = "DayView_" + name.replacingOccurrences(of: ".", with: "_")
    guard let cls = NSClassFromString(className) as? DaySwiftUIProvider.Type else {
        NSLog("%@", "day-piece-swiftui: no @objc(\(className)) provider class for swiftui(\"\(name)\")")
        return DayMissingProvider(name)
    }
    return cls.init()
}

private final class DayMissingProvider: DaySwiftUIProvider {
    private let name: String
    init(_ name: String) {
        self.name = name
        super.init()
    }
    required init() {
        self.name = "?"
        super.init()
    }
    override func body(_ params: String?) -> AnyView {
        DaySwiftUI.errorView(name)
    }
}

/// Create (or, under a state key, revive) the hosting view for `name` (nullable `params` JSON)
/// and return it as a +1-retained pointer — the Rust caller takes ownership (wraps it as
/// `Retained<NSView/UIView>`).
@_cdecl("day_swiftui_make")
public func day_swiftui_make(
    _ namePtr: UnsafePointer<CChar>,
    _ paramsPtr: UnsafePointer<CChar>?,
    _ stateKeyPtr: UnsafePointer<CChar>?
) -> UnsafeMutableRawPointer {
    let name = String(cString: namePtr)
    let params = paramsPtr.map { String(cString: $0) }
    let stateKey = stateKeyPtr.map { String(cString: $0) }

    // A retained view from a prior mount: hand back the SAME instance — its SwiftUI state graph is
    // intact — with this mount's params applied through its provider (locale switches and other
    // data changes that happened while unmounted land here). Defensively unparent it: Day removed
    // it on release, but a stale superview (or a misuse mounting one key twice) must not wedge the
    // insert.
    if let stateKey, let view = dayRetainedViews[stateKey] {
        view.removeFromSuperview()
        if let provider = objc_getAssociatedObject(view, &dayProviderKey) as? DaySwiftUIProvider {
            #if os(macOS)
            (view as? NSHostingView<AnyView>)?.rootView = provider.body(params)
            #else
            (objc_getAssociatedObject(view, &dayControllerKey) as? UIHostingController<AnyView>)?
                .rootView = provider.body(params)
            #endif
        }
        return Unmanaged.passRetained(view).toOpaque()
    }

    let provider = dayProvider(named: name)
    let root = provider.body(params)
    #if os(macOS)
    let view = NSHostingView(rootView: root)
    objc_setAssociatedObject(view, &dayProviderKey, provider, .OBJC_ASSOCIATION_RETAIN)
    #else
    let controller = UIHostingController(rootView: root)
    // Transparent, like NSHostingView: the Day surface behind the view stays visible.
    controller.view.backgroundColor = .clear
    let view: UIView = controller.view
    objc_setAssociatedObject(view, &dayProviderKey, provider, .OBJC_ASSOCIATION_RETAIN)
    objc_setAssociatedObject(view, &dayControllerKey, controller, .OBJC_ASSOCIATION_RETAIN)
    #endif
    if let stateKey {
        dayRetainedViews[stateKey] = view
    }
    return Unmanaged.passRetained(view).toOpaque()
}

/// New params for a live hosting view: re-invoke its provider's body and replace the root.
/// SwiftUI diffing preserves `@State` while the provider keeps returning the same view type.
@_cdecl("day_swiftui_update")
public func day_swiftui_update(
    _ viewPtr: UnsafeMutableRawPointer,
    _ paramsPtr: UnsafePointer<CChar>?
) {
    let params = paramsPtr.map { String(cString: $0) }
    #if os(macOS)
    let view = Unmanaged<NSView>.fromOpaque(viewPtr).takeUnretainedValue()
    guard let provider = objc_getAssociatedObject(view, &dayProviderKey) as? DaySwiftUIProvider,
          let host = view as? NSHostingView<AnyView>
    else { return }
    host.rootView = provider.body(params)
    #else
    let view = Unmanaged<UIView>.fromOpaque(viewPtr).takeUnretainedValue()
    guard let provider = objc_getAssociatedObject(view, &dayProviderKey) as? DaySwiftUIProvider,
          let controller = objc_getAssociatedObject(view, &dayControllerKey)
              as? UIHostingController<AnyView>
    else { return }
    controller.rootView = provider.body(params)
    #endif
}

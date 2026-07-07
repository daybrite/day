// iOS: UIKit's feedback generators. The three impact intensities go through
// UIImpactFeedbackGenerator, the three notification outcomes through UINotificationFeedbackGenerator,
// and a selection tick through UISelectionFeedbackGenerator — the standard Apple mapping (see the
// `Haptic` doc). These classes are MainThreadOnly and day runs on the main thread; if somehow called
// off it, MainThreadMarker::new() returns None and we no-op. The Simulator has no Taptic engine, so
// the calls are silently ignored there — they must not (and do not) crash.

use super::Haptic;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_ui_kit::{
    UIImpactFeedbackGenerator, UIImpactFeedbackStyle, UINotificationFeedbackGenerator,
    UINotificationFeedbackType, UISelectionFeedbackGenerator,
};

pub fn play(h: Haptic) {
    // Haptics are UI feedback: only valid on the main thread. Off it, do nothing.
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    match h {
        Haptic::Light | Haptic::Medium | Haptic::Heavy => {
            let style = match h {
                Haptic::Light => UIImpactFeedbackStyle::Light,
                Haptic::Heavy => UIImpactFeedbackStyle::Heavy,
                _ => UIImpactFeedbackStyle::Medium,
            };
            // initWithStyle: is flagged deprecated in the objc2 bindings, but it is the only way to
            // pick an impact intensity without attaching the generator to a UIView.
            #[allow(deprecated)]
            let generator = UIImpactFeedbackGenerator::initWithStyle(
                UIImpactFeedbackGenerator::alloc(mtm),
                style,
            );
            generator.prepare();
            generator.impactOccurred();
        }
        Haptic::Success | Haptic::Warning | Haptic::Error => {
            let kind = match h {
                Haptic::Success => UINotificationFeedbackType::Success,
                Haptic::Warning => UINotificationFeedbackType::Warning,
                _ => UINotificationFeedbackType::Error,
            };
            let generator = UINotificationFeedbackGenerator::new(mtm);
            generator.prepare();
            generator.notificationOccurred(kind);
        }
        Haptic::Selection => {
            let generator = UISelectionFeedbackGenerator::new(mtm);
            generator.prepare();
            generator.selectionChanged();
        }
    }
}

pub fn is_supported() -> bool {
    true
}

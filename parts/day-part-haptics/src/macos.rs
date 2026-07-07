// macOS: AppKit's NSHapticFeedbackManager drives the Force Touch trackpad. It offers only three
// patterns — Generic, Alignment, LevelChange — so the seven `Haptic` styles fold onto them
// sensibly:
//   Light / Selection      → Alignment   (the subtlest "snap into place" tick)
//   Medium / Heavy         → LevelChange (a firmer detent, as when stepping a value)
//   Success/Warning/Error  → Generic     (the general-purpose feedback)
// A Mac with no Force Touch trackpad (or an external mouse in use) simply feels nothing — the call is
// harmless. Feedback is delivered "now"; day runs on the main thread, where this belongs.

use super::Haptic;
use objc2_app_kit::{
    NSHapticFeedbackManager, NSHapticFeedbackPattern, NSHapticFeedbackPerformanceTime,
    NSHapticFeedbackPerformer,
};

pub fn play(h: Haptic) {
    let pattern = match h {
        Haptic::Light | Haptic::Selection => NSHapticFeedbackPattern::Alignment,
        Haptic::Medium | Haptic::Heavy => NSHapticFeedbackPattern::LevelChange,
        Haptic::Success | Haptic::Warning | Haptic::Error => NSHapticFeedbackPattern::Generic,
    };
    let performer = NSHapticFeedbackManager::defaultPerformer();
    performer.performFeedbackPattern_performanceTime(pattern, NSHapticFeedbackPerformanceTime::Now);
}

pub fn is_supported() -> bool {
    true
}

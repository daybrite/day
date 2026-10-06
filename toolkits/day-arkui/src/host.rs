// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The toolkit duties HarmonyOS exposes only in ArkTS, reached through daybridge arms
//! (docs/bridge.md) rather than the C node shim: the app's color mode (docs/appearance.md), the
//! app-icon badge (docs/badge.md) and screen-reader announcements (docs/accessibility.md; the C
//! API's announce event needs an XComponent's accessibility provider, which a node tree has
//! none of). `day build` stages the ArkTS half beside every other bridged
//! crate's, so a duty added here needs no shim or host-page change.

use day_bridge::Support;

/// Force the app's color mode (`Some(true)` dark, `Some(false)` light) or follow the system
/// again (`None`). The change comes back through [`watch_appearance`] like a system switch.
pub(crate) fn set_color_mode(dark: Option<bool>) {
    set_color_mode_native(match dark {
        Some(false) => 0,
        Some(true) => 1,
        None => 2,
    });
}

/// Whether the ArkTS arms are staged into this build (an older host project, or a bare
/// `cargo build`, has none).
pub(crate) fn color_mode_support() -> Support {
    set_color_mode_native_support()
}

/// Put `count` on the app icon; 0 clears it.
pub(crate) fn set_badge(count: u32) {
    set_badge_native(i32::try_from(count).unwrap_or(i32::MAX));
}

pub(crate) fn badge_support() -> Support {
    set_badge_native_support()
}

/// Speak `text` through the screen reader without moving its focus (docs/accessibility.md);
/// `urgent` interrupts what is being read.
pub(crate) fn announce(text: &str, urgent: bool) {
    announce_native(text, urgent);
}

pub(crate) fn announce_support() -> Support {
    announce_native_support()
}

/// Report every color-mode change the system applies, the app's own override included, as
/// `dark`. Runs `on_change` on Day's UI thread (the ArkTS environment callback is the JS thread,
/// which is that thread, but the stream's callback has to be `Send` either way).
pub(crate) fn watch_appearance(on_change: fn(bool)) {
    let _ = watch_color_mode_native_stream(move |item| {
        if let day_bridge::Item::Value(mode) = item {
            day_reactive::on_main(move || on_change(mode == 1));
        }
    });
}

day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        /// The application context's color mode: 0 light, 1 dark, 2 follow the system.
        fn set_color_mode_native(mode: i32);
        /// The app icon's badge number; 0 clears it.
        fn set_badge_native(count: i32);
        /// The accessibility kit's announcement event: `urgent` interrupts the current speech.
        fn announce_native(text: &str, urgent: bool);
        /// The color mode after every configuration change: 1 dark, 0 light.
        fn watch_color_mode_native(emit: day_bridge::Emit<i32>) -> Result<(), day_bridge::Error>;
    }

    #[day_bridge::impl(arkts, platforms = [ohos])]
    arkts!(
        prelude = r#"
            import { common, Configuration, ConfigurationConstant } from '@kit.AbilityKit';
            import { notificationManager } from '@kit.NotificationKit';
            import { accessibility } from '@kit.AccessibilityKit';
            import { BusinessError } from '@kit.BasicServicesKit';
        "#,
        body = r#"
            function dayAppContext(): common.ApplicationContext {
              return (getContext() as common.UIAbilityContext).getApplicationContext();
            }

            export function set_color_mode_native(mode: number): void {
              dayAppContext().setColorMode(
                mode === 1 ? ConfigurationConstant.ColorMode.COLOR_MODE_DARK
                  : mode === 0 ? ConfigurationConstant.ColorMode.COLOR_MODE_LIGHT
                  : ConfigurationConstant.ColorMode.COLOR_MODE_NOT_SET);
            }

            export function set_badge_native(count: number): void {
              notificationManager.setBadgeNumber(count).catch((e: BusinessError) => {
                console.warn(`day-arkui: badge ${count}: ${e.code} ${e.message}`);
              });
            }

            export function announce_native(text: string, urgent: boolean): void {
              // Nothing to say to nobody: the event is only sent while a screen reader runs.
              if (!accessibility.isScreenReaderOpenSync()) {
                return;
              }
              const bundle = (getContext() as common.UIAbilityContext).abilityInfo.bundleName;
              const info = new accessibility.EventInfo(
                urgent ? 'announceForAccessibility' : 'announceForAccessibilityNotInterrupt',
                bundle, 'common');
              info.textAnnouncedForAccessibility = text;
              accessibility.sendAccessibilityEvent(info).catch((e: BusinessError) => {
                console.warn(`day-arkui: announce: ${e.code} ${e.message}`);
              });
            }

            export function watch_color_mode_native(emit: number): void {
              dayAppContext().on('environment', {
                onConfigurationUpdated: (config: Configuration): void => {
                  if (config.colorMode !== undefined) {
                    watch_color_mode_native_emit(emit,
                      config.colorMode === ConfigurationConstant.ColorMode.COLOR_MODE_DARK ? 1 : 0);
                  }
                },
                onMemoryLevel: (): void => {}
              });
            }
        "#,
    );

    #[day_bridge::impl(rust, platforms = [other])]
    fn set_color_mode_native(_mode: i32) {}

    #[day_bridge::impl(rust, platforms = [other])]
    fn set_badge_native(_count: i32) {}

    #[day_bridge::impl(rust, platforms = [other])]
    fn announce_native(_text: &str, _urgent: bool) {}

    #[day_bridge::impl(rust, platforms = [other])]
    fn watch_color_mode_native(_emit: day_bridge::Emit<i32>) -> Result<(), day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }
}

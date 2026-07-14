// Android: android.os.Build, read via this crate's OWN Java shim (android/java/…/DayDeviceInfo.java)
// — staged into the app's Gradle build by `day build` through [package.metadata.day.android], exactly
// like the UI pieces, but registering NO renderer. Build.* are static fields (no Context needed), but
// crossing into Java still rides day-android's re-exported `jni` + attached JVM. No permission needed.
//
// `read()` returns one string with the four fields joined by U+001F (the ASCII unit separator, which
// cannot appear in a Build value): model, "Android", VERSION.RELEASE, and "1"/"0" for the emulator
// flag. We split it back apart here.

use super::DeviceInfo;
use day_android::jni::objects::JString;
use day_android::DayEnv;
use day_android::with_env;

const DEVICEINFO_CLASS: &str = "dev/daybrite/day/deviceinfo/DayDeviceInfo";
const SEP: char = '\u{1f}';

pub fn get() -> DeviceInfo {
    let joined: Option<String> = with_env(|env| {
        let obj = env
            .dcall_static(DEVICEINFO_CLASS, "read", "()Ljava/lang/String;", &[])
            .ok()?
            .l()
            .ok()?;
        if obj.is_null() {
            return None;
        }
        env.dstr(&day_android::as_jstring(obj)).ok().map(|s| s.into())
    });
    parse(joined.as_deref())
}

/// Split the packed `model␟Android␟release␟simFlag` string into a [`DeviceInfo`], filling any missing
/// field with a sensible fallback. Factored out so it is unit-testable off-device.
fn parse(joined: Option<&str>) -> DeviceInfo {
    let mut parts = joined.unwrap_or("").split(SEP);
    let field = |p: Option<&str>, fallback: &str| {
        p.filter(|s| !s.is_empty()).unwrap_or(fallback).to_string()
    };
    let model = field(parts.next(), "Unknown");
    let system_name = field(parts.next(), "Android");
    let system_version = field(parts.next(), "Unknown");
    let is_simulator = parts.next() == Some("1");
    DeviceInfo {
        model,
        system_name,
        system_version,
        is_simulator,
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parses_full_line() {
        let d = parse(Some("Pixel 7\u{1f}Android\u{1f}14\u{1f}0"));
        assert_eq!(d.model, "Pixel 7");
        assert_eq!(d.system_name, "Android");
        assert_eq!(d.system_version, "14");
        assert!(!d.is_simulator);
    }

    #[test]
    fn parses_emulator_flag() {
        let d = parse(Some("sdk_gphone64_arm64\u{1f}Android\u{1f}15\u{1f}1"));
        assert!(d.is_simulator);
    }

    #[test]
    fn fills_fallbacks_when_empty() {
        let d = parse(None);
        assert_eq!(d.model, "Unknown");
        assert_eq!(d.system_name, "Android");
        assert_eq!(d.system_version, "Unknown");
        assert!(!d.is_simulator);
    }
}

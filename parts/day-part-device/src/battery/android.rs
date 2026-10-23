pub fn status() -> Option<super::BatteryStatus> {
    crate::android::battery_status()
}

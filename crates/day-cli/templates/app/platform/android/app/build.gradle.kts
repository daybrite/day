plugins {
    // Configures this module from Day.toml and the app's pieces
    // (https://daybrite.dev/docs/platforms/android-mdc#the-gradle-project).
    id("dev.daybrite.day.android")
}

// This app's own Android settings and libraries. They apply after Day's plugin, so they win.
android {
}

dependencies {
}

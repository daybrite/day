// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

plugins {
    `java-gradle-plugin`
}

// The Android Gradle Plugin every Day app builds with. `dev.daybrite.day.android` applies
// `com.android.application` from this dependency, so an app's own scripts pin no AGP version.
//
// Before raising it, check that a current Android Studio release supports the new version:
// https://developer.android.com/studio/releases#android_gradle_plugin_and_android_studio_compatibility
// AGP ships ahead of the IDE, and Android Studio refuses to sync a project on a newer AGP than it
// supports ("incompatible version (AGP 9.4.0)… Latest supported version is AGP 9.3.0" on 2026.1.3),
// so every Day app would stop opening there while `day build` kept working.
dependencies {
    implementation("com.android.tools.build:gradle:9.3.2")
}

// Loads on any JDK that Gradle and AGP accept.
tasks.withType<JavaCompile>().configureEach {
    options.release.set(17)
}

gradlePlugin {
    plugins {
        create("daySettings") {
            id = "dev.daybrite.day.settings"
            implementationClass = "dev.daybrite.day.gradle.DaySettingsPlugin"
        }
        create("dayAndroid") {
            id = "dev.daybrite.day.android"
            implementationClass = "dev.daybrite.day.gradle.DayAndroidPlugin"
        }
    }
}

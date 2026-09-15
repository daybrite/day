// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Day's Gradle plugins for android-mdc apps. `day build` and `day prepare` stage this directory
// into the app's build/day/android/gradle-plugin/, and the app's settings.gradle.kts includes it
// with `pluginManagement { includeBuild(…) }`, so the plugins always match the day-android crate
// the app builds against.
dependencyResolutionManagement {
    repositories {
        google()
        mavenCentral()
    }
}
rootProject.name = "day-gradle-plugin"

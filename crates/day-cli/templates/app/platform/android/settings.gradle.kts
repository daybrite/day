pluginManagement {
    // Day's Gradle plugins, put in place by `day build`, `day prepare`, and `day open` from the Day
    // this app builds against (https://daybrite.dev/docs/platforms/android-mdc#the-gradle-project).
    if (!file("../../build/day/android/gradle-plugin").isDirectory) {
        throw GradleException(
            "day: build/day/android/gradle-plugin is missing. Run `day prepare` or " +
            "`day build -p android-mdc` in the project root first."
        )
    }
    includeBuild("../../build/day/android/gradle-plugin")
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}
plugins {
    // Where the app's dependencies resolve from, including repositories its pieces declare.
    id("dev.daybrite.day.settings")
}
// A constant: Gradle shows it in the IDE and nothing else reads it, so it need not
// carry the package name into a second file (DESIGN.md §17.5 "Renaming a project").
rootProject.name = "dayapp"
include(":app")

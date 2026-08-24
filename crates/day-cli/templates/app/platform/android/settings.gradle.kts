pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}
dependencyResolutionManagement {
    repositories {
        google()
        mavenCentral()
        // Extra Maven repos contributed by standalone pieces (docs/extending.md), staged by
        // `day build` from cargo metadata. Read generically — no per-piece edits here.
        val piecesFile = settingsDir.resolve("../../build/day/android/day-pieces.json")
        if (piecesFile.exists()) {
            @Suppress("UNCHECKED_CAST")
            val pieces = groovy.json.JsonSlurper().parse(piecesFile) as Map<String, Any>
            @Suppress("UNCHECKED_CAST")
            (pieces["repositories"] as? List<String>).orEmpty().forEach { url -> maven { setUrl(url) } }
        }
    }
}
// A constant: Gradle shows it in the IDE and nothing else reads it, so it need not
// carry the package name into a second file (DESIGN.md §17.5 "Renaming a project").
rootProject.name = "dayapp"
include(":app")

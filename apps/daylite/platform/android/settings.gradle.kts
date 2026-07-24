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
rootProject.name = "daylite"
include(":app")

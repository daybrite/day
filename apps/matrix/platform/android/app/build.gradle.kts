plugins {
    id("com.android.application")
}

// Standalone-piece backend contributions (docs/extending.md): `day build` resolves every piece in
// the app's dependency tree from `cargo metadata` and stages its Java dirs + Gradle deps here. Read
// generically — a piece adds native Android code with NO edits to this file.
val dayPiecesFile = rootProject.projectDir.resolve("../../build/day/android/day-pieces.json")
@Suppress("UNCHECKED_CAST")
val dayPieces: Map<String, Any> =
    if (dayPiecesFile.exists()) groovy.json.JsonSlurper().parse(dayPiecesFile) as Map<String, Any>
    else emptyMap()
@Suppress("UNCHECKED_CAST")
val pieceJavaDirs = (dayPieces["javaSrcDirs"] as? List<String>) ?: emptyList()
@Suppress("UNCHECKED_CAST")
val pieceDeps = (dayPieces["dependencies"] as? List<String>) ?: emptyList()
@Suppress("UNCHECKED_CAST")
val piecePermissions = (dayPieces["permissions"] as? List<String>) ?: emptyList()

android {
    namespace = "dev.daybrite.matrix"
    compileSdk = 35
    defaultConfig {
        applicationId = "dev.daybrite.matrix"
        minSdk = 24
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
    }
    sourceSets {
        getByName("main") {
            // The day-android Java shim ships with the framework (§17.1).
            java.srcDir(rootProject.projectDir.resolve("../../../../toolkits/day-android/java"))
            // Standalone pieces' own Java/Kotlin (docs/extending.md).
            pieceJavaDirs.forEach { java.srcDir(it) }
            // Rust .so staged by `day build` / `day gradle-backend build` (§17.4 — never src/main).
            jniLibs.srcDir(rootProject.projectDir.resolve("../../build/day/jniLibs"))
            assets.srcDir(rootProject.projectDir.resolve("../../assets"))
        }
        // Android <uses-permission>s contributed by standalone pieces (docs/extending.md) live in a
        // generated overlay manifest that AGP merges into the app manifest. Point the build-type
        // source-set manifests at it (a source set has one manifest slot; main keeps the app's).
        val pieceManifest = rootProject.projectDir.resolve("../../build/day/android/day-pieces-manifest.xml")
        if (piecePermissions.isNotEmpty() && pieceManifest.exists()) {
            getByName("debug").manifest.srcFile(pieceManifest)
            getByName("release").manifest.srcFile(pieceManifest)
        }
    }
    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    // Material Components — the M3 Expressive theme (res/values/styles.xml) and the Material
    // widgets the day-android shim creates (MaterialButton, MaterialSwitch, Slider, text fields,
    // progress/loading indicators, BottomNavigationView tabs, Material dialogs).
    implementation("com.google.android.material:material:1.14.0")
    // Gradle dependencies contributed by standalone pieces (docs/extending.md).
    pieceDeps.forEach { implementation(it) }
}

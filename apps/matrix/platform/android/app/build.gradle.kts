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
val pieceResDirs = (dayPieces["resSrcDirs"] as? List<String>) ?: emptyList()
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
            // The day-android Java shim (DayActivity, DayBridge, …): `day build` resolves it
            // from the day-android crate via cargo metadata and stages the path in
            // day-pieces.json — wherever cargo has the crate (workspace, git checkout, or
            // registry source). See the guard below for what happens when it is absent.
            (dayPieces["dayJavaSrcDir"] as? String)?.let { java.srcDir(it) }
            // Standalone pieces' own Java/Kotlin and Android resources (docs/extending.md).
            pieceJavaDirs.forEach { java.srcDir(it) }
            pieceResDirs.forEach { res.srcDir(it) }
            // Rust .so staged by `day build` / `day gradle-backend build` (§17.4 — never src/main).
            jniLibs.srcDir(rootProject.projectDir.resolve("../../build/day/jniLibs"))
            // The project's `resource/assets/` — raw data bundled into the APK `assets/` root and
            // read via the NDK `AAssetManager` (§18.3).
            assets.srcDir(rootProject.projectDir.resolve("../../resource/assets"))
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
    // Fragment-managed navigation (DayNavHost): fragment 1.7+ dispatches system back through
    // OnBackPressedDispatcher; 1.8+ with transition 1.5+ SEEKS the pop transition under the
    // predictive back gesture (docs/navigation.md).
    implementation("androidx.fragment:fragment:1.8.5")
    implementation("androidx.transition:transition:1.5.1")
    // Gradle dependencies contributed by standalone pieces (docs/extending.md).
    pieceDeps.forEach { implementation(it) }
}


// Without the day-android Java shim the APK would install and then CRASH at launch with
// ClassNotFoundException (DayActivity never reaches the dex). IDE sync still configures; an
// actual build fails with instructions instead of producing a broken APK.
if (dayPieces["dayJavaSrcDir"] == null) {
    tasks.configureEach {
        if (name == "preBuild") doFirst {
            throw GradleException(
                "The day-android Java shim was not staged — build through the day CLI " +
                "(`day launch -p android-widget` / `day build -p android-widget`), which writes " +
                "build/day/android/day-pieces.json. A bare Gradle build cannot produce a working APK."
            )
        }
    }
}

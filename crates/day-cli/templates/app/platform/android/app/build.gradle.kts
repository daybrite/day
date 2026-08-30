import java.util.Properties

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
// R8/ProGuard keep rules aggregated by `day build`: the framework's own (bridge + native methods)
// plus one per Part/Piece/app that hands Java classes to native code by name (docs/extending.md).
val dayProguardFile = dayPieces["dayProguardFile"] as? String
@Suppress("UNCHECKED_CAST")
val pieceProguardFiles = (dayPieces["proguardFiles"] as? List<String>) ?: emptyList()

// Day.toml identity/version, conveyed by `day build` / `day pack` (§17.5), read
// generically so this file names no project of its own.
val dayAppFile = rootProject.projectDir.resolve("../../build/day/android/day-app.properties")
val dayApp = Properties().apply {
    if (dayAppFile.exists()) dayAppFile.inputStream().use { s -> load(s) }
}

// Day.toml identity, conveyed by `day build` / `day pack` (§17.5). REQUIRED rather than
// defaulted: a plausible-looking fallback ships an APK under the wrong id or name, so an unset
// value stops the build instead. This is also what keeps this file free of project-specific
// values, so forking an app never means editing it.
fun dayRequired(key: String): String = dayApp.getProperty(key)
    ?: throw GradleException(
        "day: `$key` is not set. build/day/android/day-app.properties is generated from " +
        "Day.toml by the day CLI, so build through it (`day build -p android-mdc`, " +
        "`day launch -p android-mdc`) rather than bare Gradle."
    )

// Release signing, resolved by `day pack` (Day.toml `signing.android` env refs, or its generated
// dev keystore). Absent file ⇒ unsigned release build (a plain `day build --profile release`).
val daySigningFile = rootProject.projectDir.resolve("../../build/day/android/day-signing.properties")
val daySigning = Properties().apply {
    if (daySigningFile.exists()) daySigningFile.inputStream().use { s -> load(s) }
}

android {
    // Day.toml [app] id, conveyed by `day build` (see dayRequired above).
    namespace = dayRequired("namespace")
    compileSdk = 35
    defaultConfig {
        applicationId = dayRequired("applicationId")
        minSdk = 24
        targetSdk = 35
        versionCode = dayRequired("versionCode").toInt()
        // The app label — Day.toml [app] title, resolved per target (an [app.android] override
        // wins); the manifest references it as ${dayTitle}.
        manifestPlaceholders["dayTitle"] = dayRequired("title")
        manifestPlaceholders["dayScheme"] = dayRequired("scheme")
        versionName = dayRequired("versionName")
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
            // The project's `resource/assets/` — raw data (e.g. Lottie `hello.json`) bundled into
            // the APK `assets/` root and read via the NDK `AAssetManager` (§18.3).
            assets.srcDir(rootProject.projectDir.resolve("../../resource/assets"))
            // Processed images (§18.3): images/ staged into res/drawable* -> R.drawable, crunched by aapt2.
            res.srcDir(rootProject.projectDir.resolve("../../build/day/android/res"))
        }
        // Android <uses-permission>s AND any <receiver>/<service> components contributed by
        // standalone pieces (docs/extending.md) live in a generated overlay manifest that AGP merges
        // into the app manifest. Point the build-type source-set manifests at it (a source set has
        // one manifest slot; main keeps the app's). Gate on the FILE, not on the permission list:
        // a part can contribute components without needing a permission, and `day build` removes
        // the overlay when it has nothing to say.
        val pieceManifest = rootProject.projectDir.resolve("../../build/day/android/day-pieces-manifest.xml")
        if (pieceManifest.exists()) {
            getByName("debug").manifest.srcFile(pieceManifest)
            getByName("release").manifest.srcFile(pieceManifest)
        }
    }
    if (daySigningFile.exists()) {
        signingConfigs {
            create("release") {
                storeFile = file(daySigning.getProperty("storeFile"))
                storePassword = daySigning.getProperty("storePassword")
                keyAlias = daySigning.getProperty("keyAlias")
                keyPassword = daySigning.getProperty("keyPassword")
            }
        }
    }
    buildTypes {
        release {
            // Release builds minify with R8 (shrink + rename). Day's Java bridge — and any Part,
            // Piece, or app JNI shim — is reached from native (Rust) code BY NAME (JNI FindClass,
            // dcall_static, reflection), so `day build` folds in keep rules that stop R8 renaming
            // those classes: the framework's own (dayProguardFile) plus each component's declared
            // proguard-rules.pro (pieceProguardFiles). Without them the APK installs then crashes at
            // launch. Day's rules add -dontoptimize, so the optimize base still shrinks + renames
            // but skips the aggressive optimizations that break reflection-heavy deps (Room/WorkManager).
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"))
            dayProguardFile?.let { proguardFiles(it) }
            pieceProguardFiles.forEach { proguardFiles(it) }
            if (daySigningFile.exists()) {
                signingConfig = signingConfigs.getByName("release")
            }
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
    // Gradle dependencies contributed by standalone pieces (docs/extending.md) AND by the
    // day-android shim itself (e.g. SlidingPaneLayout) — the shim declares its own deps in
    // [package.metadata.day.android] so they track the framework version, not this template.
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
                "(`day launch -p android-mdc` / `day build -p android-mdc`), which writes " +
                "build/day/android/day-pieces.json. A bare Gradle build cannot produce a working APK."
            )
        }
    }
}

// Reproducible archives (DESIGN.md §20.3). Gradle stamps each ZIP entry with the file's mtime and
// walks the tree in filesystem order, so two builds of identical sources differ by the wall-clock
// gap between them and by whatever order the directory happened to yield. These two flags are the
// documented fix; AGP zeroes APK timestamps on its own, but the AAB and every intermediate jar
// still need them. Applies to every archive task the build declares.
tasks.withType<AbstractArchiveTask>().configureEach {
    isPreserveFileTimestamps = false
    isReproducibleFileOrder = true
}

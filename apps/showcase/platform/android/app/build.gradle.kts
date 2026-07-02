plugins {
    id("com.android.application")
}

android {
    namespace = "dev.day.showcase"
    compileSdk = 35
    defaultConfig {
        applicationId = "dev.day.showcase"
        minSdk = 24
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
    }
    sourceSets {
        getByName("main") {
            // The day-android Java shim ships with the framework (§17.1).
            java.srcDir(rootProject.projectDir.resolve("../../../../toolkits/day-android/java"))
            // Rust .so staged by `day build` / `day gradle-backend build` (§17.4 — never src/main).
            jniLibs.srcDir(rootProject.projectDir.resolve("../../build/day/jniLibs"))
            assets.srcDir(rootProject.projectDir.resolve("../../assets"))
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

// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

package dev.daybrite.day.gradle;

import java.util.List;
import java.util.Map;
import java.util.Properties;

import com.android.build.api.dsl.AndroidSourceSet;
import com.android.build.api.dsl.ApkSigningConfig;
import com.android.build.api.dsl.ApplicationBuildType;
import com.android.build.api.dsl.ApplicationDefaultConfig;
import com.android.build.api.dsl.ApplicationExtension;
import org.gradle.api.Action;
import org.gradle.api.GradleException;
import org.gradle.api.JavaVersion;
import org.gradle.api.Plugin;
import org.gradle.api.Project;
import org.gradle.api.Task;
import org.gradle.api.artifacts.dsl.DependencyHandler;
import org.gradle.api.file.Directory;
import org.gradle.api.provider.ProviderFactory;
import org.gradle.api.tasks.bundling.AbstractArchiveTask;

/**
 * {@code dev.daybrite.day.android}: the app module's whole Day configuration.
 *
 * <p>Applies {@code com.android.application} and configures it from what `day build` generates:
 * Day.toml identity (§17.5), piece contributions (docs/extending.md), release signing, and the
 * native libraries and resources it stages. It runs before the app's own {@code android {}} and
 * {@code dependencies {}} blocks, so anything an app sets there overrides the values set here.
 */
public class DayAndroidPlugin implements Plugin<Project> {
    static final int COMPILE_SDK = 37;
    static final int MIN_SDK = 24;
    static final int TARGET_SDK = 37;

    /**
     * Libraries the day-android shim needs that are not declared in its
     * {@code [package.metadata.day.android]} table: Material Components (the M3 theme and widgets),
     * and fragment + transition, whose 1.8/1.5 releases seek the pop transition under the
     * predictive back gesture (docs/navigation.md).
     */
    static final List<String> SHIM_DEPENDENCIES = List.of(
            "com.google.android.material:material:1.14.0",
            "androidx.fragment:fragment:1.8.5",
            "androidx.transition:transition:1.5.1");

    @Override
    public void apply(Project project) {
        project.getPluginManager().apply("com.android.application");

        ProviderFactory providers = project.getProviders();
        Directory gradleRoot = project.getRootProject().getLayout().getProjectDirectory();
        Map<String, Object> pieces =
                DayGenerated.json(DayGenerated.text(providers, gradleRoot, "day-pieces.json"));
        Properties app =
                DayGenerated.properties(DayGenerated.text(providers, gradleRoot, "day-app.properties"));
        String signing = DayGenerated.text(providers, gradleRoot, "day-signing.properties");

        ApplicationExtension android = project.getExtensions().getByType(ApplicationExtension.class);
        android.setNamespace(required(app, "namespace"));
        android.setCompileSdk(COMPILE_SDK);

        ApplicationDefaultConfig config = android.getDefaultConfig();
        config.setApplicationId(required(app, "applicationId"));
        config.setMinSdk(MIN_SDK);
        config.setTargetSdk(TARGET_SDK);
        config.setVersionCode(Integer.parseInt(required(app, "versionCode")));
        config.setVersionName(required(app, "versionName"));
        // The manifest's app label and deep-link scheme, and Day.toml [window] for the activities'
        // <layout> element (docs/size-classes.md). The window values default to Day.toml's own
        // defaults for a CLI that predates them.
        Map<String, Object> placeholders = config.getManifestPlaceholders();
        placeholders.put("dayTitle", required(app, "title"));
        placeholders.put("dayScheme", required(app, "scheme"));
        placeholders.put("dayWidth", app.getProperty("windowWidth", "960"));
        placeholders.put("dayHeight", app.getProperty("windowHeight", "640"));
        placeholders.put("dayMinWidth", app.getProperty("windowMinWidth", "320"));
        placeholders.put("dayMinHeight", app.getProperty("windowMinHeight", "400"));

        AndroidSourceSet main = android.getSourceSets().getByName("main");
        // The day-android Java shim, then the pieces' own Java/Kotlin and resources.
        String shim = DayGenerated.string(pieces, "dayJavaSrcDir");
        if (shim != null) {
            main.getJava().getDirectories().add(shim);
        }
        main.getJava().getDirectories().addAll(DayGenerated.strings(pieces, "javaSrcDirs"));
        main.getRes().getDirectories().addAll(DayGenerated.strings(pieces, "resSrcDirs"));
        // The Rust .so (never src/main), resource/assets, processed images, and the launcher icons.
        main.getJniLibs().getDirectories().add(path(gradleRoot, "../../build/day/jniLibs"));
        main.getAssets().getDirectories().add(path(gradleRoot, "../../resource/assets"));
        main.getRes().getDirectories().add(path(gradleRoot, "../../build/day/android/res"));
        main.getRes().getDirectories().add(path(gradleRoot, "../../build/day/host/android/res"));
        // Permissions and components contributed by pieces, parts and Day.toml [[shortcuts]] merge
        // in from a generated overlay. A source set has one manifest slot and main keeps the app's,
        // so the overlay takes the build types'.
        if (DayGenerated.exists(providers, gradleRoot, "day-pieces-manifest.xml")) {
            var overlay = DayGenerated.file(gradleRoot, "day-pieces-manifest.xml").getAsFile();
            android.getSourceSets().getByName("debug").getManifest().srcFile(overlay);
            android.getSourceSets().getByName("release").getManifest().srcFile(overlay);
        }

        // Release builds shrink and rename with R8. Native code reaches Java classes by name, so the
        // framework's keep rules and each component's are always included.
        ApplicationBuildType release = android.getBuildTypes().getByName("release");
        release.setMinifyEnabled(true);
        release.proguardFiles(android.getDefaultProguardFile("proguard-android-optimize.txt"));
        String dayRules = DayGenerated.string(pieces, "dayProguardFile");
        if (dayRules != null) {
            release.proguardFiles(dayRules);
        }
        DayGenerated.strings(pieces, "proguardFiles").forEach(release::proguardFiles);
        // `day pack` writes the signing file; without it a release build is unsigned.
        if (signing != null) {
            Properties keys = DayGenerated.properties(signing);
            ApkSigningConfig releaseKey = android.getSigningConfigs().create("release");
            releaseKey.setStoreFile(project.file(keys.getProperty("storeFile")));
            releaseKey.setStorePassword(keys.getProperty("storePassword"));
            releaseKey.setKeyAlias(keys.getProperty("keyAlias"));
            releaseKey.setKeyPassword(keys.getProperty("keyPassword"));
            release.setSigningConfig(releaseKey);
        }

        android.getCompileOptions().setSourceCompatibility(JavaVersion.VERSION_17);
        android.getCompileOptions().setTargetCompatibility(JavaVersion.VERSION_17);

        DependencyHandler dependencies = project.getDependencies();
        SHIM_DEPENDENCIES.forEach(coordinates -> dependencies.add("implementation", coordinates));
        DayGenerated.strings(pieces, "dependencies")
                .forEach(coordinates -> dependencies.add("implementation", coordinates));

        // Without the shim the APK installs and then crashes at launch with
        // ClassNotFoundException. IDE sync still configures; a build stops with instructions.
        if (shim == null) {
            project.getTasks().configureEach(task -> {
                if (task.getName().equals("preBuild")) {
                    task.doFirst(new FailWithoutShim());
                }
            });
        }

        // Reproducible archives (DESIGN.md §20.3): no file timestamps, a fixed entry order.
        project.getTasks().withType(AbstractArchiveTask.class).configureEach(task -> {
            task.setPreserveFileTimestamps(false);
            task.setReproducibleFileOrder(true);
        });
    }

    /** An absolute path under the Gradle root, the form a source set's directory list holds. */
    private static String path(Directory gradleRoot, String relative) {
        return gradleRoot.dir(relative).getAsFile().toPath().normalize().toString();
    }

    /**
     * Day.toml identity is required rather than defaulted: a plausible fallback ships an APK
     * under the wrong id or name.
     */
    private static String required(Properties app, String key) {
        String value = app.getProperty(key);
        if (value == null) {
            throw new GradleException(
                    "day: `" + key + "` is not set. build/day/android/day-app.properties is "
                            + "generated from Day.toml by the day CLI, so build through it "
                            + "(`day build -p android-mdc`, `day launch -p android-mdc`) rather "
                            + "than bare Gradle.");
        }
        return value;
    }

    /** A named class rather than a lambda, so the configuration cache can store the action. */
    static final class FailWithoutShim implements Action<Task> {
        @Override
        public void execute(Task task) {
            throw new GradleException(
                    "The day-android Java shim was not staged — build through the day CLI "
                            + "(`day launch -p android-mdc` / `day build -p android-mdc`), which "
                            + "writes build/day/android/day-pieces.json. A bare Gradle build "
                            + "cannot produce a working APK.");
        }
    }
}

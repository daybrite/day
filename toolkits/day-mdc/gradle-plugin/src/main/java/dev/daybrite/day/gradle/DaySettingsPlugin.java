// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

package dev.daybrite.day.gradle;

import org.gradle.api.Plugin;
import org.gradle.api.artifacts.dsl.RepositoryHandler;
import org.gradle.api.initialization.Settings;

/**
 * {@code dev.daybrite.day.settings}: the repositories an app's dependencies resolve from: Google
 * and Maven Central, plus the extra Maven repositories its pieces declare in
 * {@code [package.metadata.day.android] gradle-repositories} (docs/extending.md).
 */
public class DaySettingsPlugin implements Plugin<Settings> {
    @Override
    public void apply(Settings settings) {
        RepositoryHandler repos = settings.getDependencyResolutionManagement().getRepositories();
        repos.google();
        repos.mavenCentral();
        String pieces = DayGenerated.text(
                settings.getProviders(),
                settings.getLayout().getSettingsDirectory(),
                "day-pieces.json");
        for (String url : DayGenerated.strings(DayGenerated.json(pieces), "repositories")) {
            repos.maven(repo -> repo.setUrl(url));
        }
    }
}

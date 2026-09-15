// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

package dev.daybrite.day.gradle;

import java.io.IOException;
import java.io.StringReader;
import java.io.UncheckedIOException;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Properties;

import org.gradle.api.file.Directory;
import org.gradle.api.file.RegularFile;
import org.gradle.api.provider.ProviderFactory;

/**
 * The files `day build` generates for Gradle under the project's {@code build/day/android/}.
 *
 * <p>Every read goes through {@link ProviderFactory#fileContents}, which the configuration cache
 * records as an input: a file whose contents change, or that appears or goes away, discards the
 * cached configuration, and a rewrite with the same contents keeps it.
 */
final class DayGenerated {
    /** The generated directory, relative to the Gradle root ({@code platform/android}). */
    static final String DIR = "../../build/day/android";

    private DayGenerated() {}

    static RegularFile file(Directory gradleRoot, String name) {
        return gradleRoot.file(DIR + "/" + name);
    }

    /** The file's text, or null when `day build` has not written it. */
    static String text(ProviderFactory providers, Directory gradleRoot, String name) {
        return providers.fileContents(file(gradleRoot, name)).getAsText().getOrNull();
    }

    static boolean exists(ProviderFactory providers, Directory gradleRoot, String name) {
        return providers.fileContents(file(gradleRoot, name)).getAsBytes().isPresent();
    }

    /** {@code day-pieces.json} as a map; empty when absent. */
    @SuppressWarnings("unchecked")
    static Map<String, Object> json(String text) {
        if (text == null) {
            return Map.of();
        }
        Object parsed = new groovy.json.JsonSlurper().parseText(text);
        return parsed instanceof Map<?, ?> map ? (Map<String, Object>) map : Map.of();
    }

    static Properties properties(String text) {
        Properties props = new Properties();
        if (text != null) {
            try {
                props.load(new StringReader(text));
            } catch (IOException e) {
                throw new UncheckedIOException(e);
            }
        }
        return props;
    }

    /** The string entries of a JSON list; empty when the key is absent. */
    static List<String> strings(Map<String, Object> map, String key) {
        List<String> out = new ArrayList<>();
        if (map.get(key) instanceof List<?> list) {
            for (Object item : list) {
                if (item instanceof String s) {
                    out.add(s);
                }
            }
        }
        return out;
    }

    static String string(Map<String, Object> map, String key) {
        return map.get(key) instanceof String s ? s : null;
    }
}

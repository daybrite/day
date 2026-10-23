# day-android-runtime: R8 optimization off, for apps that have not set `[android] optimize = true` in
# Day.toml. `day build` adds this file to the release configuration only then.
#
# AGP requires the `proguard-android-optimize.txt` base file, but its optimizations (inlining,
# class merging) break reflection-heavy libraries whose consumer rules miss it; WorkManager's Room
# `WorkDatabase` failed to instantiate. R8 still shrinks unused code and renames what no keep rule
# protects. An app whose release walkthrough passes with optimization on can opt in.
-dontoptimize

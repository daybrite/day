<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Landing page copy review

This tool compares alternative wording in the landing page’s layout. It runs on a separate
local server and is disabled by default. Nothing in this directory is imported by the website
or copied into its production output.

## Enable or disable the review

From the repository root:

```sh
DAY_COPY_REVIEW=1 npm --prefix website run copy:review
```

Open `http://127.0.0.1:4335/`. The command builds the website, snapshots its current landing
page, generates the proposals, and starts the local server. Node dependencies and Python 3
are required. A build may fetch gallery images and release metadata.

Stop the command with **Ctrl-C**. The environment flag applies only to that invocation;
ordinary `dev`, `build`, and `preview` commands never enable the review. Running
`npm --prefix website run copy:review` without the flag prints a disabled message and exits.
Do not add the flag to `.env`, production configuration, or CI.

Both [the launcher](start.mjs) and [the server](serve.py) document and enforce the toggle.
To use another port, append `-- --port 4336` to the npm command. A session lock prevents a
second launcher from replacing the snapshot while a review is open. If a process was killed
without cleanup, check that the PID recorded in `.local/session.lock` has exited before
removing that lock.

## Review copy

Use a section’s review button or the floating section selector. Original/A/B/C/D previews a
version without saving it. **Compare all versions** shows the complete alternatives.
**Edit this wording** changes individual fields. **Choose this version** saves the exact text,
including edits. **Review choices** summarizes decisions; **Export choices** downloads them
along with the unapproved previews.

Choices are written to `.local/review-state.json`. Browser previews persist separately in
local storage. A failed save is reported in the toolbar; export your work if local saving is
unavailable. Conflicting saves from another tab are rejected instead of overwriting decisions.
Saving does not modify the website or run an editing agent. Ask the assistant to apply the
saved choices when the review is complete.

## Prepare another editing session

1. Edit the alternatives in [drafts.py](drafts.py). Each section has four named options; the
   order of the values matches its fields in `.local/inventory.json`.
2. Start the enabled review command. It snapshots the current working copy, including
   uncommitted edits. The source commit and source-file hash identify the baseline.
3. If the landing page structure changes, update the section IDs and extraction rules in
   [snapshot.mjs](snapshot.mjs), then align the draft fields. The generator checks field counts.
4. When applying choices, compare the stored source hash with the source being edited.
   Apply saved records only; previews are not approvals. Preserve the exact edited wording.
5. Archive the decisions and their matching proposals under `sessions/` when integrating them.
   The [November 2026 decisions](sessions/2026-11-06/choices.json) are an example.

Relaunching the same review preserves saved choices. If the baseline or alternatives change,
[build.py](build.py) archives the previous proposal and choices under `.local/history/` and
starts a new review. It does not transfer approvals to changed copy. Keep important archives
under `sessions/`; `.local/` is ignored by Git.

## Implementation and checks

[snapshot.mjs](snapshot.mjs) extracts the main page sections, freezes a real gallery capture,
and copies only the assets it needs. It removes site scripts, including analytics, from the
review snapshot. Review controls are supplied by [review.js](review.js) and
[review.css](review.css). They are never added to the Astro page.

UI strings live in [strings.en.json](strings.en.json). The generator produces named
`res.str` accessors in `.local/res.mjs`; use those accessors for review controls.
The proposed marketing copy is publication content, kept separately in the draft data.

The server binds to loopback. It validates the host, origin, token, revision, and field set
before atomically saving a decision. Its file allowlist excludes decision archives and source
files. Editable text is inserted as text, not HTML.

Run the tool’s checks from the repository root:

```sh
npm --prefix website run copy:review:test
```

After applying copy, build the website and run its rendered-page and link checks. Verify that
the chosen text appears in the output and that the review controls do not.

---
title: Reference
description: Look up app APIs, command-line options, configuration, platform requirements, and terminology.
order: 61
section: Reference
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

These reference pages are published from the repository’s `docs/` directory. They cover API
details, platform support, implementation decisions, and edge cases. Start with the task guides
for usage examples, or use the [Glossary](/docs/reference/glossary) to look up framework terminology.

Use the reference index below for app contracts. [Backend contracts](/contributing/backend-contracts),
[design records](/contributing/design), and [explanations](/docs/explanation) have separate indexes.

## Related projects

The [web view piece](https://github.com/daybrite/day-piece-webview) and
[Day Lite](https://github.com/daybrite/day-lite), for embedded JavaScript and TypeScript apps,
have their own repositories and documentation.

## Find a technical task

| Task | Procedure | Contract or explanation |
| --- | --- | --- |
| Fetch data and cancel work | [HTTP](/docs/how-to/guide-http) | [HTTP reference](/docs/reference/http), [async ownership](/docs/explanation/async) |
| Save settings or files | [Storage](/docs/how-to/guide-storage) | [Preferences](/docs/reference/prefs), [file storage](/docs/reference/fs) |
| Request consent and post reminders | [Permissions](/docs/how-to/guide-permissions), [notifications](/docs/how-to/guide-notifications) | [Permission states](/docs/reference/permissions), [notification defaults](/docs/reference/notify) |
| Share an action across UI surfaces | [Commands](/docs/how-to/guide-commands) | [Command ownership](/docs/reference/commands) |
| Open documents or another window | [Documents](/docs/how-to/guide-documents), [desktop UI](/docs/how-to/guide-desktop) | [File handling](/docs/reference/documents), [window defaults](/docs/reference/windows) |
| Localize text and bundle data | [Localization](/docs/how-to/localization), [resources](/docs/how-to/resources) | [Text](/docs/reference/text), [resource contract](/docs/reference/resources) |
| Make UI accessible | [Accessibility](/docs/how-to/accessibility) | [Annotations](/docs/reference/accessibility) |
| Diagnose or package an app | [Troubleshooting](/docs/how-to/troubleshooting), [packaging](/docs/how-to/packaging) | [CLI defaults](/docs/reference/cli#defaults-and-errors), [compatibility](/docs/reference/platforms) |

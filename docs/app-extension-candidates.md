---
title: "App extension candidates"
description: "Which Daybrite apps would gain from an iOS or macOS app extension, and which kind, now that Day.toml declares them."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# App extension candidates

[App extensions](app-extensions.md) let a Day app reach places outside its own window: the Home
Screen, the share sheet, Safari, Spotlight, the Files app. This page records, app by app, the
extensions that would do the most for each Daybrite app on iOS and macOS. None is built yet. The
order within each app is the suggested order to build them in.

Each needs data from the app, so each also needs an App Group shared by the app and the extension
([Sharing data with the app](app-extensions.md#sharing-data-with-the-app)): the app writes a small
snapshot (JSON or a SQLite file) into the group container, and the extension reads it.

| App | Extension | Kind | What it gives the person |
| --- | --- | --- | --- |
| App Fair | `MarketplaceExtension` (declared) | `marketplace` | Request headers, failed-request handling and automatic updates for MarketplaceKit installs, even while App Fair is closed. |
| App Fair | Updates widget | `widget` | The number of apps with updates, tapping through to the Updates page. |
| Day Skies | Forecast widget | `widget` | Today's conditions for a saved city on the Home Screen and the macOS desktop, drawn from the forecast the app already caches. |
| Day Skies | Weather intents | `app-intents` | "What's the weather in Valletta?" in Siri and Shortcuts without opening the app. |
| Day Trader | Watchlist widget | `widget` | The watchlist's day change at a glance, which is what the app opens on. |
| Day Trader | Price alert service | `notification-service` | Attaches a sparkline image to a price-alert push before it is shown, once the app has server-sent alerts. |
| Fleet Reader | Subscribe from Safari | `share` | "Subscribe in Fleet Reader" in the share sheet for the page or feed link being viewed, which the app's `feed:` URL scheme only covers for feed links. |
| Fleet Reader | Latest articles widget | `widget` | The newest unread headlines; tapping one opens it in the reader. |
| Fleet Reader | Spotlight index | `spotlight-index` | Lets Spotlight re-index saved articles after the system drops its index. |
| Net Skip | Open in Net Skip | `share` (or `action`) | Gives iOS the share-sheet entry Android already has (`share_target` in Day.toml is Android-only), opening a shared link in Net Skip. |
| Net Skip | Search widget | `widget` | A Home Screen field that opens a new tab with the search. |
| Net Skip | Content blocker | `content-blocker` | Net Skip's block list applied in Safari too. Net Skip's own web views would load the same JSON rule list through `WKContentRuleListStore`, so one list serves both. |
| Stanza Redux | EPUB previews | `quicklook-preview`, `quicklook-thumbnail` | Covers and a first-page preview for `.epub` files in Files, Finder and Mail (`QLSupportedContentTypes = ["org.idpf.epub-container"]`, the UTI the app already declares). |
| Stanza Redux | EPUB metadata in Spotlight | `spotlight-import` | Title and author search for books stored outside the app. |
| Stanza Redux | Reading widget | `widget` | The current book and its progress, opening at the last page read. |
| Day Sketch | Drawing thumbnails | `quicklook-thumbnail` | Real thumbnails for drawing files in Files and Finder instead of a generic document icon. |
| Day Tunes | Recently played widget | `widget` | Recent albums and stations; a control (iOS 18) to resume playback. |
| Day Tunes | Playback intents | `app-intents` | "Play <station> in Day Tunes" from Siri and Shortcuts. |
| Games Fair | Daily puzzle widget | `widget` | Today's Sudoku or Pipes puzzle, with the streak. |
| Games Fair | Stickers | `sticker-pack` | Game-piece stickers in Messages; needs artwork only, no code. |
| Day Hub | Actions status widget | `widget` (macOS) | The Actions status grid from the menu bar on the macOS desktop and in Notification Center. |
| Day Showcase | Extension demos | `widget`, `share` | One working example of each, as the catalog's reference for app developers, exercised in CI by the iOS walkthrough's install step. |

Day Travel and Day Rise are left out on purpose: Day Travel is a localization demo whose content
does not change, and Day Rise mirrors what `day new app` creates, which has no extensions.

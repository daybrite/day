# Platform-toolkit icons

One SVG per platform-toolkit id, rendered by `src/components/PlatformIcon.astro`. Each file is
vendored **exactly as fetched** — unedited, `<title>` and all — so its origin and license stay
checkable. The component strips what it does not need at render time rather than editing these.

| File | Source | Upstream name | License |
| --- | --- | --- | --- |
| `web-dom.svg` | [Material Symbols](https://fonts.google.com/icons) | `language` (outlined) | [Apache 2.0](https://www.apache.org/licenses/LICENSE-2.0) |
| `android-mdc.svg` | [Simple Icons](https://simpleicons.org) 16.28.0 | Android | [CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/) |
| `ios-uikit.svg` | Simple Icons 16.28.0 | iOS | CC0 1.0 |
| `harmony-arkui.svg` | Simple Icons 16.28.0 | HarmonyOS | CC0 1.0 |
| `macos-appkit.svg` | Simple Icons 16.28.0 | Apple | CC0 1.0 |
| `windows-xaml.svg` | [Font Awesome Free](https://fontawesome.com) 7.3.1 | `windows` (brands) | [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) |
| `linux-gtk.svg` | Simple Icons 16.28.0 | GNOME | CC0 1.0 |
| `linux-qt.svg` | Simple Icons 16.28.0 | KDE | CC0 1.0 |

## Why three sources

Simple Icons is the default: CC0 asks nothing of us, and it is the only one of the three that
carries GNOME and KDE, which is what `linux-gtk` and `linux-qt` need to say something more useful
than "Linux".

Three icons come from elsewhere:

- **`windows-xaml`** — Simple Icons carries no Microsoft mark. Microsoft's legal team asked for the
  removal of Windows, Office, and LinkedIn in 2024
  ([simple-icons#11236](https://github.com/simple-icons/simple-icons/issues/11236)). Font Awesome
  Free still ships one, under CC BY 4.0 rather than CC0 — so this icon is the one that carries an
  attribution requirement, satisfied by this file.
- **`ios-uikit`** — Simple Icons' `ios` is a wordmark, and reusing the Apple logo would make the
  iOS and macOS cards identical. A device silhouette distinguishes them. The outlined weight is
  deliberate: the filled `phone_iphone` reads as a solid blob at card size.
- **`web-dom`** — the HTML5 shield puts a large "5" on the card, which names a markup version
  rather than the platform. A globe says "the web" without dating itself.

`harmony-arkui` is the official HarmonyOS wordmark rather than Huawei's corporate flower: Day
targets OpenHarmony, an OpenAtom foundation project, so the vendor's mark would be inaccurate.

## Trademarks

These marks identify the platforms Day builds for. That is nominative use, and it grants no
affiliation or endorsement — every mark remains the property of its owner, whose brand guidelines
govern any use beyond identifying the platform.

## Updating

Refetch from the source rather than editing in place, and update the version in the table:

```sh
curl -o src/icons/linux-gtk.svg https://cdn.jsdelivr.net/npm/simple-icons@16.28.0/icons/gnome.svg
```

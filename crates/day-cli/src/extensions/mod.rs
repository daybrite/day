// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! App extensions (`[[extensions]]` in Day.toml, docs/app-extensions.md).
//!
//! An app extension is a separate bundle Apple platforms load into another process: a widget on
//! the Home Screen, a share sheet entry, a notification service, a Safari web extension, the
//! MarketplaceKit extension an alternative app marketplace ships. Day builds each one as its own
//! native target in the generated Xcode project, written in Swift and embedded in the app.
//!
//! The pieces of the contract, in the order a build meets them:
//!
//! * **Declaration** ([`ExtensionDecl`]): one `[[extensions]]` table per extension, a `kind`
//!   preset ([`KINDS`]) supplying the extension point, the principal class, the attributes and
//!   the entitlements the point needs, so a common extension is two lines of TOML.
//! * **Resolution** ([`resolve`]): per Apple platform (`ios`, `macos`), with the app's identity
//!   for the target (flavors included), its deployment target, and the defaults filled in.
//! * **Staging** ([`stage`]): the extension's `Info.plist` and entitlements, generated under
//!   `build/day/extensions/<os>/<Name>/` from the preset, the source folder's own `Info.plist`
//!   and Day.toml's overlays. Written only when the bytes change.
//! * **Project generation** ([`project::apply`]): one target per extension in the Xcode 27 JSON5
//!   project, its sources a synchronized folder (`platform/<os>/<Name>/`), embedded by the
//!   Runner target into `PlugIns/` (NSExtension) or `Extensions/` (ExtensionKit).
//! * **Signing** (`mobile.rs`, `sign.rs`, `pack/`): every embedded extension is signed before
//!   the app, with its own entitlements and, on a device or for distribution, its own profile.
//!
//! An extension never links the app's Rust code: the app's static library carries the UI toolkit
//! and `day_main`, and every extension point runs under a memory ceiling (a widget gets roughly
//! 30 MB) that a full Day runtime would not fit in. Extensions share data with the app through an
//! App Group container instead (`app-groups`).

pub mod project;
pub mod scaffold;
pub mod starters;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::meta::Project;

/// One Apple platform an extension can be built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Os {
    Ios,
    Macos,
}

impl Os {
    pub const ALL: [Os; 2] = [Os::Ios, Os::Macos];

    /// The platform directory under `platform/` and `build/day/`: `ios` or `macos`.
    pub fn key(self) -> &'static str {
        match self {
            Os::Ios => "ios",
            Os::Macos => "macos",
        }
    }

    /// The Day target that builds the app this extension is embedded in.
    pub fn target(self) -> &'static str {
        match self {
            Os::Ios => "ios-uikit",
            Os::Macos => "macos-appkit",
        }
    }

    /// The deployment-target build setting for this platform.
    pub fn deployment_setting(self) -> &'static str {
        match self {
            Os::Ios => "IPHONEOS_DEPLOYMENT_TARGET",
            Os::Macos => "MACOSX_DEPLOYMENT_TARGET",
        }
    }

    pub fn parse(s: &str) -> Option<Os> {
        match s {
            "ios" => Some(Os::Ios),
            "macos" => Some(Os::Macos),
            _ => None,
        }
    }

    /// The platform a `platform/<os>` key names, from the generator's string form.
    pub fn from_platform(platform: &str) -> Option<Os> {
        Os::parse(platform)
    }
}

/// How the system discovers and hosts the extension, which decides its product type, the
/// `Info.plist` dictionary that names the extension point, and where the app embeds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// An NSExtension app extension: `NSExtension` in its `Info.plist`, embedded in `PlugIns/`.
    Classic,
    /// An ExtensionKit extension: `EXAppExtensionAttributes`, embedded in `Extensions/`.
    ExtensionKit,
    /// An iMessage app: an app extension with the Messages product type.
    Messages,
    /// A sticker pack: an asset catalog with no executable.
    StickerPack,
}

impl Family {
    /// The Xcode product type, abbreviated the way project.xcproj writes it.
    pub fn product_type(self) -> &'static str {
        match self {
            Family::Classic => "app-extension",
            Family::ExtensionKit => "extensionkit-extension",
            Family::Messages => "app-extension.messages",
            Family::StickerPack => "app-extension.messages-sticker-pack",
        }
    }

    /// Whether the app embeds it in `Extensions/` (ExtensionKit) rather than `PlugIns/`.
    pub fn in_extensions_folder(self) -> bool {
        self == Family::ExtensionKit
    }

    /// The `Info.plist` key holding the extension point and its attributes.
    pub fn info_key(self) -> &'static str {
        match self {
            Family::ExtensionKit => "EXAppExtensionAttributes",
            _ => "NSExtension",
        }
    }

    fn point_key(self) -> &'static str {
        match self {
            Family::ExtensionKit => "EXExtensionPointIdentifier",
            _ => "NSExtensionPointIdentifier",
        }
    }

    fn principal_key(self) -> &'static str {
        match self {
            Family::ExtensionKit => "EXPrincipalClass",
            _ => "NSExtensionPrincipalClass",
        }
    }

    fn attributes_key(self) -> &'static str {
        match self {
            Family::ExtensionKit => "EXAppExtensionAttributes",
            _ => "NSExtensionAttributes",
        }
    }
}

/// A kind preset: what a `kind = "…"` line stands for. The extension point identifiers, principal
/// classes and attribute keys follow Xcode 27.2's own target templates
/// (`Platforms/*/Developer/Library/Xcode/Templates/Project Templates/*/Application Extension`).
#[derive(Debug, PartialEq)]
pub struct KindSpec {
    pub name: &'static str,
    /// One line for `day project add-extension --list` and the docs table.
    pub summary: &'static str,
    pub family: Family,
    /// The extension point on iOS, or `None` where the platform has none.
    pub ios: Option<&'static str>,
    /// The extension point on macOS, or `None`.
    pub macos: Option<&'static str>,
    /// The class the system instantiates, written unqualified; Swift classes are qualified with
    /// the extension's module (`$(PRODUCT_MODULE_NAME).ShareViewController`) when staged.
    pub principal: Option<&'static str>,
    /// The oldest iOS / macOS the preset's starter source and point need.
    pub ios_min: &'static str,
    pub macos_min: &'static str,
}

impl KindSpec {
    pub fn point(&self, os: Os) -> Option<&'static str> {
        match os {
            Os::Ios => self.ios,
            Os::Macos => self.macos,
        }
    }

    pub fn min(&self, os: Os) -> &'static str {
        match os {
            Os::Ios => self.ios_min,
            Os::Macos => self.macos_min,
        }
    }

    pub fn platforms(&self) -> Vec<Os> {
        Os::ALL
            .into_iter()
            .filter(|os| self.point(*os).is_some() || self.name == "custom")
            .collect()
    }
}

use Family::{Classic, ExtensionKit, Messages, StickerPack};

/// Every kind preset, in the order the docs list them.
pub const KINDS: &[KindSpec] = &[
    KindSpec {
        name: "widget",
        summary: "Home Screen, Lock Screen and desktop widgets, Live Activities and controls (WidgetKit)",
        family: Classic,
        ios: Some("com.apple.widgetkit-extension"),
        macos: Some("com.apple.widgetkit-extension"),
        principal: None,
        ios_min: "14.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "share",
        summary: "an entry in the share sheet that receives links, text or files",
        family: Classic,
        ios: Some("com.apple.share-services"),
        macos: Some("com.apple.share-services"),
        principal: Some("ShareViewController"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "action",
        summary: "an action in the share sheet that transforms or views content in place",
        family: Classic,
        ios: Some("com.apple.ui-services"),
        macos: Some("com.apple.ui-services"),
        principal: Some("ActionViewController"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "notification-service",
        summary: "rewrites a remote notification before it is shown (decryption, attachments)",
        family: Classic,
        ios: Some("com.apple.usernotifications.service"),
        macos: Some("com.apple.usernotifications.service"),
        principal: Some("NotificationService"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "notification-content",
        summary: "a custom interface for an expanded notification",
        family: Classic,
        ios: Some("com.apple.usernotifications.content-extension"),
        macos: Some("com.apple.usernotifications.content-extension"),
        principal: Some("NotificationViewController"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "safari-web-extension",
        summary: "a Safari web extension: a WebExtensions manifest, scripts and a native handler",
        family: Classic,
        ios: Some("com.apple.Safari.web-extension"),
        macos: Some("com.apple.Safari.web-extension"),
        principal: Some("SafariWebExtensionHandler"),
        ios_min: "15.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "content-blocker",
        summary: "a Safari content blocker: a JSON rule list Safari compiles and applies",
        family: Classic,
        ios: Some("com.apple.Safari.content-blocker"),
        macos: Some("com.apple.Safari.content-blocker"),
        principal: Some("ContentBlockerRequestHandler"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "keyboard",
        summary: "a custom system-wide keyboard",
        family: Classic,
        ios: Some("com.apple.keyboard-service"),
        macos: None,
        principal: Some("KeyboardViewController"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "intents",
        summary: "handles SiriKit intents (messaging, workouts, media) in the background",
        family: Classic,
        ios: Some("com.apple.intents-service"),
        macos: Some("com.apple.intents-service"),
        principal: Some("IntentHandler"),
        ios_min: "13.0",
        macos_min: "12.0",
    },
    KindSpec {
        name: "intents-ui",
        summary: "a custom interface Siri and Maps show for a SiriKit intent",
        family: Classic,
        ios: Some("com.apple.intents-ui-service"),
        macos: None,
        principal: Some("IntentViewController"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "app-intents",
        summary: "App Intents (Shortcuts, Siri, Spotlight actions) that run without launching the app",
        family: ExtensionKit,
        ios: Some("com.apple.appintents-extension"),
        macos: Some("com.apple.appintents-extension"),
        principal: None,
        ios_min: "16.0",
        macos_min: "13.0",
    },
    KindSpec {
        name: "file-provider",
        summary: "exposes the app's documents in the Files app or Finder (replicated File Provider)",
        family: Classic,
        ios: Some("com.apple.fileprovider-nonui"),
        macos: Some("com.apple.fileprovider-nonui"),
        principal: Some("FileProviderExtension"),
        ios_min: "16.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "file-provider-ui",
        summary: "custom actions on File Provider items in the Files app",
        family: Classic,
        ios: Some("com.apple.fileprovider-actionsui"),
        macos: None,
        principal: Some("FileProviderActionViewController"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "photo-editing",
        summary: "edits photos and videos from inside the Photos app",
        family: Classic,
        ios: Some("com.apple.photo-editing"),
        macos: Some("com.apple.photo-editing"),
        principal: Some("PhotoEditingViewController"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "broadcast-upload",
        summary: "receives a ReplayKit screen broadcast's audio and video samples",
        family: Classic,
        ios: Some("com.apple.broadcast-services-upload"),
        macos: Some("com.apple.broadcast-services-upload"),
        principal: Some("SampleHandler"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "broadcast-setup-ui",
        summary: "the sign-in or setup screen shown before a ReplayKit broadcast starts",
        family: Classic,
        ios: Some("com.apple.broadcast-services-setupui"),
        macos: Some("com.apple.broadcast-services-setupui"),
        principal: Some("BroadcastSetupViewController"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "call-directory",
        summary: "identifies or blocks incoming callers (CallKit)",
        family: Classic,
        ios: Some("com.apple.callkit.call-directory"),
        macos: None,
        principal: Some("CallDirectoryHandler"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "message-filter",
        summary: "files SMS and MMS from unknown senders into Junk or Promotions",
        family: Classic,
        ios: Some("com.apple.identitylookup.message-filter"),
        macos: None,
        principal: Some("MessageFilterExtension"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "credential-provider",
        summary: "offers the app's passwords and passkeys in AutoFill",
        family: Classic,
        ios: Some("com.apple.authentication-services-credential-provider-ui"),
        macos: Some("com.apple.authentication-services-credential-provider-ui"),
        principal: Some("CredentialProviderViewController"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "packet-tunnel",
        summary: "a VPN client tunnel (Network Extension)",
        family: Classic,
        ios: Some("com.apple.networkextension.packet-tunnel"),
        macos: Some("com.apple.networkextension.packet-tunnel"),
        principal: Some("PacketTunnelProvider"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "app-proxy",
        summary: "a per-app VPN proxy (Network Extension)",
        family: Classic,
        ios: Some("com.apple.networkextension.app-proxy"),
        macos: Some("com.apple.networkextension.app-proxy"),
        principal: Some("AppProxyProvider"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "dns-proxy",
        summary: "a system-wide DNS proxy (Network Extension, supervised devices)",
        family: Classic,
        ios: Some("com.apple.networkextension.dns-proxy"),
        macos: None,
        principal: Some("DNSProxyProvider"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "filter-data",
        summary: "the data half of a network content filter (Network Extension)",
        family: Classic,
        ios: Some("com.apple.networkextension.filter-data"),
        macos: None,
        principal: Some("FilterDataProvider"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "filter-control",
        summary: "the control half of a network content filter, which may update rules (Network Extension)",
        family: Classic,
        ios: Some("com.apple.networkextension.filter-control"),
        macos: None,
        principal: Some("FilterControlProvider"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "quicklook-preview",
        summary: "previews the app's document types in Quick Look",
        family: Classic,
        ios: Some("com.apple.quicklook.preview"),
        macos: Some("com.apple.quicklook.preview"),
        principal: Some("PreviewViewController"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "quicklook-thumbnail",
        summary: "draws thumbnails of the app's document types",
        family: Classic,
        ios: Some("com.apple.quicklook.thumbnail"),
        macos: Some("com.apple.quicklook.thumbnail"),
        principal: Some("ThumbnailProvider"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "spotlight-import",
        summary: "indexes the app's document types for Spotlight search",
        family: Classic,
        ios: Some("com.apple.spotlight.import"),
        macos: Some("com.apple.spotlight.import"),
        principal: Some("ImportExtension"),
        ios_min: "15.0",
        macos_min: "12.0",
    },
    KindSpec {
        name: "spotlight-index",
        summary: "re-indexes the app's searchable items when Spotlight asks (CoreSpotlight delegate)",
        family: Classic,
        ios: Some("com.apple.spotlight.index"),
        macos: Some("com.apple.spotlight.index"),
        principal: Some("IndexRequestHandler"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "imessage",
        summary: "an iMessage app in the Messages app drawer",
        family: Messages,
        ios: Some("com.apple.message-payload-provider"),
        macos: None,
        principal: Some("MessagesViewController"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "sticker-pack",
        summary: "a sticker pack for Messages, drawn from an asset catalog with no code",
        family: StickerPack,
        ios: Some("com.apple.message-payload-provider"),
        macos: None,
        principal: None,
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "location-push",
        summary: "answers a location request pushed from a server (Location Push Service)",
        family: Classic,
        ios: Some("com.apple.location.push.service"),
        macos: None,
        principal: Some("LocationPushService"),
        ios_min: "15.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "finder-sync",
        summary: "badges, context menu items and a toolbar button in Finder",
        family: Classic,
        ios: None,
        macos: Some("com.apple.FinderSync"),
        principal: Some("FinderSync"),
        ios_min: "13.0",
        macos_min: "11.0",
    },
    KindSpec {
        name: "marketplace",
        summary: "the MarketplaceKit extension of an alternative app marketplace",
        family: ExtensionKit,
        ios: Some("com.apple.marketplace.extension"),
        macos: None,
        principal: None,
        ios_min: "17.4",
        macos_min: "11.0",
    },
    KindSpec {
        name: "custom",
        summary: "any other extension point: set `point` (and `extensionkit` for an ExtensionKit point)",
        family: Classic,
        ios: None,
        macos: None,
        principal: None,
        ios_min: "13.0",
        macos_min: "11.0",
    },
];

pub fn kind_spec(name: &str) -> Option<&'static KindSpec> {
    KINDS.iter().find(|k| k.name == name)
}

/// A value that may differ per Apple platform: a bare value for both, or `{ ios = …, macos = … }`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum PerOs {
    One(String),
    Each {
        #[serde(default)]
        ios: Option<String>,
        #[serde(default)]
        macos: Option<String>,
    },
}

impl PerOs {
    pub fn get(&self, os: Os) -> Option<&str> {
        match self {
            PerOs::One(v) => Some(v),
            PerOs::Each { ios, macos } => match os {
                Os::Ios => ios.as_deref(),
                Os::Macos => macos.as_deref(),
            },
        }
    }
}

/// A Swift package an extension links, the shape `[package.metadata.day.ios] swift-packages`
/// takes: remote (`url` plus a version requirement) or local (`path`, relative to the app).
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SwiftPackageDecl {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub exact: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub products: Vec<String>,
}

impl SwiftPackageDecl {
    /// SwiftPM's identity: the last component of the URL (sans `.git`) or directory.
    pub fn identity(&self) -> String {
        let base = self.path.as_deref().or(self.url.as_deref()).unwrap_or("");
        base.trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(base)
            .trim_end_matches(".git")
            .to_string()
    }

    pub fn product_names(&self) -> Vec<String> {
        if self.products.is_empty() {
            vec![self.identity()]
        } else {
            self.products.clone()
        }
    }
}

/// One `[[extensions]]` table in Day.toml (docs/app-extensions.md).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct ExtensionDecl {
    /// The target, product (`<name>.appex`) and Swift module name.
    pub name: String,
    /// A preset from [`KINDS`]; `custom` with `point` for any other extension point.
    #[serde(default)]
    pub kind: Option<String>,
    /// The extension point identifier, overriding the preset's.
    #[serde(default)]
    pub point: Option<String>,
    /// For `kind = "custom"`: the point is an ExtensionKit one (`EXAppExtensionAttributes`,
    /// embedded in `Extensions/`) rather than an NSExtension one.
    #[serde(default)]
    pub extensionkit: Option<bool>,
    /// `ios`, `macos`, or both. Default: every Apple platform the app targets that the kind has.
    #[serde(default)]
    pub platforms: Option<Vec<String>>,
    /// The source folder, relative to the project. Default: `platform/<os>/<name>`.
    #[serde(default)]
    pub sources: Option<PerOs>,
    /// The bundle id: a suffix appended to the app's (`"Widgets"` → `<app id>.Widgets`), or a
    /// full id that starts with the app's. Default: the name.
    #[serde(default)]
    pub bundle_id: Option<String>,
    /// `CFBundleDisplayName`: what Settings and the share sheet call it. Default: the name.
    #[serde(default)]
    pub display_name: Option<String>,
    /// The oldest OS the extension runs on. Default: the kind's floor or the app's, whichever is
    /// later.
    #[serde(default)]
    pub deployment_target: Option<PerOs>,
    /// The class the system instantiates, overriding the preset's.
    #[serde(default)]
    pub principal_class: Option<String>,
    /// Keys merged into `NSExtensionAttributes` / `EXAppExtensionAttributes`.
    #[serde(default)]
    pub attributes: Option<toml::Table>,
    /// Keys merged into the top level of the extension's `Info.plist`.
    #[serde(default)]
    pub info: Option<toml::Table>,
    /// An entitlements plist to merge into the generated one, relative to the project.
    #[serde(default)]
    pub entitlements: Option<PerOs>,
    /// App Group identifiers shared with the app (`com.apple.security.application-groups`).
    #[serde(default)]
    pub app_groups: Vec<String>,
    /// System frameworks to link beyond what the Swift sources import.
    #[serde(default)]
    pub frameworks: Vec<String>,
    /// Swift packages the extension links.
    #[serde(default)]
    pub swift_packages: Vec<SwiftPackageDecl>,
    /// Widgets only: the widget bundle includes Live Activities, so the app's `Info.plist`
    /// declares `NSSupportsLiveActivities`.
    #[serde(default)]
    pub live_activities: bool,
    /// Extra target-level build settings, for anything this table does not model.
    #[serde(default)]
    pub build_settings: BTreeMap<String, String>,
}

/// An extension, resolved for one platform: everything generation, staging and signing need.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub name: String,
    pub kind: &'static KindSpec,
    pub os: Os,
    pub family: Family,
    pub point: String,
    /// The `PRODUCT_BUNDLE_IDENTIFIER` build setting: `$(inherited).<suffix>` for a suffix (so a
    /// flavor or a local override of the app's id carries over), the literal for a full id.
    pub bundle_setting: String,
    /// The bundle id for the app's identity as Day.toml resolves it, for signing lookups.
    pub bundle_id: String,
    /// The source folder, relative to the project root.
    pub sources: String,
    pub deployment_target: String,
    pub display_name: String,
    pub principal_class: Option<String>,
    pub attributes: ::plist::Dictionary,
    pub info: ::plist::Dictionary,
    /// The merged entitlements file named in Day.toml, relative to the project.
    pub entitlements_file: Option<String>,
    pub app_groups: Vec<String>,
    pub frameworks: Vec<String>,
    pub swift_packages: Vec<SwiftPackageDecl>,
    pub live_activities: bool,
    pub build_settings: BTreeMap<String, String>,
}

impl Resolved {
    /// `build/day/extensions/<os>/<Name>/`, relative to the project.
    pub fn staged_dir(&self) -> String {
        format!("build/day/extensions/{}/{}", self.os.key(), self.name)
    }

    pub fn staged_info_plist(&self) -> String {
        format!("{}/Info.plist", self.staged_dir())
    }

    pub fn staged_entitlements(&self) -> String {
        format!("{}/{}.entitlements", self.staged_dir(), self.name)
    }

    /// The bundle's file name inside the app: `<Name>.appex`.
    pub fn product_file(&self) -> String {
        format!("{}.appex", self.name)
    }

    pub fn has_executable(&self) -> bool {
        self.family != Family::StickerPack
    }

    /// A `Resources/` folder whose layout is copied verbatim into the bundle (web extension
    /// manifests, content blocker rule lists), outside the synchronized folder's flat copy.
    pub fn resources_dir(&self, root: &Path) -> Option<PathBuf> {
        let dir = root.join(&self.sources).join("Resources");
        dir.is_dir().then_some(dir)
    }
}

/// Every extension the project declares for `os`, resolved. An extension whose platforms leave
/// out `os` is skipped; one that cannot be resolved is an error naming it.
pub fn resolve(project: &Project, os: Os) -> Result<Vec<Resolved>, String> {
    let decls = &project.manifest.extensions;
    if decls.is_empty() {
        return Ok(Vec::new());
    }
    let app_id = project.manifest.resolve(os.target()).id;
    let app_floor = crate::xcconfig::deployment_target(project, os.key());
    let targets: Vec<Os> = project
        .manifest
        .app
        .targets
        .iter()
        .filter_map(|t| match t.as_str() {
            "ios-uikit" => Some(Os::Ios),
            "macos-appkit" => Some(Os::Macos),
            _ => None,
        })
        .collect();
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for decl in decls {
        let lower = decl.name.to_ascii_lowercase();
        if !seen.insert(lower) {
            return Err(format!(
                "Day.toml declares two extensions named {:?} (names must differ ignoring case)",
                decl.name
            ));
        }
        if let Some(r) = resolve_one(
            decl,
            os,
            &app_id,
            app_floor.as_deref(),
            &targets,
            &project.root,
        )? {
            out.push(r);
        }
    }
    // The system installs at most one Messages extension per app (installd refuses the app).
    let messages: Vec<&str> = out
        .iter()
        .filter(|r| matches!(r.family, Family::Messages | Family::StickerPack))
        .map(|r| r.name.as_str())
        .collect();
    if messages.len() > 1 {
        return Err(format!(
            "an app may embed one Messages extension (an iMessage app or a sticker pack), but \
             Day.toml declares {}",
            messages.join(" and ")
        ));
    }
    Ok(out)
}

/// The platforms an extension is built for: its explicit list, or every Apple platform the app
/// targets that the kind supports.
pub fn platforms_of(decl: &ExtensionDecl, app_targets: &[Os]) -> Result<Vec<Os>, String> {
    let spec = decl_kind(decl)?;
    match &decl.platforms {
        Some(list) => {
            let mut out = Vec::new();
            for p in list {
                let os = Os::parse(p).ok_or_else(|| {
                    format!(
                        "extension {:?}: unknown platform {p:?} (use \"ios\" or \"macos\")",
                        decl.name
                    )
                })?;
                if spec.name != "custom" && spec.point(os).is_none() && decl.point.is_none() {
                    return Err(format!(
                        "extension {:?}: a {} extension does not exist on {}",
                        decl.name,
                        spec.name,
                        os.key()
                    ));
                }
                if !out.contains(&os) {
                    out.push(os);
                }
            }
            Ok(out)
        }
        None => Ok(app_targets
            .iter()
            .copied()
            .filter(|os| spec.name == "custom" || spec.point(*os).is_some())
            .collect()),
    }
}

fn decl_kind(decl: &ExtensionDecl) -> Result<&'static KindSpec, String> {
    let name = decl
        .kind
        .as_deref()
        .unwrap_or(if decl.point.is_some() { "custom" } else { "" });
    if name.is_empty() {
        return Err(format!(
            "extension {:?} names no `kind` (one of: {})",
            decl.name,
            kind_names()
        ));
    }
    kind_spec(name).ok_or_else(|| {
        format!(
            "extension {:?}: unknown kind {name:?} (one of: {})",
            decl.name,
            kind_names()
        )
    })
}

pub fn kind_names() -> String {
    KINDS.iter().map(|k| k.name).collect::<Vec<_>>().join(", ")
}

/// System frameworks an extension is likely to import. The extension's name is also its Swift
/// module's, and a module named after a framework hides it: `import Intents` inside a target
/// named `Intents` imports the target itself.
const FRAMEWORK_MODULES: &[&str] = &[
    "AppIntents",
    "AppKit",
    "AuthenticationServices",
    "CallKit",
    "Combine",
    "CoreData",
    "CoreLocation",
    "CoreSpotlight",
    "ExtensionFoundation",
    "ExtensionKit",
    "FileProvider",
    "FileProviderUI",
    "FinderSync",
    "Foundation",
    "IdentityLookup",
    "Intents",
    "IntentsUI",
    "MarketplaceKit",
    "Messages",
    "NetworkExtension",
    "Photos",
    "PhotosUI",
    "Quartz",
    "QuickLook",
    "QuickLookThumbnailing",
    "ReplayKit",
    "SafariServices",
    "SwiftUI",
    "UIKit",
    "UniformTypeIdentifiers",
    "UserNotifications",
    "UserNotificationsUI",
    "WidgetKit",
];

/// Whether `name` can be a target, product and Swift module name.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name != "Runner"
        && !FRAMEWORK_MODULES.contains(&name)
}

fn valid_version(v: &str) -> bool {
    !v.is_empty()
        && v.split('.').count() <= 3
        && v.split('.')
            .all(|c| !c.is_empty() && c.chars().all(|d| d.is_ascii_digit()))
}

/// `a < b` for dotted versions, numerically per component.
pub fn version_lt(a: &str, b: &str) -> bool {
    let key = |v: &str| -> Vec<u32> {
        let mut k: Vec<u32> = v.split('.').map(|c| c.parse().unwrap_or(0)).collect();
        k.resize(3, 0);
        k
    };
    key(a) < key(b)
}

fn resolve_one(
    decl: &ExtensionDecl,
    os: Os,
    app_id: &str,
    app_floor: Option<&str>,
    app_targets: &[Os],
    root: &Path,
) -> Result<Option<Resolved>, String> {
    let name = &decl.name;
    if !valid_name(name) {
        return Err(format!(
            "extension {name:?}: the name must start with a letter and hold only ASCII letters, \
             digits and underscores (it names the Xcode target and the Swift module), and cannot \
             be \"Runner\" or the name of a system framework such as Intents or WidgetKit, \
             which the module would hide"
        ));
    }
    let spec = decl_kind(decl)?;
    if !platforms_of(decl, app_targets)?.contains(&os) {
        return Ok(None);
    }
    let family = if spec.name == "custom" {
        if decl.extensionkit.unwrap_or(false) {
            ExtensionKit
        } else {
            Classic
        }
    } else {
        if decl.extensionkit.is_some() {
            return Err(format!(
                "extension {name:?}: `extensionkit` applies only to kind = \"custom\"; the {} \
                 preset already decides it",
                spec.name
            ));
        }
        spec.family
    };
    let point = match (&decl.point, spec.point(os)) {
        (Some(p), _) => p.clone(),
        (None, Some(p)) => p.to_string(),
        (None, None) => {
            return Err(format!(
                "extension {name:?}: kind = \"custom\" needs `point`, the extension point \
                 identifier"
            ));
        }
    };
    let (bundle_setting, bundle_id) = match decl.bundle_id.as_deref() {
        None => (format!("$(inherited).{name}"), format!("{app_id}.{name}")),
        Some(id) if id.contains('.') => {
            let Some(rest) = id.strip_prefix(&format!("{app_id}.")) else {
                return Err(format!(
                    "extension {name:?}: bundle-id {id:?} must start with the app's id \
                     ({app_id}.), or be a suffix without dots"
                ));
            };
            if rest.is_empty() || rest.split('.').any(str::is_empty) {
                return Err(format!(
                    "extension {name:?}: bundle-id {id:?} has an empty component"
                ));
            }
            (id.to_string(), id.to_string())
        }
        Some(suffix) => {
            if suffix.is_empty()
                || !suffix
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-')
            {
                return Err(format!(
                    "extension {name:?}: bundle-id suffix {suffix:?} may hold only ASCII \
                     letters, digits and hyphens"
                ));
            }
            (
                format!("$(inherited).{suffix}"),
                format!("{app_id}.{suffix}"),
            )
        }
    };
    let sources = decl
        .sources
        .as_ref()
        .and_then(|s| s.get(os))
        .map(|s| s.trim_end_matches('/').to_string())
        .unwrap_or_else(|| format!("platform/{}/{name}", os.key()));
    if Path::new(&sources).is_absolute() || sources.split('/').any(|c| c == "..") {
        return Err(format!(
            "extension {name:?}: sources {sources:?} must be a folder inside the project"
        ));
    }
    let floor = spec.min(os);
    let deployment_target = match decl.deployment_target.as_ref().and_then(|d| d.get(os)) {
        Some(v) => {
            if !valid_version(v) {
                return Err(format!(
                    "extension {name:?}: deployment-target {v:?} is not a version like \"17.0\""
                ));
            }
            v.to_string()
        }
        None => match app_floor {
            Some(app) if valid_version(app) && version_lt(floor, app) => app.to_string(),
            _ => floor.to_string(),
        },
    };
    if let Some(p) = &decl.principal_class
        && (p.is_empty() || p.contains(char::is_whitespace))
    {
        return Err(format!(
            "extension {name:?}: principal-class {p:?} is not a class name"
        ));
    }
    if decl.live_activities && spec.name != "widget" {
        return Err(format!(
            "extension {name:?}: live-activities applies only to kind = \"widget\""
        ));
    }
    for group in &decl.app_groups {
        let ok = match os {
            // iOS accepts only `group.`-prefixed identifiers; macOS also takes the older
            // team-prefixed form (`TEAMID.name`).
            Os::Ios => group.starts_with("group.") && group.len() > "group.".len(),
            Os::Macos => group.contains('.') && !group.starts_with('.'),
        };
        if !ok {
            return Err(format!(
                "extension {name:?}: app group {group:?} must look like \"group.{app_id}\""
            ));
        }
    }
    for pkg in &decl.swift_packages {
        if pkg.url.is_some() == pkg.path.is_some() {
            return Err(format!(
                "extension {name:?}: each swift-packages entry needs exactly one of `url` or \
                 `path`"
            ));
        }
    }
    for fw in &decl.frameworks {
        if fw.is_empty() || !fw.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!(
                "extension {name:?}: framework {fw:?} is not a framework name"
            ));
        }
    }
    // Named in Day.toml, or by convention `<sources>/<name>.entitlements`.
    let entitlements_file = decl
        .entitlements
        .as_ref()
        .and_then(|e| e.get(os))
        .map(str::to_string)
        .or_else(|| {
            let conventional = format!("{sources}/{name}.entitlements");
            root.join(&conventional).is_file().then_some(conventional)
        });
    let principal_class = decl
        .principal_class
        .clone()
        .or_else(|| spec.principal.map(str::to_string));
    Ok(Some(Resolved {
        name: name.clone(),
        kind: spec,
        os,
        family,
        point,
        bundle_setting,
        bundle_id,
        sources,
        deployment_target,
        display_name: decl.display_name.clone().unwrap_or_else(|| name.clone()),
        principal_class,
        attributes: decl
            .attributes
            .as_ref()
            .map(toml_table_to_plist)
            .unwrap_or_default(),
        info: decl
            .info
            .as_ref()
            .map(toml_table_to_plist)
            .unwrap_or_default(),
        entitlements_file,
        app_groups: decl.app_groups.clone(),
        frameworks: decl.frameworks.clone(),
        swift_packages: decl.swift_packages.clone(),
        live_activities: decl.live_activities,
        build_settings: decl.build_settings.clone(),
    }))
}

// --- plist values -------------------------------------------------------------------------------

fn toml_to_plist(v: &toml::Value) -> ::plist::Value {
    match v {
        toml::Value::String(s) => ::plist::Value::String(s.clone()),
        toml::Value::Integer(i) => ::plist::Value::Integer((*i).into()),
        toml::Value::Float(f) => ::plist::Value::Real(*f),
        toml::Value::Boolean(b) => ::plist::Value::Boolean(*b),
        toml::Value::Datetime(d) => ::plist::Value::String(d.to_string()),
        toml::Value::Array(a) => ::plist::Value::Array(a.iter().map(toml_to_plist).collect()),
        toml::Value::Table(t) => ::plist::Value::Dictionary(toml_table_to_plist(t)),
    }
}

fn toml_table_to_plist(t: &toml::Table) -> ::plist::Dictionary {
    let mut d = ::plist::Dictionary::new();
    for (k, v) in t {
        d.insert(k.clone(), toml_to_plist(v));
    }
    d
}

/// Merge `over` into `base`: dictionaries merge key by key, anything else replaces.
pub(crate) fn deep_merge(base: &mut ::plist::Dictionary, over: &::plist::Dictionary) {
    for (k, v) in over {
        match (base.get_mut(k), v) {
            (Some(::plist::Value::Dictionary(b)), ::plist::Value::Dictionary(o)) => {
                deep_merge(b, o)
            }
            _ => {
                base.insert(k.clone(), v.clone());
            }
        }
    }
}

/// Sort every dictionary's keys, so the written plist is the same bytes for the same content.
fn sorted(v: ::plist::Value) -> ::plist::Value {
    match v {
        ::plist::Value::Dictionary(d) => ::plist::Value::Dictionary(sorted_dict(d)),
        ::plist::Value::Array(a) => ::plist::Value::Array(a.into_iter().map(sorted).collect()),
        other => other,
    }
}

fn sorted_dict(d: ::plist::Dictionary) -> ::plist::Dictionary {
    let mut entries: Vec<(String, ::plist::Value)> = d.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = ::plist::Dictionary::new();
    for (k, v) in entries {
        out.insert(k, sorted(v));
    }
    out
}

fn dict(entries: Vec<(&str, ::plist::Value)>) -> ::plist::Dictionary {
    let mut d = ::plist::Dictionary::new();
    for (k, v) in entries {
        d.insert(k.to_string(), v);
    }
    d
}

fn s(v: &str) -> ::plist::Value {
    ::plist::Value::String(v.to_string())
}

fn strings(v: &[&str]) -> ::plist::Value {
    ::plist::Value::Array(v.iter().map(|x| s(x)).collect())
}

/// What a kind adds to the plist beyond the point and principal class: attributes, keys inside
/// the extension dictionary, and top-level keys.
#[derive(Default)]
struct PlistPreset {
    attributes: ::plist::Dictionary,
    extension: ::plist::Dictionary,
    top: ::plist::Dictionary,
}

/// A share or action extension's default activation: one web page link, or text. Not
/// `TRUEPREDICATE`, which App Review rejects.
fn share_activation() -> ::plist::Value {
    ::plist::Value::Dictionary(dict(vec![
        ("NSExtensionActivationSupportsText", true.into()),
        (
            "NSExtensionActivationSupportsWebURLWithMaxCount",
            ::plist::Value::Integer(1.into()),
        ),
    ]))
}

fn preset(r: &Resolved, app_groups: &[String]) -> PlistPreset {
    let mut p = PlistPreset::default();
    match r.kind.name {
        "share" | "action" => {
            p.attributes
                .insert("NSExtensionActivationRule".into(), share_activation());
        }
        "notification-content" => {
            p.attributes
                .insert("UNNotificationExtensionCategory".into(), s(&r.name));
            p.attributes.insert(
                "UNNotificationExtensionInitialContentSizeRatio".into(),
                ::plist::Value::Real(1.0),
            );
        }
        "keyboard" => {
            p.attributes.insert("PrimaryLanguage".into(), s("en-US"));
            p.attributes.insert("IsASCIICapable".into(), false.into());
            p.attributes
                .insert("PrefersRightToLeft".into(), false.into());
            p.attributes
                .insert("RequestsOpenAccess".into(), false.into());
        }
        "intents" => {
            p.attributes.insert("IntentsSupported".into(), strings(&[]));
            p.attributes
                .insert("IntentsRestrictedWhileLocked".into(), strings(&[]));
        }
        "intents-ui" => {
            p.attributes.insert("IntentsSupported".into(), strings(&[]));
        }
        "file-provider" => {
            p.extension.insert(
                "NSExtensionFileProviderSupportsEnumeration".into(),
                true.into(),
            );
            if r.os == Os::Ios
                && let Some(group) = app_groups.first()
            {
                p.extension
                    .insert("NSExtensionFileProviderDocumentGroup".into(), s(group));
            }
        }
        "file-provider-ui" => {
            p.extension.insert(
                "NSExtensionFileProviderActions".into(),
                ::plist::Value::Array(Vec::new()),
            );
        }
        "photo-editing" => {
            p.attributes
                .insert("PHSupportedMediaTypes".into(), strings(&["Image"]));
        }
        "broadcast-upload" => {
            p.extension.insert(
                "RPBroadcastProcessMode".into(),
                s("RPBroadcastProcessModeSampleBuffer"),
            );
        }
        "broadcast-setup-ui" => {
            p.attributes.insert(
                "NSExtensionActivationRule".into(),
                ::plist::Value::Dictionary(dict(vec![(
                    "NSExtensionActivationSupportsReplayKitStreaming",
                    true.into(),
                )])),
            );
        }
        "quicklook-preview" => {
            p.attributes
                .insert("QLSupportedContentTypes".into(), strings(&[]));
            p.attributes
                .insert("QLSupportsSearchableItems".into(), false.into());
        }
        "quicklook-thumbnail" => {
            p.attributes
                .insert("QLSupportedContentTypes".into(), strings(&[]));
            p.attributes.insert(
                "QLThumbnailMinimumDimension".into(),
                ::plist::Value::Integer(0.into()),
            );
        }
        "spotlight-import" => {
            p.attributes
                .insert("CSSupportedContentTypes".into(), strings(&[]));
            p.top.insert("CSExtensionLabel".into(), s(&r.name));
        }
        "spotlight-index" => {
            p.top.insert("CSExtensionLabel".into(), s(&r.name));
        }
        "sticker-pack" => {
            // A Messages framework class: the pack has no code of its own to name.
            p.extension.insert(
                "NSExtensionPrincipalClass".into(),
                s("StickerBrowserViewController"),
            );
        }
        "finder-sync" => {
            p.attributes = ::plist::Dictionary::new();
            p.top.insert("LSUIElement".into(), true.into());
        }
        _ => {}
    }
    p
}

/// The principal class as the plist names it: a Swift class qualified with its module, unless
/// the declaration already qualified it (`MyModule.Handler`).
fn qualified_principal(r: &Resolved) -> Option<String> {
    let class = r.principal_class.as_ref()?;
    if class.contains('.') {
        Some(class.clone())
    } else {
        Some(format!("$(PRODUCT_MODULE_NAME).{class}"))
    }
}

/// The extension's complete `Info.plist`: the bundle keys every extension needs, the kind's
/// preset, the source folder's own `Info.plist` (when it has one), then Day.toml's `info` and
/// `attributes`, each layer merged over the one before.
pub fn info_plist(r: &Resolved, root: &Path) -> Result<::plist::Dictionary, String> {
    let mut d = dict(vec![
        ("CFBundleDevelopmentRegion", s("$(DEVELOPMENT_LANGUAGE)")),
        ("CFBundleDisplayName", s(&r.display_name)),
        ("CFBundleIdentifier", s("$(PRODUCT_BUNDLE_IDENTIFIER)")),
        ("CFBundleInfoDictionaryVersion", s("6.0")),
        ("CFBundleName", s("$(PRODUCT_NAME)")),
        ("CFBundlePackageType", s("$(PRODUCT_BUNDLE_PACKAGE_TYPE)")),
        ("CFBundleShortVersionString", s("$(MARKETING_VERSION)")),
        ("CFBundleVersion", s("$(CURRENT_PROJECT_VERSION)")),
    ]);
    if r.has_executable() {
        d.insert("CFBundleExecutable".into(), s("$(EXECUTABLE_NAME)"));
    }
    let preset = preset(r, &r.app_groups);
    let mut ext = dict(vec![(r.family.point_key(), s(&r.point))]);
    if let Some(class) = qualified_principal(r) {
        ext.insert(r.family.principal_key().into(), s(&class));
    }
    deep_merge(&mut ext, &preset.extension);
    let mut attributes = preset.attributes;
    deep_merge(&mut attributes, &r.attributes);
    match r.family {
        // ExtensionKit attributes live beside the point, in the one dictionary.
        Family::ExtensionKit => deep_merge(&mut ext, &attributes),
        _ => {
            if !attributes.is_empty() || r.kind.name == "finder-sync" {
                ext.insert(
                    r.family.attributes_key().into(),
                    ::plist::Value::Dictionary(attributes),
                );
            }
        }
    }
    d.insert(r.family.info_key().into(), ::plist::Value::Dictionary(ext));
    deep_merge(&mut d, &preset.top);
    if r.family == Family::StickerPack {
        d.insert("NSStickerSharingLevel".into(), s("OS"));
    }
    let own = root.join(&r.sources).join("Info.plist");
    if own.is_file() {
        let v = ::plist::Value::from_file(&own).map_err(|e| format!("{}: {e}", own.display()))?;
        let own = v
            .into_dictionary()
            .ok_or_else(|| format!("{}: not a dictionary plist", own.display()))?;
        deep_merge(&mut d, &own);
    }
    deep_merge(&mut d, &r.info);
    Ok(sorted_dict(d))
}

/// The kind's own entitlements: what its extension point refuses to load without.
fn kind_entitlements(r: &Resolved) -> ::plist::Dictionary {
    let ne = |value: &str| {
        dict(vec![(
            "com.apple.developer.networking.networkextension",
            strings(&[value]),
        )])
    };
    match r.kind.name {
        "credential-provider" => dict(vec![(
            "com.apple.developer.authentication-services.autofill-credential-provider",
            true.into(),
        )]),
        "packet-tunnel" => ne("packet-tunnel-provider"),
        "app-proxy" => ne("app-proxy-provider"),
        "dns-proxy" => ne("dns-proxy"),
        "filter-data" | "filter-control" => ne("content-filter-provider"),
        "location-push" => dict(vec![("com.apple.developer.location.push", true.into())]),
        _ => ::plist::Dictionary::new(),
    }
}

/// The extension's entitlements: the kind's, the App Groups, the App Sandbox every macOS
/// extension runs in, then the file Day.toml names merged over them. Always a dictionary, empty
/// when nothing applies: every extension target names a file, which is what keeps the app's
/// own entitlements from reaching it through a command-line setting.
pub fn entitlements(r: &Resolved, root: &Path) -> Result<::plist::Dictionary, String> {
    let mut d = kind_entitlements(r);
    if r.os == Os::Macos {
        d.insert("com.apple.security.app-sandbox".into(), true.into());
    }
    if !r.app_groups.is_empty() {
        d.insert(
            "com.apple.security.application-groups".into(),
            ::plist::Value::Array(r.app_groups.iter().map(|g| s(g)).collect()),
        );
    }
    if let Some(rel) = &r.entitlements_file {
        let path = root.join(rel);
        let v = ::plist::Value::from_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let own = v
            .into_dictionary()
            .ok_or_else(|| format!("{}: not a dictionary plist", path.display()))?;
        deep_merge(&mut d, &own);
    }
    Ok(sorted_dict(d))
}

fn xml(d: &::plist::Dictionary) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    ::plist::Value::Dictionary(d.clone())
        .to_writer_xml(&mut out)
        .map_err(|e| e.to_string())?;
    out.push(b'\n');
    Ok(out)
}

/// Write each extension's `Info.plist` and entitlements under `build/day/extensions/<os>/`, and
/// remove the folders of extensions no longer declared. Files are rewritten only when their
/// bytes change, so an unchanged extension stays up to date for Xcode's incremental build.
pub fn stage(project: &Project, os: Os) -> Result<Vec<Resolved>, String> {
    let exts = resolve(project, os)?;
    let base = project.root.join("build/day/extensions").join(os.key());
    for r in &exts {
        let dir = project.root.join(r.staged_dir());
        std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
        let info = info_plist(r, &project.root)?;
        if let Some(problem) = uninstallable(r, &info) {
            return Err(format!("extension {} ({}): {problem}", r.name, os.key()));
        }
        crate::pieces::write_bytes_if_changed(
            &project.root.join(r.staged_info_plist()),
            &xml(&info)?,
        )?;
        crate::pieces::write_bytes_if_changed(
            &project.root.join(r.staged_entitlements()),
            &xml(&entitlements(r, &project.root)?)?,
        )?;
    }
    if let Ok(rd) = std::fs::read_dir(&base) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !exts.iter().any(|r| r.name == name) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    Ok(exts)
}

/// Whether the staged files match what [`stage`] would write (`day prepare --check`).
pub fn check_staged(project: &Project, os: Os) -> Result<(), String> {
    for r in resolve(project, os)? {
        for (file, want) in [
            (r.staged_info_plist(), xml(&info_plist(&r, &project.root)?)?),
            (
                r.staged_entitlements(),
                xml(&entitlements(&r, &project.root)?)?,
            ),
        ] {
            if std::fs::read(project.root.join(&file)).ok().as_deref() != Some(want.as_slice()) {
                return Err(format!("{file} is missing or stale — run `day prepare`"));
            }
        }
    }
    Ok(())
}

// --- the host app ---------------------------------------------------------------------------------

/// Declare `NSSupportsLiveActivities` in the app's `Info.plist` when a widget extension carries
/// Live Activities. The key is only ever added: a developer who declared it by hand for some
/// other reason keeps it.
pub fn sync_host_info_plist(project: &Project, os: Os, plist_path: &Path) -> Result<(), String> {
    if os != Os::Ios {
        return Ok(());
    }
    let exts = resolve(project, os)?;
    if !exts.iter().any(|r| r.live_activities) {
        return Ok(());
    }
    let before = std::fs::read_to_string(plist_path)
        .map_err(|e| format!("{}: {e}", plist_path.display()))?;
    if before.contains("<key>NSSupportsLiveActivities</key>") {
        return Ok(());
    }
    let after = crate::plist::apply_value_key(
        &before,
        "NSSupportsLiveActivities",
        Some(&::plist::Value::Boolean(true)),
    )
    .map_err(|e| format!("{}: {e}", plist_path.display()))?;
    if after != before {
        std::fs::write(plist_path, after).map_err(|e| format!("{}: {e}", plist_path.display()))?;
    }
    Ok(())
}

// --- xcodebuild command-line settings ---------------------------------------------------------------
//
// A setting passed to xcodebuild on the command line reaches every target in the build, which is
// why Day passes the app's deployment-target floor and its macOS entitlements there: the SwiftPM
// package targets cannot be edited any other way. An extension target is the exception, with its
// own floor and its own entitlements. When the project has extensions, Day passes those settings
// as `$(DAY_TARGET_<SETTING>:default=<app value>)`: an extension target defines
// `DAY_TARGET_<SETTING>` and gets its own value, every other target gets the app's.

/// `SETTING=value`, or the per-target form when `os` has extensions.
pub fn cli_setting(setting: &str, value: &str, has_extensions: bool) -> String {
    if has_extensions {
        format!("{setting}=$(DAY_TARGET_{setting}:default={value})")
    } else {
        format!("{setting}={value}")
    }
}

// --- lint ---------------------------------------------------------------------------------------------

/// `day lint` findings for `[[extensions]]`.
pub fn lint(project: &Project) -> Vec<crate::lint::Finding> {
    use crate::lint::Finding;
    let mut out = Vec::new();
    if project.manifest.extensions.is_empty() {
        return out;
    }
    let apple = project
        .manifest
        .app
        .targets
        .iter()
        .any(|t| t == "ios-uikit" || t == "macos-appkit");
    if !apple {
        out.push(Finding {
            code: "day::lint::extension-unused",
            message: "Day.toml declares [[extensions]], but [app] targets has neither ios-uikit \
                      nor macos-appkit, so no extension is built"
                .into(),
            ..Default::default()
        });
    }
    for os in Os::ALL {
        let exts = match resolve(project, os) {
            Ok(e) => e,
            Err(e) => {
                out.push(Finding {
                    code: "day::lint::extension",
                    message: e,
                    ..Default::default()
                });
                return out;
            }
        };
        for r in &exts {
            let at = format!("{} ({})", r.name, os.key());
            if !project.root.join(&r.sources).is_dir() {
                out.push(Finding {
                    code: "day::lint::extension",
                    message: format!(
                        "extension {at}: its source folder {} does not exist (`day project \
                         add-extension {} --name {}` writes a starter)",
                        r.sources, r.kind.name, r.name
                    ),
                    ..Default::default()
                });
            }
            if version_lt(&r.deployment_target, r.kind.min(os)) {
                out.push(Finding {
                    code: "day::lint::extension",
                    message: format!(
                        "extension {at}: deployment-target {} is older than {} {}, the oldest a \
                         {} extension supports",
                        r.deployment_target,
                        if os == Os::Ios { "iOS" } else { "macOS" },
                        r.kind.min(os),
                        r.kind.name
                    ),
                    ..Default::default()
                });
            }
            if let Some(rel) = &r.entitlements_file
                && !project.root.join(rel).is_file()
            {
                out.push(Finding {
                    code: "day::lint::extension",
                    message: format!("extension {at}: entitlements file {rel} does not exist"),
                    ..Default::default()
                });
            }
            for pkg in &r.swift_packages {
                if let Some(p) = &pkg.path
                    && !project.root.join(p).join("Package.swift").is_file()
                {
                    out.push(Finding {
                        code: "day::lint::extension",
                        message: format!("extension {at}: swift package {p} has no Package.swift"),
                        ..Default::default()
                    });
                }
            }
            match info_plist(r, &project.root) {
                Err(e) => out.push(Finding {
                    code: "day::lint::extension",
                    message: format!("extension {at}: {e}"),
                    ..Default::default()
                }),
                Ok(info) => out.extend(
                    uninstallable(r, &info)
                        .map(|message| Finding {
                            code: "day::lint::extension",
                            message: format!("extension {at}: {message}"),
                            ..Default::default()
                        })
                        .into_iter()
                        .chain(incomplete(r, &info).into_iter().map(|message| Finding {
                            code: "day::lint::extension-incomplete",
                            message: format!("extension {at}: {message}"),
                            ..Default::default()
                        })),
                ),
            }
            out.extend(app_group_findings(project, r));
            if let Some(f) = provisioning_finding(project, r) {
                out.push(f);
            }
        }
    }
    out
}

/// What makes the system refuse to install the whole app, found before a build produces it.
pub fn uninstallable(r: &Resolved, info: &::plist::Dictionary) -> Option<String> {
    if !matches!(r.kind.name, "intents" | "intents-ui") {
        return None;
    }
    let supported = info
        .get("NSExtension")
        .and_then(::plist::Value::as_dictionary)
        .and_then(|e| e.get("NSExtensionAttributes"))
        .and_then(::plist::Value::as_dictionary)
        .and_then(|a| a.get("IntentsSupported"))
        .and_then(::plist::Value::as_array);
    match supported {
        Some(list) if !list.is_empty() => None,
        _ => Some(
            "attributes.IntentsSupported is empty, and the system refuses to install an app \
             whose Intents extension handles no intent; list the intent classes it handles, \
             such as [\"INSendMessageIntent\"]"
                .into(),
        ),
    }
}

/// Attribute values the preset leaves for the app to fill in, and values App Review rejects.
fn incomplete(r: &Resolved, info: &::plist::Dictionary) -> Vec<String> {
    let mut out = Vec::new();
    let ext = info
        .get(r.family.info_key())
        .and_then(::plist::Value::as_dictionary);
    let attrs = ext
        .and_then(|e| e.get(r.family.attributes_key()))
        .and_then(::plist::Value::as_dictionary);
    let empty = |key: &str| {
        attrs
            .and_then(|a| a.get(key))
            .and_then(::plist::Value::as_array)
            .is_some_and(Vec::is_empty)
    };
    for (kind, key, what) in [
        (
            "quicklook-preview",
            "QLSupportedContentTypes",
            "the document types (UTIs) it previews",
        ),
        (
            "quicklook-thumbnail",
            "QLSupportedContentTypes",
            "the document types (UTIs) it draws",
        ),
        (
            "spotlight-import",
            "CSSupportedContentTypes",
            "the document types (UTIs) it indexes",
        ),
    ] {
        if r.kind.name == kind && empty(key) {
            out.push(format!(
                "attributes.{key} is empty; list {what}, or the system never calls it"
            ));
        }
    }
    if attrs
        .and_then(|a| a.get("NSExtensionActivationRule"))
        .and_then(::plist::Value::as_string)
        == Some("TRUEPREDICATE")
    {
        out.push(
            "NSExtensionActivationRule is TRUEPREDICATE, which App Review rejects; describe the \
             content it accepts instead"
                .into(),
        );
    }
    out
}

/// App Groups an extension claims that the app's own entitlements leave out: the shared
/// container would then exist for the extension alone.
fn app_group_findings(project: &Project, r: &Resolved) -> Vec<crate::lint::Finding> {
    let claimed = entitlements(r, &project.root)
        .map(|d| groups_of(&d))
        .unwrap_or_default();
    if claimed.is_empty() {
        return Vec::new();
    }
    let app_groups: Vec<String> = match r.os {
        Os::Ios => crate::sign::ios_app_entitlements(project, "Debug")
            .ok()
            .flatten()
            .and_then(|e| ::plist::Value::from_file(&e.path).ok())
            .and_then(|v| v.into_dictionary())
            .map(|d| groups_of(&d))
            .unwrap_or_default(),
        Os::Macos => crate::sandbox::entitlements(project, crate::cli::Profile::Debug)
            .ok()
            .flatten()
            .and_then(|p| ::plist::Value::from_file(&p).ok())
            .and_then(|v| v.into_dictionary())
            .map(|d| groups_of(&d))
            .unwrap_or_default(),
    };
    claimed
        .iter()
        .filter(|g| !app_groups.contains(g))
        .map(|g| crate::lint::Finding {
            code: "day::lint::extension-app-group",
            message: format!(
                "extension {} ({}) shares the app group {g}, but the app's own entitlements do \
                 not claim it, so the two cannot see one container (docs/app-extensions.md, \
                 \"Sharing data with the app\")",
                r.name,
                r.os.key()
            ),
            ..Default::default()
        })
        .collect()
}

/// A macOS extension that claims a restricted entitlement (`com.apple.developer.*`: Network
/// Extension, AutoFill) cannot be signed ad hoc, so even a local build needs a team.
fn provisioning_finding(project: &Project, r: &Resolved) -> Option<crate::lint::Finding> {
    if r.os != Os::Macos {
        return None;
    }
    let ents = entitlements(r, &project.root).ok()?;
    let restricted: Vec<&String> = ents
        .keys()
        .filter(|k| k.starts_with("com.apple.developer."))
        .collect();
    if restricted.is_empty() {
        return None;
    }
    let team = ["DayApp.xcconfig", "DayApp.local.xcconfig"]
        .iter()
        .any(|f| {
            std::fs::read_to_string(project.root.join("platform/macos").join(f)).is_ok_and(|t| {
                t.lines().any(|l| {
                    let l = l.trim();
                    !l.starts_with("//")
                        && l.split_once('=').is_some_and(|(k, v)| {
                            k.trim() == "DEVELOPMENT_TEAM" && !v.trim().is_empty()
                        })
                })
            })
        });
    (!team).then(|| crate::lint::Finding {
        code: "day::lint::extension-provisioning",
        message: format!(
            "extension {} (macos) claims {}, which Xcode signs only with a development team and \
             profile, never ad hoc: set DEVELOPMENT_TEAM in platform/macos/DayApp.local.xcconfig \
             or the macOS build fails",
            r.name,
            restricted
                .iter()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ..Default::default()
    })
}

fn groups_of(d: &::plist::Dictionary) -> Vec<String> {
    d.get("com.apple.security.application-groups")
        .and_then(::plist::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(::plist::Value::as_string)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

// --- signing ------------------------------------------------------------------------------------------

/// The extension a built `.appex` came from, matched by its product file name.
pub fn for_bundle<'a>(exts: &'a [Resolved], appex: &Path) -> Option<&'a Resolved> {
    let file = appex.file_name()?.to_str()?;
    exts.iter().find(|r| r.product_file() == file)
}

/// Every `.appex` embedded in an app bundle, in `PlugIns/` and `Extensions/` (iOS layout) or
/// `Contents/PlugIns/` and `Contents/Extensions/` (macOS), sorted.
pub fn embedded(app: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in [
        "PlugIns",
        "Extensions",
        "Contents/PlugIns",
        "Contents/Extensions",
    ] {
        if let Ok(rd) = std::fs::read_dir(app.join(dir)) {
            out.extend(
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("appex")),
            );
        }
    }
    out.sort();
    out
}

/// The bundle id an `.appex` declares.
pub fn bundle_id_of(appex: &Path) -> Option<String> {
    let plist = if appex.join("Contents/Info.plist").is_file() {
        appex.join("Contents/Info.plist")
    } else {
        appex.join("Info.plist")
    };
    ::plist::Value::from_file(plist)
        .ok()?
        .as_dictionary()?
        .get("CFBundleIdentifier")?
        .as_string()
        .map(str::to_string)
}

#[cfg(test)]
mod tests;

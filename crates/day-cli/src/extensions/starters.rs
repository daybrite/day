// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Starter sources `day project add-extension` writes for each kind: the smallest Swift that
//! builds, runs and shows where the work goes. The tokens `__NAME__` (the extension's name, also
//! its module) and `__TITLE__` (its display name) are filled in when written.

use super::Os;

/// The files to write into the extension's source folder, as `(relative path, contents)`.
pub fn files(kind: &str, os: Os, name: &str, title: &str) -> Vec<(String, String)> {
    let fill = |s: &str| s.replace("__NAME__", name).replace("__TITLE__", title);
    raw(kind, os)
        .into_iter()
        .map(|(path, body)| (fill(path), fill(body)))
        .collect()
}

fn raw(kind: &str, os: Os) -> Vec<(&'static str, &'static str)> {
    let ios = os == Os::Ios;
    match kind {
        "widget" => vec![("__NAME__.swift", WIDGET)],
        "share" if ios => vec![("ShareViewController.swift", SHARE_IOS)],
        "share" => vec![("ShareViewController.swift", SHARE_MACOS)],
        "action" if ios => vec![("ActionViewController.swift", ACTION_IOS)],
        "action" => vec![("ActionViewController.swift", ACTION_MACOS)],
        "notification-service" => vec![("NotificationService.swift", NOTIFICATION_SERVICE)],
        "notification-content" if ios => {
            vec![("NotificationViewController.swift", NOTIFICATION_CONTENT_IOS)]
        }
        "notification-content" => {
            vec![(
                "NotificationViewController.swift",
                NOTIFICATION_CONTENT_MACOS,
            )]
        }
        "safari-web-extension" => vec![
            ("SafariWebExtensionHandler.swift", SAFARI_HANDLER),
            ("Resources/manifest.json", SAFARI_MANIFEST),
            ("Resources/background.js", SAFARI_BACKGROUND),
            ("Resources/content.js", SAFARI_CONTENT),
        ],
        "content-blocker" => vec![
            ("ContentBlockerRequestHandler.swift", CONTENT_BLOCKER),
            ("Resources/blockerList.json", BLOCKER_LIST),
        ],
        "keyboard" => vec![("KeyboardViewController.swift", KEYBOARD)],
        "intents" => vec![("IntentHandler.swift", INTENTS)],
        "intents-ui" => vec![("IntentViewController.swift", INTENTS_UI)],
        "app-intents" => vec![("AppIntents.swift", APP_INTENTS)],
        "file-provider" => vec![("FileProviderExtension.swift", FILE_PROVIDER)],
        "file-provider-ui" => vec![("FileProviderActionViewController.swift", FILE_PROVIDER_UI)],
        "photo-editing" if ios => vec![("PhotoEditingViewController.swift", PHOTO_EDITING_IOS)],
        "photo-editing" => vec![("PhotoEditingViewController.swift", PHOTO_EDITING_MACOS)],
        "broadcast-upload" => vec![("SampleHandler.swift", BROADCAST_UPLOAD)],
        "broadcast-setup-ui" if ios => {
            vec![("BroadcastSetupViewController.swift", BROADCAST_SETUP_IOS)]
        }
        "broadcast-setup-ui" => vec![("BroadcastSetupViewController.swift", BROADCAST_SETUP_MACOS)],
        "call-directory" => vec![("CallDirectoryHandler.swift", CALL_DIRECTORY)],
        "message-filter" => vec![("MessageFilterExtension.swift", MESSAGE_FILTER)],
        "credential-provider" if ios => vec![(
            "CredentialProviderViewController.swift",
            CREDENTIAL_PROVIDER_IOS,
        )],
        "credential-provider" => vec![(
            "CredentialProviderViewController.swift",
            CREDENTIAL_PROVIDER_MACOS,
        )],
        "packet-tunnel" => vec![("PacketTunnelProvider.swift", PACKET_TUNNEL)],
        "app-proxy" => vec![("AppProxyProvider.swift", APP_PROXY)],
        "dns-proxy" => vec![("DNSProxyProvider.swift", DNS_PROXY)],
        "filter-data" => vec![("FilterDataProvider.swift", FILTER_DATA)],
        "filter-control" => vec![("FilterControlProvider.swift", FILTER_CONTROL)],
        "quicklook-preview" if ios => vec![("PreviewViewController.swift", QL_PREVIEW_IOS)],
        "quicklook-preview" => vec![("PreviewViewController.swift", QL_PREVIEW_MACOS)],
        "quicklook-thumbnail" => vec![("ThumbnailProvider.swift", QL_THUMBNAIL)],
        "spotlight-import" => vec![("ImportExtension.swift", SPOTLIGHT_IMPORT)],
        "spotlight-index" => vec![("IndexRequestHandler.swift", SPOTLIGHT_INDEX)],
        "imessage" => vec![
            ("MessagesViewController.swift", IMESSAGE),
            ("Assets.xcassets/Contents.json", ASSET_CATALOG_CONTENTS),
            (
                "Assets.xcassets/iMessage App Icon.stickersiconset/Contents.json",
                STICKERS_ICON_SET,
            ),
        ],
        "sticker-pack" => vec![
            ("Stickers.xcassets/Contents.json", ASSET_CATALOG_CONTENTS),
            (
                "Stickers.xcassets/Sticker Pack.stickerpack/Contents.json",
                STICKER_PACK,
            ),
            (
                "Stickers.xcassets/iMessage App Icon.stickersiconset/Contents.json",
                STICKERS_ICON_SET,
            ),
        ],
        "location-push" => vec![("LocationPushService.swift", LOCATION_PUSH)],
        "finder-sync" => vec![("FinderSync.swift", FINDER_SYNC)],
        "marketplace" => vec![("MarketplaceProvider.swift", MARKETPLACE)],
        _ => vec![("__NAME__.swift", CUSTOM)],
    }
}

const WIDGET: &str = r#"import SwiftUI
import WidgetKit

// A widget reads what it shows from data the app leaves in a shared App Group container
// (docs/app-extensions.md, "Sharing data with the app"); the app asks WidgetKit to reload it
// after writing (`WidgetCenter.shared.reloadAllTimelines()`).

struct Entry: TimelineEntry {
    let date: Date
}

struct Provider: TimelineProvider {
    func placeholder(in context: Context) -> Entry {
        Entry(date: Date())
    }

    func getSnapshot(in context: Context, completion: @escaping (Entry) -> Void) {
        completion(Entry(date: Date()))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<Entry>) -> Void) {
        let now = Date()
        completion(Timeline(entries: [Entry(date: now)], policy: .after(now.addingTimeInterval(15 * 60))))
    }
}

struct EntryView: View {
    let entry: Entry

    var body: some View {
        VStack(alignment: .leading) {
            Text("__TITLE__").font(.headline)
            Text(entry.date, style: .time).font(.title2)
        }
    }
}

struct __NAME__Widget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "__NAME__", provider: Provider()) { entry in
            if #available(iOS 17.0, macOS 14.0, *) {
                EntryView(entry: entry).containerBackground(.fill.tertiary, for: .widget)
            } else {
                EntryView(entry: entry).padding()
            }
        }
        .configurationDisplayName("__TITLE__")
        .description("Shows the time it was last updated.")
    }
}

@main
struct __NAME__Bundle: WidgetBundle {
    var body: some Widget {
        __NAME__Widget()
    }
}
"#;

const SHARE_IOS: &str = r#"import UIKit

/// The share sheet's entry for this app. `extensionContext.inputItems` holds what was shared:
/// read the attachments' item providers, hand the result to the app through the App Group
/// container, then complete the request.
final class ShareViewController: UIViewController {
    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        let count = extensionContext?.inputItems.count ?? 0
        let label = UILabel()
        label.text = "\(count) item(s) to share with __TITLE__"
        let done = UIButton(type: .system)
        done.setTitle("Done", for: .normal)
        done.addTarget(self, action: #selector(finish), for: .primaryActionTriggered)
        let stack = UIStackView(arrangedSubviews: [label, done])
        stack.axis = .vertical
        stack.spacing = 12
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            stack.centerYAnchor.constraint(equalTo: view.centerYAnchor),
        ])
    }

    @objc private func finish() {
        extensionContext?.completeRequest(returningItems: nil, completionHandler: nil)
    }
}
"#;

const SHARE_MACOS: &str = r#"import AppKit

/// The share menu's entry for this app. `extensionContext.inputItems` holds what was shared.
final class ShareViewController: NSViewController {
    override func loadView() {
        let count = extensionContext?.inputItems.count ?? 0
        let label = NSTextField(labelWithString: "\(count) item(s) to share with __TITLE__")
        let done = NSButton(title: "Done", target: self, action: #selector(finish))
        let stack = NSStackView(views: [label, done])
        stack.orientation = .vertical
        stack.edgeInsets = NSEdgeInsets(top: 20, left: 20, bottom: 20, right: 20)
        view = stack
    }

    @objc private func finish() {
        extensionContext?.completeRequest(returningItems: nil, completionHandler: nil)
    }
}
"#;

const ACTION_IOS: &str = r#"import UIKit

/// An action on shared content: read `extensionContext.inputItems`, transform or show it, and
/// return the result (or nothing) when the person is done.
final class ActionViewController: UIViewController {
    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        let done = UIButton(type: .system)
        done.setTitle("Done", for: .normal)
        done.addTarget(self, action: #selector(finish), for: .primaryActionTriggered)
        done.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(done)
        NSLayoutConstraint.activate([
            done.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            done.centerYAnchor.constraint(equalTo: view.centerYAnchor),
        ])
    }

    @objc private func finish() {
        extensionContext?.completeRequest(returningItems: extensionContext?.inputItems, completionHandler: nil)
    }
}
"#;

const ACTION_MACOS: &str = r#"import AppKit

/// An action on shared content: read `extensionContext.inputItems`, transform or show it, and
/// return the result when the person is done.
final class ActionViewController: NSViewController {
    override func loadView() {
        let done = NSButton(title: "Done", target: self, action: #selector(finish))
        let stack = NSStackView(views: [NSTextField(labelWithString: "__TITLE__"), done])
        stack.orientation = .vertical
        stack.edgeInsets = NSEdgeInsets(top: 20, left: 20, bottom: 20, right: 20)
        view = stack
    }

    @objc private func finish() {
        extensionContext?.completeRequest(returningItems: extensionContext?.inputItems, completionHandler: nil)
    }
}
"#;

const NOTIFICATION_SERVICE: &str = r#"import UserNotifications

/// Runs before a remote notification with `"mutable-content": 1` is shown: decrypt it, rewrite
/// the text, or download an attachment, then hand the content back. The system allows about
/// 30 seconds before `serviceExtensionTimeWillExpire`.
final class NotificationService: UNNotificationServiceExtension {
    private var contentHandler: ((UNNotificationContent) -> Void)?
    private var bestAttempt: UNMutableNotificationContent?

    override func didReceive(
        _ request: UNNotificationRequest,
        withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        self.contentHandler = contentHandler
        guard let content = request.content.mutableCopy() as? UNMutableNotificationContent else {
            contentHandler(request.content)
            return
        }
        bestAttempt = content
        // Change `content` here.
        contentHandler(content)
    }

    override func serviceExtensionTimeWillExpire() {
        if let contentHandler, let bestAttempt {
            contentHandler(bestAttempt)
        }
    }
}
"#;

const NOTIFICATION_CONTENT_IOS: &str = r#"import UIKit
import UserNotifications
import UserNotificationsUI

/// The expanded view of a notification whose category is listed in
/// `attributes.UNNotificationExtensionCategory` in Day.toml.
final class NotificationViewController: UIViewController, UNNotificationContentExtension {
    private let label = UILabel()

    override func viewDidLoad() {
        super.viewDidLoad()
        label.numberOfLines = 0
        label.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(label)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: view.layoutMarginsGuide.leadingAnchor),
            label.trailingAnchor.constraint(equalTo: view.layoutMarginsGuide.trailingAnchor),
            label.centerYAnchor.constraint(equalTo: view.centerYAnchor),
        ])
    }

    func didReceive(_ notification: UNNotification) {
        label.text = notification.request.content.body
    }
}
"#;

const NOTIFICATION_CONTENT_MACOS: &str = r#"import AppKit
import UserNotifications
import UserNotificationsUI

/// The expanded view of a notification whose category is listed in
/// `attributes.UNNotificationExtensionCategory` in Day.toml.
final class NotificationViewController: NSViewController, UNNotificationContentExtension {
    private let label = NSTextField(wrappingLabelWithString: "")

    override func loadView() {
        view = label
    }

    func didReceive(_ notification: UNNotification) {
        label.stringValue = notification.request.content.body
    }
}
"#;

const SAFARI_HANDLER: &str = r#"import Foundation
import SafariServices

/// Answers `browser.runtime.sendNativeMessage` from the extension's scripts (Resources/).
final class SafariWebExtensionHandler: NSObject, NSExtensionRequestHandling {
    func beginRequest(with context: NSExtensionContext) {
        let request = context.inputItems.first as? NSExtensionItem
        let message = request?.userInfo?[SFExtensionMessageKey]
        let response = NSExtensionItem()
        response.userInfo = [SFExtensionMessageKey: ["echo": String(describing: message)]]
        context.completeRequest(returningItems: [response], completionHandler: nil)
    }
}
"#;

const SAFARI_MANIFEST: &str = r#"{
    "manifest_version": 3,
    "name": "__TITLE__",
    "description": "__TITLE__ for Safari.",
    "version": "1.0",
    "background": {
        "service_worker": "background.js"
    },
    "content_scripts": [
        {
            "js": ["content.js"],
            "matches": ["https://example.com/*"]
        }
    ],
    "permissions": ["nativeMessaging"]
}
"#;

const SAFARI_BACKGROUND: &str = r#"browser.runtime.onMessage.addListener((request, sender, sendResponse) => {
    if (request.greeting === "hello") {
        sendResponse({ farewell: "goodbye" });
    }
});
"#;

const SAFARI_CONTENT: &str = r#"browser.runtime.sendMessage({ greeting: "hello" }).then((response) => {
    console.log("__TITLE__ received", response);
});
"#;

const CONTENT_BLOCKER: &str = r#"import Foundation

/// Hands Safari the rule list in Resources/blockerList.json. Safari compiles it once and applies
/// it without running this code again; call
/// `SFContentBlockerManager.reloadContentBlocker(withIdentifier:)` from the app after the rules
/// change.
final class ContentBlockerRequestHandler: NSObject, NSExtensionRequestHandling {
    func beginRequest(with context: NSExtensionContext) {
        let item = NSExtensionItem()
        if let url = Bundle.main.url(forResource: "blockerList", withExtension: "json") {
            item.attachments = [NSItemProvider(contentsOf: url)].compactMap { $0 }
        }
        context.completeRequest(returningItems: [item], completionHandler: nil)
    }
}
"#;

const BLOCKER_LIST: &str = r#"[
    {
        "action": { "type": "block" },
        "trigger": { "url-filter": "example-tracker\\.js" }
    }
]
"#;

const KEYBOARD: &str = r#"import UIKit

/// A custom keyboard. Type through `textDocumentProxy`; show the globe key only when
/// `needsInputModeSwitchKey` says the system has no other way to switch keyboards.
final class KeyboardViewController: UIInputViewController {
    override func viewDidLoad() {
        super.viewDidLoad()
        let hello = UIButton(type: .system)
        hello.setTitle("Hello", for: .normal)
        hello.addTarget(self, action: #selector(typeHello), for: .primaryActionTriggered)
        let next = UIButton(type: .system)
        next.setTitle("🌐", for: .normal)
        next.addTarget(self, action: #selector(handleInputModeList(from:with:)), for: .allTouchEvents)
        next.isHidden = !needsInputModeSwitchKey
        let stack = UIStackView(arrangedSubviews: [next, hello])
        stack.spacing = 16
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            stack.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            view.heightAnchor.constraint(equalToConstant: 216),
        ])
    }

    @objc private func typeHello() {
        textDocumentProxy.insertText("Hello ")
    }
}
"#;

const INTENTS: &str = r#"import Intents

/// Returns the object that handles each SiriKit intent. List the intent classes it handles in
/// `attributes.IntentsSupported` in Day.toml.
final class IntentHandler: INExtension {
    override func handler(for intent: INIntent) -> Any {
        self
    }
}
"#;

const INTENTS_UI: &str = r#"import IntentsUI
import UIKit

/// The view Siri shows for an intent listed in `attributes.IntentsSupported` in Day.toml.
final class IntentViewController: UIViewController, INUIHostedViewControlling {
    func configureView(
        for parameters: Set<INParameter>,
        of interaction: INInteraction,
        interactiveBehavior: INUIInteractiveBehavior,
        context: INUIHostedViewContext,
        completion: @escaping (Bool, Set<INParameter>, CGSize) -> Void
    ) {
        completion(true, parameters, extensionContext?.hostedViewMaximumAllowedSize ?? .zero)
    }
}
"#;

const APP_INTENTS: &str = r#"import AppIntents

/// App Intents that Shortcuts, Siri and Spotlight run in this extension, without launching the app.
@main
struct IntentsHost: AppIntentsExtension {}

struct Ping__NAME__: AppIntent {
    static let title: LocalizedStringResource = "Ping __TITLE__"

    func perform() async throws -> some IntentResult & ProvidesDialog {
        .result(dialog: "__TITLE__ is here.")
    }
}
"#;

const FILE_PROVIDER: &str = r#"import FileProvider

/// A replicated File Provider: the system keeps a local copy of what `enumerator(for:request:)`
/// lists and asks for contents on demand. The app registers its domain with
/// `NSFileProviderManager.add(_:completionHandler:)`.
final class FileProviderExtension: NSObject, NSFileProviderReplicatedExtension {
    required init(domain: NSFileProviderDomain) {
        super.init()
    }

    func invalidate() {}

    func item(
        for identifier: NSFileProviderItemIdentifier,
        request: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, Error?) -> Void
    ) -> Progress {
        completionHandler(nil, NSFileProviderError(.noSuchItem))
        return Progress()
    }

    func fetchContents(
        for itemIdentifier: NSFileProviderItemIdentifier,
        version requestedVersion: NSFileProviderItemVersion?,
        request: NSFileProviderRequest,
        completionHandler: @escaping (URL?, NSFileProviderItem?, Error?) -> Void
    ) -> Progress {
        completionHandler(nil, nil, NSFileProviderError(.noSuchItem))
        return Progress()
    }

    func createItem(
        basedOn itemTemplate: NSFileProviderItem,
        fields: NSFileProviderItemFields,
        contents url: URL?,
        options: NSFileProviderCreateItemOptions = [],
        request: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, Error?) -> Void
    ) -> Progress {
        completionHandler(nil, [], false, NSError(domain: NSCocoaErrorDomain, code: NSFeatureUnsupportedError))
        return Progress()
    }

    func modifyItem(
        _ item: NSFileProviderItem,
        baseVersion version: NSFileProviderItemVersion,
        changedFields: NSFileProviderItemFields,
        contents newContents: URL?,
        options: NSFileProviderModifyItemOptions = [],
        request: NSFileProviderRequest,
        completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, Error?) -> Void
    ) -> Progress {
        completionHandler(nil, [], false, NSError(domain: NSCocoaErrorDomain, code: NSFeatureUnsupportedError))
        return Progress()
    }

    func deleteItem(
        identifier: NSFileProviderItemIdentifier,
        baseVersion version: NSFileProviderItemVersion,
        options: NSFileProviderDeleteItemOptions = [],
        request: NSFileProviderRequest,
        completionHandler: @escaping (Error?) -> Void
    ) -> Progress {
        completionHandler(NSError(domain: NSCocoaErrorDomain, code: NSFeatureUnsupportedError))
        return Progress()
    }

    func enumerator(
        for containerItemIdentifier: NSFileProviderItemIdentifier,
        request: NSFileProviderRequest
    ) throws -> NSFileProviderEnumerator {
        throw NSFileProviderError(.noSuchItem)
    }
}
"#;

const FILE_PROVIDER_UI: &str = r#"import FileProviderUI
import UIKit

/// Custom actions on File Provider items, listed as `NSExtensionFileProviderActions` under
/// `info.NSExtension` in Day.toml.
final class FileProviderActionViewController: FPUIActionExtensionViewController {
    override func prepare(forAction actionIdentifier: String, itemIdentifiers: [NSFileProviderItemIdentifier]) {
        extensionContext.completeRequest()
    }

    override func prepare(forError error: Error) {
        extensionContext.cancelRequest(withError: error)
    }
}
"#;

const PHOTO_EDITING_IOS: &str = r#"import Photos
import PhotosUI
import UIKit

/// Edits a photo from inside Photos: show the input, then return the edited rendering and the
/// adjustment data that lets the edit be resumed later.
final class PhotoEditingViewController: UIViewController, PHContentEditingController {
    private var input: PHContentEditingInput?

    func canHandle(_ adjustmentData: PHAdjustmentData) -> Bool {
        false
    }

    func startContentEditing(with contentEditingInput: PHContentEditingInput, placeholderImage: UIImage) {
        input = contentEditingInput
    }

    func finishContentEditing(completionHandler: @escaping (PHContentEditingOutput?) -> Void) {
        guard let input else {
            completionHandler(nil)
            return
        }
        completionHandler(PHContentEditingOutput(contentEditingInput: input))
    }

    var shouldShowCancelConfirmation: Bool {
        false
    }

    func cancelContentEditing() {}
}
"#;

const PHOTO_EDITING_MACOS: &str = r#"import AppKit
import Photos
import PhotosUI

/// Edits a photo from inside Photos: show the input, then return the edited rendering and the
/// adjustment data that lets the edit be resumed later.
final class PhotoEditingViewController: NSViewController, PHContentEditingController {
    private var input: PHContentEditingInput?

    override func loadView() {
        view = NSView()
    }

    func canHandle(_ adjustmentData: PHAdjustmentData) -> Bool {
        false
    }

    func startContentEditing(with contentEditingInput: PHContentEditingInput, placeholderImage: NSImage) {
        input = contentEditingInput
    }

    func finishContentEditing(completionHandler: @escaping (PHContentEditingOutput?) -> Void) {
        guard let input else {
            completionHandler(nil)
            return
        }
        completionHandler(PHContentEditingOutput(contentEditingInput: input))
    }

    var shouldShowCancelConfirmation: Bool {
        false
    }

    func cancelContentEditing() {}
}
"#;

const BROADCAST_UPLOAD: &str = r#"import ReplayKit

/// Receives the broadcast's samples: encode them and send them to the streaming service.
final class SampleHandler: RPBroadcastSampleHandler {
    override func broadcastStarted(withSetupInfo setupInfo: [String: NSObject]?) {}

    override func broadcastPaused() {}

    override func broadcastResumed() {}

    override func broadcastFinished() {}

    override func processSampleBuffer(_ sampleBuffer: CMSampleBuffer, with sampleBufferType: RPSampleBufferType) {}
}
"#;

const BROADCAST_SETUP_IOS: &str = r#"import ReplayKit
import UIKit

/// Shown before a broadcast starts: sign the person in, then hand ReplayKit the stream's URL.
final class BroadcastSetupViewController: UIViewController {
    func userDidFinishSetup() {
        guard let url = URL(string: "https://example.com/broadcast") else { return }
        extensionContext?.completeRequest(withBroadcast: url, setupInfo: nil)
    }

    func userDidCancelSetup() {
        extensionContext?.cancelRequest(withError: NSError(domain: "__NAME__", code: -1))
    }
}
"#;

const BROADCAST_SETUP_MACOS: &str = r#"import AppKit
import ReplayKit

/// Shown before a broadcast starts: sign the person in, then hand ReplayKit the stream's URL.
final class BroadcastSetupViewController: NSViewController {
    override func loadView() {
        view = NSView()
    }

    func userDidFinishSetup() {
        guard let url = URL(string: "https://example.com/broadcast") else { return }
        extensionContext?.completeRequest(withBroadcast: url, setupInfo: nil)
    }

    func userDidCancelSetup() {
        extensionContext?.cancelRequest(withError: NSError(domain: "__NAME__", code: -1))
    }
}
"#;

const CALL_DIRECTORY: &str = r#"import CallKit
import Foundation

/// Supplies blocked and identified numbers, in ascending order, when the app calls
/// `CXCallDirectoryManager.reloadExtension(withIdentifier:completionHandler:)`.
final class CallDirectoryHandler: CXCallDirectoryProvider {
    override func beginRequest(with context: CXCallDirectoryExtensionContext) {
        if !context.isIncremental {
            context.addIdentificationEntry(withNextSequentialPhoneNumber: 1_800_555_5555, label: "__TITLE__")
        }
        context.completeRequest()
    }
}
"#;

const MESSAGE_FILTER: &str = r#"import IdentityLookup

/// Files messages from unknown senders. Decide offline when you can; defer to the server named
/// in `attributes.ILMessageFilterExtensionNetworkURL` when you cannot.
final class MessageFilterExtension: ILMessageFilterExtension {}

extension MessageFilterExtension: ILMessageFilterQueryHandling {
    func handle(
        _ queryRequest: ILMessageFilterQueryRequest,
        context: ILMessageFilterExtensionContext,
        completion: @escaping (ILMessageFilterQueryResponse) -> Void
    ) {
        let response = ILMessageFilterQueryResponse()
        response.action = .none
        completion(response)
    }
}
"#;

const CREDENTIAL_PROVIDER_IOS: &str = r#"import AuthenticationServices

/// Offers the app's saved credentials in AutoFill. Read them from the App Group container or
/// the shared keychain group, never from the network while the sheet is up.
final class CredentialProviderViewController: ASCredentialProviderViewController {
    override func prepareCredentialList(for serviceIdentifiers: [ASCredentialServiceIdentifier]) {}

    @objc func cancel() {
        extensionContext.cancelRequest(
            withError: NSError(domain: ASExtensionErrorDomain, code: ASExtensionError.userCanceled.rawValue)
        )
    }
}
"#;

const CREDENTIAL_PROVIDER_MACOS: &str = r#"import AppKit
import AuthenticationServices

/// Offers the app's saved credentials in AutoFill. Read them from the App Group container or
/// the shared keychain group, never from the network while the sheet is up.
final class CredentialProviderViewController: ASCredentialProviderViewController {
    override func loadView() {
        view = NSView()
    }

    override func prepareCredentialList(for serviceIdentifiers: [ASCredentialServiceIdentifier]) {}

    @objc func cancel() {
        extensionContext.cancelRequest(
            withError: NSError(domain: ASExtensionErrorDomain, code: ASExtensionError.userCanceled.rawValue)
        )
    }
}
"#;

const PACKET_TUNNEL: &str = r#"import NetworkExtension

/// A VPN tunnel: connect to the server, apply the tunnel's network settings, then move packets
/// between `packetFlow` and the server.
final class PacketTunnelProvider: NEPacketTunnelProvider {
    override func startTunnel(options: [String: NSObject]?, completionHandler: @escaping (Error?) -> Void) {
        completionHandler(nil)
    }

    override func stopTunnel(with reason: NEProviderStopReason, completionHandler: @escaping () -> Void) {
        completionHandler()
    }
}
"#;

const APP_PROXY: &str = r#"import NetworkExtension

/// A per-app proxy: accept each new flow and relay it.
final class AppProxyProvider: NEAppProxyProvider {
    override func startProxy(options: [String: Any]?, completionHandler: @escaping (Error?) -> Void) {
        completionHandler(nil)
    }

    override func stopProxy(with reason: NEProviderStopReason, completionHandler: @escaping () -> Void) {
        completionHandler()
    }

    override func handleNewFlow(_ flow: NEAppProxyFlow) -> Bool {
        false
    }
}
"#;

const DNS_PROXY: &str = r#"import NetworkExtension

/// A DNS proxy: receive each DNS flow and answer it.
final class DNSProxyProvider: NEDNSProxyProvider {
    override func startProxy(options: [String: Any]? = nil, completionHandler: @escaping (Error?) -> Void) {
        completionHandler(nil)
    }

    override func stopProxy(with reason: NEProviderStopReason, completionHandler: @escaping () -> Void) {
        completionHandler()
    }

    override func handleNewFlow(_ flow: NEAppProxyFlow) -> Bool {
        false
    }
}
"#;

const FILTER_DATA: &str = r#"import NetworkExtension

/// Sees each new network flow and allows or drops it.
final class FilterDataProvider: NEFilterDataProvider {
    override func startFilter(completionHandler: @escaping (Error?) -> Void) {
        completionHandler(nil)
    }

    override func stopFilter(with reason: NEProviderStopReason, completionHandler: @escaping () -> Void) {
        completionHandler()
    }

    override func handleNewFlow(_ flow: NEFilterFlow) -> NEFilterNewFlowVerdict {
        .allow()
    }
}
"#;

const FILTER_CONTROL: &str = r#"import NetworkExtension

/// Updates the filter's rules; the only half of a content filter that may use the network.
final class FilterControlProvider: NEFilterControlProvider {
    override func startFilter(completionHandler: @escaping (Error?) -> Void) {
        completionHandler(nil)
    }

    override func stopFilter(with reason: NEProviderStopReason, completionHandler: @escaping () -> Void) {
        completionHandler()
    }
}
"#;

const QL_PREVIEW_IOS: &str = r#"import QuickLook
import UIKit

/// Previews the document types listed in `attributes.QLSupportedContentTypes` in Day.toml.
final class PreviewViewController: UIViewController, QLPreviewingController {
    private let label = UILabel()

    override func viewDidLoad() {
        super.viewDidLoad()
        label.frame = view.bounds
        label.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        label.textAlignment = .center
        view.addSubview(label)
    }

    func preparePreviewOfFile(at url: URL) async throws {
        label.text = url.lastPathComponent
    }
}
"#;

const QL_PREVIEW_MACOS: &str = r#"import AppKit
import Quartz

/// Previews the document types listed in `attributes.QLSupportedContentTypes` in Day.toml.
final class PreviewViewController: NSViewController, QLPreviewingController {
    private let label = NSTextField(labelWithString: "")

    override func loadView() {
        view = label
    }

    func preparePreviewOfFile(at url: URL) async throws {
        label.stringValue = url.lastPathComponent
    }
}
"#;

const QL_THUMBNAIL: &str = r#"import QuickLookThumbnailing

/// Draws thumbnails of the document types listed in `attributes.QLSupportedContentTypes` in
/// Day.toml.
final class ThumbnailProvider: QLThumbnailProvider {
    override func provideThumbnail(
        for request: QLFileThumbnailRequest,
        _ handler: @escaping (QLThumbnailReply?, Error?) -> Void
    ) {
        handler(QLThumbnailReply(contextSize: request.maximumSize, currentContextDrawing: { true }), nil)
    }
}
"#;

const SPOTLIGHT_IMPORT: &str = r#"import CoreSpotlight

/// Fills in Spotlight's attributes for a file of a type listed in
/// `attributes.CSSupportedContentTypes` in Day.toml.
final class ImportExtension: CSImportExtension {
    override func update(_ attributes: CSSearchableItemAttributeSet, forFileAt url: URL) throws {
        attributes.title = url.deletingPathExtension().lastPathComponent
    }
}
"#;

const SPOTLIGHT_INDEX: &str = r#"import CoreSpotlight

/// Re-indexes the app's searchable items when Spotlight loses its index or asks for some again.
final class IndexRequestHandler: CSIndexExtensionRequestHandler {
    override func searchableIndex(
        _ searchableIndex: CSSearchableIndex,
        reindexAllSearchableItemsWithAcknowledgementHandler acknowledgementHandler: @escaping () -> Void
    ) {
        acknowledgementHandler()
    }

    override func searchableIndex(
        _ searchableIndex: CSSearchableIndex,
        reindexSearchableItemsWithIdentifiers identifiers: [String],
        acknowledgementHandler: @escaping () -> Void
    ) {
        acknowledgementHandler()
    }
}
"#;

const IMESSAGE: &str = r#"import Messages
import UIKit

/// An iMessage app: compose an `MSMessage` and insert it into the conversation.
final class MessagesViewController: MSMessagesAppViewController {
    override func viewDidLoad() {
        super.viewDidLoad()
        let send = UIButton(type: .system)
        send.setTitle("Send __TITLE__", for: .normal)
        send.addTarget(self, action: #selector(compose), for: .primaryActionTriggered)
        send.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(send)
        NSLayoutConstraint.activate([
            send.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            send.centerYAnchor.constraint(equalTo: view.centerYAnchor),
        ])
    }

    @objc private func compose() {
        let layout = MSMessageTemplateLayout()
        layout.caption = "__TITLE__"
        let message = MSMessage()
        message.layout = layout
        activeConversation?.insert(message, completionHandler: nil)
    }
}
"#;

const ASSET_CATALOG_CONTENTS: &str = r#"{
  "info" : {
    "author" : "xcode",
    "version" : 1
  }
}
"#;

const STICKER_PACK: &str = r#"{
  "info" : {
    "author" : "xcode",
    "version" : 1
  },
  "properties" : {
    "grid-size" : "regular"
  },
  "stickers" : [ ]
}
"#;

const STICKERS_ICON_SET: &str = r#"{
  "images" : [ ],
  "info" : {
    "author" : "xcode",
    "version" : 1
  }
}
"#;

const LOCATION_PUSH: &str = r#"import CoreLocation

/// Answers a location request pushed from a server: get one location, send it, then call
/// `completion`.
final class LocationPushService: NSObject, CLLocationPushServiceExtension {
    func didReceiveLocationPushPayload(_ payload: [String: Any], completion: @escaping () -> Void) {
        completion()
    }

    func serviceExtensionWillTerminate() {}
}
"#;

const FINDER_SYNC: &str = r#"import AppKit
import FinderSync

/// Badges and menu items for the folders in `directoryURLs`.
final class FinderSync: FIFinderSync {
    override init() {
        super.init()
        FIFinderSyncController.default().directoryURLs = []
    }

    override var toolbarItemName: String {
        "__TITLE__"
    }

    override var toolbarItemToolTip: String {
        "__TITLE__"
    }

    override var toolbarItemImage: NSImage {
        NSImage(systemSymbolName: "star", accessibilityDescription: nil) ?? NSImage()
    }

    override func menu(for menuKind: FIMenuKind) -> NSMenu {
        NSMenu(title: "__TITLE__")
    }
}
"#;

const MARKETPLACE: &str = r#"import ExtensionFoundation
import Foundation
import MarketplaceKit

/// The MarketplaceKit extension: the system calls it, even while the marketplace app is not
/// running, to add headers to requests for the marketplace's server, to handle a failed request,
/// and to offer automatic updates.
@main
struct MarketplaceProvider: MarketplaceExtension {
    func additionalHeaders(for request: URLRequest, account: String) -> [String: String]? {
        nil
    }

    func availableAppVersions(forAppleItemIDs ids: [AppleItemID]) -> [AppVersion]? {
        nil
    }

    func requestFailed(with response: HTTPURLResponse) -> Bool {
        false
    }

    func automaticUpdates(for installedAppVersions: [AppVersion]) async throws -> [AutomaticUpdate] {
        []
    }
}
"#;

const CUSTOM: &str = r#"import Foundation

// The principal class (`principal-class` in Day.toml) or `@main` type for the extension point
// this extension declares goes here.
"#;

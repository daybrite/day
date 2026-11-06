# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Proposed publication copy. These strings are review content, not application UI."""
DRAFTS = {
'hero': [
('Keep the headline', [
'Native apps in Rust', 'One app.\nEvery platform.\nAll Day.',
'Build for iOS, Android, macOS, Windows, and Linux from one Rust project. Day brings your screens to each platform’s native UI toolkit, with tools for testing, translations, and packaging included.',
'Build your first app', 'See the apps →']),
('Lead with native UI', [
'Day · App framework', 'Rust at the core.\nNative on the screen.',
'Write your app in Rust. Give it the controls people already use on their phones and desktops. Day connects the two, so you can share the code while keeping the platform’s native UI.',
'Try the tutorial', 'Compare platforms →']),
('Lead with reach', [
'Day · Desktop and mobile', 'Your app.\nOn more devices.',
'A Windows build. An iPhone app. A Linux desktop. Day lets you bring the same Rust project to each, using native controls and shared app logic.',
'Create your first app', 'Browse the gallery →']),
('Lead with Rust', [
'Day · Native app development', 'Build it in Rust.\nMake it native.',
'Use Rust for the screens as well as the code behind them. Day builds desktop and mobile apps with native UI toolkits and keeps their tests, translations, and release packages in the same project.',
'Start building', 'See Day in use →']),
],
'shared': [
('Adding another platform', [
'', 'Add a platform. Keep your code.',
'Taking an app to another OS means another build to maintain. Day lets you share the screens and app logic, then write platform-specific code where it’s needed.',
'Share the screens, too', 'Define layout, state, and event handlers in Rust. Day connects your UI to the toolkit used by each build.', 'See the project structure →',
'Prepare each release', 'Build desktop installers and mobile packages from the same project. Use Day’s store tools to translate listings and submit updates.', 'Prepare a release →',
'Use what the platform offers', 'Add native controls and services through extensions. Choose a fallback on platforms where a feature isn’t available.', 'Use an extension →']),
('Small-team focus', [
'', 'More platforms for a small team',
'A separate UI for every OS is a lot of code to keep in sync. With Day, you can work on the app in one Rust project and check the results on each target.',
'Change a screen in one place', 'Keep UI code and app logic together. Each target connects that shared code to its platform toolkit.', 'How a project fits together →',
'Keep releases in the project', 'Prepare installers, mobile packages, and translated store listings with Day’s build and submission tools.', 'Publishing guide →',
'Make room for differences', 'A platform-specific feature doesn’t have to dictate the whole app. Add it through an extension and handle its absence on other targets.', 'Extension guide →']),
('Shared UI focus', [
'Desktop and mobile', 'Share more than the business logic',
'Day lets you reuse the interface as well as the data model. Your layouts and event handlers are Rust code, backed by the UI toolkit on each platform.',
'One place for UI changes', 'Compose screens with Rust functions. The same definitions can be used by your desktop and mobile builds.', 'Explore the app structure →',
'Packages for each target', 'Use the same project to prepare installers and mobile packages, then manage store listings and submissions.', 'See the release tools →',
'Native features when you need them', 'Extend the app with platform controls and services. Provide a fallback where the feature isn’t supported.', 'Add platform features →']),
('Product maintenance focus', [
'', 'Keep working on the same app',
'Adding a new target shouldn’t mean maintaining a second product. Day keeps shared screens, state, and behavior in Rust, with room for the differences between platforms.',
'Reuse the implementation', 'Keep layouts and event handlers alongside your app logic. Day supplies the connections to the native UI toolkits.', 'Inside a Day project →',
'Manage the release work', 'Build installers and mobile packages, translate store listings, and submit updates using tools in the project.', 'Release an app →',
'Keep platform-specific features', 'Use an extension for a native control or service. Your app decides what to offer on targets that don’t have it.', 'How extensions work →']),
],
'native': [
('Show how the API works', [
'', 'The UI is Rust code',
'A screen is a function returning impl Piece. Compose controls with function calls, configure them with builder methods, and attach closures to events. Your UI can call the same Cargo crates as the rest of the app.',
'Day uses AppKit on macOS, UIKit on iOS, Material Components on Android, and desktop toolkits on Windows and Linux. Standard controls use native widgets; composed controls and custom drawing have separate implementations. Appearance and capabilities vary by target.',
'Take the API tour →',
'Keep your libraries', 'Call your parsing, storage, and other Rust libraries from the UI. Check that their dependencies support each target you plan to ship.',
'Update what changed', 'A signal change reruns the bindings that read it and updates the affected controls. Value changes don’t rebuild the whole UI; structural changes can create or remove controls.',
'Build a native binary', 'Rust compiles your code to machine code for each desktop and mobile target, alongside Day’s toolkit integrations.']),
('Native behavior first', [
'', 'Use the platform’s controls',
'A Day text field on macOS is an AppKit text field. Its editing and selection behavior come from the toolkit. Your Rust code handles the app’s data and what happens when someone uses the control.',
'The other targets work with their platform toolkits, including UIKit and Android’s Material Components. Standard controls use toolkit widgets. Composed controls and custom drawing vary by implementation, so check the controls your app needs on each target.',
'See the Rust API →',
'Call your Rust code directly', 'Use Cargo crates for the app’s data model and libraries. The UI calls that code on the targets its dependencies support.',
'Bind controls to state', 'Signals notify the bindings that use them. Updating a label’s value keeps the existing control; replacing a screen can build new ones.',
'Compile for the target', 'Each desktop or mobile build contains compiled Rust code and the native toolkit integration for that target.']),
('State model first', [
'Rust UI', 'Change the state. Update the control.',
'Bind a label to a counter and update the counter in a button’s closure. Day updates the label when the value changes. The function that built the screen doesn’t have to run again.',
'Controls connect to AppKit, UIKit, Material Components, and the desktop toolkits on Windows and Linux. Standard widgets get their behavior from those toolkits. Composed controls and custom drawing have separate implementations, with differences between targets.',
'Explore signals and controls →',
'Use the crates you need', 'App logic stays in Cargo crates. Reuse your libraries wherever they and their dependencies support the target.',
'Keep updates local', 'Signal bindings update existing controls. Structural operations, such as replacing a page, can build and dispose subtrees.',
'Compile your app', 'Rust produces machine code for your desktop and mobile targets, built alongside Day’s platform integrations.']),
('Rust developer first', [
'', 'Write the screen in the language you use',
'Use functions for components, closures for event handlers, and signals for changing values. The screens and the rest of your app can share Rust types and call the same libraries.',
'Day connects that code to AppKit, UIKit, Material Components, and the desktop toolkits on Windows and Linux. Standard controls use native widgets. Custom drawing and composed controls have separate implementations; expect some differences in appearance and capability.',
'Read some UI code →',
'Bring your data model', 'Keep data handling and storage in Cargo crates and call them from the UI. Verify their dependencies on each target.',
'Follow changes with signals', 'Bindings rerun when the signals they read change. An ordinary value update doesn’t rebuild the screen; adding or replacing UI can create controls.',
'Build for desktop and mobile', 'Rust compiles to native machine code for each target. Day’s toolkit integrations are built with the app.']),
],
'workflow': [
('Repeatable testing', [
'', 'Test the same app on every target',
'Write down the interactions that matter, then run them against your builds. A DayScript walkthrough can check behavior and capture the same screens across targets, languages, and themes.',
'Work in VS Code →', 'Choose a target, build, and launch from the Day extension in your editor.',
'Check your setup →', 'Create the project with day new app. Run day doctor to find missing SDKs and build tools.',
'Assert behavior →', 'Find controls by stable IDs, perform actions, and check the results with DayScript.',
'Compare screenshots →', 'Capture the same point in a walkthrough in different languages and themes.',
'Prepare packages →', 'Build installers and mobile packages with day pack. Review signing status before distributing them.',
'Collect crash reports →', 'The crash-reporting extension records reports on the device. Users choose whether to send them by email, GitHub issue, or HTTP.',
'Each target needs its SDK. Signing credentials and store accounts are required for distribution.']),
('Day-to-day development', [
'', 'A workflow you can repeat',
'Create the project, run it, check the UI, and prepare a release. Day’s CLI handles the build steps; walkthrough scripts give you repeatable checks for the app.',
'Edit and run →', 'The VS Code extension keeps target selection, builds, and launches in the editor.',
'Set up the project →', 'day new app generates the project and platform host files. day doctor checks the tools you’ll need.',
'Test the UI →', 'Use stable control IDs in a DayScript walkthrough and run the same interactions on your targets.',
'Capture the result →', 'Add screenshots to the walkthrough to inspect translated layouts and theme changes.',
'Build a release →', 'day pack prepares installers and mobile packages and reports their signing status.',
'Investigate crashes →', 'Add on-device crash reports that users can choose to share by email, GitHub issue, or HTTP.',
'You’ll still need the platform SDKs, plus signing credentials and store accounts for distribution.']),
('Release confidence', [
'', 'Check the build you’re about to ship',
'A feature working on your laptop is a start. Use DayScript to exercise it on other targets, then inspect screenshots of the same flow before packaging the release.',
'Choose a target →', 'Build and launch from VS Code with the Day extension.',
'Check the tools →', 'Generate a project with day new app and check its SDK requirements with day doctor.',
'Repeat the flow →', 'A DayScript walkthrough locates controls by stable IDs and checks the app’s responses.',
'Inspect the screens →', 'Capture the flow in different languages and themes to review the layout.',
'Build the package →', 'Create installers and mobile packages with day pack and check their signing status.',
'Receive crash reports →', 'Users can send reports collected by the crash-reporting extension through email, GitHub issues, or HTTP.',
'Building requires each target’s SDK. Distribution also requires the appropriate signing credentials and store accounts.']),
('CLI and automation', [
'Development tools', 'Put the routine work in scripts',
'Day provides a common CLI for setup, builds, tests, and packaging. Keep a UI walkthrough with the app so you can repeat the checks as the code changes.',
'Editor integration →', 'Choose targets and run builds from the Day extension for VS Code.',
'Project setup →', 'day new app creates the project and host files. day doctor reports missing tools.',
'UI automation →', 'DayScript uses stable control IDs for actions and assertions across targets.',
'Screen captures →', 'Walkthroughs can capture screenshots for several languages and themes.',
'Release packages →', 'day pack creates installers and mobile packages. Check signing status before release.',
'Crash reporting →', 'Collect reports on the device and let users choose to submit them by email, GitHub issue, or HTTP.',
'The CLI uses the platform SDKs. Signing credentials and store accounts remain part of your release setup.']),
],
'library': [
('Build on existing code', ['', 'You’ve already written the hard part.', 'Keep your parser, database, or other Rust library and build a native UI around it. Day screens can call it directly. Check its dependencies on each target you want to support.', 'See the project structure →']),
('From tool to app', ['', 'Give your Rust tool a window', 'A command-line tool can be useful to people who never open a terminal. Use Day to add screens around its Rust library, keeping the existing logic in a Cargo crate. Verify the library’s dependencies on your chosen targets.', 'Build a UI around your library →']),
('Technical and direct', ['Existing Rust code', 'Keep the library. Add the interface.', 'Your data model can stay in its Cargo crate. Add a Day UI that calls it, then build for the desktop and mobile targets its dependencies support.', 'How a Day project fits together →']),
('Wider audience', ['', 'Let more people use your Rust code', 'Build a native app around the library you’ve been working on. Day gives you the UI code and build tools; your library continues to handle the work it was written for. Check its dependencies before adding a target.', 'Start with the app structure →']),
],
'languages': [
('Part of development', [
'', 'Build translation and accessibility into the app',
'Check translated screens', 'Keep messages in Fluent catalogs. Generated Rust accessors check names and arguments at build time. Run the same walkthrough in several languages to see how the text fits.', 'Translate an app →',
'Work with assistive technology', 'Native controls provide accessibility roles and actions. Add labels through Day’s shared API. On macOS and iOS, DayScript can check labels, roles, and values against the native accessibility tree.', 'Check accessibility support →']),
('Concrete user needs', [
'', 'A longer label. A screen reader. The same app.',
'Make room for other languages', 'Use Fluent catalogs and generated Rust accessors for translated text. Capture walkthroughs in different languages to catch clipped labels and crowded layouts.', 'Localization guide →',
'Give controls useful labels', 'Day exposes native accessibility roles and actions and lets you label controls. DayScript can verify labels, roles, and values through the accessibility tree on macOS and iOS.', 'Accessibility guide →']),
('Engineering focus', [
'Localization and accessibility', 'Check more than the default screen',
'Typed messages', 'Fluent catalogs generate Rust accessors, so message names and arguments are checked at build time. Use walkthroughs to review the rendered translations, too.', 'How localization works →',
'Native accessibility', 'Controls supply their toolkit’s roles and actions. Add your app’s labels through Day’s API; on macOS and iOS, test the resulting labels, roles, and values with DayScript.', 'Support and limitations →']),
('Reach more users', [
'', 'Make the app work for more people',
'Translate the interface', 'Put messages in Fluent catalogs and use generated Rust accessors in the UI. Check translated layouts by running your walkthroughs in several languages.', 'Add translations →',
'Support the way people navigate', 'Use native controls’ accessibility roles and actions, then supply meaningful labels. DayScript can inspect the native accessibility tree on macOS and iOS to verify labels, roles, and values.', 'Add accessibility labels →']),
],
'extensions': [
('Missing API', [
'', 'Need something Day doesn’t include?',
'Wrap the native API', 'Add a control or platform service in a Cargo crate. It can contain Swift, Java, ArkTS, or C++; Day’s build tools include the declared native sources and dependencies.', 'Write an extension →',
'Build from existing controls', 'Combine Day controls into a reusable Rust component. It uses the toolkit implementations of those controls and can be published as a crate.', 'Compose a component →',
'', 'The source is there when you need it', 'Day’s framework and build tools are open source under MPL-2.0. Inspect an implementation, test a change locally, or contribute a fix.', 'Browse the code →', 'Read MPL-2.0 →']),
('Native developer focus', [
'', 'Bring your platform code along',
'Use Swift, Java, ArkTS, or C++', 'Expose a native widget or service through a Rust API. Package the sources and dependencies in an extension crate for Day’s build tools to include.', 'Wrap a platform feature →',
'Share UI between apps', 'A component made from Day controls can live in a Cargo crate. It reuses their toolkit implementations.', 'Write a reusable component →',
'', 'Work on the framework, too', 'The framework and build tools use MPL-2.0. Read the toolkit code, try a local fix, and send it upstream.', 'Open the repository →', 'License details →']),
('Cargo ecosystem focus', [
'Extensions', 'Ship a component as a crate',
'A native control or service', 'Write a Rust API backed by the native code your feature needs. Day includes the extension’s declared sources and dependencies in the platform build.', 'Create a native extension →',
'A composition of Day controls', 'Combine existing controls in Rust and reuse the result across apps. Their toolkit implementations handle the underlying widgets.', 'Create a composite component →',
'', 'Inspect the implementation', 'Day’s source is available under MPL-2.0, including the toolkit integrations and build tools. You can debug into it and test changes locally.', 'Read the source →', 'Read the license →']),
('Practical customization', [
'', 'Add the part your app needs',
'Extend a platform integration', 'Wrap a native control or service behind a Rust API. An extension crate can include Swift, Java, ArkTS, and C++ sources with their build dependencies.', 'Extension guide →',
'Factor out a useful component', 'Put a group of Day controls in a reusable Rust component. Publish it as a crate when you want to use it in another app.', 'Component tutorial →',
'', 'Fix the framework when you need to', 'The framework and its tools are open source under MPL-2.0. If a bug blocks your app, you can inspect the code and test a fix in your checkout.', 'Find the code →', 'Check the license →']),
],
'start': [
('Tutorial first', ['Get your first window open', 'Build a session counter in the first tutorial. You’ll connect a button to some state and run the app on your machine.', 'Build the counter', 'Try Day Showcase']),
('Evaluate first', ['Try the controls before you choose', 'Run Day Showcase on your platform, then build the tutorial app to see what the code is like.', 'Build the tutorial', 'Download Day Showcase']),
('Project first', ['Start with a small app', 'The first tutorial takes you from project setup to a working counter. Use it to get the tools running before starting your app.', 'Start the tutorial', 'See the controls']),
('Code first', ['See how it feels to build with Day', 'Write a small Rust UI, run it, and change it. The session counter tutorial gives you a project to experiment with.', 'Try the tutorial', 'Explore Day Showcase']),
],
}

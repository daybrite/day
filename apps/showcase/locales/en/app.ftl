app-title = Day Showcase
counter-value = { $count ->
    [one] { $count } click
   *[other] { $count } clicks
}
decrement = −
increment = +
name-placeholder = Your name
greeting = Hello, { $name }!
volume-label = Volume
progress-label = Progress
busy-label = Busy
subscribe-a11y = Subscribe to updates
flavor-label = Flavor
history-title = History
history-entry = count became { $value }
nav-controls = Controls
nav-text = Text
nav-gauge = Gauge
nav-shapes = Shapes
nav-pickers = Pickers
nav-tabs = Tabs
nav-stack = Stack
nav-list = List
nav-webview = Web View
nav-lottie = Lottie
nav-about = About

shapes-kinds = Kinds
shapes-transform = Transform
shapes-angle = Angle
shapes-tap = Tap to recolor
shapes-drag = Drag to move

picker-segmented = Segmented
picker-menu = Menu
picker-inline = Inline
list-add = Add 100
list-caption = { $count } rows — only the visible cells are built

webview-url-hint = Enter a URL
webview-go = Go
webview-back = Back
webview-forward = Forward
webview-stop = Stop
webview-reload = Reload

lottie-caption = A native Lottie animation, bundled as JSON (lottie-ios / lottie-android)
lottie-speed = Speed
stack-root-body = A genuine push/pop stack. Its path is an app-owned signal.
stack-push = Push a detail
stack-detail-title = Level { $depth }
stack-detail-body = Pushed onto the path. The native back button writes the pop back.
tab-one = Overview
tab-two = Details
tab-three = Settings
tab-one-body = The overview tab. Each tab keeps its own state.
tab-two-body = The details tab, selected by its route key.
tab-three-body = The settings tab. Deep links and dayscript select tabs by key.
about-text = A native cross-platform app built with day.
nav-modals = Modals
modal-alert = Show alert
modal-confirm = Confirm
modal-delete = Delete…
modal-sheet = Pick flavor
modal-prompt = Enter name
alert-title = Notice
alert-body = Your changes have been saved.
ok = OK
confirm-title = Quit?
confirm-body = Are you sure you want to quit?
delete-title = Delete item?
delete-body = This cannot be undone.
delete = Delete
flavor-title = Choose a flavor
cancel = Cancel
vanilla = vanilla
pistachio = pistachio

# Text playground (typography)
text-caption = Semantic styles map to the platform's native text styles and accessibility text scaling.
text-styles-header = Styles
text-weights-header = Weights
text-styling-header = Bold & italic
text-colors-header = Color
text-custom-header = Custom sizes
text-custom-note = Font.System(pt) — still scaled by the accessibility text size (Dynamic Type / font scale).

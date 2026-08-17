# Day core UI catalog — standard strings the framework itself needs (dialog buttons, standard menu
# commands). Keys are namespaced `day-*` so an app's own catalog never clashes; an app CAN override
# any of these by defining the same key in its own `.ftl`. English is the ultimate fallback.

# Dialog buttons (docs/dialogs.md)
day-ok = OK
day-cancel = Cancel
day-yes = Yes
day-no = No
day-done = Done
day-save = Save
day-open = Open
day-close = Close
day-delete = Delete

# Standard menu commands (docs/menus.md — MenuRole)
day-cut = Cut
day-copy = Copy
day-paste = Paste
day-select-all = Select All
day-undo = Undo
day-redo = Redo
day-about = About
day-quit = Quit
day-preferences = Preferences
day-minimize = Minimize
day-fullscreen = Enter Full Screen

# Standard menu titles + app-name commands (used by the AppKit default/standard app menu)
day-edit = Edit
day-about-app = About {$app}
day-quit-app = Quit {$app}

# Window management (docs/windows.md — the Window menu, New Window, tab commands)
day-window = Window
day-new-window = New Window
day-zoom = Zoom
day-bring-all-front = Bring All to Front

# Settings pieces (docs/windows.md — day-piece-settings)
day-settings-language = Language
day-settings-theme = Appearance
day-theme-light = Light
day-theme-dark = Dark
day-theme-system = System
day-file = File
day-view = View
day-help = Help
day-services = Services
day-hide = Hide
day-hide-others = Hide Others
day-show-all = Show All

# The composed color picker (docs/colorpicker.md) — the panel Day draws itself on the two
# toolkits that ship no color chooser, and on any target that asks for one picker everywhere.
# These name the three drawn controls for a screen reader; the panel itself carries no visible
# labels, because a hue strip explains itself and a translated caption over every band does not.
day-color = Colors
day-color-hue = Hue
day-color-shade = Saturation and brightness
day-color-opacity = Opacity

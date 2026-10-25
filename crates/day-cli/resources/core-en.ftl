# Day core UI catalog: standard strings the framework itself needs (dialog buttons, standard menu
# commands). Keys are namespaced `day_*` so an app's own catalog never clashes; an app can override
# any of these by defining the same key in its own `.ftl`. English is the ultimate fallback.
#
# Every key must be a Rust identifier: day-localization's build script reads this file and generates
# `day_localization::key::<key>` and the typed `day::res::str::<key>()` from it (docs/localization.md),
# which is how code names these strings. Adding a message here adds its accessors.

# Dialog buttons (docs/dialogs.md)
day_ok = OK
day_cancel = Cancel
day_yes = Yes
day_no = No
day_done = Done
day_save = Save
day_open = Open
day_open_folder = Choose Folder
day_close = Close
day_delete = Delete

# Standard menu commands (docs/menus.md, MenuRole)
day_cut = Cut
day_copy = Copy
day_paste = Paste
day_select_all = Select All
day_undo = Undo
day_redo = Redo
day_about = About
day_quit = Quit
day_preferences = Preferences
day_minimize = Minimize
day_fullscreen = Enter Full Screen

# Standard menu titles + app-name commands (used by the AppKit default/standard app menu)
day_edit = Edit
day_about_app = About {$app}
day_quit_app = Quit {$app}

# Window management (docs/windows.md: the Window menu, New Window, tab commands)
day_window = Window
day_new_window = New Window
day_zoom = Zoom
day_bring_all_front = Bring All to Front

# Settings pieces (docs/windows.md, day-piece-settings)
day_settings_language = Language
day_settings_theme = Appearance
day_theme_light = Light
day_theme_dark = Dark
day_theme_system = System
day_toggle_sidebar = Toggle Sidebar
day_file = File
day_view = View
day_help = Help
# The web toolbar button that opens the app menu (docs/menus.md).
day_menu = Menu
day_services = Services
day_hide = Hide
day_hide_others = Hide Others
day_show_all = Show All

# The composed color picker (docs/colorpicker.md): the panel Day draws itself on the two
# toolkits that ship no color chooser, and on any target that asks for one picker everywhere.
# These name the three drawn controls for a screen reader; the panel itself carries no visible
# labels, because a hue strip explains itself and a translated caption over every band does not.
day_color = Colors
day_color_hue = Hue
day_color_shade = Saturation and brightness
day_color_opacity = Opacity

# Settings and About surfaces (docs/settings.md, day-piece-settings)
day_settings = Settings
day_check_for_updates = Check for Updates…
day_settings_general = General
day_settings_updates = Updates
day_about_version = Version {$version}
day_about_build = Build {$build}
day_about_website = Website
day_about_source = Source Code
day_about_privacy = Privacy Policy
day_about_licenses = Licenses
day_about_acknowledgements = Acknowledgements
day_settings_done = Done

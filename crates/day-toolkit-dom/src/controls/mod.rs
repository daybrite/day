mod colorpicker;
mod combobox;
mod datetime;
mod searchfield;
pub(crate) fn register() {
    searchfield::register();
    combobox::register();
    datetime::register();
    colorpicker::register();
}

mod detail;
mod navigate;
mod settings;
mod welcome;

pub(crate) use navigate::{delete_selected, done_selected, navigate_page, new_item};
pub(crate) use settings::{settings_body, settings_page};
pub(crate) use welcome::welcome_page;

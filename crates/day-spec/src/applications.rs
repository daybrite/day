// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! System application associations. Identifiers are opaque, platform-owned values.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    /// Persist this identifier to reopen with a chosen app. It may become stale after removal.
    pub id: String,
    /// Localized application name supplied by the OS, not an app translation key.
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandlerQuery {
    /// Absolute URL, including file URLs for existing documents.
    Url(String),
    /// Scheme without a colon, for example `https` or `mailto`.
    Scheme(String),
    MimeType(String),
    /// Extension without a leading dot, for example `pdf`.
    Extension(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApplicationHandlers {
    pub default: Option<Application>,
    /// Deduplicated applications, including the default if one exists.
    pub applications: Vec<Application>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationError {
    Unsupported,
    InvalidInput,
    NotFound,
    LaunchFailed,
}

/// May run on an OS completion queue; deliver to the UI executor before touching app state.
pub type OpenApplicationCompletion = Box<dyn FnOnce(Result<(), ApplicationError>) + Send>;

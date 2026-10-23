// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Targets with no store a secret may rely on: the web, and anything not named in `lib.rs`.
//! Every probe says so and every call fails with [`KeychainError::Unsupported`], so an app
//! linking this crate there gets a correct "no" and can keep its secret elsewhere.

use crate::{Entry, KeychainError, OnDone, Protection, Support};

pub fn support() -> Support {
    Support::Unsupported
}

pub fn protection_support(_protection: Protection) -> Support {
    Support::Unsupported
}

pub fn backend() -> &'static str {
    "none"
}

pub fn set(_key: &str, _secret: &[u8], _protection: Protection) -> Result<(), KeychainError> {
    Err(KeychainError::Unsupported)
}

pub fn get(_key: &str) -> Result<Vec<u8>, KeychainError> {
    Err(KeychainError::Unsupported)
}

pub fn get_with_presence(_key: String, _reason: String, on_done: OnDone) {
    on_done(Err(KeychainError::Unsupported));
}

pub fn remove(_key: &str) -> Result<(), KeychainError> {
    Err(KeychainError::Unsupported)
}

pub fn contains(_key: &str) -> Result<bool, KeychainError> {
    Err(KeychainError::Unsupported)
}

pub fn entries() -> Result<Vec<Entry>, KeychainError> {
    Err(KeychainError::Unsupported)
}

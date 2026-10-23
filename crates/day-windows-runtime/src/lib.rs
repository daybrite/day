// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Shared Windows services for Day toolkits and headless parts.
//!
//! This crate depends on Windows APIs, independently of WinUI, system XAML, GTK,
//! Qt, and the Day rendering tree. UI operations run on the calling UI thread.

#![cfg(windows)]

pub mod share;

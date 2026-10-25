// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

use day_spec::applications::*;
use objc2::rc::Retained;
use objc2_app_kit::{NSRunningApplication, NSWorkspace, NSWorkspaceOpenConfiguration};
use objc2_foundation::{NSArray, NSError, NSFileManager, NSString, NSURL};
use objc2_uniform_type_identifiers::UTType;
use std::sync::Mutex;

fn absolute_url(value: &str) -> Result<Retained<NSURL>, ApplicationError> {
    let url =
        NSURL::URLWithString(&NSString::from_str(value)).ok_or(ApplicationError::InvalidInput)?;
    if url.scheme().is_none() {
        return Err(ApplicationError::InvalidInput);
    }
    Ok(url)
}

fn application(url: &NSURL) -> Option<Application> {
    let id = url.absoluteString()?.to_string();
    let path = url.path()?;
    let name = NSFileManager::defaultManager()
        .displayNameAtPath(&path)
        .to_string();
    Some(Application {
        id,
        name: name.strip_suffix(".app").unwrap_or(&name).to_string(),
    })
}

pub(super) fn handlers(query: &HandlerQuery) -> Result<ApplicationHandlers, ApplicationError> {
    let workspace = NSWorkspace::sharedWorkspace();
    let (default, urls) = match query {
        HandlerQuery::Url(value) => {
            let url = absolute_url(value)?;
            (
                workspace.URLForApplicationToOpenURL(&url),
                workspace.URLsForApplicationsToOpenURL(&url),
            )
        }
        HandlerQuery::Scheme(scheme) => {
            if !scheme
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphabetic)
                || !scheme
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"+-.".contains(&c))
            {
                return Err(ApplicationError::InvalidInput);
            }
            // This is only a Launch Services lookup; no network request occurs.
            let url = absolute_url(&format!("{scheme}://example.invalid/"))?;
            (
                workspace.URLForApplicationToOpenURL(&url),
                workspace.URLsForApplicationsToOpenURL(&url),
            )
        }
        HandlerQuery::MimeType(value) | HandlerQuery::Extension(value) => {
            let content = match query {
                HandlerQuery::MimeType(_) => {
                    let valid = value.split_once('/').is_some_and(|(kind, subtype)| {
                        let token = |s: &str| {
                            !s.is_empty()
                                && s.bytes().all(|c| {
                                    c.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&c)
                                })
                        };
                        token(kind) && token(subtype)
                    });
                    if !valid {
                        return Err(ApplicationError::InvalidInput);
                    }
                    UTType::typeWithMIMEType(&NSString::from_str(value))
                }
                _ => {
                    if value.is_empty()
                        || value.contains(['.', '/', '\\'])
                        || value.chars().any(char::is_whitespace)
                    {
                        return Err(ApplicationError::InvalidInput);
                    }
                    UTType::typeWithFilenameExtension(&NSString::from_str(value))
                }
            };
            let Some(content) = content else {
                return Ok(ApplicationHandlers::default());
            };
            (
                workspace.URLForApplicationToOpenContentType(&content),
                workspace.URLsForApplicationsToOpenContentType(&content),
            )
        }
    };
    let default = default.as_deref().and_then(application);
    let mut applications: Vec<_> = urls.iter().filter_map(|url| application(&url)).collect();
    if let Some(app) = &default {
        applications.push(app.clone());
    }
    applications.sort_by(|a, b| a.id.cmp(&b.id));
    applications.dedup_by(|a, b| a.id == b.id);
    applications.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    Ok(ApplicationHandlers {
        default,
        applications,
    })
}

pub(super) fn open(url: &str, app: &str, completion: OpenApplicationCompletion) {
    let parsed = absolute_url(url).and_then(|url| {
        let app = absolute_url(app)?;
        if !app.isFileURL() {
            return Err(ApplicationError::InvalidInput);
        }
        if !app
            .path()
            .is_some_and(|p| NSFileManager::defaultManager().fileExistsAtPath(&p))
        {
            return Err(ApplicationError::NotFound);
        }
        Ok((url, app))
    });
    let (url, app) = match parsed {
        Ok(values) => values,
        Err(error) => {
            completion(Err(error));
            return;
        }
    };
    // AppKit invokes completion on a concurrent queue. The Send callback only resolves the
    // core oneshot; app UI code resumes on its original executor.
    let completion = Mutex::new(Some(completion));
    let block = block2::RcBlock::new(
        move |running: *mut NSRunningApplication, error: *mut NSError| {
            if let Some(complete) = completion.lock().unwrap_or_else(|e| e.into_inner()).take() {
                complete(if error.is_null() && !running.is_null() {
                    Ok(())
                } else {
                    Err(ApplicationError::LaunchFailed)
                });
            }
        },
    );
    NSWorkspace::sharedWorkspace().openURLs_withApplicationAtURL_configuration_completionHandler(
        &NSArray::from_slice(&[&*url]),
        &app,
        &NSWorkspaceOpenConfiguration::configuration(),
        Some(&block),
    );
}

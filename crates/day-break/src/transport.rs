//! Pluggable upload transports (docs/break.md). A [`Reporter`] is the ONLY path a report takes off
//! the device, and it is always driven by app code from a user action — day-break never uploads on
//! its own. Three built-ins cover the common shapes; an app can implement its own.

use crate::report::Report;

/// Why an upload did not succeed.
#[derive(Debug)]
pub enum SendError {
    /// The transport could not reach the destination (network/transport failure).
    Transport(String),
    /// The server accepted the request but answered with a non-success status.
    Rejected { status: u16 },
    /// The transport handed the report to the platform (browser / mail client) and cannot confirm
    /// delivery — the user completes it. Not really an error; reported so the UI can say so.
    HandedOff,
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::Transport(e) => write!(f, "could not send the report: {e}"),
            SendError::Rejected { status } => {
                write!(f, "the server rejected the report (HTTP {status})")
            }
            SendError::HandedOff => {
                write!(f, "handed the report to the platform to finish sending")
            }
        }
    }
}

impl std::error::Error for SendError {}

/// A crash-report upload transport. `describe` is shown to the user on the consent surface so they
/// know exactly where the report goes before they choose to send it.
pub trait Reporter: Send + Sync {
    /// A short name for the destination (e.g. `"our crash server"`, `"GitHub"`, `"email"`).
    fn name(&self) -> &str;
    /// A one-line disclosure of what sending does and where the report goes.
    fn describe(&self) -> String;
    /// Send `report`. `done` is called with the outcome (possibly on another thread).
    fn send(&self, report: &Report, done: Box<dyn FnOnce(Result<(), SendError>) + Send>);
}

/// POST the report JSON to an HTTP endpoint via the native stack (`day-part-http`), off the UI
/// thread. Also the shape a GitHub-issue **proxy** takes: the endpoint receives the report JSON and
/// opens the issue server-side (docs/break.md), keeping any repo token off the device.
pub struct RestReporter {
    url: String,
    name: String,
}

impl RestReporter {
    pub fn new(url: impl Into<String>) -> RestReporter {
        RestReporter {
            url: url.into(),
            name: "the crash server".into(),
        }
    }
    /// Override the display name shown on the consent surface.
    pub fn named(mut self, name: impl Into<String>) -> RestReporter {
        self.name = name.into();
        self
    }
}

impl Reporter for RestReporter {
    fn name(&self) -> &str {
        &self.name
    }
    fn describe(&self) -> String {
        format!("Uploads the report to {} ({}).", self.name, self.url)
    }
    fn send(&self, report: &Report, done: Box<dyn FnOnce(Result<(), SendError>) + Send>) {
        let url = self.url.clone();
        let body = report.to_json().into_bytes();
        // A blocking native fetch, moved off the UI thread.
        std::thread::spawn(move || {
            let req =
                day_part_http::Request::post(url, body).header("content-type", "application/json");
            let result = match day_part_http::fetch(&req) {
                Ok(resp) if (200..300).contains(&resp.status) => Ok(()),
                Ok(resp) => Err(SendError::Rejected {
                    status: resp.status,
                }),
                Err(e) => Err(SendError::Transport(e.to_string())),
            };
            done(result);
        });
    }
}

/// Open a prefilled "new issue" page in the browser (via the `open_url` toolkit duty). Zero
/// infrastructure and maximally consent-preserving: the user reviews and submits the issue on
/// GitHub themselves. The body is truncated to keep the URL within platform limits.
pub struct GithubIssueReporter {
    owner: String,
    repo: String,
    max_body: usize,
}

impl GithubIssueReporter {
    pub fn new(owner: impl Into<String>, repo: impl Into<String>) -> GithubIssueReporter {
        GithubIssueReporter {
            owner: owner.into(),
            repo: repo.into(),
            max_body: 6000,
        }
    }
}

impl Reporter for GithubIssueReporter {
    fn name(&self) -> &str {
        "GitHub"
    }
    fn describe(&self) -> String {
        format!(
            "Opens a prefilled new issue at github.com/{}/{} — you review and submit it.",
            self.owner, self.repo
        )
    }
    fn send(&self, report: &Report, done: Box<dyn FnOnce(Result<(), SendError>) + Send>) {
        let title = issue_title(report);
        let body = truncate(&report.display_text(), self.max_body);
        let url = format!(
            "https://github.com/{}/{}/issues/new?title={}&body={}",
            self.owner,
            self.repo,
            urlencode(&title),
            urlencode(&body),
        );
        day_core::open_url(&url);
        done(Ok(())); // open_url is fire-and-forget; the user completes the submit.
    }
}

/// Open the user's mail client with a prefilled message (via the `open_url` duty and a `mailto:`
/// URL). The user sends the mail. Body is truncated to a mail-client-friendly length.
pub struct EmailReporter {
    to: String,
    subject_prefix: String,
    max_body: usize,
}

impl EmailReporter {
    pub fn new(to: impl Into<String>) -> EmailReporter {
        EmailReporter {
            to: to.into(),
            subject_prefix: "Crash report".into(),
            max_body: 4000,
        }
    }
    pub fn subject_prefix(mut self, prefix: impl Into<String>) -> EmailReporter {
        self.subject_prefix = prefix.into();
        self
    }
}

impl Reporter for EmailReporter {
    fn name(&self) -> &str {
        "email"
    }
    fn describe(&self) -> String {
        format!(
            "Opens your mail app with a message to {} — you send it.",
            self.to
        )
    }
    fn send(&self, report: &Report, done: Box<dyn FnOnce(Result<(), SendError>) + Send>) {
        let subject = format!("{}: {}", self.subject_prefix, issue_title(report));
        let body = truncate(&report.display_text(), self.max_body);
        let url = format!(
            "mailto:{}?subject={}&body={}",
            self.to,
            urlencode(&subject),
            urlencode(&body)
        );
        day_core::open_url(&url);
        done(Ok(()));
    }
}

fn issue_title(report: &Report) -> String {
    let head = report.message.lines().next().unwrap_or("").trim();
    let head = truncate(head, 80);
    if head.is_empty() {
        format!("{} crash", report.kind_str)
    } else {
        format!("{}: {head}", report.kind_str)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    // Truncate on a char boundary.
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Percent-encode for a URL query/`mailto` value (RFC 3986 unreserved set stays literal).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Report;

    fn sample() -> Report {
        Report {
            kind_str: "panic".into(),
            message: "boom & bang: état".into(),
            ..Default::default()
        }
    }

    #[test]
    fn urlencode_escapes_reserved_and_unicode() {
        assert_eq!(urlencode("a b&c"), "a%20b%26c");
        // é is two UTF-8 bytes, both escaped.
        assert_eq!(urlencode("é"), "%C3%A9");
        assert_eq!(urlencode("Aa0-_.~"), "Aa0-_.~");
    }

    #[test]
    fn issue_title_summarizes_first_line() {
        let mut r = sample();
        r.message = "first line\nsecond".into();
        assert_eq!(issue_title(&r), "panic: first line");
        r.message = String::new();
        assert_eq!(issue_title(&r), "panic crash");
    }

    #[test]
    fn truncate_is_char_safe() {
        let s = "ééééé"; // 10 bytes
        let t = truncate(s, 5);
        assert!(t.ends_with('…'));
        // Never split a multibyte char.
        assert!(t.chars().all(|c| c == 'é' || c == '…'));
    }

    #[test]
    fn describe_names_destination() {
        assert!(
            RestReporter::new("https://x.dev/i")
                .describe()
                .contains("https://x.dev/i")
        );
        assert!(
            GithubIssueReporter::new("o", "r")
                .describe()
                .contains("o/r")
        );
        assert!(EmailReporter::new("a@b.c").describe().contains("a@b.c"));
    }
}

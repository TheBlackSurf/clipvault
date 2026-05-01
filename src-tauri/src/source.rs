use crate::models::SourceMetadata;
use scraper::{Html, Selector};
use std::process::Command;
use url::Url;

const PASSWORD_SOURCE_HINTS: &[&str] = &[
    "1password",
    "bitwarden",
    "dashlane",
    "keeper",
    "lastpass",
    "proton pass",
    "nordpass",
    "enpass",
    "keychain",
    "password",
    "haslo",
    "hasło",
];

const APP_SOURCE_SKIP_HINTS: &[&str] = &["clipvault"];

pub fn detect_source() -> SourceMetadata {
    #[cfg(target_os = "macos")]
    {
        detect_source_macos()
    }

    #[cfg(not(target_os = "macos"))]
    {
        SourceMetadata::default()
    }
}

pub fn should_skip_source(source: &SourceMetadata) -> bool {
    let mut haystack = String::new();
    for value in [
        source.app_name.as_deref(),
        source.window_title.as_deref(),
        source.page_title.as_deref(),
        source.page_url.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        haystack.push_str(value);
        haystack.push('\n');
    }

    let haystack = haystack.to_lowercase();
    APP_SOURCE_SKIP_HINTS
        .iter()
        .chain(PASSWORD_SOURCE_HINTS.iter())
        .any(|hint| haystack.contains(hint))
}

pub fn is_clipvault_app(app_name: &str) -> bool {
    let app_name = app_name.to_lowercase();
    APP_SOURCE_SKIP_HINTS
        .iter()
        .any(|hint| app_name.contains(hint))
}

pub fn domain_from_url(value: &str) -> Option<String> {
    Url::parse(value)
        .ok()
        .and_then(|url| url.host_str().map(normalize_domain))
}

pub fn clipboard_link_for_text(text: &str) -> Option<(String, Option<String>, Option<String>)> {
    let html = clipboard_html()?;
    link_from_html(&html, text)
}

fn normalize_domain(domain: &str) -> String {
    domain.trim_start_matches("www.").to_lowercase()
}

fn link_from_html(html: &str, text: &str) -> Option<(String, Option<String>, Option<String>)> {
    let document = Html::parse_fragment(html);
    let selector = Selector::parse("a[href]").ok()?;
    let needle = normalize_text(text);
    let mut fallback = None;

    for element in document.select(&selector) {
        let href = element.value().attr("href")?.trim();
        if !(href.starts_with("http://") || href.starts_with("https://")) {
            continue;
        }

        let label = normalize_text(&element.text().collect::<Vec<_>>().join(" "));
        let title = element
            .value()
            .attr("title")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(String::from);
        let domain = domain_from_url(href);
        let candidate = (href.to_string(), title, domain);

        if !needle.is_empty() && (label.contains(&needle) || needle.contains(&label)) {
            return Some(candidate);
        }

        if fallback.is_none() {
            fallback = Some(candidate);
        }
    }

    fallback
}

fn normalize_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(target_os = "macos")]
fn clipboard_html() -> Option<String> {
    let output = Command::new("pbpaste")
        .arg("-Prefer")
        .arg("html")
        .output()
        .ok()?;

    if output.status.success() {
        String::from_utf8(output.stdout)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| value.contains('<'))
    } else {
        None
    }
}

#[cfg(not(target_os = "macos"))]
fn clipboard_html() -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
pub fn activate_app(app_name: &str) -> Result<(), String> {
    let escaped = app_name.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(r#"tell application "{escaped}" to activate"#);
    run_osascript(&script)
        .map(|_| ())
        .ok_or_else(|| format!("Nie udało się przywrócić fokusa do aplikacji: {app_name}"))
}

#[cfg(not(target_os = "macos"))]
pub fn activate_app(_app_name: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn detect_source_macos() -> SourceMetadata {
    let base_script = r#"
tell application "System Events"
  set frontApp to name of first application process whose frontmost is true
  set windowTitle to ""
  try
    set windowTitle to name of front window of application process frontApp
  end try
end tell
return frontApp & linefeed & windowTitle
"#;

    let output = run_osascript(base_script);
    let mut source = SourceMetadata::default();
    if let Some(output) = output {
        let mut lines = output.lines();
        source.app_name = lines
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        source.window_title = lines
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
    }

    let Some(app_name) = source.app_name.clone() else {
        return source;
    };

    if let Some((url, title)) = browser_tab_for_macos(&app_name) {
        source.domain = domain_from_url(&url);
        source.page_url = Some(url);
        source.page_title = if title.trim().is_empty() {
            source.window_title.clone()
        } else {
            Some(title)
        };
    }

    source
}

#[cfg(target_os = "macos")]
fn browser_tab_for_macos(app_name: &str) -> Option<(String, String)> {
    let lower = app_name.to_lowercase();
    let script = if lower.contains("safari") {
        format!(
            r#"
tell application "{app_name}"
  try
    set tabUrl to URL of current tab of front window
    set tabTitle to name of current tab of front window
    return tabUrl & linefeed & tabTitle
  end try
end tell
"#
        )
    } else if [
        "google chrome",
        "chrome",
        "brave browser",
        "microsoft edge",
        "arc",
        "dia",
        "vivaldi",
        "opera",
        "chromium",
    ]
    .iter()
    .any(|name| lower.contains(name))
    {
        format!(
            r#"
tell application "{app_name}"
  try
    set tabUrl to URL of active tab of front window
    set tabTitle to title of active tab of front window
    return tabUrl & linefeed & tabTitle
  end try
end tell
"#
        )
    } else {
        return None;
    };

    let output = run_osascript(&script)?;
    let mut lines = output.lines();
    let url = lines.next()?.trim().to_string();
    let title = lines.next().unwrap_or("").trim().to_string();
    if url.starts_with("http://") || url.starts_with("https://") {
        Some((url, title))
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn run_osascript(script: &str) -> Option<String> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .ok()?;

    if output.status.success() {
        String::from_utf8(output.stdout).ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::link_from_html;

    #[test]
    fn extracts_matching_anchor_href_from_clipboard_html() {
        let html = r#"
          <div>
            <a href="https://example.com/general">General</a>
            <a href="https://example.com/video/123">Real teen</a>
          </div>
        "#;

        let (url, _title, domain) = link_from_html(html, "Real teen").expect("matching link");

        assert_eq!(url, "https://example.com/video/123");
        assert_eq!(domain.as_deref(), Some("example.com"));
    }
}

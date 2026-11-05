use crate::{CompilerDiagnostic, CompilerSpan, HotReloadFailure, ReloadPhase, YappinError};
use serde_json::Value;
use std::path::{Component, Path};

pub(super) const MAX_MESSAGE: usize = 2048;
const MAX_DIAGNOSTICS: usize = 32;
const MAX_SPANS: usize = 8;
const MAX_TEXT: usize = 32 * 1024;

// Diagnostics are compiler-controlled text. Do not forward rendered output,
// source excerpts, suggestions, raw stderr, environment or absolute paths.
struct Sanitizer {
    environment: Vec<String>,
    credentials: Vec<String>,
}
impl Sanitizer {
    fn new() -> Self {
        let mut credentials = Vec::new();
        let mut environment: Vec<_> = std::env::vars_os()
            .filter_map(|(key, value)| {
                let key = key.to_string_lossy().to_ascii_uppercase();
                let value = value.to_string_lossy().into_owned();
                let sensitive = ["TOKEN", "SECRET", "PASSWORD", "CREDENTIAL", "AUTH", "KEY"]
                    .iter()
                    .any(|part| key.contains(part));
                if sensitive && !value.is_empty() {
                    credentials.push(value.clone());
                }
                (!value.is_empty() && (value.len() >= 4 || sensitive)).then_some(value)
            })
            .collect();
        environment.sort_by_key(|value| std::cmp::Reverse(value.len()));
        environment.dedup();
        credentials.sort_by_key(|value| std::cmp::Reverse(value.len()));
        credentials.dedup();
        Self {
            environment,
            credentials,
        }
    }
    fn text(&self, input: &str) -> String {
        let mut text = input.to_owned();
        for value in &self.environment {
            text = text.replace(value, "[redacted]");
        }
        // Preserve punctuation/ordinary Rust type names while removing path-like
        // tokens. URLs are not compiler locations and are omitted as well.
        let mut output = String::new();
        let mut token = String::new();
        let flush = |token: &mut String, output: &mut String| {
            let absolute = token.contains('/') || token.contains('\\');
            if absolute {
                output.push_str("[path]");
            } else {
                output.push_str(token);
            }
            token.clear();
        };
        for ch in text.chars() {
            if ch.is_whitespace()
                || matches!(
                    ch,
                    '\'' | '"' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | '=' | ','
                )
            {
                flush(&mut token, &mut output);
                if !ch.is_control() || matches!(ch, '\n' | '\t') {
                    output.push(ch);
                }
            } else if !ch.is_control() {
                token.push(ch);
            }
        }
        flush(&mut token, &mut output);
        output
    }
}
fn shorten(mut text: String, limit: usize, truncated: &mut bool) -> String {
    if text.len() > limit {
        let mut end = limit;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        *truncated = true;
    }
    text
}

pub(super) struct CompilerReport<'a> {
    pub failure: HotReloadFailure,
    root: &'a Path,
    sanitizer: Sanitizer,
    count: usize,
    bytes: usize,
}
impl<'a> CompilerReport<'a> {
    pub fn new(root: &'a Path) -> Self {
        Self {
            failure: HotReloadFailure::new(
                ReloadPhase::Compilation,
                "Cargo did not produce a usable candidate",
            ),
            root,
            sanitizer: Sanitizer::new(),
            count: 0,
            bytes: 0,
        }
    }
    fn text(&mut self, text: &str) -> String {
        let text = self.sanitizer.text(text);
        let text = shorten(
            text,
            MAX_MESSAGE.min(MAX_TEXT.saturating_sub(self.bytes)),
            &mut self.failure.truncated,
        );
        self.bytes += text.len();
        text
    }
    fn diagnostic(&mut self, value: &Value, depth: usize) -> Option<CompilerDiagnostic> {
        if self.count >= MAX_DIAGNOSTICS || self.bytes >= MAX_TEXT || depth > 2 {
            self.failure.truncated = true;
            return None;
        }
        self.count += 1;
        let message = self.text(value["message"].as_str().unwrap_or("compiler diagnostic"));
        let code = value["code"]["code"]
            .as_str()
            .filter(|code| {
                code.len() <= 16 && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            })
            .map(str::to_owned);
        let level = match value["level"].as_str() {
            Some("error") => "error",
            Some("warning") => "warning",
            Some("help") => "help",
            Some("note") => "note",
            Some("failure-note") => "failure-note",
            _ => "unknown",
        }
        .into();
        let raw_spans = value["spans"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        if raw_spans.len() > MAX_SPANS {
            self.failure.truncated = true;
        }
        let spans = raw_spans
            .iter()
            .take(MAX_SPANS)
            .map(|span| {
                let file = span["file_name"].as_str().and_then(|name| {
                    let path = Path::new(name);
                    let relative = if path.is_absolute() {
                        path.strip_prefix(self.root).ok()?
                    } else {
                        path
                    };
                    if !relative
                        .components()
                        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
                        || name.contains('\\') && !cfg!(windows)
                        || name.contains(':') && !path.is_absolute()
                    {
                        return None;
                    }
                    let mut text = relative
                        .to_string_lossy()
                        .replace(std::path::MAIN_SEPARATOR, "/");
                    // Relative source locations are metadata, not arbitrary text.
                    // CI often has a non-secret branch variable equal to `main`.
                    for value in &self.sanitizer.credentials {
                        text = text.replace(value, "[redacted]");
                        if value.contains('\\') {
                            text = text.replace(&value.replace('\\', "/"), "[redacted]");
                        }
                    }
                    let text = shorten(
                        text,
                        MAX_MESSAGE.min(MAX_TEXT.saturating_sub(self.bytes)),
                        &mut self.failure.truncated,
                    );
                    self.bytes += text.len();
                    (!text.is_empty()).then_some(text)
                });
                CompilerSpan {
                    file,
                    line_start: span["line_start"].as_u64().unwrap_or(0),
                    line_end: span["line_end"].as_u64().unwrap_or(0),
                    column_start: span["column_start"].as_u64().unwrap_or(0),
                    column_end: span["column_end"].as_u64().unwrap_or(0),
                    is_primary: span["is_primary"].as_bool().unwrap_or(false),
                }
            })
            .collect();
        let children = value["children"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(|child| self.diagnostic(child, depth + 1))
            .collect();
        Some(CompilerDiagnostic {
            code,
            level,
            message,
            spans,
            children,
        })
    }
    pub fn push(&mut self, value: &Value) {
        if let Some(diagnostic) = self.diagnostic(value, 0) {
            self.failure.diagnostics.push(diagnostic);
        }
    }
}

pub(crate) fn error(
    phase: ReloadPhase,
    message: &str,
    cause: Option<&YappinError>,
    started: bool,
    publication: bool,
    client: Option<&crate::Client>,
) -> YappinError {
    let mut failure = HotReloadFailure::new(phase, message);
    failure.candidate_started = started;
    failure.publication_attempted = publication;
    if let Some(cause) = cause {
        let mut value = cause.to_json();
        if let Some(client) = client {
            value = client.redact_credentials(value);
        }
        let mut source: crate::RemoteError = serde_json::from_value(value).expect("error envelope");
        let sanitizer = Sanitizer::new();
        let mut bytes = 0;
        fn clean(
            value: &mut Value,
            sanitizer: &Sanitizer,
            bytes: &mut usize,
            truncated: &mut bool,
            depth: usize,
            nodes: &mut usize,
        ) {
            if depth > 8 || *bytes >= MAX_TEXT || *nodes >= 512 {
                *value = Value::Null;
                *truncated = true;
                return;
            }
            *nodes += 1;
            match value {
                Value::String(text) => {
                    *text = shorten(
                        sanitizer.text(text),
                        MAX_MESSAGE.min(MAX_TEXT.saturating_sub(*bytes)),
                        truncated,
                    );
                    *bytes += text.len();
                }
                Value::Array(items) => {
                    if items.len() > 32 {
                        items.truncate(32);
                        *truncated = true;
                    }
                    let mut retained = 0;
                    for item in items.iter_mut() {
                        if *nodes >= 512 {
                            *truncated = true;
                            break;
                        }
                        clean(item, sanitizer, bytes, truncated, depth + 1, nodes);
                        retained += 1;
                    }
                    items.truncate(retained);
                }
                Value::Object(fields) => {
                    if fields.len() > 32 {
                        *truncated = true;
                    }
                    let old = std::mem::take(fields);
                    for (key, mut item) in old.into_iter().take(32) {
                        if *nodes >= 512 {
                            *truncated = true;
                            break;
                        }
                        let key = shorten(sanitizer.text(&key), 128, truncated);
                        *bytes += key.len();
                        clean(&mut item, sanitizer, bytes, truncated, depth + 1, nodes);
                        fields.insert(key, item);
                    }
                }
                _ => (),
            }
        }
        source.message = shorten(
            sanitizer.text(&source.message),
            MAX_MESSAGE,
            &mut failure.truncated,
        );
        let mut details = Value::Object(source.details.into_iter().collect());
        let mut facts = Vec::new();
        for key in ["outcome", "retry_safety", "cause_kind", "failed_step"] {
            if let Some(value) = details
                .as_object_mut()
                .and_then(|fields| fields.remove(key))
            {
                if value.is_number()
                    || value.as_str().is_some_and(|text| {
                        text.len() <= 64
                            && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    })
                {
                    facts.push((key, value));
                }
            }
        }
        clean(
            &mut details,
            &sanitizer,
            &mut bytes,
            &mut failure.truncated,
            0,
            &mut 0,
        );
        source.details = details
            .as_object()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect();
        for (key, value) in facts {
            source.details.insert(key.into(), value);
        }
        failure.source = Some(source);
    }
    YappinError::HotReload {
        failure: Box::new(failure),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compiler_locations_are_relative_and_text_is_sanitized() {
        let root = std::env::temp_dir().join("trusted-project");
        let source = root.join("src").join("main.rs");
        let mut report = CompilerReport::new(&root);
        report.sanitizer.environment = vec!["secret-env-value".into()];
        report.push(&json!({"level":"error","code":{"code":"E0425"},
            "message":"missing `item`; secret-env-value at note:/unrelated/private.rs and C:\\private\\file.rs",
            "rendered":"SECRET rendered output", "spans":[
                {"file_name":source,"line_start":4,"line_end":4,"column_start":2,"column_end":9,"is_primary":true,"text":[{"text":"SECRET excerpt"}]},
                {"file_name":"/unrelated/private.rs"}, {"file_name":"../outside.rs"},
                {"file_name":"C:\\private\\file.rs"}
            ]}));
        let diagnostic = &report.failure.diagnostics[0];
        assert_eq!(diagnostic.code.as_deref(), Some("E0425"));
        assert_eq!(diagnostic.spans[0].file.as_deref(), Some("src/main.rs"));
        assert_eq!(diagnostic.spans[0].line_start, 4);
        assert!(diagnostic.spans[1..].iter().all(|span| span.file.is_none()));
        let text = serde_json::to_string(&report.failure).unwrap();
        assert!(!text.contains(root.to_string_lossy().as_ref()));
        for secret in [
            "secret-env-value",
            "unrelated",
            "private",
            "SECRET",
            "trusted/project",
            "trusted-project",
        ] {
            assert!(!text.contains(secret), "{text}");
        }
    }

    #[test]
    fn diagnostic_tree_and_utf8_text_are_bounded() {
        let mut report = CompilerReport::new(Path::new("."));
        let diagnostic = json!({"level":"error","message":"🦀".repeat(5000),
            "spans":(0..20).map(|_| json!({"file_name":"src/main.rs"})).collect::<Vec<_>>(),
            "children":(0..50).map(|_| json!({"message":"child","level":"note"})).collect::<Vec<_>>()});
        for _ in 0..100 {
            report.push(&diagnostic);
        }
        assert!(report.failure.truncated);
        assert!(report.count <= MAX_DIAGNOSTICS);
        assert!(report.bytes <= MAX_TEXT);
        assert!(report.failure.diagnostics[0].message.len() <= MAX_MESSAGE);
        assert_eq!(report.failure.diagnostics[0].spans.len(), MAX_SPANS);
        assert!(serde_json::to_vec(&report.failure).unwrap().len() < 64 * 1024);
    }

    #[test]
    fn ordinary_environment_words_do_not_corrupt_relative_source_locations() {
        let mut report = CompilerReport::new(Path::new("/project"));
        report.sanitizer.environment = vec!["main".into(), "private_token".into()];
        report.sanitizer.credentials = vec!["private_token".into()];
        report.push(
            &json!({"message":"private_token in main","level":"error","spans":[
                {"file_name":"src/main.rs"}, {"file_name":"src/private_token.rs"}
            ]}),
        );
        let diagnostic = &report.failure.diagnostics[0];
        assert_eq!(diagnostic.spans[0].file.as_deref(), Some("src/main.rs"));
        assert_eq!(
            diagnostic.spans[1].file.as_deref(),
            Some("src/[redacted].rs")
        );
        assert!(!diagnostic.message.contains("private_token"));
    }

    #[test]
    fn publication_status_and_unknown_outcome_survive_bounds_and_remote_round_trip() {
        let cause = YappinError::Api {
            status: 503,
            retry_after: None,
            message: "x".repeat(100_000),
        };
        let error = error(
            ReloadPhase::Publication,
            "publication failed",
            Some(&cause),
            true,
            true,
            None,
        );
        let native = error.hot_reload_failure().unwrap();
        assert!(native.truncated);
        assert_eq!(
            native.source.as_ref().unwrap().details["outcome"],
            "unknown"
        );
        assert_eq!(error.status_code(), Some(503));
        let remote: crate::RemoteError = serde_json::from_value(error.to_json()).unwrap();
        assert_eq!(remote.hot_reload_failure(), Some(native.clone()));
        let partial = YappinError::PartialWrite {
            completed: vec![json!({"receipt":"confirmed"})],
            failed_step: 2,
            source: Box::new(remote.clone().into()),
        };
        assert_eq!(
            partial.to_remote().hot_reload_failure(),
            Some(native.clone())
        );
        let callback = YappinError::FailureHandling {
            source: Box::new(remote.into()),
            handling_error: Box::new(YappinError::RateLimited { retry_after: 0.5 }),
        };
        assert_eq!(callback.hot_reload_failure(), Some(native.clone()));
        assert_eq!(callback.to_remote().hot_reload_failure(), Some(native));
    }

    #[test]
    fn numeric_cause_trees_are_bounded_without_losing_outcome_facts() {
        let tree = Value::Array(
            (0..32)
                .map(|_| Value::Array((0..32).map(|_| Value::Array(vec![json!(1); 32])).collect()))
                .collect(),
        );
        let cause: crate::RemoteError = serde_json::from_value(json!({"kind":"api","message":"failed","status":503,"retry_after":0.25,"outcome":"unknown","retry_safety":"unknown_write_outcome","a_large":tree})).unwrap();
        let error = error(
            ReloadPhase::Publication,
            "publication failed",
            Some(&cause.into()),
            true,
            true,
            None,
        );
        let failure = error.hot_reload_failure().unwrap();
        assert!(failure.truncated);
        assert_eq!(
            failure.source.as_ref().unwrap().details["outcome"],
            "unknown"
        );
        assert_eq!(error.status_code(), Some(503));
        assert_eq!(
            error.retry_after(),
            Some(std::time::Duration::from_millis(250))
        );
        assert!(error.to_json().to_string().len() < 64 * 1024);
    }
}

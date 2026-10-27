//! A report an app sends to issues.guimadaleno.com: what it may hold, how
//! its text is cleaned, what is sent and what the answer means.
//!
//! This file is guimadaleno/base's `issues/client/report.rs`, copied into each
//! app unchanged; `issues/README.md` there is the protocol and the rules. It
//! is pure: the form, showing the person what will be sent and the HTTPS POST
//! are the app's. What it decides is whether a report can be sent at all, the
//! bytes that are, and what the app tells the person afterwards.

use serde::Serialize;

/// Where reports go: this, then the app's name.
pub const ENDPOINT: &str = "https://issues.guimadaleno.com/v1/";

/// The most each field holds, in `char`s; `DETAILS` in entries, `VALUE` for
/// each of them.
pub const TITLE: usize = 120;
pub const DESCRIPTION: usize = 8000;
pub const SYSTEM: usize = 300;
pub const CONTACT: usize = 200;
pub const DETAILS: usize = 24;
pub const VALUE: usize = 300;
pub const LOG: usize = 20000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Something does not work.
    Bug,
    /// Something could be better.
    Idea,
}

/// What an app sends. `title`, `system`, `contact` and each detail are one
/// line, as [`line`] leaves it; `description` and `log` are several, as
/// [`text`] does. A field that is not already clean is refused, not
/// repaired: what the person was shown is what is filed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Report<'a> {
    pub kind: Kind,
    pub title: &'a str,
    pub description: &'a str,
    /// The app's own version, or whatever names the build.
    pub version: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<&'a str>,
    /// Where the person can be answered, when they chose to say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<&'a str>,
    /// What the app adds, by name: lowercase letters, digits and `_`,
    /// starting with a letter. None is an empty slice.
    #[serde(skip_serializing_if = "none", serialize_with = "pairs")]
    pub details: &'a [(&'a str, &'a str)],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log: Option<&'a str>,
}

/// The part of a report a refusal names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// The report as a whole: something the service takes from no one.
    Report,
    Kind,
    Title,
    Description,
    Version,
    System,
    Contact,
    Details,
    Log,
}

/// What became of a report, from the service's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Filed. The reference is shown to the person, as it is.
    Sent(String),
    /// Refused for that field; nothing was filed.
    Invalid(Field),
    /// Not now: too many reports, or GitHub is away. Seconds until it is
    /// worth trying again.
    Later(u32),
    /// The app is not set up for this, or the service changed.
    Refused,
}

fn none(details: &&[(&str, &str)]) -> bool {
    details.is_empty()
}

fn pairs<S: serde::Serializer>(details: &&[(&str, &str)], to: S) -> Result<S::Ok, S::Error> {
    to.collect_map(details.iter().map(|(name, value)| (name, value)))
}

/// What no field keeps: control characters, and the characters that reorder
/// the text around them, which make a line read as something it is not.
fn dropped(c: char) -> bool {
    matches!(c,
        '\u{0}'..='\u{8}' | '\u{B}'..='\u{1F}' | '\u{7F}'..='\u{9F}'
        | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

/// A field of one line as it is sent: every kind of line break and tab a
/// space, what no field keeps left out, spaces single and none at the ends.
pub fn line(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        let c = match c {
            '\t' | '\n' | '\u{B}' | '\u{C}' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}' => ' ',
            c => c,
        };
        if dropped(c) || (c == ' ' && (out.is_empty() || out.ends_with(' '))) {
            continue;
        }
        out.push(c);
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// A field of several lines as it is sent: every kind of line break a
/// newline, what no field keeps left out, nothing blank at the ends.
pub fn text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        let c = match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                '\n'
            }
            '\u{85}' | '\u{2028}' | '\u{2029}' => '\n',
            c => c,
        };
        if !dropped(c) {
            out.push(c);
        }
    }
    out.trim_matches(|c| matches!(c, ' ' | '\t' | '\n')).to_string()
}

fn within(value: &str, clean: fn(&str) -> String, most: usize) -> bool {
    !value.is_empty() && clean(value) == value && value.chars().count() <= most
}

fn version(value: &str) -> bool {
    let b = value.as_bytes();
    (1..=40).contains(&b.len())
        && b[0].is_ascii_alphanumeric()
        && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'+' | b'_' | b'-'))
}

fn name(value: &str) -> bool {
    let b = value.as_bytes();
    (1..=32).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

impl Report<'_> {
    /// The first field that keeps this report from being sent.
    pub fn check(&self) -> Result<(), Field> {
        if !within(self.title, line, TITLE) {
            return Err(Field::Title);
        }
        if !within(self.description, text, DESCRIPTION) {
            return Err(Field::Description);
        }
        if !version(self.version) {
            return Err(Field::Version);
        }
        if self.system.is_some_and(|s| !within(s, line, SYSTEM)) {
            return Err(Field::System);
        }
        if self.contact.is_some_and(|c| !within(c, line, CONTACT)) {
            return Err(Field::Contact);
        }
        let details = self.details;
        let ok = details.len() <= DETAILS
            && details.iter().enumerate().all(|(i, (key, value))| {
                name(key) && within(value, line, VALUE) && details[..i].iter().all(|(other, _)| other != key)
            });
        if !ok {
            return Err(Field::Details);
        }
        if self.log.is_some_and(|l| !within(l, text, LOG)) {
            return Err(Field::Log);
        }
        Ok(())
    }

    /// The body of the POST, when the report can be sent: JSON, to go with
    /// `Content-Type: application/json`.
    pub fn body(&self) -> Result<Vec<u8>, Field> {
        self.check()?;
        serde_json::to_vec(self).map_err(|_| Field::Report)
    }
}

/// Where `app`'s reports are posted.
pub fn address(app: &str) -> String {
    format!("{ENDPOINT}{app}")
}

impl Field {
    /// The field as the protocol spells it.
    pub fn name(self) -> &'static str {
        match self {
            Field::Report => "report",
            Field::Kind => "kind",
            Field::Title => "title",
            Field::Description => "description",
            Field::Version => "version",
            Field::System => "system",
            Field::Contact => "contact",
            Field::Details => "details",
            Field::Log => "log",
        }
    }

    fn named(name: &str) -> Field {
        use Field::*;
        [Kind, Title, Description, Version, System, Contact, Details, Log]
            .into_iter()
            .find(|f| f.name() == name)
            .unwrap_or(Report)
    }
}

/// What the service's answer means, from its status and its body. An app
/// that could not reach the service at all has no answer to read: that is
/// its own to tell the person.
pub fn answer(status: u16, body: &[u8]) -> Answer {
    // Anything that is not the service's own answer reads as an empty one:
    // something in between spoke.
    let said: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
    let said = said.as_object();
    let field = |name| said.and_then(|s| s.get(name));
    match status {
        200 | 201 => match field("reference").and_then(|r| r.as_str()) {
            Some(r)
                if (1..=20).contains(&r.len())
                    && r.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') =>
            {
                Answer::Sent(r.to_string())
            }
            _ => Answer::Refused,
        },
        400 if field("error").and_then(|e| e.as_str()) == Some("invalid") => {
            Answer::Invalid(Field::named(field("field").and_then(|f| f.as_str()).unwrap_or("")))
        }
        // A whole number of seconds, up to a day; a minute when it says
        // anything else.
        429 | 503 => Answer::Later(
            field("retry")
                .and_then(|r| r.as_f64())
                .filter(|r| r.fract() == 0.0 && (1.0..=86400.0).contains(r))
                .map_or(60, |r| r as u32),
        ),
        _ => Answer::Refused,
    }
}

/// The vectors in guimadaleno/base's issues/testdata/, which an app copies to
/// `tests/report/`. Every copy of this file reaches the same verdicts, and
/// so does the service.
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    macro_rules! vectors {
        ($name:literal) => {
            serde_json::from_str::<Vec<Value>>(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/report/",
                $name
            )))
            .unwrap()
        };
    }

    fn example() -> Report<'static> {
        Report {
            kind: Kind::Bug,
            title: "The window opens behind the others",
            description: "It does.",
            version: "0.6.0",
            system: None,
            contact: None,
            details: &[],
            log: None,
        }
    }

    #[test]
    fn fields_are_cleaned_as_the_vectors_say() {
        for v in vectors!("clean.json") {
            let (from, to) = (v["from"].as_str().unwrap(), v["to"].as_str().unwrap());
            let clean = if v["as"] == "line" { line } else { text };
            assert_eq!(clean(from), to, "{from:?}");
            assert_eq!(clean(to), to, "{to:?}");
        }
    }

    #[test]
    fn reports_are_sent_or_refused_as_the_vectors_say() {
        let vectors = vectors!("reports.json");
        assert!(vectors.len() > 20);
        for v in vectors {
            let r = &v["report"];
            let details: Vec<(&str, &str)> = r["details"]
                .as_object()
                .map(|d| d.iter().map(|(k, v)| (k.as_str(), v.as_str().unwrap())).collect())
                .unwrap_or_default();
            let report = Report {
                kind: if r["kind"] == "idea" { Kind::Idea } else { Kind::Bug },
                title: r["title"].as_str().unwrap(),
                description: r["description"].as_str().unwrap(),
                version: r["version"].as_str().unwrap(),
                system: r["system"].as_str(),
                contact: r["contact"].as_str(),
                details: &details,
                log: r["log"].as_str(),
            };
            let name = v["name"].as_str().unwrap();
            // Details with nothing in them cannot be told from no details
            // here, and no details is a report that can be sent.
            if name == "details with nothing in them" {
                assert_eq!(report.check(), Ok(()));
                continue;
            }
            let verdict = report.check().err().map(Field::name);
            assert_eq!(verdict, v["verdict"].as_str(), "{name}");
            match report.body() {
                // What is sent is the report the vector holds, field for field.
                Ok(body) => assert_eq!(&serde_json::from_slice::<Value>(&body).unwrap(), r, "{name}"),
                Err(field) => assert_eq!(Some(field.name()), v["verdict"].as_str(), "{name}"),
            }
        }
    }

    #[test]
    fn each_field_holds_what_it_says_and_not_one_character_more() {
        let full = |n| "x".repeat(n);
        let (t, d, s, c, l, v) = (full(TITLE), full(DESCRIPTION), full(SYSTEM), full(CONTACT), full(LOG), full(VALUE));
        let details = [("key", v.as_str())];
        let report = Report { title: &t, description: &d, system: Some(&s), contact: Some(&c), details: &details, log: Some(&l), ..example() };
        assert_eq!(report.check(), Ok(()));

        let (t, d, s, c, l, v) = (full(TITLE + 1), full(DESCRIPTION + 1), full(SYSTEM + 1), full(CONTACT + 1), full(LOG + 1), full(VALUE + 1));
        assert_eq!(Report { title: &t, ..example() }.check(), Err(Field::Title));
        assert_eq!(Report { description: &d, ..example() }.check(), Err(Field::Description));
        assert_eq!(Report { system: Some(&s), ..example() }.check(), Err(Field::System));
        assert_eq!(Report { contact: Some(&c), ..example() }.check(), Err(Field::Contact));
        assert_eq!(Report { log: Some(&l), ..example() }.check(), Err(Field::Log));
        assert_eq!(Report { details: &[("key", v.as_str())], ..example() }.check(), Err(Field::Details));
    }

    #[test]
    fn a_detail_is_named_once() {
        let twice = [("gpu", "one"), ("locale", "pt-BR"), ("gpu", "two")];
        assert_eq!(Report { details: &twice, ..example() }.check(), Err(Field::Details));
        assert_eq!(Report { details: &twice[..2], ..example() }.check(), Ok(()));
    }

    #[test]
    fn the_body_is_the_report_and_nothing_it_does_not_hold() {
        assert_eq!(
            String::from_utf8(example().body().unwrap()).unwrap(),
            r#"{"kind":"bug","title":"The window opens behind the others","description":"It does.","version":"0.6.0"}"#
        );
        let report = Report { kind: Kind::Idea, contact: Some("a@b.c"), details: &[("z", "1"), ("a", "2")], ..example() };
        let body = String::from_utf8(report.body().unwrap()).unwrap();
        assert!(body.starts_with(r#"{"kind":"idea","#));
        // Details are sent in the order the app gave them.
        assert!(body.ends_with(r#""contact":"a@b.c","details":{"z":"1","a":"2"}}"#), "{body}");
        assert_eq!(Report { title: "", ..example() }.body(), Err(Field::Title));
        assert_eq!(address("example"), "https://issues.guimadaleno.com/v1/example");
    }

    #[test]
    fn answers_mean_what_the_vectors_say() {
        let vectors = vectors!("answers.json");
        assert!(vectors.len() > 20);
        for v in vectors {
            let o = &v["outcome"];
            let expected = match o["outcome"].as_str().unwrap() {
                "sent" => Answer::Sent(o["reference"].as_str().unwrap().to_string()),
                "invalid" => Answer::Invalid(Field::named(o["field"].as_str().unwrap())),
                "later" => Answer::Later(o["seconds"].as_u64().unwrap() as u32),
                _ => Answer::Refused,
            };
            let (status, body) = (v["status"].as_u64().unwrap() as u16, v["body"].as_str().unwrap());
            assert_eq!(answer(status, body.as_bytes()), expected, "{status} {body}");
        }
        assert_eq!(answer(201, &[0xff, 0xfe]), Answer::Refused);
        assert_eq!(answer(429, &[0xff, 0xfe]), Answer::Later(60));
    }
}

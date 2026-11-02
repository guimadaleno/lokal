//! What the spinner says while the model works: what it's doing, for how
//! long, and how far and how fast the reply has come.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::{config, layout, style, text};

/// When the spinner starts saying what the model is doing.
const LABEL_AFTER: Duration = Duration::from_secs(2);
/// How long the model has to generate before its speed shows: a few
/// tokens' worth says little about it.
const SPEED_AFTER: Duration = Duration::from_secs(1);

/// Tokens a second, once the model has generated for long enough.
pub fn speed(tokens: u64, time: Duration) -> Option<u64> {
    (time >= SPEED_AFTER).then(|| (tokens as f64 / time.as_secs_f64()).round() as u64)
}

/// A tool call as the model writes it.
pub struct ToolProgress {
    pub name: String,
    pub path: Option<String>,
    pub bytes: usize,
}

/// What the spinner says while the model works, once it has taken a moment:
/// a long silence looks like a hang otherwise. The mode, when it isn't
/// manual, shows at once and first, so Shift+Tab is seen to take; then the
/// time, and `progress`, most important first.
pub fn working_label(
    started: Instant,
    writing: Option<&ToolProgress>,
    progress: &[String],
    mode: Option<&str>,
) -> String {
    let elapsed = started.elapsed();
    if elapsed < LABEL_AFTER && mode.is_none() {
        return String::new();
    }
    let mut parts: Vec<String> = mode.into_iter().map(String::from).collect();
    if elapsed >= LABEL_AFTER {
        parts.push(text::duration(elapsed));
        // A server may hold a tool call's arguments back until they're
        // complete: while nothing arrives to count, only the time, rather
        // than a size, a token count and a speed that sit still.
        if writing.is_none_or(|call| call.bytes > 0) {
            parts.extend_from_slice(progress);
        }
    }
    // On one line, or the spinner's redraw would leave the rest behind.
    let room = style::terminal_width().map_or(80, |width| width.saturating_sub(4));
    // The tool's name and the path are the model's words, as they stream.
    let what = |path: Option<&str>| match writing {
        None => text::THINKING.to_string(),
        Some(call) => text::writing_tool(&layout::printable(&call.name), path, call.bytes),
    };
    let path = writing
        .and_then(|call| call.path.as_deref())
        .map(|path| layout::printable(&config::shown(Path::new(path))).into_owned());
    // The progress gives way from its end, before the file's name would.
    let name = path.as_deref().map(|path| {
        let name = Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy());
        format!("…{}", name.unwrap_or(path.into()))
    });
    while parts.len() > 1
        && format!("{} · {}", what(name.as_deref()), parts.join(" · "))
            .chars()
            .count()
            > room
    {
        parts.pop();
    }
    let time = parts.join(" · ");
    // A long path loses its start: the file's name, its size and the time stay.
    let spare = room.saturating_sub(format!("{} · {time}", what(Some(""))).chars().count());
    let path = path.as_deref().map(|path| tail(path, spare));
    let label = format!("{} · {time}", what(path.as_deref()));
    format!(" {}", style::pulse_faint(&label, started))
}

/// The end of a text that fits in `room` characters, marked with … when cut.
fn tail(text: &str, room: usize) -> String {
    let length = text.chars().count();
    if length <= room {
        return text.to_string();
    }
    let kept: String = text.chars().skip(length + 1 - room.max(1)).collect();
    format!("…{kept}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_shows_after_a_second_of_generating() {
        assert_eq!(speed(9, Duration::from_millis(900)), None);
        assert_eq!(speed(0, Duration::ZERO), None);
        assert_eq!(speed(50, Duration::from_millis(2100)), Some(24));
    }

    #[test]
    fn the_mode_shows_at_once_and_first() {
        let progress = ["~2k of 33k tokens".to_string()];
        assert_eq!(working_label(Instant::now(), None, &progress, None), "");
        let at_once = working_label(Instant::now(), None, &progress, Some("accept edits"));
        assert!(at_once.contains("thinking · accept edits") && !at_once.contains("tokens"));
        let later = Instant::now() - Duration::from_secs(3);
        let label = working_label(later, None, &progress, Some("accept edits"));
        assert!(label.contains("thinking · accept edits · 3s · ~2k of 33k tokens"));
    }

    #[test]
    fn the_label_leaves_progress_out_while_arguments_are_held_back() {
        let started = Instant::now() - Duration::from_secs(3);
        let progress = ["~2k of 33k tokens".to_string(), "52 tok/s".to_string()];
        assert!(
            working_label(started, None, &progress, None)
                .contains("3s · ~2k of 33k tokens · 52 tok/s")
        );
        let held = ToolProgress {
            name: "run_command".into(),
            path: None,
            bytes: 0,
        };
        assert!(!working_label(started, Some(&held), &progress, None).contains("tokens"));
        // Where it doesn't fit on the line, the progress goes from its end.
        let long = ["~2k of 33k tokens".to_string(), "x".repeat(80)];
        let label = working_label(started, None, &long, None);
        assert!(label.contains("3s · ~2k of 33k tokens") && !label.contains('x'));
    }

    #[test]
    fn the_speed_gives_way_before_the_file_name() {
        let started = Instant::now() - Duration::from_secs(3);
        let progress = ["~2k of 33k tokens".to_string(), "52 tok/s".to_string()];
        let writing = ToolProgress {
            name: "write_file".into(),
            path: Some("/srv/some/deep/folder/a_rather_long_file_name_123.rs".into()),
            bytes: 2000,
        };
        let label = working_label(started, Some(&writing), &progress, None);
        assert!(label.contains("a_rather_long_file_name_123.rs"));
        assert!(label.contains("tokens") && !label.contains("tok/s"));
    }
}

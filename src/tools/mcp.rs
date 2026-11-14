//! A call to a tool of an MCP server: shown with its arguments, asked
//! about every time, on No, and what it gives back as an output.

use serde_json::Value;

use super::{CONTEXT_FULL, FILE_LIMIT, LEAST_ROOM, OUTPUT_LIMIT, Outcome, clip, limit, refused};
use crate::context::CHARS_PER_TOKEN;
use crate::image;
use crate::input::Input;
use crate::mcp::{Failure, Mcp};
use crate::{layout, style, text, tty};

/// The lines of arguments shown before the question; "Show the rest"
/// prints the others.
const ARGUMENTS_SHOWN: usize = 12;
/// The rows of what a call gave shown on screen, as for a command.
const RESULT_SHOWN: usize = 12;

/// Calls the tool at `index` once the person says yes. Auto never judges a
/// call and mail never approves one: the server's code runs as the person,
/// and only they know what it does. "Don't ask again" covers this exact
/// call, for the session.
pub(super) fn call(
    mcp: &mut Mcp,
    index: usize,
    args: &Value,
    input: &mut Input,
    room: Option<usize>,
    vision: Option<bool>,
) -> Result<Outcome, String> {
    // MCP takes an object; anything else would be asked about unseen.
    if !args.is_object() {
        return Err(text::MCP_ARGUMENTS_NOT_OBJECT.to_string());
    }
    let tool = mcp.tool(index);
    let exact = format!("{} {args}", tool.exposed);
    showln!(
        "{} {}",
        style::accent(mcp.server_name(tool)),
        layout::printable(&tool.name)
    );
    let mut lines = argument_lines(args);
    let rest = lines.split_off(lines.len().min(ARGUMENTS_SHOWN));
    print_lines(&lines);
    if !rest.is_empty() {
        showln!("  {}", style::gray(&text::more_lines(rest.len())));
    }
    showln!();
    if input.call_approved_before(&exact) {
        showln!("  {}", style::faint(text::APPROVED_FOR_SESSION));
    } else {
        let approval = input.approve_shown(text::CALL_QUESTION, rest, print_lines, true, false);
        input.keep_call_approval(&approval, &exact);
        if let Some(outcome) = refused(approval) {
            return Ok(outcome);
        }
    }
    match mcp.call(index, args) {
        Ok(result) => Ok(outcome(&result, room, vision)),
        Err(Failure::Stopped) => Ok(Outcome::Stopped),
        Err(failure) => {
            let message = failure.message();
            showln!();
            print_indented(&message, style::red);
            Ok(Outcome::Done(format!("Error: {message}")))
        }
    }
}

/// The arguments as the JSON the server gets, with control and hidden
/// characters as marks. A line break inside a value shows as `\n`, so no
/// value can pass for another parameter, as it could in a `name: value`
/// view.
fn argument_lines(args: &Value) -> Vec<String> {
    let width = tty::width().unwrap_or(80);
    let json = serde_json::to_string_pretty(args).unwrap_or_default();
    json.lines()
        .flat_map(|line| {
            let indent = 2 + line.len() - line.trim_start().len();
            layout::indented(&layout::printable(line.trim_start()), indent, width)
                .lines()
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// What the model hears of a call, from what the server gave back: its
/// text cut to the room left, the images it fits for a model that sees, and
/// a line for anything else.
fn outcome(result: &Value, room: Option<usize>, vision: Option<bool>) -> Outcome {
    // A tool that needs more from the person: Lokal can't give it.
    if result["resultType"]
        .as_str()
        .is_some_and(|kind| kind != "complete")
    {
        return Outcome::Done(text::MCP_WANTS_INPUT.to_string());
    }
    let mut parts = Vec::new();
    let mut images = Vec::new();
    for part in result["content"].as_array().into_iter().flatten() {
        match (part["type"].as_str(), vision) {
            (Some("text"), _) => parts.push(part["text"].as_str().unwrap_or_default().to_string()),
            (Some("image"), Some(true)) => {
                match image::from_base64(part["data"].as_str().unwrap_or("")) {
                    Some(image) => images.push(image),
                    None => parts.push(text::MCP_IMAGE_UNREAD.to_string()),
                }
            }
            (Some("image"), Some(false)) => parts.push(text::MCP_IMAGE_BLIND.to_string()),
            (Some("image"), None) => parts.push(text::MCP_IMAGE_UNSAID.to_string()),
            (Some("resource"), _) => match part["resource"]["text"].as_str() {
                Some(contents) => parts.push(contents.to_string()),
                None => parts.push(text::mcp_binary(
                    part["resource"]["uri"].as_str().unwrap_or("?"),
                )),
            },
            (Some("resource_link"), _) => parts.push(text::mcp_link(
                part["name"].as_str().unwrap_or(""),
                part["uri"].as_str().unwrap_or("?"),
            )),
            (Some("audio"), _) => parts.push(text::MCP_AUDIO.to_string()),
            (kind, _) => parts.push(text::mcp_unknown_part(kind.unwrap_or("?"))),
        }
    }
    if parts.is_empty()
        && images.is_empty()
        && let Some(structured) = result.get("structuredContent")
    {
        parts.push(structured.to_string());
    }
    let mut output = parts.join("\n");
    if result["isError"] == true {
        output = format!("{}\n{output}", text::MCP_TOOL_FAILED);
    }
    let limit = limit(room, OUTPUT_LIMIT);
    let mut text = if output.trim().is_empty() && images.is_empty() {
        "(no output)".to_string()
    } else if output.trim().is_empty() {
        String::new()
    } else if limit < LEAST_ROOM {
        format!(
            "The tool ran, but there's no room left in the context for what it gave. {CONTEXT_FULL}"
        )
    } else {
        clip(&output, limit, 0)
    };
    // Images take what the text leaves; where the context's size isn't
    // known, as much as a read of text.
    let mut left = room.unwrap_or(FILE_LIMIT).saturating_sub(text.len());
    let mut urls = Vec::new();
    let mut shown = Vec::new();
    for (image, url) in images {
        let size = image.tokens() as usize * CHARS_PER_TOKEN;
        if url.len() / 4 * 3 > image::MOST_BYTES as usize {
            text.push_str(&format!(
                "\n{}",
                text::mcp_image_too_big(image.width, image.height)
            ));
        } else if size <= left {
            left -= size;
            shown.push(text::image_size(image.width, image.height));
            urls.push(url);
        } else {
            text.push_str(&format!(
                "\n{}",
                text::mcp_image_no_room(image.width, image.height, image.tokens())
            ));
        }
    }
    show_result(&output, &shown);
    if urls.is_empty() {
        Outcome::Done(text)
    } else {
        Outcome::Images { text, urls }
    }
}

/// What the call gave, as a command's output shows: its first rows, faint
/// and indented, after a blank line, and the images by their size. A
/// server's result is often prose, so a long line goes on under the indent,
/// broken at spaces, rather than at the terminal's edge.
fn show_result(output: &str, images: &[String]) {
    if output.lines().all(|line| line.trim().is_empty()) && images.is_empty() {
        return;
    }
    let output = output.to_string();
    let images = images.to_vec();
    // Laid out again at a new width (#49).
    crate::screen::fitted(Box::new(move |width| result_at(&output, &images, width)));
}

/// The result's rows at `width`, after a blank line.
fn result_at(output: &str, images: &[String], width: usize) -> String {
    let rows: Vec<String> = output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .flat_map(|line| {
            // No more of a line than the rows shown can hold.
            let line: String = layout::printable(line.trim_end())
                .chars()
                .take(RESULT_SHOWN * width)
                .collect();
            layout::indented(&line, 2, width)
                .lines()
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .collect();
    let mut out = String::from("\n");
    for row in rows.iter().take(RESULT_SHOWN) {
        out.push_str(&format!("{}\n", style::gray(row)));
    }
    if rows.len() > RESULT_SHOWN {
        let more = text::more_lines(rows.len() - RESULT_SHOWN);
        out.push_str(&format!("  {}\n", style::gray(&more)));
    }
    for size in images {
        out.push_str(&format!(
            "  {}\n",
            style::gray(&text::mcp_image_shown(size))
        ));
    }
    out
}

fn print_lines(lines: &[String]) {
    for line in lines {
        showln!("{line}");
    }
}

fn print_indented(message: &str, paint: fn(&str) -> String) {
    crate::screen::fitted(layout::indented_rows(&layout::printable(message), 2, paint));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text_of(outcome: Outcome) -> String {
        match outcome {
            Outcome::Done(text) => text,
            Outcome::Images { text, .. } => text,
            _ => panic!("not an output"),
        }
    }

    #[test]
    fn text_and_the_rest_become_an_output() {
        let result = json!({"content": [
            {"type": "text", "text": "first"},
            {"type": "resource_link", "uri": "file:///a.txt", "name": "a.txt"},
            {"type": "resource", "resource": {"uri": "file:///b", "text": "inside b"}},
            {"type": "resource", "resource": {"uri": "file:///c.bin", "blob": "AAAA"}},
            {"type": "audio", "data": "AAAA", "mimeType": "audio/wav"},
            {"type": "hologram"}
        ]});
        let text = text_of(outcome(&result, Some(10_000), Some(true)));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "first");
        assert_eq!(lines[1], text::mcp_link("a.txt", "file:///a.txt"));
        assert_eq!(lines[2], "inside b");
        assert_eq!(lines[3], text::mcp_binary("file:///c.bin"));
        assert_eq!(lines[4], text::MCP_AUDIO);
        assert_eq!(lines[5], text::mcp_unknown_part("hologram"));
    }

    #[test]
    fn a_failure_and_structured_content_say_so() {
        let failed =
            json!({"isError": true, "content": [{"type": "text", "text": "no such repo"}]});
        assert_eq!(
            text_of(outcome(&failed, None, None)),
            format!("{}\nno such repo", text::MCP_TOOL_FAILED)
        );
        let structured = json!({"content": [], "structuredContent": {"temperature": 21}});
        assert_eq!(
            text_of(outcome(&structured, None, None)),
            r#"{"temperature":21}"#
        );
        let asks = json!({"resultType": "input_required", "inputRequests": {}});
        assert_eq!(text_of(outcome(&asks, None, None)), text::MCP_WANTS_INPUT);
    }

    #[test]
    fn an_image_goes_to_a_model_that_sees_when_it_fits() {
        let url = image::data_url(image::Kind::Png, &image::png_start(64, 64));
        let data = &url["data:image/png;base64,".len()..];
        let result = json!({"content": [{"type": "image", "data": data, "mimeType": "image/png"}]});
        let Outcome::Images { urls, .. } = outcome(&result, Some(10_000), Some(true)) else {
            panic!("no image");
        };
        assert!(urls[0].starts_with("data:image/png;base64,"));
        assert_eq!(
            text_of(outcome(&result, Some(10_000), Some(false))),
            text::MCP_IMAGE_BLIND
        );
        let url = image::data_url(image::Kind::Png, &image::png_start(640, 480));
        let data = &url["data:image/png;base64,".len()..];
        let big = json!({"content": [{"type": "image", "data": data, "mimeType": "image/png"}]});
        let squeezed = text_of(outcome(&big, Some(1_200), Some(true)));
        assert_eq!(squeezed.trim(), text::mcp_image_no_room(640, 480, 455));
    }

    #[test]
    fn arguments_show_as_json_so_no_value_passes_for_a_parameter() {
        let lines = argument_lines(&json!({"a": "notes\n  text: hello", "text": "x\u{1b}[2J"}));
        assert_eq!(
            lines,
            [
                "  {",
                r#"    "a": "notes\n  text: hello","#,
                r#"    "text": "x\u001b[2J""#,
                "  }"
            ]
        );
    }
}

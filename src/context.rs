//! How much of the context a conversation takes, and what makes way when
//! it's full: what the model has seen and replied after, oldest first.
//! The sizes are estimates, rough and on the high side.

use serde_json::Value;

/// About three characters a token: rough, and on the high side.
pub const CHARS_PER_TOKEN: usize = 3;
/// Stands in for a tool's output once it's dropped to free up context.
const DROPPED_OUTPUT: &str = "[Output dropped to free up context, after you saw it.]";
/// Stands in for the text a file change carried once it's dropped.
const DROPPED_WRITE: &str = "[Dropped to free up context; read the file for what it holds now.]";
/// Frame a command the person ran at the `>`, ahead of their next request:
/// the command between the first two, its result before the last, which
/// `make_way` looks for to drop it.
const OWN_START: &str = "[The user ran this command themselves, at their prompt: `";
const OWN_OUTPUT: &str = "`. What it gave:]\n";
const OWN_END: &str = "\n[End of what the user's command gave.]";

/// A command the person ran at the `>` and what it gave, framed for the
/// model, so `make_way` can drop what it gave later.
pub fn own_command(command: &str, result: &str) -> String {
    format!("{OWN_START}{command}{OWN_OUTPUT}{result}{OWN_END}")
}

/// A text's size in tokens: rough, and on the high side.
pub fn tokens_in(text: &str) -> u64 {
    (text.len() / CHARS_PER_TOKEN) as u64
}

/// What the conversation may fill: a quarter of the context stays for the
/// model's reasoning and reply.
fn budget(context: u64) -> u64 {
    context * 3 / 4
}

/// The request's size in tokens with these messages, and `fixed`
/// characters besides: the system prompt and the tools.
pub fn estimate(fixed: usize, messages: &[Value]) -> u64 {
    tokens(fixed + json_length(messages))
}

fn tokens(characters: usize) -> u64 {
    (characters / CHARS_PER_TOKEN) as u64
}

/// The length of the JSON a value is sent as.
fn json_length<T: serde::Serialize + ?Sized>(value: &T) -> usize {
    serde_json::to_string(value).map_or(0, |json| json.len())
}

/// Near the end of the context, the oldest of what the model has seen
/// and replied after goes first: tool outputs, and the text of the files
/// it wrote, which the files hold now. They take the most room and matter
/// least later on. The outputs the model hasn't seen yet stay, cut to fit
/// instead (`room_for_output`): dropped unseen, it would only ask for them
/// again, and again.
pub fn free(messages: &mut [Value], fixed: usize, context: u64) {
    // Measured once, then by what each message gives up: measuring the
    // whole conversation again for each one would cost the square of it.
    let mut characters = fixed + json_length(messages);
    for index in 0..last_reply(messages) {
        if tokens(characters) <= budget(context) {
            return;
        }
        characters -= json_length(&messages[index]);
        make_way(&mut messages[index]);
        characters += json_length(&messages[index]);
    }
}

/// How many characters the next tool output may take: the room left once
/// everything that can make way has, since that goes first.
pub fn room_for_output(messages: &[Value], fixed: usize, context: u64) -> usize {
    let mut characters = fixed + json_length(messages);
    for message in &messages[..last_reply(messages)] {
        let mut made_way = message.clone();
        make_way(&mut made_way);
        characters = characters + json_length(&made_way) - json_length(message);
    }
    let room = budget(context).saturating_sub(tokens(characters));
    room as usize * CHARS_PER_TOKEN
}

/// Where the model's last reply is: everything before it, the model has
/// seen and replied after.
fn last_reply(messages: &[Value]) -> usize {
    messages
        .iter()
        .rposition(|message| message["role"] == "assistant")
        .unwrap_or(0)
}

/// Drops what a message the model has replied after holds: a tool's output,
/// what the person's own commands gave, or the text of the file changes it
/// asked for.
fn make_way(message: &mut Value) {
    if message["role"] == "tool" {
        message["content"] = DROPPED_OUTPUT.into();
    }
    if message["role"] == "user"
        && let Some(content) = message["content"].as_str().and_then(drop_own)
    {
        message["content"] = content.into();
    }
    // get_mut, since indexing to change would add the keys a message lacks.
    let calls = message.get_mut("tool_calls").and_then(Value::as_array_mut);
    for call in calls.into_iter().flatten() {
        if let Some(function) = call.get_mut("function") {
            drop_written(function);
        }
    }
}

/// A request with what each of the person's own commands gave dropped, the
/// commands kept. None when there's nothing to drop.
fn drop_own(content: &str) -> Option<String> {
    let mut kept = String::new();
    let mut rest = content;
    let mut dropped = false;
    while let Some(start) = rest.find(OWN_START)
        && let Some(output) = rest[start..]
            .find(OWN_OUTPUT)
            .map(|at| start + at + OWN_OUTPUT.len())
        && let Some(end) = rest[output..].find(OWN_END).map(|at| output + at)
    {
        kept.push_str(&rest[..output]);
        if end - output > DROPPED_OUTPUT.len() {
            kept.push_str(DROPPED_OUTPUT);
            dropped = true;
        } else {
            kept.push_str(&rest[output..end]);
        }
        rest = &rest[end..];
    }
    kept.push_str(rest);
    dropped.then_some(kept)
}

/// A write_file or edit_file call keeps its path and loses its text. The
/// arguments stay valid JSON: the server reads them back into the prompt.
fn drop_written(function: &mut Value) {
    let fields: &[&str] = match function["name"].as_str() {
        Some("write_file") => &["content"],
        Some("edit_file") => &["old_text", "new_text"],
        _ => return,
    };
    let Some(Ok(mut arguments)) = function["arguments"]
        .as_str()
        .map(serde_json::from_str::<Value>)
    else {
        return;
    };
    let mut dropped = false;
    for &field in fields {
        if arguments[field]
            .as_str()
            .is_some_and(|text| text.len() > DROPPED_WRITE.len())
        {
            arguments[field] = DROPPED_WRITE.into();
            dropped = true;
        }
    }
    if dropped {
        function["arguments"] = arguments.to_string().into();
    }
}

/// A call's arguments as the conversation keeps them. The server reads them
/// back into the prompt, and arguments that aren't JSON, such as ones cut
/// short, make it fail every request after. Those are kept as `{}`, and so
/// is anything but an object, which no tool takes.
pub fn kept_arguments(arguments: &str) -> &str {
    if is_object(arguments) {
        arguments
    } else {
        "{}"
    }
}

pub fn is_object(arguments: &str) -> bool {
    serde_json::from_str::<Value>(arguments).is_ok_and(|value| value.is_object())
}

/// A saved conversation may hold arguments the server can't read back:
/// kept as they are, it couldn't go on.
pub fn mend_arguments(messages: &mut [Value]) {
    let calls = messages
        .iter_mut()
        .filter_map(|message| message.get_mut("tool_calls"))
        .filter_map(Value::as_array_mut)
        .flatten();
    for arguments in calls.filter_map(|call| call.pointer_mut("/function/arguments")) {
        let kept = kept_arguments(arguments.as_str().unwrap_or_default()).to_string();
        *arguments = kept.into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CONTEXT: u64 = 8192;

    /// What the system prompt and the tools take, as in a conversation
    /// without other sessions and with an empty prompt.
    fn fixed() -> usize {
        crate::tools::definitions(false).to_string().len()
    }

    fn messages() -> Vec<Value> {
        vec![json!({"role": "user", "content": "read it"})]
    }

    /// The model replies with a step.
    fn reply(messages: &mut Vec<Value>) {
        let call = json!({"id": "1", "type": "function",
            "function": {"name": "read_file", "arguments": "{}"}});
        messages.push(json!({"role": "assistant", "content": "", "tool_calls": [call]}));
    }

    /// The step's output arrives.
    fn output(messages: &mut Vec<Value>, characters: usize) {
        messages
            .push(json!({"role": "tool", "tool_call_id": "1", "content": "x".repeat(characters)}));
    }

    /// The model writes a file with this much text, and it's written.
    fn write(messages: &mut Vec<Value>, characters: usize) {
        let arguments = json!({"path": "a.txt", "content": "x".repeat(characters)}).to_string();
        let call = json!({"id": "2", "type": "function",
            "function": {"name": "write_file", "arguments": arguments}});
        messages.push(json!({"role": "assistant", "content": "", "tool_calls": [call]}));
        messages.push(json!({"role": "tool", "tool_call_id": "2", "content": "Wrote a.txt."}));
    }

    fn arguments(messages: &[Value], index: usize) -> Value {
        let arguments = &messages[index]["tool_calls"][0]["function"]["arguments"];
        serde_json::from_str(arguments.as_str().unwrap()).unwrap()
    }

    fn content_length(messages: &[Value], index: usize) -> usize {
        messages[index]["content"].as_str().unwrap().len()
    }

    /// As `free` and `room_for_output` were, measuring everything at each
    /// step: what they must still give.
    fn free_as_before(messages: &mut [Value], fixed: usize, context: u64) {
        for index in 0..last_reply(messages) {
            if estimate(fixed, messages) <= budget(context) {
                return;
            }
            make_way(&mut messages[index]);
        }
    }

    fn room_as_before(messages: &[Value], fixed: usize, context: u64) -> usize {
        let mut messages = messages.to_vec();
        let seen = last_reply(&messages);
        for message in &mut messages[..seen] {
            make_way(message);
        }
        let room = budget(context).saturating_sub(estimate(fixed, &messages));
        room as usize * CHARS_PER_TOKEN
    }

    #[test]
    fn measuring_once_gives_what_measuring_each_time_gave() {
        for steps in 0..12 {
            let mut messages = vec![
                json!({"role": "user", "content": own_command("ls", &"é".repeat(steps * 700))}),
            ];
            for step in 0..steps {
                reply(&mut messages);
                output(&mut messages, 1000 + step * 1700);
                if step % 3 == 0 {
                    write(&mut messages, 500 + step * 2300);
                }
            }
            for context in [4096, 8192, 16_384, 32_768] {
                assert_eq!(
                    room_for_output(&messages, fixed(), context),
                    room_as_before(&messages, fixed(), context),
                    "{steps} steps, {context}"
                );
                let mut freed = messages.clone();
                free(&mut freed, fixed(), context);
                let mut before = messages.clone();
                free_as_before(&mut before, fixed(), context);
                assert_eq!(freed, before, "{steps} steps, {context}");
            }
        }
    }

    #[test]
    fn older_outputs_make_way_for_the_newest() {
        let mut messages = messages();
        reply(&mut messages);
        output(&mut messages, 9000);
        reply(&mut messages);
        output(&mut messages, 12_000);
        free(&mut messages, fixed(), CONTEXT);
        assert_eq!(messages[2]["content"], DROPPED_OUTPUT);
        assert_eq!(content_length(&messages, 4), 12_000);
    }

    #[test]
    fn an_output_the_model_hasnt_seen_is_never_dropped() {
        let mut messages = messages();
        reply(&mut messages);
        output(&mut messages, 30_000);
        free(&mut messages, fixed(), CONTEXT);
        assert_eq!(content_length(&messages, 2), 30_000);
    }

    #[test]
    fn room_for_output_counts_seen_outputs_as_gone() {
        let mut messages = messages();
        reply(&mut messages);
        output(&mut messages, 9000);
        reply(&mut messages);
        // About 18,000 characters under the budget; the 9,000 seen don't count.
        let room = room_for_output(&messages, fixed(), CONTEXT);
        assert!(room > 12_000, "room {room}");
        // A first output of this step, not seen yet, does count.
        output(&mut messages, 3000);
        let after = room_for_output(&messages, fixed(), CONTEXT);
        assert!(room - after >= 3000, "room {room}, after {after}");
    }

    #[test]
    fn a_written_file_makes_way_once_the_model_replied_after_it() {
        let mut messages = messages();
        write(&mut messages, 20_000);
        reply(&mut messages);
        free(&mut messages, fixed(), CONTEXT);
        let arguments = arguments(&messages, 1);
        assert_eq!(arguments["path"], "a.txt");
        assert_eq!(arguments["content"], DROPPED_WRITE);
    }

    #[test]
    fn the_last_reply_keeps_what_it_wrote() {
        let mut messages = messages();
        write(&mut messages, 20_000);
        free(&mut messages, fixed(), CONTEXT);
        let arguments = arguments(&messages, 1);
        assert_eq!(arguments["content"].as_str().unwrap().len(), 20_000);
    }

    #[test]
    fn room_for_output_counts_written_files_as_gone() {
        let mut messages = messages();
        write(&mut messages, 15_000);
        reply(&mut messages);
        // About 18,000 characters under the budget; kept, the file would leave under 3,500.
        let room = room_for_output(&messages, fixed(), CONTEXT);
        assert!(room > 10_000, "room {room}");
    }

    #[test]
    fn an_edit_loses_its_long_text_and_keeps_the_rest() {
        let arguments = json!({"path": "a.txt", "old_text": "x".repeat(500), "new_text": "y"});
        let mut function = json!({"name": "edit_file", "arguments": arguments.to_string()});
        drop_written(&mut function);
        let arguments: Value =
            serde_json::from_str(function["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(arguments["path"], "a.txt");
        assert_eq!(arguments["old_text"], DROPPED_WRITE);
        assert_eq!(arguments["new_text"], "y");
    }

    #[test]
    fn what_the_persons_commands_gave_makes_way_and_the_rest_stays() {
        let request = format!(
            "{}\n\n{}\n\nwhy does it fail?",
            own_command("cargo test", &"x".repeat(30_000)),
            own_command("git status", "ok")
        );
        let mut messages = vec![json!({"role": "user", "content": request})];
        reply(&mut messages);
        free(&mut messages, fixed(), CONTEXT);
        let kept = messages[0]["content"].as_str().unwrap().to_string();
        assert_eq!(
            kept,
            format!(
                "{}\n\n{}\n\nwhy does it fail?",
                own_command("cargo test", DROPPED_OUTPUT),
                own_command("git status", "ok")
            )
        );
        // Already dropped, or as short as the note: nothing more to drop.
        assert_eq!(drop_own(&kept), None);
        assert_eq!(drop_own("[The user ran this command themselves"), None);
    }

    #[test]
    fn making_way_adds_nothing_to_a_message_without_calls() {
        let mut message = json!({"role": "user", "content": "hi"});
        make_way(&mut message);
        assert_eq!(message, json!({"role": "user", "content": "hi"}));
    }

    #[test]
    fn a_saved_conversation_with_arguments_the_server_cant_read_is_mended() {
        let call = |arguments: &str| {
            json!({"id": "1", "type": "function",
                "function": {"name": "write_file", "arguments": arguments}})
        };
        let calls = [
            call(r#"{"path": "a"#),
            call("[1]"),
            call(r#"{"path": "b"}"#),
        ];
        let mut messages = vec![
            json!({"role": "user", "content": "hi"}),
            json!({"role": "assistant", "content": "", "tool_calls": calls}),
        ];
        mend_arguments(&mut messages);
        let kept: Vec<&str> = messages[1]["tool_calls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|call| call["function"]["arguments"].as_str().unwrap())
            .collect();
        assert_eq!(kept, ["{}", "{}", r#"{"path": "b"}"#]);
        assert_eq!(messages[0], json!({"role": "user", "content": "hi"}));
    }
}

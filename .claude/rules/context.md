---
paths:
  - "src/memory.rs"
  - "src/recon.rs"
  - "src/prompt.rs"
  - "src/context.rs"
  - "src/compact.rs"
  - "src/conversation.rs"
  - "src/chat.rs"
  - "src/image.rs"
  - "src/commands.rs"
  - "src/tools/command.rs"
  - "src/tools/files.rs"
  - "src/shell.rs"
---

# The context

## The project's notes (`memory.rs`)

- **LOKAL.md, then AGENTS.md, in the git root and every folder down to the working directory,** each file once however it's reached. The walk stops at the git root, and a note that leads outside the project is skipped: a repository could link its AGENTS.md to any file of the person's, and it would reach the model unasked.
- **They get a share of the context** (`NOTES_SHARE`), cut at a line, with the model told to read the rest: they go with every request, and on a small context a long AGENTS.md would leave no room to work.
- **They come before user.md in the prompt,** framed as the project's: what the person wrote wins over what a repository says. The model changes them like any file, with a diff and a yes.
- **They're guidance, never secrets:** they're versioned with the project, and other tools read them too.
- **A change counts from the next turn, with a line saying so;** `/memory` opens them, and `/info` shows each one and what it costs.

## `/recon` (`recon.rs`)

- **Lokal's code, never the model's:** fixed probes that change nothing, run without a shell, each within a time limit. A model sent to look around would spend the context, and could be steered by what it finds.
- **What could be a secret is left out by the code:** a remote's address loses its user, password and query; a cloud tool is "configured" from whether its settings file or variable exists, never from what it holds; of the other variables, only `PATH`, `SHELL` and `TZ` are read.
- **Text from the disk is one short clean line,** given to the model as facts, not instructions: a branch or a folder name could carry some.
- **The PATH's relative folders are skipped,** so a project can't plant a program the probes would run; under WSL, so are Windows' drives, slow to look through.
- **It lasts for the session** and shows whole on screen, just as the model gets it; nothing of it is written to disk.

## Room

- **One reply gets at most half the context** (`max_tokens`): a model reasoning in circles stops there instead of filling all of it.
- **`/context` shows what fills the context, part by part:** the system prompt's sections (`prompt::sections`, which join to the prompt itself), each tool, the conversation and its outputs, what's free and the share kept for replies. Lokal's estimates, on the high side, with the server's count beside them once a reply has given one. `/context raw` prints the next request as it goes.
- **An output always reaches the model before it can be dropped:** each tool output is cut to the room left once the outputs the model already saw make way (`room_for_output`). Dropped unseen, the model only asks again, and each try leaves the history fuller, until nothing fits.
- **Nothing it reads can fill Lokal's memory:** a command's output is kept as its start and end (`shell.rs`), and a read takes a regular file only, bounded (`files::read_text`).
- **What the person's `!` commands gave is an output too,** cut the same way, inside their request's own message, since some chat templates refuse two from the user in a row; framed so `drop_own` finds it to make way.
- **What the model wrote into a file makes way like an output** once it has replied after the write (`make_way`): it is most of a conversation that writes files. The call keeps its path, its arguments stay valid JSON, since the chat template reads them back, and the note in their place says to read the file.

## `/compact` (`compact.rs`, #4)

- **It sums up what came before the last turn,** which stays exact, as a transcript in a request of its own, with no tools and reasoning off.
- **The summary goes in as a user note and an assistant reply,** never in the system prompt: it carries text from files the model read, and there it would weigh like instructions.
- **All or nothing:** Esc, Ctrl+C or a failure leaves the conversation as it was. Text after `/compact` says what to focus on.

## Images

- **Only to a model the server says sees** (LM Studio's `vlm`, Ollama's `vision`): a model that can't see may drop the image and answer as if it had. One the model read goes in its read's own `tool` message, never as the person's words; one pasted, in the person's own message.
- **It counts by its pixels, never its base64** (`image::tokens_in_url`), on the high side. One that doesn't fit the room left is left out with its estimate before anything older makes way: Lokal carries no decoder to shrink it. A pasted one left out is told to the model, or it may answer as if it saw it.
- **A file that reads as UTF-8 is text,** whatever its first bytes. An image is PNG, JPEG or WebP (Ollama refuses a GIF), bounded in size (`image::MOST_BYTES`).
- **It makes way like an output** once the model has replied after it, and the saved conversation keeps a line in its place.
- **One the server refuses gives way to a note** with the server's words, and the model is asked again, once (`leave_out_unseen_images`): left in, it would fail every request after it. Only on a refusal (`http::Failure::Refused`): a server that never answered never saw the image. A truncated PNG or an animated WebP reads as an image, and only the server finds out.

## Replies cut short

- **A reply that ends with nothing is never silent:** out of room (`finish_reason: "length"`), Lokal asks once more with a note to reason briefly, and says the context ran out if that fails too. While the model works, the spinner says how much of the context the reply has reached.
- **A step cut short never runs, and never stays as it came:** arguments cut short aren't JSON, and LM Studio fails every later request while they're in the conversation. They're kept as `{}` (`kept_arguments`), nothing that reply asked for runs, and the model hears why, once (`CUT_SHORT_RESULT`); a second time ends the turn. `lokal -r` mends a saved conversation the same way.

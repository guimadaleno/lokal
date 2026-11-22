---
paths:
  - "src/peers.rs"
  - "src/peers/**"
  - "src/tools/mail.rs"
  - "src/main.rs"
  - "src/conversation.rs"
  - "src/config.rs"
  - "src/input.rs"
  - "src/commands.rs"
---

# Mail

The person's Lokal sessions on one machine mail each other, so work can be split among them and they agree on who works where. Another model wrote the mail, and a file it read may have steered it.

## Let mail from another session act for the person, or run without bounds

- **`mail = auto`, the default, lets mail start a turn on its own,** in a session just opened too, but only at a `>` on an empty line: mail waits while the person types or a turn runs. `mail = ask` makes each wait for Enter on an empty line (Esc drops it); `mail = off` means no socket and no tools.
- **Esc stops the exchange, not only the turn:** after the person stops a turn mail started, or says no in one, mail waits for their next request there. Otherwise two models answering each other couldn't be stopped. Enter on an empty line passes waiting mail on sooner.
- **At most `mail_turns` turns in a row start on mail,** counted from the person's last request; past that, mail waits the same way, and the sender's model hears so and tells its person. Without a bound, two models answering each other would fill the GPU and the context.
- **The prompt and the `mail` tool steer away from loops of politeness:** answer what asks or needs something, a plain yes or no included, never mail only to thank. A mail marked `now: false` starts no turn and goes with the next one. The bound is still the guard: a prompt is only a nudge, and a steered model ignores it.
- **Mail never approves anything:** every step that isn't read-only asks the person at that terminal, and Esc or Ctrl+C stops a turn mail started like any other.
- **The model gets mail as another session's words,** framed with its name and folder, never as the person's, except mail the person wrote at the other session's `>` (`@name …`), which Lokal sends itself and frames as theirs.
- **What another model wrote always shows:** each mail whole, control characters as marks, and bidi controls and characters that show as nothing by their code (`<U+202E>`); sending shows as a step. Lokal's own facts about the other sessions go to the model without showing, as its facts about the machine do; `/sessions` shows them.
- **What the other sessions see of this one is Lokal's, never the model's** (`who`): its folder, the branch read from `.git/HEAD` (git itself would run what a repository's settings name), whether it's working or waiting for a yes, the person's last request cut short, and the files it changed.
- **Sessions in the same repository (or folder) that came, went or moved branch are told to the model as a note in that turn's request,** never in the system prompt: a change there would make LM Studio read the whole conversation again.
- **Only the person's own sessions talk:** a socket per session in the private scratch base. The kernel says who connected (`SO_PEERCRED`): the same user, and the very process the sender's name points to, asked back who it is. Nothing that runs unasked can open a socket, so a model can't pass its words off as a session's or the person's.
- **Conversations at a terminal only,** where someone sees mail arrive and can stop what it starts.
- **Bounded** (`peers::MESSAGE_LIMIT`, `WAITING_LIMIT`): the sender's model is told to shorten it, or that the inbox is full. The socket is all it puts on disk, and it goes as the scratch folder does.

## Printing mail

- **Mail prints only while the person is at the `>`,** through rustyline's `ExternalPrinter`; outside readline that printer would write into a turn's output or an open editor, so mail waits for the next prompt.
- **rustyline 19 or later:** 18, with a printer, waited on the terminal with keys it had already read, so a paste or fast typing stalled until the next key.
- **Mail ends an idle `>` with a SIGINT to Lokal itself,** sent while the inbox is held, so the person can't leave the `>` in between; nothing else ends readline from another thread.
- **The two tools and the prompt's line are the same all session,** so the prompt cache holds.

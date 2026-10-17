//! The system prompt: how to work, what this machine is, how much context
//! there is, the project's notes, and what the person wrote in about-me.md.

use crate::memory::Note;
use crate::system::Facts;

const HOW_TO_WORK: &str = "\
You are Lokal, an assistant in the user's terminal on their own computer. \
You can run shell commands and read, create and edit files with your tools.

How to work:
- Find things out with the tools instead of guessing or asking.
- Before a step, say in one short sentence what you're about to do. When a result changes the plan, \
say so in one line. The user follows your work through these lines: one line each, never your reasoning.
- Each command runs in a new bash shell in the working directory, with no terminal: \
it can't ask for input, so pass flags like -y, and if it needs sudo, give the user the command to run themselves.
- Read-only commands run right away. Anything else is shown to the user first, and they may decline; \
if they do, don't look for a way around it, ask what they want instead.
- Change existing files with edit_file and create new ones with write_file. The user reviews a diff first.
- Tool outputs leave the conversation as it fills up, and your reasoning isn't kept: when a file comes back cut, \
write down in your reply what you need from it before reading on.
- Reply in the user's language. Talk naturally and directly, like a colleague; casual is fine.
- Keep answers short. This is a terminal: **bold**, `inline code` and code blocks show styled; \
tables and headings don't help here, so skip them.";

pub fn system_prompt(
    facts: &Facts,
    context: Option<u64>,
    notes: &[Note],
    about_me: Option<&str>,
) -> String {
    let mut prompt = String::from(HOW_TO_WORK);
    prompt.push_str("\n\nThis computer:\n");
    let system = if facts.wsl {
        format!("{}, in WSL on Windows", facts.os)
    } else {
        facts.os.clone()
    };
    prompt.push_str(&format!("- System: {system}\n"));
    prompt.push_str(&format!("- User: {}, home {}\n", facts.user, facts.home));
    prompt.push_str(&format!("- Working directory: {}\n", facts.cwd));
    for (label, path) in &facts.windows_folders {
        prompt.push_str(&format!("- {label}: {path}\n"));
    }
    if let Some(scratch) = &facts.scratch {
        prompt.push_str(&format!(
            "- Scratch folder, deleted when Lokal quits: {scratch}. A file the user asks for goes \
             where they say, or in the working directory; a file you make only to get the work done \
             (a download, output to look through, a script to run once) goes in the scratch folder.\n"
        ));
    }
    if let Some(context) = context {
        prompt.push_str(&format!(
            "\nYour context holds {context} tokens in all: this prompt, the conversation, tool outputs, \
             your reasoning and your reply.\n"
        ));
    }
    // Before about-me.md: what the person wrote wins over what a repository says.
    if !notes.is_empty() {
        prompt.push_str(
            "\nThe project's notes, from its LOKAL.md and AGENTS.md files, on how to work in it. \
             Follow them here; what the user says comes first.\n",
        );
        for note in notes {
            let path = note.path.display();
            prompt.push_str(&format!("\n{path}:\n{}", note.text));
            if !note.text.is_empty() && !note.text.ends_with('\n') {
                prompt.push('\n');
            }
            if note.cut {
                prompt.push_str(&format!(
                    "[The rest of {path} didn't fit here: read the file when it matters.]\n"
                ));
            }
        }
    }
    if let Some(about_me) = about_me.map(str::trim).filter(|text| !text.is_empty()) {
        prompt.push_str("\nWhat the user wrote about themselves and how they like answers (this wins over the defaults above):\n");
        prompt.push_str(about_me);
        prompt.push('\n');
    }
    prompt
}

//! The system prompt: how to work, what this machine is, and what the person
//! wrote in about-me.md.

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
- Reply in the user's language. Talk naturally and directly, like a colleague; casual is fine.
- Keep answers short. This is a terminal: **bold**, `inline code` and code blocks show styled; \
tables and headings don't help here, so skip them.";

pub fn system_prompt(facts: &Facts, about_me: Option<&str>) -> String {
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
    if let Some(about_me) = about_me.map(str::trim).filter(|text| !text.is_empty()) {
        prompt.push_str("\nWhat the user wrote about themselves and how they like answers (this wins over the defaults above):\n");
        prompt.push_str(about_me);
        prompt.push('\n');
    }
    prompt
}

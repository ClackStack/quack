//! The planner/editor handoff (`[model] planner`): when a plan is accepted, the messages
//! the planner added are replaced by one user message for the session's model. That model
//! never sees the planner's tool calls, so its cached prefix (everything before plan mode)
//! stays valid and the only new tokens are the brief (docs/plan/04-agent-loop-and-tools.md).

use quack_providers::Message;

/// The brief that replaces the planning messages: the user's requests, their answers to
/// clarifying questions, and the last assistant reply (the summary of the plan).
/// `plan_note` is the plan-mode request appended to each user message, taken off again;
/// `ask` is the name the profile gives `ask_user`; `todo` is the name the implementing
/// profile gives `todo`; `same_model` is true for a single-model fresh-context handoff.
pub(crate) fn brief(
    segment: &[Message],
    plan_note: &str,
    ask: Option<&str>,
    todo: Option<&str>,
    planner: &str,
    same_model: bool,
) -> String {
    let suffix = format!("\n\n{plan_note}");
    let mut requests = Vec::new();
    let mut answers = Vec::new();
    let mut summary = "";
    let mut asks: Vec<&str> = Vec::new();
    for m in segment {
        match m {
            Message::User { content, .. } => {
                let text = content.strip_suffix(&suffix).unwrap_or(content).trim();
                if !text.is_empty() {
                    requests.push(text);
                }
            }
            Message::Assistant {
                content,
                tool_calls,
                ..
            } => {
                if !content.trim().is_empty() {
                    summary = content.trim();
                }
                asks.extend(
                    tool_calls
                        .iter()
                        .filter(|c| Some(c.name.as_str()) == ask)
                        .map(|c| c.id.as_str()),
                );
            }
            Message::Tool {
                call_id, content, ..
            } if asks.contains(&call_id.as_str()) => {
                answers.push(content.trim());
            }
            _ => {}
        }
    }
    let lead = if same_model {
        "[Plan handoff] Exploration from plan mode was cleared to start implementation with a \
         fresh context. Read any file you need before changing it."
            .to_owned()
    } else {
        format!(
            "[Plan handoff] {planner} explored the code with read-only tools and wrote the plan \
             below; you carry it out. Its exploration is not repeated here, so read a file before \
             you change it."
        )
    };
    let mut out = format!("{lead}\n\nRequest:\n{}", requests.join("\n\n"));
    if !answers.is_empty() {
        out.push_str(&format!(
            "\n\nThe user's answers to its questions:\n{}",
            answers.join("\n\n")
        ));
    }
    if !summary.is_empty() {
        out.push_str(&format!("\n\nPlan:\n{summary}"));
    }
    match todo {
        Some(tool) => out.push_str(&format!(
            "\n\nCarry out the plan, and keep the todo list up to date with `{tool}` as you go."
        )),
        None => {
            out.push_str("\n\nCarry out the plan, and keep the todo list up to date as you go.");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use quack_providers::ToolCall;

    #[test]
    fn the_brief_keeps_requests_answers_and_the_summary() {
        let call = |id: &str, name: &str| ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: "{}".into(),
        };
        let segment = [
            Message::User {
                content: "Add retries\n\n[Plan mode] plan first".into(),
                images: Vec::new(),
            },
            Message::Assistant {
                content: "Looking around.".into(),
                reasoning: None,
                tool_calls: vec![call("r", "Read"), call("q", "AskUserQuestion")],
                sealed: None,
            },
            Message::Tool {
                call_id: "r".into(),
                content: "fn fetch() {}".into(),
                images: Vec::new(),
            },
            Message::Tool {
                call_id: "q".into(),
                content: "Q: How many?\nA: 3".into(),
                images: Vec::new(),
            },
            Message::Assistant {
                content: "1. Wrap fetch in a loop.".into(),
                reasoning: None,
                tool_calls: Vec::new(),
                sealed: None,
            },
        ];
        let b = brief(
            &segment,
            "[Plan mode] plan first",
            Some("AskUserQuestion"),
            Some("update_plan"),
            "big/model",
            false,
        );
        assert!(b.starts_with("[Plan handoff] big/model"), "{b}");
        assert!(
            b.contains("Request:\nAdd retries\n\n"),
            "the plan request is taken off: {b}"
        );
        assert!(
            b.contains("answers to its questions:\nQ: How many?\nA: 3"),
            "{b}"
        );
        assert!(
            b.contains("Plan:\n1. Wrap fetch in a loop."),
            "the last reply is the plan: {b}"
        );
        assert!(b.contains("`update_plan`"), "{b}");
        assert!(!b.contains("fn fetch"), "tool output stays behind: {b}");
        assert!(!b.contains("Looking around"), "{b}");

        let fresh = brief(
            &segment,
            "[Plan mode] plan first",
            Some("AskUserQuestion"),
            None,
            "same/model",
            true,
        );
        assert!(fresh.contains("fresh context"), "{fresh}");
        assert!(!fresh.contains("fn fetch"), "{fresh}");
    }
}

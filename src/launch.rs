//! Port of `buildAgentLaunch` (`src/lib/sessionLaunch.ts`): the CLI arguments a sub-tab boots with,
//! including session resume so a restarted pane continues the same conversation.

pub struct AgentLaunch {
    pub args: Vec<String>,
    /// Session id to persist on the sub-tab (Claude mints one up front).
    pub session_id: Option<String>,
}

fn strip_flag_with_value(args: &[String], flags: &[&str]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if flags.contains(&arg.as_str()) {
            skip_next = true;
            continue;
        }
        if flags.iter().any(|f| arg.starts_with(&format!("{f}="))) {
            continue;
        }
        out.push(arg.clone());
    }
    out
}

pub fn new_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn build_agent_launch(
    agent: &str,
    base_args: &[String],
    session_id: Option<&str>,
) -> AgentLaunch {
    let session = session_id
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string);
    let with = |prefix: Vec<&str>, clean: Vec<String>, session: Option<String>| AgentLaunch {
        args: prefix.into_iter().map(String::from).chain(clean).collect(),
        session_id: session,
    };
    match agent {
        "shell" | "wsl" => AgentLaunch {
            args: base_args.to_vec(),
            session_id: None,
        },
        "claude" => {
            let clean: Vec<String> =
                strip_flag_with_value(base_args, &["--resume", "-r", "--session-id"])
                    .into_iter()
                    .filter(|a| a != "--continue" && a != "-c")
                    .collect();
            match session {
                Some(id) => with(vec!["--resume", &id.clone()], clean, Some(id)),
                None => {
                    let id = new_uuid();
                    with(vec!["--session-id", &id.clone()], clean, Some(id))
                }
            }
        }
        "codex" | "codewhale" => {
            let clean: Vec<String> = base_args
                .iter()
                .filter(|a| a.as_str() != "resume")
                .cloned()
                .collect();
            match session {
                Some(id) => with(vec!["resume", &id.clone()], clean, Some(id)),
                None => with(vec![], clean, None),
            }
        }
        "opencode" => {
            let clean = strip_flag_with_value(base_args, &["--session", "-s"]);
            match session {
                Some(id) => with(vec!["--session", &id.clone()], clean, Some(id)),
                None => with(vec![], clean, None),
            }
        }
        "antigravity" => {
            let clean = strip_flag_with_value(base_args, &["--conversation"]);
            match session {
                Some(id) => with(vec!["--conversation", &id.clone()], clean, Some(id)),
                None => with(vec![], clean, None),
            }
        }
        "kiro" => with(vec!["chat"], base_args.to_vec(), None),
        "cursor" | "grok" => {
            let clean = strip_flag_with_value(base_args, &["--resume"]);
            match session {
                Some(id) => with(vec!["--resume", &id.clone()], clean, Some(id)),
                None => with(vec![], clean, None),
            }
        }
        _ => AgentLaunch {
            args: base_args.to_vec(),
            session_id: session,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_mints_a_session_then_resumes_it() {
        let first = build_agent_launch("claude", &["--dangerously-skip-permissions".into()], None);
        assert_eq!(first.args[0], "--session-id");
        let id = first.session_id.clone().unwrap();
        assert_eq!(first.args[1], id);
        let again = build_agent_launch("claude", &first.args, Some(&id));
        assert_eq!(
            again.args,
            vec![
                "--resume".to_string(),
                id,
                "--dangerously-skip-permissions".into()
            ]
        );
    }

    #[test]
    fn codex_resumes_with_the_subcommand() {
        let launch = build_agent_launch("codex", &[], Some("abc"));
        assert_eq!(launch.args, vec!["resume", "abc"]);
    }

    #[test]
    fn shells_take_no_session() {
        assert!(build_agent_launch("shell", &[], Some("x"))
            .session_id
            .is_none());
    }
}

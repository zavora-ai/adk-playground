//! The terminal side of the headless desk: the agent's turns as they happen, and held cards
//! decided from the keyboard.

use std::collections::HashSet;
use std::io::IsTerminal;
use std::sync::Arc;
use std::time::Duration;

use adk_core::{ApprovalDecision, ApprovalService, Evidence, InboxQuery, Part, SubjectRef};
use adk_session::{GetRequest, SessionService};
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::desk::{APP, SESSION, USER};

/// What the terminal does with a held card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum CardMode {
    /// Asks on the terminal; holds the card when stdin is not a terminal.
    Ask,
    /// Leaves the card held, for the web home or a later decision.
    Hold,
    /// Approves every card.
    Approve,
    /// Declines every card.
    Deny,
}

fn short(text: &str, limit: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if flat.chars().count() <= limit {
        flat
    } else {
        format!("{}…", flat.chars().take(limit).collect::<String>())
    }
}

/// Prints each new event of the desk session: the wake notice, tool calls and their answers,
/// and the agent's text.
pub async fn narrate(sessions: Arc<dyn SessionService>, cancel: CancellationToken) {
    let mut printed = 0usize;
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(Duration::from_millis(500)) => {}
        }
        let Ok(session) = sessions
            .get(GetRequest {
                app_name: APP.into(),
                user_id: USER.into(),
                session_id: SESSION.into(),
                num_recent_events: None,
                after: None,
            })
            .await
        else {
            continue;
        };
        let events = session.events().all();
        for event in events.iter().skip(printed) {
            let Some(content) = &event.llm_response.content else {
                continue;
            };
            for part in &content.parts {
                match part {
                    Part::Text { text } if content.role == "user" && !text.trim().is_empty() => {
                        if let Some(at) = text.find("[kernel wake") {
                            println!("[wake] {}", short(&text[at..], 160));
                        } else {
                            println!("[user] {}", short(text, 200));
                        }
                    }
                    Part::Text { text } if !text.trim().is_empty() => {
                        println!("[agent] {}", text.trim());
                    }
                    Part::FunctionCall { name, args, .. } => {
                        println!("[call] {name} {}", short(&args.to_string(), 220));
                    }
                    Part::FunctionResponse {
                        function_response, ..
                    } => {
                        println!(
                            "[answer] {} {}",
                            function_response.name,
                            short(&function_response.response.to_string(), 260)
                        );
                    }
                    _ => {}
                }
            }
        }
        printed = events.len();
    }
}

/// Lines typed on the terminal, when stdin is one.
fn keyboard() -> Option<mpsc::Receiver<String>> {
    if !std::io::stdin().is_terminal() {
        return None;
    }
    let (tx, rx) = mpsc::channel(8);
    tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if tx.send(line).await.is_err() {
                return;
            }
        }
    });
    Some(rx)
}

/// Shows every held call and decides it: a `vision_watch` call as `watches` says when set,
/// everything else as `mode` says.
pub async fn cards(
    approvals: Arc<dyn ApprovalService>,
    mode: CardMode,
    watches: Option<CardMode>,
    cancel: CancellationToken,
) {
    let mut seen = HashSet::new();
    let mut keys = match mode {
        CardMode::Ask => keyboard(),
        CardMode::Hold | CardMode::Approve | CardMode::Deny => None,
    };
    let me = SubjectRef::human(USER);
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(Duration::from_millis(700)) => {}
        }
        let query = InboxQuery::for_approver(me.clone(), ["owner"])
            .with_app_name(APP)
            .with_limit(20);
        let Ok(page) = approvals.inbox(query).await else {
            continue;
        };
        for request in page.items {
            if !seen.insert(request.id.clone()) {
                continue;
            }
            println!();
            println!("┌ card held for you: {}", request.tool_name);
            println!("│ {}", short(&request.summary, 200));
            if let Some(args) = request.args_redacted.as_object() {
                for (key, value) in args {
                    let text = value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_string);
                    println!("│ {key}: {}", short(&text, 200));
                }
            }
            for evidence in &request.evidence {
                if let Evidence::Image(image) = evidence {
                    println!(
                        "│ evidence: {} ({} × {}, artifact {} v{}, in memory only)",
                        image.digest,
                        image.width,
                        image.height,
                        image.artifact.file_name,
                        image.artifact.version
                    );
                }
            }
            let mode = match watches {
                Some(watches) if request.tool_name == "vision_watch" => watches,
                Some(_) | None => mode,
            };
            let decision = match (mode, keys.as_mut()) {
                (CardMode::Approve, _) => Some(true),
                (CardMode::Deny, _) => Some(false),
                (CardMode::Ask, Some(keys)) => {
                    println!("└ approve? [y/N]");
                    let line = tokio::select! {
                        () = cancel.cancelled() => return,
                        line = keys.recv() => line.unwrap_or_default(),
                    };
                    Some(line.trim().eq_ignore_ascii_case("y"))
                }
                (CardMode::Ask | CardMode::Hold, _) => {
                    println!(
                        "└ held: it waits for a decision (run with --cards ask in a terminal, or use `serve`)"
                    );
                    None
                }
            };
            let Some(approve) = decision else { continue };
            let decided = if approve {
                ApprovalDecision::approve(me.clone())
            } else {
                ApprovalDecision::reject(me.clone(), "declined on the terminal")
            }
            .with_groups(["owner"]);
            match approvals
                .decide(&request.id, request.version, decided)
                .await
            {
                Ok(_) => println!("└ {}", if approve { "approved" } else { "declined" }),
                Err(error) => println!("└ the decision was not recorded: {error}"),
            }
        }
    }
}

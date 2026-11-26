//! The terminal side of a headless run: each second look as it happens, and the parts put to
//! the engineer decided as `--cards` says.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use adk_core::{ApprovalDecision, ApprovalService, InboxQuery, Part, SubjectRef};
use adk_session::GetRequest;
use tokio_util::sync::CancellationToken;

use crate::agent::{APP, Desk, USER};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum CardMode {
    /// Leaves each part on the engineer's desk; the summary counts them as stopped.
    Hold,
    /// Accepts the agent's suggestion for each.
    Approve,
    /// Declines each.
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

/// Prints each second look's calls, answers and the agent's text.
pub async fn narrate(desk: Arc<Desk>, cancel: CancellationToken) {
    let mut printed: HashMap<String, usize> = HashMap::new();
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(Duration::from_millis(400)) => {}
        }
        for id in desk.started() {
            let Ok(session) = desk
                .sessions
                .get(GetRequest {
                    app_name: APP.into(),
                    user_id: USER.into(),
                    session_id: id.clone(),
                    num_recent_events: None,
                    after: None,
                })
                .await
            else {
                continue;
            };
            let events = session.events().all();
            let done = printed.entry(id.clone()).or_default();
            for event in events.iter().skip(*done) {
                let Some(content) = &event.llm_response.content else {
                    continue;
                };
                for part in &content.parts {
                    match part {
                        Part::Text { text }
                            if content.role != "user" && !text.trim().is_empty() =>
                        {
                            println!("[{id}] agent: {}", short(text.trim(), 300));
                        }
                        Part::FunctionCall { name, args, .. } => {
                            println!("[{id}] call {name} {}", short(&args.to_string(), 200))
                        }
                        Part::FunctionResponse {
                            function_response, ..
                        } => {
                            println!(
                                "[{id}] answer {} {}",
                                function_response.name,
                                short(&function_response.response.to_string(), 220)
                            );
                        }
                        _ => {}
                    }
                }
            }
            *done = events.len();
        }
    }
}

pub async fn cards(approvals: Arc<dyn ApprovalService>, mode: CardMode, cancel: CancellationToken) {
    let mut seen = HashSet::new();
    let me = SubjectRef::human(USER);
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(Duration::from_millis(600)) => {}
        }
        let query = InboxQuery::for_approver(me.clone(), ["owner"])
            .with_app_name(APP)
            .with_limit(100);
        let Ok(page) = approvals.inbox(query).await else {
            continue;
        };
        for request in page.items {
            if !seen.insert(request.id.clone()) {
                continue;
            }
            let args = &request.args_redacted;
            println!(
                "┌ [{}] on the engineer's desk: {}",
                args["id"].as_str().unwrap_or_default(),
                short(args["title"].as_str().unwrap_or_default(), 140)
            );
            println!(
                "│ reasoner: {}",
                short(args["reasoner"].as_str().unwrap_or_default(), 200)
            );
            println!(
                "│ second:   {}",
                short(args["verifier"].as_str().unwrap_or_default(), 200)
            );
            println!(
                "│ measured {} · suggests {}",
                args["measured"].as_str().unwrap_or("—"),
                args["suggestion"].as_str().unwrap_or_default()
            );
            let approve = match mode {
                CardMode::Hold => {
                    println!("└ held: it waits on the engineer's desk (`serve` shows it)");
                    continue;
                }
                CardMode::Approve => true,
                CardMode::Deny => false,
            };
            let decision = if approve {
                ApprovalDecision::approve(me.clone())
            } else {
                ApprovalDecision::reject(me.clone(), "declined by the engineer")
            }
            .with_groups(["owner"]);
            match approvals
                .decide(&request.id, request.version, decision)
                .await
            {
                Ok(_) => println!("└ {}", if approve { "accepted" } else { "declined" }),
                Err(error) => println!("└ the decision was not recorded: {error}"),
            }
        }
    }
}

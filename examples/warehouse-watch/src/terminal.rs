//! The terminal side of a headless run: each incident's turn as it happens, and the
//! supervisor's held cards decided as `--cards` says.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use adk_core::{ApprovalDecision, ApprovalService, InboxQuery, Part, SubjectRef};
use adk_session::{GetRequest, SessionService};
use tokio_util::sync::CancellationToken;

use crate::watch::{APP, Incidents, USER};

/// What the terminal does with a held incident card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum CardMode {
    /// Leaves each card held for the supervisor; the summary lists them.
    Hold,
    /// Signs off every card, as a supervisor who agrees with each would.
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

/// Prints each incident session's new events: tool calls, their answers and the agent's text.
pub async fn narrate(
    sessions: Arc<dyn SessionService>,
    incidents: Arc<Incidents>,
    cancel: CancellationToken,
) {
    let mut printed: HashMap<String, usize> = HashMap::new();
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(Duration::from_millis(500)) => {}
        }
        for incident in incidents.all() {
            let Ok(session) = sessions
                .get(GetRequest {
                    app_name: APP.into(),
                    user_id: USER.into(),
                    session_id: incident.id.clone(),
                    num_recent_events: None,
                    after: None,
                })
                .await
            else {
                continue;
            };
            let events = session.events().all();
            let done = printed.entry(incident.id.clone()).or_default();
            for event in events.iter().skip(*done) {
                let Some(content) = &event.llm_response.content else {
                    continue;
                };
                for part in &content.parts {
                    let id = &incident.id;
                    match part {
                        Part::Text { text }
                            if content.role != "user" && !text.trim().is_empty() =>
                        {
                            println!("[{id}] agent: {}", short(text.trim(), 400));
                        }
                        Part::FunctionCall { name, args, .. } => {
                            println!("[{id}] call {name} {}", short(&args.to_string(), 200));
                        }
                        Part::FunctionResponse {
                            function_response, ..
                        } => {
                            println!(
                                "[{id}] answer {} {}",
                                function_response.name,
                                short(&function_response.response.to_string(), 300)
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

/// Decides every held card as `mode` says, printing it first.
pub async fn cards(approvals: Arc<dyn ApprovalService>, mode: CardMode, cancel: CancellationToken) {
    let mut seen = HashSet::new();
    let me = SubjectRef::human(USER);
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(Duration::from_millis(700)) => {}
        }
        let query = InboxQuery::for_approver(me.clone(), ["owner"])
            .with_app_name(APP)
            .with_limit(50);
        let Ok(page) = approvals.inbox(query).await else {
            continue;
        };
        for request in page.items {
            if !seen.insert(request.id.clone()) {
                continue;
            }
            let title = request.args_redacted["title"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let id = request.args_redacted["id"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            println!("┌ [{id}] held for the supervisor: {}", short(&title, 160));
            println!(
                "│ {}",
                short(
                    request.args_redacted["detail"].as_str().unwrap_or_default(),
                    300
                )
            );
            let approve = match mode {
                CardMode::Hold => {
                    println!("└ held: it waits on the supervisor's desk (`serve` shows it)");
                    continue;
                }
                CardMode::Approve => true,
                CardMode::Deny => false,
            };
            let decision = if approve {
                ApprovalDecision::approve(me.clone())
            } else {
                ApprovalDecision::reject(me.clone(), "declined by the supervisor")
            }
            .with_groups(["owner"]);
            match approvals
                .decide(&request.id, request.version, decision)
                .await
            {
                Ok(_) => println!("└ {}", if approve { "signed off" } else { "declined" }),
                Err(error) => println!("└ the decision was not recorded: {error}"),
            }
        }
    }
}

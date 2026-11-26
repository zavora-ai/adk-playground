//! The terminal side of a headless run: each invoice's turn as it happens, and the clerk's
//! held exceptions decided as `--cards` says.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adk_core::{ApprovalDecision, ApprovalService, InboxQuery, Part, SubjectRef};
use adk_session::{GetRequest, SessionService};
use tokio_util::sync::CancellationToken;

use crate::desk::{APP, USER};

/// What the terminal does with a held exception.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum CardMode {
    /// Leaves each exception on the clerk's desk; the summary lists them.
    Hold,
    /// Accepts every exception, as a clerk who agrees with each would.
    Approve,
    /// Declines every exception.
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

/// Prints each started document's new events: calls, their answers and the agent's text.
pub async fn narrate(
    sessions: Arc<dyn SessionService>,
    docs: Arc<Mutex<Vec<String>>>,
    cancel: CancellationToken,
) {
    let mut printed: HashMap<String, usize> = HashMap::new();
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(Duration::from_millis(400)) => {}
        }
        let started = docs.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for doc in started {
            let Ok(session) = sessions
                .get(GetRequest {
                    app_name: APP.into(),
                    user_id: USER.into(),
                    session_id: doc.clone(),
                    num_recent_events: None,
                    after: None,
                })
                .await
            else {
                continue;
            };
            let events = session.events().all();
            let done = printed.entry(doc.clone()).or_default();
            for event in events.iter().skip(*done) {
                let Some(content) = &event.llm_response.content else {
                    continue;
                };
                for part in &content.parts {
                    match part {
                        Part::Text { text }
                            if content.role != "user" && !text.trim().is_empty() =>
                        {
                            println!("[{doc}] agent: {}", short(text.trim(), 300));
                        }
                        Part::FunctionCall { name, args, .. } => {
                            println!("[{doc}] call {name} {}", short(&args.to_string(), 220));
                        }
                        Part::FunctionResponse {
                            function_response, ..
                        } => {
                            println!(
                                "[{doc}] answer {} {}",
                                function_response.name,
                                short(&function_response.response.to_string(), 200)
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

/// Decides every held exception as `mode` says, printing it first.
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
            let doc = args["doc"].as_str().unwrap_or_default();
            println!(
                "┌ [{doc}] on the clerk's desk ({}): {}",
                args["kind"].as_str().unwrap_or_default(),
                short(args["title"].as_str().unwrap_or_default(), 140)
            );
            println!(
                "│ {}",
                short(args["detail"].as_str().unwrap_or_default(), 300)
            );
            let approve = match mode {
                CardMode::Hold => {
                    println!("└ held: it waits on the clerk's desk (`serve` shows it)");
                    continue;
                }
                CardMode::Approve => true,
                CardMode::Deny => false,
            };
            let decision = if approve {
                ApprovalDecision::approve(me.clone())
            } else {
                ApprovalDecision::reject(me.clone(), "declined by the clerk")
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

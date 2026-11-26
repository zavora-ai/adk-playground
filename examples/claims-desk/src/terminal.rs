//! The terminal side of a headless run: each claim's turn as it happens, and the estimates
//! and referrals held for the adjuster decided as `--cards` says.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adk_core::{ApprovalDecision, ApprovalService, InboxQuery, Part, SubjectRef};
use adk_session::{GetRequest, SessionService};
use tokio_util::sync::CancellationToken;

use crate::desk::{APP, USER};

/// What the terminal does with a held estimate or referral.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum CardMode {
    /// Leaves each on the adjuster's desk.
    Hold,
    /// Approves each, as an adjuster who agrees with each would.
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

/// Prints each started claim's new events: calls, their answers and the agent's text.
pub async fn narrate(
    sessions: Arc<dyn SessionService>,
    claims: Arc<Mutex<Vec<String>>>,
    cancel: CancellationToken,
) {
    let mut printed: HashMap<String, usize> = HashMap::new();
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(Duration::from_millis(400)) => {}
        }
        let started = claims.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for claim in started {
            let Ok(session) = sessions
                .get(GetRequest {
                    app_name: APP.into(),
                    user_id: USER.into(),
                    session_id: claim.clone(),
                    num_recent_events: None,
                    after: None,
                })
                .await
            else {
                continue;
            };
            let events = session.events().all();
            let done = printed.entry(claim.clone()).or_default();
            for event in events.iter().skip(*done) {
                let Some(content) = &event.llm_response.content else {
                    continue;
                };
                for part in &content.parts {
                    match part {
                        Part::Text { text }
                            if content.role != "user" && !text.trim().is_empty() =>
                        {
                            println!("[{claim}] agent: {}", short(text.trim(), 300));
                        }
                        Part::FunctionCall { name, args, .. } => {
                            println!("[{claim}] call {name} {}", short(&args.to_string(), 220));
                        }
                        Part::FunctionResponse {
                            function_response, ..
                        } => {
                            println!(
                                "[{claim}] answer {} {}",
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

/// Decides every held estimate and referral as `mode` says, printing it first.
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
            let claim = args["claim"].as_str().unwrap_or_default();
            let what = if request.tool_name == "refer_claim" {
                format!("referral ({})", args["reason"].as_str().unwrap_or_default())
            } else {
                format!(
                    "estimate ({})",
                    args["severity"].as_str().unwrap_or_default()
                )
            };
            println!(
                "┌ [{claim}] on the adjuster's desk, {what}: {}",
                short(args["title"].as_str().unwrap_or_default(), 140)
            );
            println!(
                "│ {}",
                short(
                    args["notes"]
                        .as_str()
                        .or(args["detail"].as_str())
                        .unwrap_or_default(),
                    300
                )
            );
            let approve = match mode {
                CardMode::Hold => {
                    println!("└ held: it waits on the adjuster's desk (`serve` shows it)");
                    continue;
                }
                CardMode::Approve => true,
                CardMode::Deny => false,
            };
            let decision = if approve {
                ApprovalDecision::approve(me.clone())
            } else {
                ApprovalDecision::reject(me.clone(), "declined by the adjuster")
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

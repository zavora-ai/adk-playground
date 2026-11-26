//! The measurement: what the desk read and decided for each document, against FATURA's
//! annotations and the decision the policy gives the generated ERP.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::data::{Arrival, Books, Header};

/// What the agent did with one document: its last `post_invoice` or `raise_exception`.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub doc: String,
    pub decision: Option<String>,
    pub fields: Option<Value>,
    /// `vision_describe` calls: crops sent to the cloud.
    pub crops: usize,
}

impl Outcome {
    pub fn from_calls(doc: &str, calls: &[(String, Value)]) -> Self {
        let mut outcome = Self {
            doc: doc.to_string(),
            ..Self::default()
        };
        for (name, args) in calls {
            match name.as_str() {
                "post_invoice" => {
                    outcome.decision = Some("post".into());
                    outcome.fields = Some(args.clone());
                }
                "raise_exception" => {
                    outcome.decision = args["kind"].as_str().map(str::to_string);
                    outcome.fields = Some(args["fields"].clone());
                }
                "vision_describe" => outcome.crops += 1,
                _ => {}
            }
        }
        outcome
    }
}

pub const FIELDS: [&str; 6] = ["supplier", "number", "date", "po", "total", "currency"];

fn alnum(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// Whether the agent's value for `field` is the annotation's.
pub fn field_ok(field: &str, expected: &Header, got: &Value) -> bool {
    let text = |key: &str| {
        got[key]
            .as_str()
            .map(str::trim)
            .unwrap_or_default()
            .to_string()
    };
    match field {
        "supplier" => alnum(&text("supplier")) == alnum(&expected.supplier),
        "number" => alnum(&text("number")) == alnum(&expected.number),
        "date" => text("date") == expected.date,
        "total" => got["total"]
            .as_f64()
            .is_some_and(|t| (t - expected.total).abs() < 0.005),
        "currency" => text("currency").eq_ignore_ascii_case(&expected.currency),
        "po" => {
            let got = text("po");
            let got = got.trim_start_matches('0');
            match &expected.po {
                Some(po) => !got.is_empty() && got == po.trim_start_matches('0'),
                None => got.is_empty(),
            }
        }
        _ => false,
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub documents: usize,
    pub decided: usize,
    /// Correct fields over all documents, by field.
    pub fields: BTreeMap<String, (usize, usize)>,
    /// Documents with every field right, by how they arrived.
    pub whole_by_arrival: BTreeMap<String, (usize, usize)>,
    pub decisions_right: usize,
    /// Expected decision → the desk's → count.
    pub confusion: BTreeMap<String, BTreeMap<String, usize>>,
    /// Exceptions the policy calls for, and how many the desk raised.
    pub exceptions_expected: usize,
    pub exceptions_caught: usize,
    /// Invoices the policy posts that the desk held instead.
    pub false_exceptions: usize,
    /// Invoices the desk posted that the policy holds.
    pub wrongly_posted: usize,
    pub crops: usize,
}

pub fn score(books: &Books, outcomes: &[Outcome]) -> Report {
    let mut report = Report {
        documents: outcomes.len(),
        ..Report::default()
    };
    for outcome in outcomes {
        let Some(expected) = books.expected.iter().find(|e| e.doc == outcome.doc) else {
            continue;
        };
        let want = expected.decision.as_str();
        let got = outcome.decision.clone().unwrap_or_else(|| "none".into());
        report.crops += outcome.crops;
        if outcome.decision.is_some() {
            report.decided += 1;
        }
        *report
            .confusion
            .entry(want.into())
            .or_default()
            .entry(got.clone())
            .or_default() += 1;
        if got == want {
            report.decisions_right += 1;
        }
        if want != "post" {
            report.exceptions_expected += 1;
            if got != "post" && got != "none" {
                report.exceptions_caught += 1;
            }
        } else if got != "post" && got != "none" {
            report.false_exceptions += 1;
        }
        if want != "post" && got == "post" {
            report.wrongly_posted += 1;
        }
        let fields = outcome.fields.clone().unwrap_or(Value::Null);
        let mut whole = true;
        for field in FIELDS {
            let ok = field_ok(field, &expected.header, &fields);
            let entry = report.fields.entry(field.into()).or_default();
            entry.1 += 1;
            if ok {
                entry.0 += 1;
            }
            whole &= ok;
        }
        let arrival = match expected.arrival {
            Arrival::Clean => "clean",
            Arrival::Phone => "phone photo",
            Arrival::Resent => "re-sent scan",
        };
        let entry = report.whole_by_arrival.entry(arrival.into()).or_default();
        entry.1 += 1;
        if whole {
            entry.0 += 1;
        }
    }
    report
}

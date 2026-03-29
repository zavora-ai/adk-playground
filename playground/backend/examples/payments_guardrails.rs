use adk_guardrail::Severity;
use adk_payments::domain::{
    Cart, CartLine, CommerceActor, CommerceActorRole, CommerceMode, EvidenceReference, MerchantRef,
    Money, ProtocolDescriptor, ProtocolExtensions, TransactionId, TransactionRecord,
};
use adk_payments::guardrail::{
    redact_payment_text, redact_payment_value, AmountThresholdGuardrail,
    MerchantAllowlistGuardrail, PaymentPolicyGuardrail, PaymentPolicySet,
    SensitivePaymentDataGuardrail,
};
use chrono::{TimeZone, Utc};

// ── Payment Guardrails — Policy Enforcement & Data Redaction ──
// ADK-Rust enforces commerce-specific guardrails before any payment persists.
//
// Key concepts:
//   - `AmountThresholdGuardrail` — soft-review + hard-stop on transaction totals
//   - `MerchantAllowlistGuardrail` — restrict payments to approved merchants
//   - `PaymentPolicySet` — ordered evaluation, deny > escalate > allow
//   - `SensitivePaymentDataGuardrail` — redacts card numbers, CVCs, billing PII
//   - `redact_payment_text` / `redact_payment_value` — standalone redaction helpers
//
// No LLM needed — this demonstrates the policy engine directly.

fn make_record(merchant_id: &str, merchant_name: &str, amount_minor: i64) -> TransactionRecord {
    TransactionRecord::new(
        TransactionId::from(format!("tx-{merchant_id}").as_str()),
        CommerceActor {
            actor_id: "shopper-agent".to_string(),
            role: CommerceActorRole::AgentSurface,
            display_name: Some("Shopping Assistant".to_string()),
            tenant_id: Some("tenant-1".to_string()),
            extensions: ProtocolExtensions::default(),
        },
        MerchantRef {
            merchant_id: merchant_id.to_string(),
            legal_name: merchant_name.to_string(),
            display_name: Some(merchant_name.to_string()),
            statement_descriptor: None,
            country_code: Some("US".to_string()),
            website: None,
            extensions: ProtocolExtensions::default(),
        },
        CommerceMode::HumanPresent,
        Cart {
            cart_id: Some("cart-1".to_string()),
            lines: vec![CartLine {
                line_id: "line-1".to_string(),
                merchant_sku: Some("sku-1".to_string()),
                title: "Purchase".to_string(),
                quantity: 1,
                unit_price: Money::new("USD", amount_minor, 2),
                total_price: Money::new("USD", amount_minor, 2),
                product_class: None,
                extensions: ProtocolExtensions::default(),
            }],
            subtotal: Some(Money::new("USD", amount_minor, 2)),
            adjustments: Vec::new(),
            total: Money::new("USD", amount_minor, 2),
            affiliate_attribution: None,
            extensions: ProtocolExtensions::default(),
        },
        Utc.with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
            .single()
            .expect("valid constant date"),
    )
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== 🛡️ Payment Guardrails — Policy Enforcement & Data Redaction ===\n");

    let protocol = ProtocolDescriptor::acp("2026-01-30");

    // ── 1. Amount threshold guardrail ──
    println!("── 1. Amount Threshold Guardrail ──\n");

    let amount_guard = AmountThresholdGuardrail::new(
        Some(10_000),  // $100.00 review threshold
        Some(100_000), // $1,000.00 hard limit
    );

    let test_amounts = [
        ("Small purchase", 5_000),   // $50.00
        ("Medium purchase", 25_000), // $250.00
        ("Large purchase", 150_000), // $1,500.00
    ];

    for (label, amount) in &test_amounts {
        let record = make_record("merchant-1", "TechStore", *amount);
        let decision = amount_guard.evaluate(&record, &protocol);
        let dollars = *amount as f64 / 100.0;
        let icon = if decision.is_allow() {
            "✓"
        } else if decision.is_escalate() {
            "⚠"
        } else {
            "✗"
        };
        let status = if decision.is_allow() {
            "ALLOW".to_string()
        } else if decision.is_escalate() {
            format!("ESCALATE — {}", decision.findings()[0].reason)
        } else {
            format!("DENY — {}", decision.findings()[0].reason)
        };
        println!("  {icon} {label} (${dollars:.2}): {status}");
    }
    println!();

    // ── 2. Merchant allowlist guardrail ──
    println!("── 2. Merchant Allowlist Guardrail ──\n");

    let merchant_guard =
        MerchantAllowlistGuardrail::new(["merchant-approved-1", "merchant-approved-2"]);

    let merchants = [
        ("merchant-approved-1", "Approved Store"),
        ("merchant-blocked-99", "Blocked Store"),
    ];

    for (id, name) in &merchants {
        let record = make_record(id, name, 5_000);
        let decision = merchant_guard.evaluate(&record, &protocol);
        let icon = if decision.is_allow() { "✓" } else { "✗" };
        let status = if decision.is_allow() {
            "ALLOW".to_string()
        } else {
            format!("DENY — {}", decision.findings()[0].reason)
        };
        println!("  {icon} {name} ({id}): {status}");
    }
    println!();

    // ── 3. Combined policy set (deny > escalate > allow) ──
    println!("── 3. Combined Policy Set — Deny Takes Precedence ──\n");

    let policy_set = PaymentPolicySet::new()
        .with(AmountThresholdGuardrail::new(Some(10_000), Some(100_000)))
        .with(MerchantAllowlistGuardrail::new(["merchant-approved-1"]));

    let scenarios = [
        (
            "Approved + small",
            "merchant-approved-1",
            "Approved Store",
            5_000,
        ),
        (
            "Approved + medium",
            "merchant-approved-1",
            "Approved Store",
            25_000,
        ),
        (
            "Approved + huge",
            "merchant-approved-1",
            "Approved Store",
            150_000,
        ),
        (
            "Blocked + small",
            "merchant-blocked-99",
            "Blocked Store",
            5_000,
        ),
        (
            "Blocked + huge",
            "merchant-blocked-99",
            "Blocked Store",
            150_000,
        ),
    ];

    for (label, id, name, amount) in &scenarios {
        let record = make_record(id, name, *amount);
        let decision = policy_set.evaluate(&record, &protocol);
        let dollars = *amount as f64 / 100.0;
        let icon = if decision.is_allow() {
            "✓"
        } else if decision.is_escalate() {
            "⚠"
        } else {
            "✗"
        };
        let status = if decision.is_allow() {
            "ALLOW".to_string()
        } else {
            let findings: Vec<String> = decision
                .findings()
                .iter()
                .map(|f| format!("[{}] {}", f.guardrail, f.reason))
                .collect();
            let kind = if decision.is_deny() {
                "DENY"
            } else {
                "ESCALATE"
            };
            format!("{kind} — {}", findings.join("; "))
        };
        println!("  {icon} {label:20} (${dollars:.2}): {status}");
    }

    println!(
        "\n  Severity ranking: {:?} < {:?} < {:?} < {:?}",
        Severity::Low,
        Severity::Medium,
        Severity::High,
        Severity::Critical
    );
    println!();

    // ── 4. Sensitive data redaction ──
    println!("── 4. Sensitive Payment Data Redaction ──\n");

    let redactor = SensitivePaymentDataGuardrail::new();

    let sensitive_texts = [
        "Card: 4111111111111111, CVC: 123, Exp: 12/28",
        "Payment token: sk_live_abc123def456 for billing address: 123 Main St, Springfield IL 62701",
        "Transaction authorized with card 5500000000000004",
    ];

    for text in &sensitive_texts {
        let redacted = redactor.redact_text(text);
        println!("  Original:  {text}");
        println!("  Redacted:  {redacted}\n");
    }

    // Standalone helpers
    println!("  Standalone redaction helpers:");
    let card_text = "Pay with 4242424242424242 please";
    println!(
        "    redact_payment_text: \"{}\"",
        redact_payment_text(card_text)
    );

    let json_val = serde_json::json!({
        "card_number": "4111111111111111",
        "amount": 2500,
        "cvc": "456"
    });
    let redacted_val = redact_payment_value(&json_val);
    println!(
        "    redact_payment_value: {}\n",
        serde_json::to_string_pretty(&redacted_val)?
    );

    // ── 5. Evidence references (immutable audit trail) ──
    println!("── 5. Evidence References — Immutable Audit Trail ──\n");

    let mut record = make_record("merchant-approved-1", "TechStore", 50_000);

    let evidence = EvidenceReference {
        evidence_id: "ev-checkout-001".to_string(),
        protocol: ProtocolDescriptor::acp("2026-01-30"),
        artifact_kind: "checkout_session".to_string(),
        digest: Some("sha256:a1b2c3d4e5f6".to_string()),
    };
    record.attach_evidence_ref(evidence);

    let evidence2 = EvidenceReference {
        evidence_id: "ev-receipt-001".to_string(),
        protocol: ProtocolDescriptor::acp("2026-01-30"),
        artifact_kind: "payment_receipt".to_string(),
        digest: Some("sha256:f6e5d4c3b2a1".to_string()),
    };
    record.attach_evidence_ref(evidence2);

    println!(
        "  {} evidence references attached:",
        record.evidence_refs.len()
    );
    for ev in &record.evidence_refs {
        println!(
            "    {} — {} ({})",
            ev.evidence_id,
            ev.artifact_kind,
            ev.digest.as_deref().unwrap_or("none")
        );
    }
    println!("  Raw artifacts stored in adk-artifact, NOT in agent transcript");
    println!("  Only SafeTransactionSummary appears in conversation\n");

    println!("=== All payment guardrail checks passed! ===");
    Ok(())
}

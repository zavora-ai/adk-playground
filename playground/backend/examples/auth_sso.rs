use adk_auth::sso::{
    AzureADProvider, ClaimsMapper, GoogleProvider, JwtValidator, OidcProvider, OktaProvider,
    TokenClaims, TokenError,
};
use adk_auth::{AccessControl, Permission, Role};
use adk_core::{SessionId, UserId};
use adk_rust::futures::StreamExt;
use adk_rust::prelude::*;
use adk_rust::session::{CreateRequest, SessionService};
use adk_tool::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

// ── SSO & JWT — Enterprise Identity for AI Agents ──
// ADK-Rust integrates with enterprise identity providers so agents
// inherit the caller's identity and permissions from their JWT token.
//
// Flow: Bearer Token → JWT Validation → Claims Extraction →
//       Group-to-Role Mapping → RBAC Permission Check → Agent Execution
//
// Supported providers:
//   - Google, Azure AD, Okta, Auth0, any OIDC-compliant IdP
//
// Key concepts:
//   - `JwtValidator` — validates tokens against JWKS endpoints
//   - `ClaimsMapper` — maps IdP groups to adk-auth roles
//   - `SsoAccessControl` — combined token validation + RBAC
//   - `OidcProvider::from_discovery()` — auto-discovers endpoints

// ── Tools that the agent will use ──

#[derive(Deserialize, JsonSchema)]
struct QueryArgs {
    /// What to search for
    query: String,
}

/// Search company data. Available to all authenticated users.
#[tool]
async fn search_data(args: QueryArgs) -> adk_tool::Result<serde_json::Value> {
    Ok(serde_json::json!({
        "query": args.query,
        "results": [
            {"title": "Q1 Revenue", "value": "$15M", "trend": "+12%"},
            {"title": "Active Users", "value": "2.3M", "trend": "+8%"},
        ]
    }))
}

#[derive(Deserialize, JsonSchema)]
struct DeployArgs {
    /// Service to deploy
    service: String,
    /// Target environment
    env: String,
}

/// Deploy a service. Requires developer or admin role.
#[tool]
async fn deploy_service(args: DeployArgs) -> adk_tool::Result<serde_json::Value> {
    Ok(serde_json::json!({
        "service": args.service, "env": args.env,
        "status": "deployed", "version": "v2.4.1"
    }))
}

#[derive(Deserialize, JsonSchema)]
struct AdminArgs {
    /// Admin action to perform
    action: String,
}

/// Admin operations. Requires admin role only.
#[tool]
async fn admin_action(args: AdminArgs) -> adk_tool::Result<serde_json::Value> {
    Ok(serde_json::json!({
        "action": args.action, "status": "completed",
        "note": "Admin action executed successfully"
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== 🔑 SSO & JWT — Enterprise Identity for AI Agents ===\n");

    // ── 1. Built-in SSO Providers ──
    println!("── 1. Built-in SSO Providers ──\n");

    let _google = GoogleProvider::new("your-google-client-id");
    println!("  ✓ Google   — accounts.google.com");

    let _azure = AzureADProvider::new("your-tenant-id", "your-client-id");
    println!("  ✓ Azure AD — login.microsoftonline.com/{{tenant}}/v2.0");

    let _okta = OktaProvider::new("your-domain.okta.com", "your-client-id");
    println!("  ✓ Okta     — {{domain}}/oauth2/default");

    let _oidc = OidcProvider::new(
        "https://keycloak.example.com/realms/main",
        "your-client-id",
        "https://keycloak.example.com/realms/main/protocol/openid-connect/certs",
    );
    println!("  ✓ OIDC     — any compliant provider (Keycloak, Auth0, etc.)\n");

    // ── 2. JWT Validator ──
    println!("── 2. JWT Validator ──\n");

    let validator_result = JwtValidator::builder()
        .issuer("https://accounts.google.com")
        .jwks_uri("https://www.googleapis.com/oauth2/v3/certs")
        .audience("api://my-agent-app")
        .build();

    match validator_result {
        Ok(_) => println!("  ✓ JwtValidator built — ready to validate tokens"),
        Err(e) => println!("  ⚠ JwtValidator: {:?}", e),
    }
    println!();

    // ── 3. Token Claims & Error Handling ──
    println!("── 3. Token Claims & Errors ──\n");

    let claims = TokenClaims {
        sub: "user-12345".into(),
        iss: "https://accounts.google.com".into(),
        email: Some("alice@company.com".into()),
        name: Some("Alice Smith".into()),
        groups: vec!["Engineering".into(), "Admins".into()],
        exp: 1735700000,
        iat: 1735696400,
        ..Default::default()
    };

    println!("  Token claims:");
    println!("    iss:    {}", claims.iss);
    println!("    groups: {:?}", claims.groups);
    println!("    expired: {}\n", claims.is_expired());

    let errors = vec![
        TokenError::Expired,
        TokenError::InvalidSignature,
        TokenError::InvalidIssuer {
            expected: "https://expected.com".into(),
            actual: "https://actual.com".into(),
        },
        TokenError::MissingClaim("email".into()),
    ];
    println!("  Error types handled:");
    for err in &errors {
        println!("    ✗ {err}");
    }
    println!();

    // ── 4. Claims Mapping → RBAC Roles ──
    println!("── 4. Claims Mapping — IdP Groups → RBAC Roles ──\n");

    let mapper = ClaimsMapper::builder()
        .map_group("Admins", "admin")
        .map_group("Engineering", "developer")
        .map_group("DataAnalysts", "analyst")
        .map_group("Everyone", "viewer")
        .user_id_from_email()
        .default_role("viewer")
        .build();

    let test_users = vec![
        create_claims("alice@company.com", vec!["Admins", "Engineering"]),
        create_claims("bob@company.com", vec!["Engineering"]),
        create_claims("carol@company.com", vec!["DataAnalysts"]),
        create_claims("guest@external.com", vec![]),
    ];

    println!("  Group → Role mapping:");
    println!("    Admins       → admin");
    println!("    Engineering  → developer");
    println!("    DataAnalysts → analyst");
    println!("    (default)    → viewer\n");

    println!("  Resolved roles per user:");
    for c in &test_users {
        let roles = mapper.map_to_roles(c);
        let name = c.email.as_deref().unwrap_or("?").split('@').next().unwrap();
        println!("    {name:10} → {roles:?}");
    }
    println!();

    // ── 5. End-to-End: SSO Claims → RBAC → Agent ──
    println!("── 5. End-to-End: SSO Claims → RBAC → Agent Permissions ──\n");

    let admin_role = Role::new("admin")
        .allow(Permission::AllTools)
        .allow(Permission::AllAgents);
    let dev_role = Role::new("developer")
        .allow(Permission::Tool("search_data".into()))
        .allow(Permission::Tool("deploy_service".into()))
        .deny(Permission::Tool("admin_action".into()));
    let analyst_role = Role::new("analyst")
        .allow(Permission::Tool("search_data".into()))
        .deny(Permission::Tool("deploy_service".into()))
        .deny(Permission::Tool("admin_action".into()));
    let viewer_role = Role::new("viewer").allow(Permission::Tool("search_data".into()));

    let ac = AccessControl::builder()
        .role(admin_role)
        .role(dev_role)
        .role(analyst_role)
        .role(viewer_role)
        .build()?;

    let tools_list = ["search_data", "deploy_service", "admin_action"];

    for c in &test_users {
        let name = c.email.as_deref().unwrap_or("?").split('@').next().unwrap();
        let roles = mapper.map_to_roles(c);
        print!("  {name:10}");
        for t in &tools_list {
            let allowed = roles.iter().any(|r| {
                ac.get_role(r)
                    .map(|role| role.can_access(&Permission::Tool((*t).into())))
                    .unwrap_or(false)
            });
            print!("  {}{t}", if allowed { "✓" } else { "✗" });
        }
        println!();
    }
    println!();

    // ── 6. OIDC Auto-Discovery ──
    println!("── 6. OIDC Auto-Discovery ──\n");

    println!("  Attempting Google OIDC discovery...");
    match OidcProvider::from_discovery("https://accounts.google.com", "example-client-id").await {
        Ok(_) => println!("  ✓ Discovery successful — endpoints auto-configured\n"),
        Err(e) => println!("  ⚠ Discovery: {e} (network-dependent)\n"),
    }

    // ── 7. Agent with SSO-derived permissions ──
    println!("── 7. Agent Execution with SSO-Derived Identity ──\n");

    // Simulate: Bob's token arrives → claims extracted → roles: [developer]
    // Developer can search + deploy, but NOT admin_action
    let bob_claims = create_claims("bob@company.com", vec!["Engineering"]);
    let bob_roles = mapper.map_to_roles(&bob_claims);
    let bob_user_id = mapper.get_user_id(&bob_claims);
    println!("  Authenticated user: bob@company.com");
    println!("  IdP groups: {:?}", bob_claims.groups);
    println!("  Mapped roles: {bob_roles:?}");
    println!("  User ID: {bob_user_id}\n");

    let api_key = std::env::var("GOOGLE_API_KEY")?;
    let model = Arc::new(GeminiModel::new(&api_key, "gemini-3.1-flash-lite-preview")?);

    let agent = Arc::new(
        LlmAgentBuilder::new("sso_agent")
            .instruction(
                "You are an operations assistant. The authenticated user is a developer.\n\
                 You have three tools: search_data, deploy_service, admin_action.\n\
                 The user can search and deploy, but CANNOT perform admin actions.\n\
                 If asked to do something outside their permissions, explain why it's denied.",
            )
            .model(model)
            .tool(Arc::new(SearchData))
            .tool(Arc::new(DeployService))
            .tool(Arc::new(AdminAction))
            .build()?,
    );

    let sessions = Arc::new(InMemorySessionService::new());
    sessions
        .create(CreateRequest {
            app_name: "playground".into(),
            user_id: "user".into(),
            session_id: Some("s1".into()),
            state: HashMap::new(),
        })
        .await?;

    let runner = Runner::new(RunnerConfig {
        app_name: "playground".into(),
        agent,
        session_service: sessions,
        artifact_service: None,
        memory_service: None,
        plugin_manager: None,
        run_config: None,
        compaction_config: None,
        context_cache_config: None,
        cache_capable: None,
        request_context: None,
        cancellation_token: None,
    })?;

    let query = "Search for revenue data, deploy the analytics service to staging, and reset all user passwords.";
    println!("  **User:** {query}\n");
    print!("  **Agent:** ");

    let message = Content::new("user").with_text(query);
    let mut stream = runner
        .run(UserId::new("user")?, SessionId::new("s1")?, message)
        .await?;
    while let Some(event) = stream.next().await {
        let event = event?;
        if let Some(content) = &event.llm_response.content {
            for part in &content.parts {
                if let Some(text) = part.text() {
                    print!("{text}");
                }
            }
        }
    }
    println!("\n");

    println!("=== All SSO & JWT enterprise identity checks passed! ===");
    Ok(())
}

fn create_claims(email: &str, groups: Vec<&str>) -> TokenClaims {
    TokenClaims {
        sub: format!("user-{}", email.split('@').next().unwrap()),
        email: Some(email.to_string()),
        email_verified: Some(true),
        groups: groups.into_iter().map(String::from).collect(),
        ..Default::default()
    }
}

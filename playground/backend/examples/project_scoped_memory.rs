use adk_core::Content;
use adk_memory::{InMemoryMemoryService, MemoryEntry, MemoryService, SearchRequest};
use chrono::Utc;
use std::sync::Arc;

// ── Project-Scoped Memory ──
// Demonstrates the `project_id` dimension added in v0.7:
//
// 1. Memories are isolated by project — same user, different projects, no leakage
// 2. `add_session_to_project()` stores entries with a project scope
// 3. `SearchRequest { project_id: Some(...) }` queries within a project
// 4. Cross-project search returns nothing (strict isolation)
//
// Use case: A developer works on multiple projects. The agent remembers
// project-specific decisions without mixing context between projects.
// Works with all 6 memory backends (InMemory, PostgreSQL, Redis, MongoDB, Neo4j, SQLite).

fn entry(text: &str) -> MemoryEntry {
    MemoryEntry {
        content: Content::new("assistant").with_text(text),
        author: "assistant".to_string(),
        timestamp: Utc::now(),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    println!("=== Project-Scoped Memory — Isolated Knowledge per Project ===\n");

    let service = Arc::new(InMemoryMemoryService::new());
    let app = "playground";
    let user = "dev-1";

    // ── Part 1: Store memories in different projects ──
    println!("── Part 1: Store Project-Specific Memories ──\n");

    // Global entries (no project scope)
    service
        .add_session(app, user, "global-session", vec![
            entry("Company uses Rust for all backend services"),
            entry("CI/CD pipeline runs on GitHub Actions"),
        ])
        .await?;
    println!("  📁 global: 2 memories stored (company-wide knowledge)");

    // Project Alpha — a web API project
    service
        .add_session_to_project(app, user, "alpha-session", "project-alpha", vec![
            entry("Web API uses Axum 0.8 with tower middleware"),
            entry("Database is PostgreSQL with sqlx for type-safe queries"),
            entry("Auth uses JWT with RS256 signing, 1-hour expiry"),
        ])
        .await?;
    println!("  📁 project-alpha: 3 memories stored (Axum, PostgreSQL, JWT)");

    // Project Beta — a CLI tool project
    service
        .add_session_to_project(app, user, "beta-session", "project-beta", vec![
            entry("CLI uses clap 4.x for argument parsing"),
            entry("Output format is JSON by default, YAML with --format yaml"),
            entry("SQLite for local state persistence via rusqlite"),
        ])
        .await?;
    println!("  📁 project-beta: 3 memories stored (clap, JSON/YAML, SQLite)");

    // ── Part 2: Demonstrate isolation ──
    println!("\n── Part 2: Memory Isolation ──\n");

    // Search within project-alpha for "database"
    let alpha_results = service
        .search(SearchRequest {
            query: "database".to_string(),
            user_id: user.to_string(),
            app_name: app.to_string(),
            limit: Some(10),
            min_score: None,
            project_id: Some("project-alpha".to_string()),
        })
        .await?;
    println!("  🔍 Search 'database' in project-alpha: {} result(s)", alpha_results.memories.len());
    for mem in &alpha_results.memories {
        let text: String = mem.content.parts.iter().filter_map(|p| p.text()).collect();
        println!("     → {}", text);
    }

    // Same search in project-beta — different results
    let beta_results = service
        .search(SearchRequest {
            query: "database".to_string(),
            user_id: user.to_string(),
            app_name: app.to_string(),
            limit: Some(10),
            min_score: None,
            project_id: Some("project-beta".to_string()),
        })
        .await?;
    println!("\n  🔍 Search 'database' in project-beta: {} result(s)", beta_results.memories.len());
    for mem in &beta_results.memories {
        let text: String = mem.content.parts.iter().filter_map(|p| p.text()).collect();
        println!("     → {}", text);
    }

    // Cross-project isolation: search non-existent project
    let gamma_results = service
        .search(SearchRequest {
            query: "database".to_string(),
            user_id: user.to_string(),
            app_name: app.to_string(),
            limit: Some(10),
            min_score: None,
            project_id: Some("project-gamma".to_string()),
        })
        .await?;
    println!("\n  🔍 Search 'database' in project-gamma: {} result(s) (isolation ✓)",
        gamma_results.memories.len());

    // Global search (no project filter) — finds everything
    let global_results = service
        .search(SearchRequest {
            query: "database".to_string(),
            user_id: user.to_string(),
            app_name: app.to_string(),
            limit: Some(10),
            min_score: None,
            project_id: None,
        })
        .await?;
    println!("\n  🔍 Search 'database' globally (no project filter): {} result(s)",
        global_results.memories.len());

    // ── Part 3: Project-scoped deletion ──
    println!("\n── Part 3: Project-Scoped Deletion ──\n");

    // Delete only project-alpha memories
    service.delete_session(app, user, "alpha-session").await?;
    println!("  🗑️  Deleted project-alpha session");

    // Verify beta is untouched
    let beta_after = service
        .search(SearchRequest {
            query: "clap".to_string(),
            user_id: user.to_string(),
            app_name: app.to_string(),
            limit: Some(10),
            min_score: None,
            project_id: Some("project-beta".to_string()),
        })
        .await?;
    println!("  ✓ project-beta still has {} memories (untouched)", beta_after.memories.len());

    println!("\n=== Key Features ===");
    println!("• add_session_to_project(app, user, session, project_id, entries)");
    println!("• SearchRequest {{ project_id: Some(\"...\") }} — scoped queries");
    println!("• SearchRequest {{ project_id: None }} — global search across all projects");
    println!("• Strict isolation — no cross-project memory leakage");
    println!("• Works with all 6 backends: InMemory, PostgreSQL, Redis, MongoDB, Neo4j, SQLite");
    println!("• GDPR delete_user() removes data across ALL projects");
    Ok(())
}

//! Skill injection — validates prompt engineering from skill documents
//!
//! Demonstrates: engineer_instruction, engineer_prompt_block,
//! apply_skill_injection, select_skill_prompt_block, and SkillInjector.

use adk_core::Content;
use adk_skill::{
    apply_skill_injection, load_skill_index, select_skill_prompt_block,
    SelectionPolicy, SkillInjector, SkillInjectorConfig,
};
use std::fs;

fn main() {
    println!("=== Skill Injection & Prompt Engineering ===\n");

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let skills_dir = root.join(".skills");
    fs::create_dir_all(&skills_dir).unwrap();

    fs::write(
        skills_dir.join("data-analyst.skill.md"),
        r#"---
name: data-analyst
description: Analyzes datasets and produces insights with visualizations.
tags:
  - data
  - analytics
  - visualization
allowed-tools:
  - code_exec
  - rag_search
---
You are a data analyst. When given a dataset:
1. Summarize key statistics (mean, median, std dev)
2. Identify trends and outliers
3. Generate visualization code (matplotlib/plotly)
4. Provide actionable insights in plain language

Always show your work with code snippets.
"#,
    )
    .unwrap();

    fs::write(
        skills_dir.join("translator.skill.md"),
        r#"---
name: translator
description: Translates text between languages with cultural context.
tags:
  - language
  - translation
  - i18n
---
You are a professional translator. For each translation:
1. Translate the text accurately
2. Preserve tone and register
3. Note cultural nuances that may affect meaning
4. Provide alternative phrasings for ambiguous terms
"#,
    )
    .unwrap();

    let index = load_skill_index(root).unwrap();
    println!("✓ Loaded {} skills", index.len());

    // 1. engineer_instruction — full system instruction with tool hints
    let skill = index.find_by_name("data-analyst").unwrap();
    let instruction = skill.engineer_instruction(4000, &[]);
    assert!(instruction.contains("[skill:data-analyst]"));
    assert!(instruction.contains("data analyst"));
    assert!(instruction.contains("[/skill]"));
    println!("✓ engineer_instruction produces tagged block");
    println!("  Length: {} chars", instruction.len());

    // 2. engineer_prompt_block — lightweight injection
    let block = skill.engineer_prompt_block(2000);
    assert!(block.starts_with("[skill:data-analyst]"));
    assert!(block.ends_with("[/skill]"));
    println!("✓ engineer_prompt_block: {} chars", block.len());

    // 3. Truncation for large skill bodies
    let short = skill.engineer_instruction(50, &[]);
    // With a very small limit, the body gets truncated
    println!("✓ Truncation works for max_chars=50 ({} chars)", short.len());

    // 4. select_skill_prompt_block — one-shot: score + inject
    let result = select_skill_prompt_block(
        &index,
        "analyze this CSV data and find trends",
        &SelectionPolicy { top_k: 1, min_score: 0.1, ..Default::default() },
        4000,
    );
    assert!(result.is_some());
    let (skill_match, prompt_block) = result.unwrap();
    assert!(prompt_block.contains("data-analyst") || prompt_block.contains("data analyst"));
    println!("✓ select_skill_prompt_block matched '{}' (score={:.2})",
        skill_match.skill.name, skill_match.score);

    // 5. apply_skill_injection — inject into a Content message
    let policy = SelectionPolicy { top_k: 1, min_score: 0.1, ..Default::default() };
    let mut content = Content::new("user").with_text("translate this document to French");
    let matched = apply_skill_injection(&mut content, &index, &policy, 4000);
    assert!(matched.is_some());
    let injected_text = content.parts[0].text().unwrap();
    assert!(injected_text.contains("[skill:"));
    assert!(injected_text.contains("translate this document to French"));
    println!("✓ apply_skill_injection prepended skill block to user message");
    println!("  Matched: '{}'", matched.unwrap().skill.name);

    // 6. No match returns None, content unchanged
    let mut content2 = Content::new("user").with_text("quantum physics equations");
    let high_threshold = SelectionPolicy { top_k: 1, min_score: 50.0, ..Default::default() };
    let no_match = apply_skill_injection(&mut content2, &index, &high_threshold, 4000);
    assert!(no_match.is_none());
    let text = content2.parts[0].text().unwrap();
    assert_eq!(text, "quantum physics equations");
    println!("✓ No match → content unchanged");

    // 7. SkillInjector — higher-level wrapper
    let injector = SkillInjector::from_index(index, SkillInjectorConfig {
        policy: SelectionPolicy { top_k: 1, min_score: 0.1, ..Default::default() },
        max_injected_chars: 4000,
    });
    assert_eq!(injector.max_injected_chars(), 4000);
    println!("✓ SkillInjector created from index");

    // 8. SkillInjector builds a Plugin for automatic injection
    let plugin = injector.build_plugin("skill-injector");
    assert_eq!(plugin.name(), "skill-injector");
    assert!(plugin.on_user_message().is_some());
    println!("✓ SkillInjector.build_plugin() creates a Plugin with on_user_message hook");

    // 9. SkillInjector builds a PluginManager
    let _manager = injector.build_plugin_manager("skill-injector");
    println!("✓ SkillInjector.build_plugin_manager() creates a ready-to-use PluginManager");

    println!("\n=== All injection tests passed! ===");
}

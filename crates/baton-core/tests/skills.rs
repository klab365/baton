use baton_core::{
    domain::project::{Agent, Skill},
    features::skills::{resolve, validate},
};
#[test]
fn resolves_only_assigned_skills_in_order_without_permission_changes() {
    let mut agent: Agent = serde_json::from_value(serde_json::json!({"id":"agent","systemPrompt":"Base","runtimeKind":"fake","model":null,"credentialId":null,"rules":["Rule"],"skillIds":["second","first"]})).unwrap();
    let skills = vec![
        Skill {
            id: "first".into(),
            description: "First skill".into(),
            metadata: String::new(),
            instructions: "---\nname: first\ndescription: metadata\n---\nFirst instruction".into(),
        },
        Skill {
            id: "second".into(),
            description: "Second skill".into(),
            metadata: String::new(),
            instructions: "---\nname: second\n---\nSecond instruction".into(),
        },
        Skill {
            id: "unused".into(),
            description: "Unused skill".into(),
            metadata: String::new(),
            instructions: "---\nname: unused\n---\nNever include".into(),
        },
    ];
    let resolved = resolve(&agent, &skills).unwrap();
    assert!(resolved.skill_ids.is_empty());
    assert!(
        resolved.system_prompt.find("Second instruction").unwrap()
            < resolved.system_prompt.find("First instruction").unwrap()
    );
    assert!(!resolved.system_prompt.contains("Never include"));
    assert!(!resolved.system_prompt.contains("description: metadata"));
    assert_eq!(resolved.rules, agent.rules);
    assert_eq!(resolved.runtime_kind, agent.runtime_kind);
    agent.skill_ids.push("missing".into());
    assert!(resolve(&agent, &skills).unwrap_err().contains("missing"));
    let mut invalid = skills[0].clone();
    invalid.id = "..".into();
    assert!(validate(&invalid).is_err());
    invalid.id = "valid".into();
    invalid.instructions.clear();
    assert!(validate(&invalid).is_err());
    let mut malformed = skills[0].clone();
    malformed.instructions = "---\nname: missing close".into();
    agent.skill_ids = vec!["first".into()];
    assert!(
        resolve(&agent, &[malformed])
            .unwrap_err()
            .contains("missing closing")
    );
}

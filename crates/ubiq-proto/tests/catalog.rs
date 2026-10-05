//! The skills and MCP catalog family: every message round-trips through both wire forms, the
//! scoped ones route by their layer, and the new `skills` picks read as empty from older records.

use std::collections::BTreeMap;

use ubiq_proto::catalog::{
    CatalogMcp, McpDraftInfo, McpKind, McpParamHint, McpParamKind, RegistryMcpInfo,
    RemoteSkillInfo, SkillAdd, SkillInfo, SkillOriginInfo, SkillSourceInfo, SkillSourceKindInfo,
};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::{AgentDefinition, AgentPicks, Message};
use ubiq_proto::wire;

fn remote_mcp() -> CatalogMcp {
    CatalogMcp {
        id: "docs".to_string(),
        kind: McpKind::Http,
        command: None,
        args: Vec::new(),
        env: BTreeMap::new(),
        url: Some("https://example.com/mcp".to_string()),
        headers: BTreeMap::from([("Authorization".to_string(), "Bearer x".to_string())]),
        description: Some("docs".to_string()),
    }
}

fn local_mcp() -> CatalogMcp {
    CatalogMcp {
        id: "fs".to_string(),
        kind: McpKind::Stdio,
        command: Some("npx".to_string()),
        args: vec!["-y".to_string(), "fs-mcp".to_string()],
        env: BTreeMap::from([("ROOT".to_string(), "/tmp".to_string())]),
        url: None,
        headers: BTreeMap::new(),
        description: None,
    }
}

fn source(id: &str, kind: SkillSourceKindInfo) -> SkillSourceInfo {
    SkillSourceInfo {
        id: id.to_string(),
        label: Some(id.to_uppercase()),
        kind,
    }
}

fn every_message(scope: Option<ProjectId>) -> Vec<Message> {
    vec![
        Message::ListCatalog { scope },
        Message::AddSkill {
            scope,
            from: SkillAdd::Link {
                path: "/skills/pdf".to_string(),
                id: Some("pdf".to_string()),
            },
        },
        Message::AddSkill {
            scope,
            from: SkillAdd::Folder {
                path: "/skills".to_string(),
            },
        },
        Message::AddSkill {
            scope,
            from: SkillAdd::Remote {
                source: "anthropics".to_string(),
                path: "skills/pdf".to_string(),
                id: None,
            },
        },
        Message::RemoveSkill {
            scope,
            id: "pdf".to_string(),
        },
        Message::RemoveSkillFolder {
            scope,
            path: "/skills".to_string(),
        },
        Message::SaveSkillSources {
            sources: vec![
                source(
                    "anthropics",
                    SkillSourceKindInfo::Git {
                        url: "https://github.com/anthropics/skills".to_string(),
                        rev: Some("main".to_string()),
                        subpath: None,
                    },
                ),
                source(
                    "idx",
                    SkillSourceKindInfo::Index {
                        url: "https://example.com/skills.json".to_string(),
                    },
                ),
            ],
        },
        Message::SearchSkills {
            query: "pdf".to_string(),
            source: Some("anthropics".to_string()),
        },
        Message::SaveCatalogMcp {
            scope,
            mcp: Box::new(remote_mcp()),
            previous_id: Some("old".to_string()),
        },
        Message::RemoveCatalogMcp {
            scope,
            id: "fs".to_string(),
        },
        Message::ParseMcpConfig {
            text: r#"{"mcpServers":{}}"#.to_string(),
        },
        Message::SearchMcpRegistry {
            query: "files".to_string(),
            cursor: Some("c1".to_string()),
        },
        Message::Catalog {
            scope,
            skills: vec![
                SkillInfo {
                    id: "pdf".to_string(),
                    name: Some("PDF".to_string()),
                    description: None,
                    origin: SkillOriginInfo::Installed {
                        url: Some("https://github.com/anthropics/skills".to_string()),
                    },
                },
                SkillInfo {
                    id: "mine".to_string(),
                    name: None,
                    description: Some("mine".to_string()),
                    origin: SkillOriginInfo::Linked {
                        path: "/skills/mine".to_string(),
                    },
                },
                SkillInfo {
                    id: "scanned".to_string(),
                    name: None,
                    description: None,
                    origin: SkillOriginInfo::Folder {
                        path: "/skills".to_string(),
                    },
                },
            ],
            mcps: vec![local_mcp(), remote_mcp()],
            skill_folders: vec!["/skills".to_string()],
            sources: Vec::new(),
        },
        Message::SkillSearchResults {
            query: "pdf".to_string(),
            results: vec![RemoteSkillInfo {
                source: "anthropics".to_string(),
                id: "pdf".to_string(),
                name: Some("PDF".to_string()),
                description: Some("read pdfs".to_string()),
                path: "skills/pdf".to_string(),
            }],
            problems: vec!["hf: clone failed".to_string()],
        },
        Message::McpConfigParsed {
            servers: vec![local_mcp()],
            error: None,
        },
        Message::McpConfigParsed {
            servers: Vec::new(),
            error: Some("not JSON".to_string()),
        },
        Message::McpRegistryResults {
            query: "files".to_string(),
            servers: vec![RegistryMcpInfo {
                name: "io.example/files".to_string(),
                title: Some("Files".to_string()),
                description: None,
                version: Some("1.0.0".to_string()),
                repository: Some("https://github.com/example/files".to_string()),
                options: vec![McpDraftInfo {
                    label: "npx files-mcp".to_string(),
                    server: local_mcp(),
                    params: vec![McpParamHint {
                        name: "API_KEY".to_string(),
                        kind: McpParamKind::Env,
                        description: Some("key".to_string()),
                        required: true,
                        secret: true,
                        default: None,
                    }],
                }],
            }],
            next_cursor: Some("c2".to_string()),
            error: None,
        },
        Message::CatalogError {
            error: "no such skill".to_string(),
        },
    ]
}

#[test]
fn every_catalog_message_round_trips_through_json_and_the_frame() {
    for message in every_message(Some(ProjectId::generate())) {
        let json = serde_json::to_string(&message).unwrap();
        let from_json: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&from_json).unwrap(), json, "{json}");

        let body = wire::encode(&message).unwrap();
        let from_wire = wire::decode(&body).unwrap();
        assert_eq!(serde_json::to_string(&from_wire).unwrap(), json, "{json}");
    }
}

#[test]
fn a_catalog_mcp_written_with_only_its_id_and_kind_reads() {
    let mcp: CatalogMcp = serde_json::from_str(r#"{"id":"x","kind":"Stdio"}"#).unwrap();
    assert_eq!(mcp.id, "x");
    assert_eq!(mcp.command, None);
    assert!(mcp.args.is_empty() && mcp.env.is_empty() && mcp.headers.is_empty());
}

#[test]
fn the_scoped_messages_route_by_their_layer() {
    let project = ProjectId::generate();
    let scoped = [
        "ListCatalog",
        "AddSkill",
        "RemoveSkill",
        "RemoveSkillFolder",
        "SaveCatalogMcp",
        "RemoveCatalogMcp",
        "Catalog",
    ];
    for message in every_message(Some(project)) {
        let name = serde_json::to_value(&message).unwrap()["type"]
            .as_str()
            .unwrap()
            .to_string();
        let expected = scoped.contains(&name.as_str()).then_some(project);
        assert_eq!(message.project_id(), expected, "{name}");
    }
    for message in every_message(None) {
        assert_eq!(message.project_id(), None);
    }
}

#[test]
fn skills_default_to_empty_on_records_written_before_they_existed() {
    let picks: AgentPicks = serde_json::from_str("{}").unwrap();
    assert!(picks.skills.is_empty());

    let definition = AgentDefinition {
        id: "a".to_string(),
        agent_type: "claude-code".to_string(),
        account: None,
        model: None,
        mode: None,
        thinking: None,
        max_subagents: None,
        prompt: None,
        description: None,
        mcps: Vec::new(),
        skills: vec!["pdf".to_string()],
        mission_assistant: None,
        tags: Vec::new(),
        disabled: false,
        project: None,
        grants: Vec::new(),
    };
    let json = serde_json::to_string(&definition).unwrap();
    let back: AgentDefinition = serde_json::from_str(&json).unwrap();
    assert_eq!(back, definition);

    // A record saved before the field existed has no `skills` key at all.
    let mut old = serde_json::to_value(&definition).unwrap();
    old.as_object_mut().unwrap().remove("skills");
    let old: AgentDefinition = serde_json::from_value(old).unwrap();
    assert!(old.skills.is_empty());
}

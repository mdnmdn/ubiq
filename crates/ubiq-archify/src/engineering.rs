//! Engineering profile `deployment-ownership` (V6, P1.4): architecture documents with
//! `meta.engineering_profile` set. Port of `renderers/shared/engineering-profiles.mjs`.
//!
//! Stage order: after [`crate::graph::relationship_ids`] (V5), before
//! [`crate::evidence::check_shape`] (V7). Archify collects every finding of the profile and throws
//! them together, so this returns all of them. The profile's layout-time rule (boundary memberships
//! must nest) needs geometry and lives in `gates::architecture` (`boundary_nesting`).
//!
//! Every diagnostic is `engineering/<kebab>` with `subject.rule` equal to the code, and a subject of
//! `{diagramType, profile, collection, index, id?}`. No `subject.path`.

use serde_json::{Value, json};

use crate::diag::{Diagnostic, Subject};
use crate::model::Doc;
use crate::model::architecture::{Boundary, BoundaryKind, EngineeringProfile};
use crate::model::common::ComponentType;

const PROFILE: &str = "deployment-ownership";

/// The V6 diagnostics of `doc`: empty for any type but architecture and for a document that
/// declares no profile.
pub fn check(doc: &Doc, _raw: &Value) -> Vec<Diagnostic> {
    let Doc::Architecture(a) = doc else {
        return Vec::new();
    };
    if a.meta.engineering_profile != Some(EngineeringProfile::DeploymentOwnership) {
        return Vec::new();
    }
    let boundaries: &[Boundary] = a.boundaries.as_deref().unwrap_or_default();
    let mut out = Vec::new();

    for kind in [BoundaryKind::Region, BoundaryKind::SecurityGroup] {
        if boundaries.iter().any(|b| b.kind == kind) {
            continue;
        }
        let name = kind_name(kind);
        out.push(
            diagnostic(
                "engineering/deployment-boundary-kind",
                format!("Deployment ownership requires at least one {name} boundary."),
                subject("boundaries", -1, None),
            )
            .with_evidence(map(json!({ "requiredKind": name, "found": 0 })))
            .with_fixes([format!(
                "add one {name} boundary with an explicit wraps list"
            )]),
        );
    }

    for (index, c) in a.components.iter().enumerate() {
        if c.kind == ComponentType::External {
            continue;
        }
        let at = || subject("components", index as i64, Some(&c.id));
        let kind = kind_label(c.kind);
        if c.tag.as_deref().is_none_or(|t| t.trim().is_empty()) {
            out.push(
                diagnostic(
                    "engineering/deployment-owner-missing",
                    format!(
                        "Deployment component {} does not name its owner in tag.",
                        json_str(&c.id)
                    ),
                    at(),
                )
                .with_evidence(map(json!({ "componentType": kind, "ownerField": "tag" })))
                .with_fixes([format!(
                    "set /components/{index}/tag to the responsible team or owner"
                )]),
            );
        }

        let regions = membership(boundaries, &c.id, BoundaryKind::Region);
        if regions.is_empty() {
            out.push(
                diagnostic(
                    "engineering/deployment-region-scope",
                    format!(
                        "Deployment component {} is not assigned to a region boundary.",
                        json_str(&c.id)
                    ),
                    at(),
                )
                .with_evidence(map(
                    json!({ "componentType": kind, "regionMemberships": 0 }),
                ))
                .with_fixes(["add the component id to the real region boundary wraps list"]),
            );
        } else if regions.len() > 1 {
            out.push(
                diagnostic(
                    "engineering/deployment-region-ambiguous",
                    format!(
                        "Deployment component {} belongs to more than one region boundary.",
                        json_str(&c.id)
                    ),
                    at(),
                )
                .with_evidence(map(json!({
                    "componentType": kind,
                    "regions": regions.iter().map(|(i, b)| json!({"boundaryIndex": i, "label": b.label})).collect::<Vec<_>>(),
                })))
                .with_fixes(["keep the component id in exactly one real region boundary wraps list"]),
            );
        }

        if c.kind == ComponentType::Database
            && membership(boundaries, &c.id, BoundaryKind::SecurityGroup).is_empty()
        {
            out.push(
                diagnostic(
                    "engineering/deployment-private-state",
                    format!(
                        "Stateful component {} is not assigned to a private security-group boundary.",
                        json_str(&c.id)
                    ),
                    at(),
                )
                .with_evidence(map(json!({ "componentType": kind, "privateMemberships": 0 })))
                .with_fixes(["add the component id to the real private security-group boundary wraps list"]),
            );
        }
    }

    for (index, b) in boundaries.iter().enumerate() {
        if b.kind != BoundaryKind::SecurityGroup {
            continue;
        }
        let members: Vec<(&String, Vec<(usize, &Boundary)>)> = b
            .wraps
            .iter()
            .map(|id| (id, membership(boundaries, id, BoundaryKind::Region)))
            .collect();
        let mut region_indexes: Vec<usize> = members
            .iter()
            .flat_map(|(_, r)| r.iter().map(|(i, _)| *i))
            .collect();
        region_indexes.sort_unstable();
        region_indexes.dedup();
        let consistent = !members.is_empty()
            && members.iter().all(|(_, r)| r.len() == 1)
            && region_indexes.len() == 1;
        if consistent {
            continue;
        }
        out.push(
            diagnostic(
                "engineering/deployment-private-region-consistency",
                format!(
                    "Private boundary {} must contain components from exactly one shared region.",
                    json_str(&b.label)
                ),
                subject("boundaries", index as i64, None),
            )
            .with_evidence(map(json!({
                "boundaryKind": kind_name(b.kind),
                "members": members.iter().map(|(id, regions)| json!({
                    "id": id,
                    "regions": regions.iter().map(|(i, r)| json!({"boundaryIndex": i, "label": r.label})).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            })))
            .with_fixes(["assign every private-boundary component to exactly one shared region boundary"]),
        );
    }

    for (index, conn) in a.connections.iter().flatten().enumerate() {
        let crossed: Vec<Value> = boundaries
            .iter()
            .enumerate()
            .filter(|(_, b)| b.wraps.contains(&conn.from) != b.wraps.contains(&conn.to))
            .map(|(i, b)| json!({"boundaryIndex": i, "kind": kind_name(b.kind), "label": b.label}))
            .collect();
        let named = conn.label.as_deref().is_some_and(|l| !l.trim().is_empty());
        if crossed.is_empty() || named {
            continue;
        }
        let name = conn
            .id
            .clone()
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| format!("{}->{}", conn.from, conn.to));
        out.push(
            diagnostic(
                "engineering/deployment-crossing-mechanism",
                format!(
                    "Cross-boundary connection {} does not name its mechanism.",
                    json_str(&name)
                ),
                subject("connections", index as i64, conn.id.as_deref()),
            )
            .with_evidence(map(json!({
                "from": conn.from,
                "to": conn.to,
                "crossedBoundaries": crossed,
            })))
            .with_fixes([format!(
                "set /connections/{index}/label to the real cross-boundary mechanism"
            )]),
        );
    }
    out
}

fn diagnostic(code: &str, message: String, subject: Subject) -> Diagnostic {
    Diagnostic::error(code, &message).with_subject(subject.with_rule(code))
}

/// `subject(collection, index, item)`: `id` only when the item has a non-empty one.
fn subject(collection: &str, index: i64, id: Option<&str>) -> Subject {
    let mut s = Subject::of("architecture").with_extra("profile", json!(PROFILE));
    s.collection = Some(collection.to_owned());
    s.index = Some(index);
    s.id = id.filter(|id| !id.is_empty()).map(str::to_owned);
    s
}

/// `membership(boundaries, id, kind)`: the boundaries of `kind` that wrap `id`, with their indexes.
fn membership<'a>(
    boundaries: &'a [Boundary],
    id: &str,
    kind: BoundaryKind,
) -> Vec<(usize, &'a Boundary)> {
    boundaries
        .iter()
        .enumerate()
        .filter(|(_, b)| b.kind == kind && b.wraps.iter().any(|w| w == id))
        .collect()
}

fn kind_name(kind: BoundaryKind) -> &'static str {
    match kind {
        BoundaryKind::Region => "region",
        BoundaryKind::SecurityGroup => "security-group",
    }
}

fn kind_label(kind: ComponentType) -> &'static str {
    match kind {
        ComponentType::Frontend => "frontend",
        ComponentType::Backend => "backend",
        ComponentType::Database => "database",
        ComponentType::Cloud => "cloud",
        ComponentType::Security => "security",
        ComponentType::Messagebus => "messagebus",
        ComponentType::External => "external",
    }
}

fn json_str(s: &str) -> String {
    Value::String(s.to_owned()).to_string()
}

fn map(value: Value) -> serde_json::Map<String, Value> {
    match value {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    }
}

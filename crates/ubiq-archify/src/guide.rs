//! The agent-facing skill: embedded markdown (a pure constant, no I/O) and the table of gates the
//! server really enforces (D9). Delivered as the MCP `initialize.instructions` and by
//! `archify_guide`. Never written to a harness config path. Also the scenario recipes
//! ([`recommend`]), a deterministic keyword scorer, no model.

use std::sync::OnceLock;

use serde::Deserialize;

/// The skill text; `<!-- GATES_TABLE -->` marks where [`ENFORCED_GATES`] goes.
const SKILL: &str = include_str!("guide/skill.md");

const GATES_MARKER: &str = "<!-- GATES_TABLE -->";

/// One check the server runs today.
pub struct Gate {
    /// Stage of `00-spec` section 3.1.
    pub stage: &'static str,
    /// The diagnostic code family it reports.
    pub codes: &'static str,
    pub checks: &'static str,
}

/// What `archify_validate` enforces. A gate joins this list in the same change that implements
/// it (P1.3-P1.5 graph, P2-P3 geometry), so the skill never promises a check that does not run
/// (`02` section 8.3).
pub const ENFORCED_GATES: &[Gate] = &[
    Gate {
        stage: "V1",
        codes: "`input/json-parse`",
        checks: "the document is valid JSON",
    },
    Gate {
        stage: "V2",
        codes: "`output/meta-*`, `portable-path/*`",
        checks: "`meta.output` is a portable relative path ending `.html` (syntax only)",
    },
    Gate {
        stage: "V3",
        codes: "`schema/*`",
        checks: "the JSON Schema of the type: required fields, enums, patterns, ranges, no unknown properties",
    },
    Gate {
        stage: "V5",
        codes: "`relationship/duplicate-id`",
        checks: "optional connection, edge, message, flow and transition ids are unique per collection",
    },
    Gate {
        stage: "V6",
        codes: "`engineering/*`",
        checks: "architecture `engineering_profile: deployment-ownership`: boundary kinds, owners, private state, crossing mechanisms",
    },
    Gate {
        stage: "V7",
        codes: "`repository-evidence/*`",
        checks: "repository evidence: the shape of `meta.repository` (a pinned 40-hex revision, a credential-free origin URL, provider and link mode) and of each node's `sources` (repo-relative path, `line`, `end_line`), then, against the Git checkout the call names (`archify_validate` uses the project's own, or `repo_root`; the CLI takes `--repo-root`), the checkout's identity (top-level directory, `origin` matching the declared URL, the pinned commit present) and every citation read from the committed blob at the pinned SHA (`git --no-replace-objects`, never the working tree): the file exists, the cited lines exist. With no checkout a document with evidence ends at `repository-evidence/root-required`; that checks the citations, never what they mean",
    },
    Gate {
        stage: "V9 (dataflow)",
        codes: "`layout/constraint`, `clean-flow/*`, `composition/*`, `legend/*`",
        checks: "dataflow, complete: unique ids, stage/row ranges, endpoints, node bounds, label and sublabel fit, node overlap 10, flow length 34, orthogonal `via`, label collisions, stage width, `clean-flow/edge-through-node` and `endpoint-side-direction`, and by profile the composition rules (`container-border-run` once a profile is declared; crossings, corridors, route rhythm, label clearance and containment in showcase)",
    },
    Gate {
        stage: "V9 (sequence)",
        codes: "`layout/constraint`, `clean-flow/edge-through-node`, `composition/*`, `legend/*`",
        checks: "sequence, complete: unique participant ids, message and activation references, timeline height (viewBox height at least 327), message `y` between 160 and the lifeline bottom minus 18, span of at least 60 between columns, participant label and sublabel fit, messages less than 28 apart sharing horizontal space, label overlap, segment `y` range and label width, activation ranges, last participant inside the canvas, room for the showcase legend, and by profile the composition rules (`container-border-run` against the segment frames once a profile is declared; crossings, corridors, route rhythm, label clearance and containment in showcase)",
    },
    Gate {
        stage: "V9 (lifecycle)",
        codes: "`layout/constraint`, `clean-flow/*`, `composition/*`, `legend/*`",
        checks: "lifecycle v1 (fixed bands, canvas height of at least 566) and `schema_version` 2 on the grid router, complete; for v2: unique state and lane ids, the `main` lane, lanes and columns 0..4, state bounds (28 from the sides, `y` from 44 to the area above the legend), label width (6.2 per unit), sublabel and tag fit, states 10 apart, transition endpoints and length of at least 32, `clean-flow/edge-through-node` and `endpoint-side-direction`, and in showcase crossings (two grid routes crossing are resolved by their halos), corridors, route rhythm, label clearance and containment, and labels against the row titles",
    },
    Gate {
        stage: "V9 (architecture)",
        codes: "`layout/constraint`, `layout/*`, `clean-flow/*`, `composition/*`, `legend/*`, `internal/unclassified`",
        checks: "architecture, complete: unique component ids, grid cells or `pos`, component bounds, label (6.6 per unit), sublabel, tag and brand fit, components 8 apart, boundary `wraps`, boundary titles (title fit, inside the frame and the canvas, no overlap with components or other titles) once a profile is declared and, under `deployment-ownership`, boundary nesting, `layout/boundary-out-of-bounds`, connection endpoints and length of at least 24, `layout/self-loop-ports`, `clean-flow/edge-through-node` and `endpoint-side-direction`, `container-border-run` once a profile is declared, and in showcase `layout/route-out-of-bounds`, crossings (two automatic routes crossing are resolved by their halos), corridors, `arrowhead-collision`, route rhythm, `excessive-route-detour`, `label-gap`, label clearance and containment; showcase also relocates labels; an authored `meta.legend` that does not fit (`legend/*`)",
    },
    Gate {
        stage: "V9 (workflow)",
        codes: "`layout/constraint`, `workflow/*`, `clean-flow/*`, `composition/*`, `legend/*`, `internal/unclassified`",
        checks: "workflow v1 (fixed layout, canvas width of at least 696 and the legend inside the height) and `schema_version` 2 (the readable solver and router), complete: the semantic contract (roots, terminals, required edges and paths), unique ids, references, columns 0..5, phases (ranges, label width, no overlap), groups, `mainPath`; node label (6.8 per unit), sublabel, tag and brand fit, lane bounds, nodes 8 apart, edge length of at least 28; v1 `workflow/column-capacity` with verified fixes (move, widths, migrate to v2); v2 `workflow/node-overlap`, `explicit-pin-conflict`, `route-preset-conflict`, `solver-budget-exhausted` and `viewbox-capacity` with verified fixes; `clean-flow/edge-through-node` and `endpoint-side-direction`, `container-border-run` and route rhythm once a profile is declared, label collisions, and in showcase crossings, corridors, arrowhead collisions and label clearance (v2 reports crossings, corridors and arrowheads as warnings below showcase)",
    },
    Gate {
        stage: "V10",
        codes: "`artifact/*`, `composition/*`",
        checks: "the post-render artifact checker, run on the laid-out scene instead of on HTML: exactly one diagram, finite coordinates, orthogonal arrows (only an authored direct `straight` route may be diagonal), no arrow through the legend; and, on the routes as drawn (each rounded corner flattened into 8 chords, so two routes can cross on a corner the points do not cross on): `composition/proper-crossing` (resolved crossovers excepted), `ambiguous-corridor`, `arrowhead-collision`, `container-border-run` (every declared profile), `micro-segment` and `short-interior-segment`, `label-route-clearance` and `label-canvas-containment` (showcase), and `composition/desktop-readability` (a semantic text that projects under 6 px at the 930 px desktop budget; an error in showcase); `composition/viewport-height` is a warning. The receipt's `checks` and `composition` are the checker's own",
    },
];

/// Topics served by `archify_guide(topic)`: `(name, markdown)`.
pub const TOPICS: &[(&str, &str)] = &[
    (
        "architecture-layout-repair",
        include_str!("guide/topics/architecture-layout-repair.md"),
    ),
    ("architecture", include_str!("guide/topics/architecture.md")),
    (
        "authoring-defaults",
        include_str!("guide/topics/authoring-defaults.md"),
    ),
    ("dataflow", include_str!("guide/topics/dataflow.md")),
    ("evidence", include_str!("guide/topics/evidence.md")),
    (
        "geometry-rules",
        include_str!("guide/topics/geometry-rules.md"),
    ),
    ("labels", include_str!("guide/topics/labels.md")),
    ("lifecycle", include_str!("guide/topics/lifecycle.md")),
    (
        "mermaid-everyday",
        include_str!("guide/topics/mermaid-everyday.md"),
    ),
    ("reference", include_str!("guide/topics/reference.md")),
    ("repair", include_str!("guide/topics/repair.md")),
    ("sequence", include_str!("guide/topics/sequence.md")),
    ("workflow", include_str!("guide/topics/workflow.md")),
];

/// What a passing `archify_validate` does not prove; the guide and every receipt say it.
pub const NOT_CHECKED: &str = "the browser stage of Archify's `finalize` (`viewer/*`: the page measured \
     at four real viewports) and the HTML delivery around it, repository evidence when no Git \
     checkout is named (the editor tab names none: a document with evidence shows \
     `repository-evidence/root-required` there), and what only a person looking at the picture can \
     judge (layout, routing, labels, collisions, the geometry gates and the artifact checker are \
     complete for all five types, so a legal but unclear diagram passes)";

/// The markdown table generated from [`ENFORCED_GATES`], with the sentence that bounds it.
pub fn gates_table() -> String {
    let mut out = String::from(
        "Enforced by `archify_validate` today (a pass proves these and nothing more):\n\n\
         | Stage | Codes | Checks |\n|---|---|---|\n",
    );
    for gate in ENFORCED_GATES {
        out.push_str(&format!(
            "| {} | {} | {} |\n",
            gate.stage, gate.codes, gate.checks
        ));
    }
    out.push_str(&format!(
        "\nNot checked yet: {NOT_CHECKED}. Say so in your report instead of claiming them.\n"
    ));
    out
}

/// The skill with the gates table filled in: `initialize.instructions`, and `archify_guide()`.
pub fn skill() -> String {
    SKILL.replace(GATES_MARKER, gates_table().trim_end())
}

/// A topic's markdown, or `None` for an unknown name.
pub fn topic(name: &str) -> Option<&'static str> {
    TOPICS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, text)| *text)
}

/// A minimal valid document of `doc_type` titled `title` (`archify_new`, D30): the example of the
/// type's topic, so there is one source for it, with its `meta.title` and `meta.output`
/// (`<slug>.html`) replaced. `None` for a type with no topic.
pub fn starter(doc_type: &str, title: &str, slug: &str) -> Option<serde_json::Value> {
    let text = topic(doc_type)?;
    let body = text.split_once("```json\n")?.1.split_once("\n```")?.0;
    let mut doc: serde_json::Value = serde_json::from_str(body).ok()?;
    let meta = doc.get_mut("meta")?.as_object_mut()?;
    meta.insert("title".into(), title.into());
    meta.insert("output".into(), format!("{slug}.html").into());
    Some(doc)
}

/// The topic names, in order.
pub fn topic_names() -> Vec<&'static str> {
    TOPICS.iter().map(|(n, _)| *n).collect()
}

// ---- scenario recipes (P5.1, `02` section 1.5) ----

/// The 12 recipes of Archify's `recipes/scenarios.mjs`, as `gen-recipes.mjs` wrote them.
const RECIPES_JSON: &str = include_str!("guide/recipes.json");

/// Copy of a recipe in one language.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Copy {
    pub title: String,
    pub question: String,
    pub summary: String,
    pub use_when: String,
    pub avoid_when: String,
    pub include: Vec<String>,
    /// The repository-flavoured starter prompt.
    pub prompt: String,
}

#[derive(Debug, Deserialize)]
pub struct StartCopy {
    #[serde(rename = "descriptionPrompt")]
    pub description_prompt: String,
}

#[derive(Debug, Deserialize)]
pub struct Start {
    pub en: StartCopy,
    pub zh: StartCopy,
}

#[derive(Debug, Deserialize)]
pub struct Presentation {
    pub preset: String,
    pub motion: String,
}

/// One scenario recipe: the question it answers, the diagram type to pick, and the keyword
/// signals that route a request to it.
#[derive(Debug, Deserialize)]
pub struct Recipe {
    pub id: String,
    #[serde(rename = "type")]
    pub diagram_type: String,
    /// The name of Archify's example for it; not shipped, not shown to agents.
    pub proof: String,
    pub presentation: Presentation,
    /// The description-flavoured prompt, for the recipes that have one.
    pub start: Option<Start>,
    /// `(keyword, weight)`, English and Chinese.
    pub signals: Vec<(String, u32)>,
    pub en: Copy,
    pub zh: Copy,
}

impl Recipe {
    /// The copy in `lang` (`zh`, else `en`).
    pub fn copy(&self, lang: &str) -> &Copy {
        if lang == "zh" { &self.zh } else { &self.en }
    }

    /// The topic of `archify_guide` to read next.
    pub fn topic(&self) -> &str {
        if self.id == "layout-repair" { "architecture-layout-repair" } else { &self.diagram_type }
    }
}

/// The recipes, in Archify's order (it breaks score ties).
pub fn recipes() -> &'static [Recipe] {
    static RECIPES: OnceLock<Vec<Recipe>> = OnceLock::new();
    RECIPES.get_or_init(|| serde_json::from_str(RECIPES_JSON).expect("guide/recipes.json is valid"))
}

/// A recipe and why it was picked.
#[derive(Debug)]
pub struct Match {
    pub recipe: &'static Recipe,
    pub score: u32,
    /// The signals that scored, in recipe order.
    pub matched: Vec<&'static str>,
}

impl Match {
    /// `high` from a score of 14, `medium` from 7.
    pub fn confidence(&self) -> &'static str {
        match self.score {
            14.. => "high",
            7.. => "medium",
            _ => "low",
        }
    }
}

/// `normalized` of `scenarios.mjs`: width-folded (the part of NFKC that matters here), lower-case,
/// every run of whitespace or `_` one space, trimmed.
fn normalized(text: &str) -> String {
    let folded: String = text
        .chars()
        .map(|c| match c {
            '\u{3000}' => ' ',
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            other => other,
        })
        .collect();
    let mut out = String::new();
    let mut gap = false;
    for c in folded.to_lowercase().chars() {
        if c.is_whitespace() || c == '_' {
            gap = true;
        } else {
            if gap && !out.is_empty() {
                out.push(' ');
            }
            gap = false;
            out.push(c);
        }
    }
    out
}

/// `recommendScenario`, deterministic keyword scoring: each signal found in the text adds its
/// weight, the recipe id (or its words) alone is 100, ties go to the earlier recipe. Returns the
/// winner and up to two alternatives that scored; nothing scores: the first recipe, score 0.
pub fn recommend(text: &str) -> Vec<Match> {
    let query = normalized(text);
    let mut ranked: Vec<Match> = recipes()
        .iter()
        .map(|recipe| {
            let (score, matched) = if query.is_empty() {
                (0, Vec::new())
            } else if query == recipe.id || query == recipe.id.replace('-', " ") {
                (100, vec![recipe.id.as_str()])
            } else {
                let hits: Vec<(&str, u32)> = recipe
                    .signals
                    .iter()
                    .filter(|(signal, _)| query.contains(&normalized(signal)))
                    .map(|(signal, weight)| (signal.as_str(), *weight))
                    .collect();
                (hits.iter().map(|(_, w)| w).sum(), hits.into_iter().map(|(s, _)| s).collect())
            };
            Match { recipe, score, matched }
        })
        .collect();
    // `sort_by` is stable: equal scores keep Archify's recipe order.
    ranked.sort_by_key(|m| std::cmp::Reverse(m.score));
    if ranked[0].score == 0 {
        // Nothing matched: Archify answers with the first recipe, at low confidence.
        return vec![Match { recipe: &recipes()[0], score: 0, matched: Vec::new() }];
    }
    ranked.truncate(3);
    ranked.retain(|m| m.score > 0);
    ranked
}

/// `archify_guide(scenario)`: the top recipes for `text` as JSON, with the diagram type to use and
/// the starter notes. The language follows the text (CJK is Chinese).
pub fn scenario_json(text: &str) -> serde_json::Value {
    use serde_json::json;
    let lang = if text.chars().any(|c| ('\u{3400}'..='\u{9fff}').contains(&c)) { "zh" } else { "en" };
    let matches = recommend(text);
    let describe = |m: &Match| {
        let r = m.recipe;
        let copy = r.copy(lang);
        let start = r.start.as_ref().map(|s| if lang == "zh" { &s.zh } else { &s.en }.description_prompt.as_str());
        json!({
            "id": r.id,
            "type": r.diagram_type,
            "title": copy.title,
            "question": copy.question,
            "summary": copy.summary,
            "useWhen": copy.use_when,
            "avoidWhen": copy.avoid_when,
            "include": copy.include,
            "presentation": {"preset": r.presentation.preset, "motion": r.presentation.motion},
            "starter": {"descriptionPrompt": start, "repositoryPrompt": copy.prompt},
            "next": format!("archify_guide(topic: \"{}\"), archify_schema(type: \"{}\"), then write the file and archify_validate it", r.topic(), r.diagram_type),
        })
    };
    let winner = &matches[0];
    let alternatives: Vec<_> = matches[1..]
        .iter()
        .map(|m| {
            let mut v = describe(m);
            v["score"] = m.score.into();
            v
        })
        .collect();
    json!({
        "ok": true,
        "lang": lang,
        "scenario": text,
        "confidence": winner.confidence(),
        "matchedSignals": winner.matched,
        "recommendation": describe(winner),
        "alternatives": alternatives,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Alt {
        id: String,
        score: u32,
    }

    #[derive(Deserialize)]
    struct Case {
        query: String,
        confidence: String,
        matched: Vec<String>,
        winner: String,
        alternatives: Vec<Alt>,
    }

    /// Archify's `recommendScenario` over 78 queries (`gen-recipes.mjs`): the winner, the
    /// signals that scored, the confidence and the alternatives with their scores, exactly.
    #[test]
    fn recommend_matches_archifys_recommend_scenario() {
        let cases: Vec<Case> = serde_json::from_str(include_str!("../tests/recipe_cases.json")).unwrap();
        assert!(cases.len() > 50);
        for case in cases {
            let got = recommend(&case.query);
            let q = &case.query;
            assert_eq!(got[0].recipe.id, case.winner, "{q:?}");
            assert_eq!(got[0].confidence(), case.confidence, "{q:?}");
            assert_eq!(got[0].matched, case.matched, "{q:?}");
            let alts: Vec<(&str, u32)> = got[1..].iter().map(|m| (m.recipe.id.as_str(), m.score)).collect();
            let want: Vec<(&str, u32)> = case.alternatives.iter().map(|a| (a.id.as_str(), a.score)).collect();
            assert_eq!(alts, want, "{q:?}");
        }
    }

    #[test]
    fn the_twelve_recipes_cover_every_type() {
        assert_eq!(recipes().len(), 12);
        for kind in crate::schema::DOC_TYPES {
            assert!(recipes().iter().any(|r| r.diagram_type == *kind), "{kind}");
        }
        assert!(recipes().iter().all(|r| topic(r.topic()).is_some()));
    }

    #[test]
    fn scenario_json_names_the_type_and_the_starter() {
        let v = scenario_json("sequence diagram of an api request");
        assert_eq!(v["recommendation"]["id"], "api-request");
        assert_eq!(v["recommendation"]["type"], "sequence");
        assert!(v["recommendation"]["starter"]["repositoryPrompt"].as_str().is_some_and(|p| !p.is_empty()));
        assert_eq!(v["lang"], "en");
        assert_eq!(scenario_json("数据血缘")["lang"], "zh");
    }

    #[test]
    fn the_skill_carries_the_generated_table_and_no_marker() {
        let text = skill();
        assert!(!text.contains(GATES_MARKER));
        assert!(text.contains("| V3 | `schema/*` |"));
        assert!(text.contains("# Archify diagrams in Ubiq"));
    }

    #[test]
    fn every_type_has_a_starter_that_compiles_clean() {
        for kind in crate::schema::DOC_TYPES {
            let doc = starter(kind, "My diagram", "my-diagram").expect(kind);
            assert_eq!(doc["diagram_type"], kind);
            assert_eq!(doc["meta"]["title"], "My diagram");
            assert_eq!(doc["meta"]["output"], "my-diagram.html");
            let opts = crate::compile::Opts { input: "my-diagram.archify", ..Default::default() };
            let compiled = crate::compile::compile(&doc.to_string(), &opts);
            assert!(compiled.ok(), "{kind}: {:?}", compiled.diagnostics());
        }
        assert!(starter("nope", "t", "t").is_none());
    }

    #[test]
    fn every_topic_resolves_and_the_skill_names_them_all() {
        for name in topic_names() {
            assert!(topic(name).is_some_and(|t| !t.is_empty()), "{name}");
            assert!(SKILL.contains(&format!("`{name}`")), "skill lacks {name}");
        }
        assert!(topic("nope").is_none());
    }
}

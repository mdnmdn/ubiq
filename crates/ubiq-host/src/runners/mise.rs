//! `mise`: the tasks of a `mise.toml`. `mise tasks ls --json` when the CLI is there, because it also
//! sees file tasks and inheritance; the toml's own `[tasks]` table when it is not.

use std::path::Path;

use super::{RunnerSource, RunnerTarget, capture, first_file};

pub struct Mise;

const FILES: [&str; 3] = ["mise.toml", ".mise.toml", "mise/config.toml"];

impl RunnerSource for Mise {
    fn id(&self) -> &str {
        "mise"
    }

    fn discover(&self, root: &Path) -> Vec<RunnerTarget> {
        let Some(file) = first_file(root, &FILES) else {
            return Vec::new();
        };
        if let Some(names) = capture("mise", &["tasks", "ls", "--json"], root)
            .and_then(|out| parse_json(&out))
        {
            return targets(names);
        }
        std::fs::read_to_string(file)
            .map(|text| targets(parse_toml(&text)))
            .unwrap_or_default()
    }
}

fn targets(names: Vec<String>) -> Vec<RunnerTarget> {
    names
        .into_iter()
        .filter(|name| !name.is_empty() && !name.contains(char::is_whitespace))
        .map(|name| RunnerTarget {
            command: "mise".to_string(),
            args: format!("run {name}"),
            name,
        })
        .collect()
}

/// The `name` of each visible entry in `mise tasks ls --json`; `None` when it is not that shape.
fn parse_json(out: &str) -> Option<Vec<String>> {
    let rows: Vec<serde_json::Value> = serde_json::from_str(out).ok()?;
    Some(
        rows.iter()
            .filter(|row| row.get("hide").and_then(|hide| hide.as_bool()) != Some(true))
            .filter_map(|row| row.get("name")?.as_str().map(str::to_string))
            .collect(),
    )
}

/// The keys of a mise toml's `[tasks]` table, in file order.
fn parse_toml(text: &str) -> Vec<String> {
    toml::from_str::<toml::Table>(text)
        .ok()
        .and_then(|doc| match doc.get("tasks")? {
            toml::Value::Table(tasks) => Some(tasks.keys().cloned().collect()),
            _ => None,
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_are_the_keys_of_the_tasks_table() {
        let text = "[tools]\nnode = \"22\"\n\n[tasks.build]\nrun = \"npm run build\"\n\n[tasks.test]\ndescription = \"t\"\nrun = \"npm test\"\n\n[tasks]\nlint = \"npm run lint\"\n";
        let mut names = parse_toml(text);
        names.sort();
        assert_eq!(names, ["build", "lint", "test"]);
    }

    #[test]
    fn a_file_with_no_tasks_or_bad_toml_has_none() {
        assert!(parse_toml("[tools]\nnode = \"22\"\n").is_empty());
        assert!(parse_toml("[tasks\n").is_empty());
    }

    #[test]
    fn the_json_listing_drops_hidden_tasks() {
        let out = r#"[{"name":"build","description":"b"},{"name":"_x","hide":true},{"name":"test:unit"}]"#;
        assert_eq!(parse_json(out).unwrap(), ["build", "test:unit"]);
        assert!(parse_json("not json").is_none());
    }

    #[test]
    fn a_target_runs_through_mise_run() {
        let found = targets(vec!["test:unit".to_string()]);
        assert_eq!(found[0].command, "mise");
        assert_eq!(found[0].args, "run test:unit");
    }
}

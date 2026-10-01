//! `just`: the recipes of a `justfile`, from `just --summary` so imports and modules count.

use std::path::Path;

use super::{RunnerSource, RunnerTarget, capture, first_file};

pub struct Just;

impl RunnerSource for Just {
    fn id(&self) -> &str {
        "just"
    }

    fn discover(&self, root: &Path) -> Vec<RunnerTarget> {
        if first_file(root, &["justfile", "Justfile", ".justfile"]).is_none() {
            return Vec::new();
        }
        // `--summary` leaves private recipes out already; `--unsorted` keeps the file's order.
        capture("just", &["--summary", "--unsorted"], root)
            .map(|out| parse_summary(&out))
            .unwrap_or_default()
    }
}

/// `just --summary`'s one line of space-separated recipe names.
fn parse_summary(out: &str) -> Vec<RunnerTarget> {
    out.split_whitespace()
        .map(|name| RunnerTarget {
            name: name.to_string(),
            command: "just".to_string(),
            args: name.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_summary_is_one_target_per_word() {
        let targets = parse_summary("build test\nfmt mod::lint\n");
        let names: Vec<_> = targets.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["build", "test", "fmt", "mod::lint"]);
        assert_eq!(targets[0].command, "just");
        assert_eq!(targets[0].args, "build");
    }
}

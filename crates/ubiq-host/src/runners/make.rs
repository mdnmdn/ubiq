//! `make`: the targets of a `Makefile`, read from the file itself. No subprocess — `make -qp`
//! would run the user's `$(shell …)` just to list names.

use std::path::Path;

use super::{RunnerSource, RunnerTarget, first_file};

pub struct Make;

impl RunnerSource for Make {
    fn id(&self) -> &str {
        "make"
    }

    fn discover(&self, root: &Path) -> Vec<RunnerTarget> {
        let Some(path) = first_file(root, &["Makefile", "makefile", "GNUmakefile"]) else {
            return Vec::new();
        };
        std::fs::read_to_string(path)
            .map(|text| parse_makefile(&text))
            .unwrap_or_default()
    }
}

/// A target name worth offering: plain characters, no pattern, no variable, not a dot-target.
fn is_target(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '+'))
}

/// The explicit targets of a Makefile, in file order, once each. Recipe lines (they start with a
/// tab), assignments (`X := …`, `X ::= …`, target-specific `t: X=1`), pattern rules and `.PHONY`
/// -style special targets are not targets.
fn parse_makefile(text: &str) -> Vec<RunnerTarget> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with(['\t', ' ', '#', '.']) {
            continue;
        }
        let Some((head, rest)) = line.split_once(':') else {
            continue;
        };
        // `X := v` and `X ::= v` are assignments; `a:: b` is a double-colon rule.
        if rest.starts_with('=') || rest.starts_with(":=") || rest.contains('=') {
            continue;
        }
        if head.contains(['=', '$', '%']) {
            continue;
        }
        for name in head.split_whitespace() {
            if is_target(name) && seen.insert(name.to_string()) {
                out.push(RunnerTarget {
                    name: name.to_string(),
                    command: "make".to_string(),
                    args: name.to_string(),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(text: &str) -> Vec<String> {
        parse_makefile(text).into_iter().map(|t| t.name).collect()
    }

    #[test]
    fn targets_come_in_file_order_once_each() {
        let text = "all: build test\nbuild:\n\tcc x.c\ntest: build\n\t./t\nbuild: extra\n";
        assert_eq!(names(text), ["all", "build", "test"]);
    }

    #[test]
    fn assignments_patterns_and_special_targets_are_not_targets() {
        let text = "\
CC := gcc
CFLAGS ::= -O2
X = a:b
.PHONY: all clean
%.o: %.c
\t$(CC) -c $<
$(OBJ): x.h
all: FOO=1
clean:
\trm -rf out
";
        assert_eq!(names(text), ["clean"]);
    }

    #[test]
    fn several_targets_on_one_line_are_each_offered() {
        assert_eq!(names("a b: dep\n"), ["a", "b"]);
        assert_eq!(names("foo:: bar\n"), ["foo"]);
    }

    #[test]
    fn indented_and_commented_lines_are_skipped() {
        assert_eq!(names("# note: x\n  ifeq (a,b)\nok:\n"), ["ok"]);
    }
}

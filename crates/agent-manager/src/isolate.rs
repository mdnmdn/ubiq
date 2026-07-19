//! Wraps a [`crate::harness::Launch`] in an isol8 sandbox invocation, driven
//! by a configurable command template.
//!
//! This is a pure transform: given an already-provisioned `Launch` (the
//! program + args the harness wants exec'd) and an [`crate::spec::Isolation`]
//! setting, it rewrites the `Launch` to instead exec `isol8` (or whatever
//! custom command a template names) wrapping the original command. It
//! touches no filesystem, spawns nothing, and depends on nothing beyond
//! `std` — no new crate dependency, no feature gate — so it stays part of
//! the crate's core (still builds and is usable with
//! `--no-default-features`, e.g. for lib-mode embedders that never touch
//! clap/pty). See `_docs/architecture.md` and `_docs/cli.md`
//! for the `--isolate[=profile]` / `[isolate] command` surface this backs.

use crate::harness::Launch;
use crate::spec::Isolation;

/// The command template used to build the isol8 invocation.
///
/// The template string is tokenized on whitespace, then each token is
/// expanded independently (see [`wrap_launch`]):
///
/// - `{cmd}` expands to the original `program` followed by its `args`.
/// - `{profile_opt}` expands to `--profile <profile>` when the profile is
///   non-empty, or to nothing at all when it is empty (so a bare
///   `--isolate` with no profile doesn't pass an empty `--profile ''`).
/// - any other token containing the literal substring `{profile}` has it
///   substituted with the raw profile value, for custom templates that want
///   the profile inlined into a single flag (e.g. `--sandbox={profile}`); if
///   the substitution yields an empty token, the token is dropped.
/// - any other token is kept literally.
#[derive(Debug, Clone)]
pub struct IsolateTemplate {
    /// The template string, e.g. `"isol8 {profile_opt} -- {cmd}"`.
    pub command: String,
}

impl Default for IsolateTemplate {
    fn default() -> Self {
        Self {
            command: "isol8 {profile_opt} -- {cmd}".to_string(),
        }
    }
}

/// Wrap `launch` per `isolation`, using `template` to build the isol8 argv.
///
/// - [`Isolation::None`] is the identity transform: `launch` is returned
///   unchanged (cloned).
/// - [`Isolation::Sandboxed`] tokenizes `template.command` on whitespace,
///   expands each token per [`IsolateTemplate`], and concatenates the
///   results into a new argv; the first token becomes the new
///   `Launch::program`, the rest become `Launch::args`.
///
/// `launch.env` and `launch.env_remove` are carried through unchanged onto
/// the wrapped `Launch` — isol8 is expected to pass the child environment
/// straight through to the sandboxed process, so the hygiene/injection the
/// harness provisioner already computed still applies without isol8 needing
/// to know about it.
///
/// If the template has no `{cmd}` token at all (a misconfigured template —
/// nothing tells isol8 what to actually run), the original `program` + args
/// are appended at the end of the expanded argv anyway. This keeps the
/// sandboxed command reachable (just possibly alongside isol8 flags that
/// don't make sense) rather than silently dropping it.
pub fn wrap_launch(launch: &Launch, isolation: &Isolation, template: &IsolateTemplate) -> Launch {
    let profile = match isolation {
        Isolation::None => return launch.clone(),
        Isolation::Sandboxed(profile) => profile,
    };

    let mut argv: Vec<String> = Vec::new();
    let mut saw_cmd = false;

    for token in template.command.split_whitespace() {
        match token {
            "{cmd}" => {
                saw_cmd = true;
                argv.push(launch.program.clone());
                argv.extend(launch.args.iter().cloned());
            }
            "{profile_opt}" => {
                if !profile.is_empty() {
                    argv.push("--profile".to_string());
                    argv.push(profile.clone());
                }
            }
            t if t.contains("{profile}") => {
                let expanded = t.replace("{profile}", profile);
                if !expanded.is_empty() {
                    argv.push(expanded);
                }
            }
            t => argv.push(t.to_string()),
        }
    }

    if !saw_cmd {
        argv.push(launch.program.clone());
        argv.extend(launch.args.iter().cloned());
    }

    // `argv` always has at least one element here: either `{cmd}` expansion
    // pushed `launch.program` (even if that's an empty string), or the
    // `!saw_cmd` fallback above did. `split_first` is used defensively
    // rather than assumed infallible.
    let (program, args) = match argv.split_first() {
        Some((first, rest)) => (first.clone(), rest.to_vec()),
        None => (launch.program.clone(), launch.args.clone()),
    };

    Launch {
        program,
        args,
        env: launch.env.clone(),
        env_remove: launch.env_remove.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_launch() -> Launch {
        Launch {
            program: "claude".to_string(),
            args: vec!["--foo".to_string(), "bar".to_string()],
            env: vec![("CLAUDE_CONFIG_DIR".to_string(), "/tmp/x".to_string())],
            env_remove: vec!["PATH".to_string()],
        }
    }

    #[test]
    fn default_template_with_profile_wraps_in_isol8() {
        let launch = sample_launch();
        let wrapped = wrap_launch(
            &launch,
            &Isolation::Sandboxed("dev".to_string()),
            &IsolateTemplate::default(),
        );

        assert_eq!(wrapped.program, "isol8");
        assert_eq!(
            wrapped.args,
            vec!["--profile", "dev", "--", "claude", "--foo", "bar"]
        );
        // env/env_remove carried through unchanged.
        assert_eq!(wrapped.env, launch.env);
        assert_eq!(wrapped.env_remove, launch.env_remove);
    }

    #[test]
    fn default_template_with_empty_profile_omits_profile_flag() {
        let launch = sample_launch();
        let wrapped = wrap_launch(
            &launch,
            &Isolation::Sandboxed(String::new()),
            &IsolateTemplate::default(),
        );

        assert_eq!(wrapped.program, "isol8");
        assert_eq!(wrapped.args, vec!["--", "claude", "--foo", "bar"]);
    }

    #[test]
    fn isolation_none_is_identity() {
        let launch = sample_launch();
        let wrapped = wrap_launch(&launch, &Isolation::None, &IsolateTemplate::default());

        assert_eq!(wrapped.program, launch.program);
        assert_eq!(wrapped.args, launch.args);
        assert_eq!(wrapped.env, launch.env);
        assert_eq!(wrapped.env_remove, launch.env_remove);
    }

    #[test]
    fn custom_template_substitutes_profile_placeholder() {
        let launch = sample_launch();
        let template = IsolateTemplate {
            command: "isol8 --sandbox={profile} run -- {cmd}".to_string(),
        };
        let wrapped = wrap_launch(
            &launch,
            &Isolation::Sandboxed("dev".to_string()),
            &template,
        );

        assert_eq!(wrapped.program, "isol8");
        assert_eq!(
            wrapped.args,
            vec!["--sandbox=dev", "run", "--", "claude", "--foo", "bar"]
        );
    }

    /// A `{profile}` token only vanishes entirely when substitution leaves it
    /// empty (i.e. the token *is* `{profile}`, nothing else). When it's part
    /// of a larger literal like `--sandbox={profile}`, an empty profile just
    /// yields `--sandbox=` — still kept, since the surrounding text isn't
    /// empty.
    #[test]
    fn bare_profile_placeholder_token_is_dropped_when_profile_is_empty() {
        let launch = sample_launch();
        let template = IsolateTemplate {
            command: "isol8 {profile} run -- {cmd}".to_string(),
        };
        let wrapped = wrap_launch(&launch, &Isolation::Sandboxed(String::new()), &template);

        assert_eq!(wrapped.program, "isol8");
        assert_eq!(wrapped.args, vec!["run", "--", "claude", "--foo", "bar"]);
    }

    #[test]
    fn profile_placeholder_embedded_in_a_larger_token_is_kept_even_when_empty() {
        let launch = sample_launch();
        let template = IsolateTemplate {
            command: "isol8 --sandbox={profile} run -- {cmd}".to_string(),
        };
        let wrapped = wrap_launch(&launch, &Isolation::Sandboxed(String::new()), &template);

        assert_eq!(wrapped.program, "isol8");
        assert_eq!(
            wrapped.args,
            vec!["--sandbox=", "run", "--", "claude", "--foo", "bar"]
        );
    }

    #[test]
    fn template_without_cmd_token_appends_original_program_and_args() {
        let launch = sample_launch();
        let template = IsolateTemplate {
            command: "isol8 run {profile_opt}".to_string(),
        };
        let wrapped = wrap_launch(
            &launch,
            &Isolation::Sandboxed("dev".to_string()),
            &template,
        );

        assert_eq!(wrapped.program, "isol8");
        assert_eq!(
            wrapped.args,
            vec!["run", "--profile", "dev", "claude", "--foo", "bar"]
        );
    }
}

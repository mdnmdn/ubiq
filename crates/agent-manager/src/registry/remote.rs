//! Remote skill sources: search a git repository (or an HTTP index of them) for skills and
//! install one into a [`CatalogStore`].
//!
//! Git sources use the `git` CLI (shallow clone into a cache folder, `<cache>/<source id>`); no
//! extra dependency. An index source is an HTTP JSON listing and needs the `remote` feature.

use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use anyhow::{Context, anyhow, bail};

use crate::Result;
use crate::registry::fs::parse_skill_frontmatter;
use crate::registry::{
    CatalogStore, RemoteOrigin, SkillSource, SkillSourceKind, path_lock, valid_id,
};
use crate::source::Source;

/// A cached clone younger than this is not fetched again by [`list_source`].
const FRESH_FOR: Duration = Duration::from_secs(60 * 60);
/// How deep under a source's root a `SKILL.md` is looked for.
const SCAN_DEPTH: usize = 4;

/// A skill offered by a [`SkillSource`], not yet installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSkill {
    /// Id of the source it was found in.
    pub source: String,
    /// Suggested catalog id (the folder name).
    pub id: String,
    /// `name:` from the frontmatter (or the index).
    pub name: Option<String>,
    /// `description:` from the frontmatter (or the index).
    pub description: Option<String>,
    /// Folder of the skill inside its repository (`""` for the root).
    pub path: String,
    /// Git URL, when it differs from the source's own (index entries).
    pub url: Option<String>,
    /// Branch or tag, when the index entry names one.
    pub rev: Option<String>,
}

/// The sources used when a catalog declares none: Anthropic's and Hugging Face's skill repositories.
///
/// agentskills.io publishes no machine-readable listing that was found (the site describes the
/// format and links to repositories), so there is no `Index` source among the defaults.
pub fn default_skill_sources() -> Vec<SkillSource> {
    let git = |id: &str, label: &str, url: &str| SkillSource {
        id: id.into(),
        label: Some(label.into()),
        kind: SkillSourceKind::Git {
            url: url.into(),
            rev: None,
            subpath: None,
        },
    };
    vec![
        git(
            "anthropics",
            "Anthropic skills",
            "https://github.com/anthropics/skills",
        ),
        git(
            "huggingface",
            "Hugging Face skills",
            "https://github.com/huggingface/skills",
        ),
    ]
}

fn git(dir: Option<&Path>, args: &[&str]) -> Result<String> {
    let mut cmd = Command::new("git");
    if let Some(dir) = dir {
        cmd.arg("-C").arg(dir);
    }
    let out = cmd
        .args([
            "-c",
            "http.lowSpeedLimit=1000",
            "-c",
            "http.lowSpeedTime=30",
        ])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .context("running git (is it installed?)")?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn git_parts(src: &SkillSource) -> Result<(&str, Option<&str>, Option<&str>)> {
    match &src.kind {
        SkillSourceKind::Git { url, rev, subpath } => Ok((url, rev.as_deref(), subpath.as_deref())),
        SkillSourceKind::Index { .. } => {
            bail!("source '{}' is an index, not a git repository", src.id)
        }
    }
}

/// A `url` or `rev` git would read as an option, or a `rev` that is not a plain ref name.
fn check_git_args(url: &str, rev: Option<&str>) -> Result<()> {
    if url.is_empty() || url.starts_with('-') || url.chars().any(char::is_control) {
        bail!("invalid git url '{url}'");
    }
    if let Some(rev) = rev
        && (rev.is_empty()
            || rev.starts_with('-')
            || rev.contains("..")
            || rev.chars().any(|c| c.is_whitespace() || c.is_control()))
    {
        bail!("invalid git rev '{rev}'");
    }
    Ok(())
}

/// The cache folder of a source (`<cache>/<source id>`).
fn cache_dir(src: &SkillSource, cache: &Path) -> Result<PathBuf> {
    if !valid_id(&src.id) {
        bail!("invalid source id '{}'", src.id);
    }
    Ok(cache.join(&src.id))
}

/// Beside the cache folder: the url and rev it was cloned for.
fn marker_path(dir: &Path) -> PathBuf {
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("src");
    dir.with_file_name(format!(".{name}.am-source"))
}

fn marker_text(url: &str, rev: Option<&str>) -> String {
    format!("{url}\n{}\n", rev.unwrap_or_default())
}

/// Whether `dir` is a clone of `url` at `rev`: its origin and its marker both say so.
fn cache_matches(dir: &Path, url: &str, rev: Option<&str>) -> bool {
    dir.join(".git").exists()
        && std::fs::read_to_string(marker_path(dir)).is_ok_and(|m| m == marker_text(url, rev))
        && git(Some(dir), &["remote", "get-url", "origin"]).is_ok_and(|u| u == url)
}

/// Clone (shallow) or update the cache folder of a git source and return it.
///
/// Fetches unconditionally; [`list_source`] and [`install_remote`] only do so when the cache is
/// missing or older than an hour.
pub fn refresh_source(src: &SkillSource, cache: &Path) -> Result<PathBuf> {
    let (url, rev, _) = git_parts(src)?;
    check_git_args(url, rev)?;
    let dir = cache_dir(src, cache)?;
    let lock = path_lock(&dir);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    if cache_matches(&dir, url, rev) {
        git(
            Some(&dir),
            &[
                "fetch",
                "--depth",
                "1",
                "origin",
                "--",
                rev.unwrap_or("HEAD"),
            ],
        )?;
        git(Some(&dir), &["reset", "--hard", "FETCH_HEAD"])?;
        return Ok(dir);
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    let _ = std::fs::remove_file(marker_path(&dir));
    std::fs::create_dir_all(cache)?;
    let dir_str = dir.to_string_lossy();
    let mut args = vec!["clone", "--depth", "1"];
    if let Some(rev) = rev {
        args.extend(["--branch", rev]);
    }
    args.extend(["--", url, dir_str.as_ref()]);
    git(None, &args)?;
    std::fs::write(marker_path(&dir), marker_text(url, rev))?;
    Ok(dir)
}

/// The cache folder, refreshed when it is missing, stale or cloned for another url or rev. A
/// failed refresh of a matching cache falls back to it.
fn ensure_cached(src: &SkillSource, cache: &Path) -> Result<PathBuf> {
    let (url, rev, _) = git_parts(src)?;
    let dir = cache_dir(src, cache)?;
    let matches = cache_matches(&dir, url, rev);
    let stamp = dir.join(".git").join("FETCH_HEAD");
    let fresh = matches
        && std::fs::metadata(&stamp)
            .or_else(|_| std::fs::metadata(dir.join(".git")))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age < FRESH_FOR);
    if fresh {
        return Ok(dir);
    }
    match refresh_source(src, cache) {
        Ok(dir) => Ok(dir),
        Err(_) if cache_matches(&dir, url, rev) => Ok(dir),
        Err(e) => Err(e),
    }
}

/// Every skill a source offers.
pub fn list_source(src: &SkillSource, cache: &Path) -> Result<Vec<RemoteSkill>> {
    match &src.kind {
        SkillSourceKind::Git { subpath, .. } => {
            let dir = ensure_cached(src, cache)?;
            Ok(scan_repo(&src.id, &dir, subpath.as_deref()))
        }
        SkillSourceKind::Index { url } => fetch_index(&src.id, url),
    }
}

/// Find `SKILL.md` files under `<repo>/<subpath>` (max depth [`SCAN_DEPTH`]); a skill nested in
/// another skill's folder is part of that skill, not a separate one.
fn scan_repo(source: &str, repo: &Path, subpath: Option<&str>) -> Vec<RemoteSkill> {
    let base = match subpath {
        Some(s) if !s.is_empty() => repo.join(s),
        _ => repo.to_path_buf(),
    };
    let mut folders: Vec<PathBuf> = walkdir::WalkDir::new(&base)
        .max_depth(SCAN_DEPTH)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && e.file_name() == "SKILL.md")
        .filter_map(|e| e.path().parent().map(Path::to_path_buf))
        .collect();
    folders.sort();
    let mut out: Vec<RemoteSkill> = Vec::new();
    let mut kept: Vec<PathBuf> = Vec::new();
    for folder in folders {
        if kept.iter().any(|k| folder.starts_with(k)) {
            continue;
        }
        let rel = folder.strip_prefix(repo).unwrap_or(&folder);
        let path = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let id = rel
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(source)
            .to_string();
        let meta = std::fs::read_to_string(folder.join("SKILL.md"))
            .map(|c| parse_skill_frontmatter(&c))
            .unwrap_or_default();
        out.push(RemoteSkill {
            source: source.to_string(),
            id,
            name: meta.name,
            description: meta.description,
            path,
            url: None,
            rev: None,
        });
        kept.push(folder);
    }
    out.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| a.path.cmp(&b.path)));
    out
}

/// Parse an index listing: a JSON array, or `{ "skills": [...] }`, of
/// `{ "name", "description", "url", "path", "rev"? }` (a name that is not a valid id is used as
/// the display name and the id falls back to the last path component).
pub fn parse_index(source: &str, json: &str) -> Result<Vec<RemoteSkill>> {
    #[derive(serde::Deserialize)]
    struct Entry {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        url: Option<String>,
        #[serde(default)]
        path: String,
        #[serde(default)]
        rev: Option<String>,
    }
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Listing {
        List(Vec<Entry>),
        Wrapped { skills: Vec<Entry> },
    }
    let entries = match serde_json::from_str(json).context("parsing the skill index")? {
        Listing::List(v) | Listing::Wrapped { skills: v } => v,
    };
    Ok(entries
        .into_iter()
        .filter(|e| {
            e.url
                .as_deref()
                .is_some_and(|u| check_git_args(u, e.rev.as_deref()).is_ok())
        })
        .filter_map(|e| {
            let last = e
                .path
                .rsplit('/')
                .find(|s| !s.is_empty())
                .map(str::to_string);
            let id = e
                .name
                .clone()
                .filter(|n| valid_id(n))
                .or(last)
                .filter(|i| valid_id(i))?;
            Some(RemoteSkill {
                source: source.to_string(),
                id,
                name: e.name,
                description: e.description,
                path: e.path,
                url: e.url,
                rev: e.rev,
            })
        })
        .collect())
}

#[cfg(feature = "remote")]
fn fetch_index(source: &str, url: &str) -> Result<Vec<RemoteSkill>> {
    // Proxies come from the environment by default in ureq 3.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into();
    let body = agent
        .get(url)
        .call()
        .map_err(|e| anyhow!("fetching {url}: {e}"))?
        .body_mut()
        .read_to_string()?;
    parse_index(source, &body)
}

#[cfg(not(feature = "remote"))]
fn fetch_index(_source: &str, _url: &str) -> Result<Vec<RemoteSkill>> {
    Err(anyhow!("index sources need the `remote` feature"))
}

/// Search every source for skills whose id, name or description contains `query` (case-insensitive;
/// empty = all). A source that fails becomes a line in the second value, never an error.
pub fn search(
    sources: &[SkillSource],
    cache: &Path,
    query: &str,
) -> (Vec<RemoteSkill>, Vec<String>) {
    let needle = query.trim().to_lowercase();
    let hit = |s: &RemoteSkill| {
        needle.is_empty()
            || s.id.to_lowercase().contains(&needle)
            || s.name
                .as_deref()
                .is_some_and(|n| n.to_lowercase().contains(&needle))
            || s.description
                .as_deref()
                .is_some_and(|d| d.to_lowercase().contains(&needle))
    };
    let mut found = Vec::new();
    let mut problems = Vec::new();
    for src in sources {
        match list_source(src, cache) {
            Ok(list) => found.extend(list.into_iter().filter(|s| hit(s))),
            Err(e) => problems.push(format!("{}: {e:#}", src.id)),
        }
    }
    (found, problems)
}

/// Install `skill` (found in `src`) into `store` as `id` (default: the skill's own id) and return
/// the id. The commit and location are recorded beside the copy.
pub fn install_remote(
    store: &dyn CatalogStore,
    src: &SkillSource,
    cache: &Path,
    skill: &RemoteSkill,
    id: Option<&str>,
) -> Result<String> {
    let rel = Path::new(&skill.path);
    if rel.is_absolute()
        || rel
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
    {
        bail!("unsafe skill path '{}'", skill.path);
    }
    // An index entry names its own repository; otherwise the source is one.
    let repo_src = match &skill.url {
        Some(url) => SkillSource {
            id: match &skill.rev {
                Some(rev) => format!("{}-{}-{}", src.id, slug(url), slug(rev)),
                None => format!("{}-{}", src.id, slug(url)),
            },
            label: None,
            kind: SkillSourceKind::Git {
                url: url.clone(),
                rev: skill.rev.clone(),
                subpath: None,
            },
        },
        None => src.clone(),
    };
    let repo = ensure_cached(&repo_src, cache)?;
    let folder = repo.join(rel);
    if !folder.join("SKILL.md").is_file() {
        bail!("no SKILL.md at '{}' in source '{}'", skill.path, src.id);
    }
    // A symlink inside the repository must not lead the copy out of it.
    let folder = folder.canonicalize()?;
    if !folder.starts_with(repo.canonicalize()?) {
        bail!("skill path '{}' leaves the repository", skill.path);
    }
    let (url, rev, _) = git_parts(&repo_src)?;
    let origin = RemoteOrigin {
        url: url.to_string(),
        rev: rev.map(str::to_string),
        path: skill.path.clone(),
        commit: git(Some(&repo), &["rev-parse", "HEAD"]).ok(),
    };
    let id = id.unwrap_or(&skill.id);
    store.install_skill(id, &Source::Dir(folder), Some(&origin))?;
    Ok(id.to_string())
}

/// A folder-name-safe rendering of a URL.
fn slug(url: &str) -> String {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches(".git")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{FsRegistry, Registry, SkillOrigin};
    use tempfile::TempDir;

    fn skill(dir: &Path, rel: &str, name: &str, desc: &str) {
        let d = dir.join(rel);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {desc}\n---\n"),
        )
        .unwrap();
    }

    /// A local repository with two skills (one nested in another) and a non-skill folder.
    fn repo() -> TempDir {
        let t = TempDir::new().unwrap();
        skill(t.path(), "skills/pdf", "pdf", "Read and fill PDF forms");
        skill(
            t.path(),
            "skills/pdf/templates/inner",
            "inner",
            "not separate",
        );
        skill(t.path(), "skills/web", "web", "Design web pages");
        std::fs::create_dir_all(t.path().join("docs")).unwrap();
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["-c", "user.name=t", "-c", "user.email=t@t", "add", "-A"][..],
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "init",
            ][..],
        ] {
            git(Some(t.path()), args).unwrap();
        }
        t
    }

    fn source(id: &str, repo: &Path, subpath: Option<&str>) -> SkillSource {
        SkillSource {
            id: id.into(),
            label: None,
            kind: SkillSourceKind::Git {
                url: repo.to_string_lossy().into_owned(),
                rev: None,
                subpath: subpath.map(str::to_string),
            },
        }
    }

    #[test]
    fn scan_search_and_install_from_a_local_git_repo() {
        let repo = repo();
        let cache = TempDir::new().unwrap();
        let src = source("local", repo.path(), None);

        let all = list_source(&src, cache.path()).unwrap();
        let ids: Vec<_> = all
            .iter()
            .map(|s| (s.id.as_str(), s.path.as_str()))
            .collect();
        assert_eq!(ids, [("pdf", "skills/pdf"), ("web", "skills/web")]);

        let (hits, problems) = search(std::slice::from_ref(&src), cache.path(), "PDF");
        assert!(problems.is_empty());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].description.as_deref(),
            Some("Read and fill PDF forms")
        );
        assert_eq!(
            search(std::slice::from_ref(&src), cache.path(), "").0.len(),
            2
        );
        assert_eq!(
            search(std::slice::from_ref(&src), cache.path(), "web pages")
                .0
                .len(),
            1
        );

        let scoped = source("scoped", repo.path(), Some("skills/web"));
        assert_eq!(list_source(&scoped, cache.path()).unwrap().len(), 1);

        let store_dir = TempDir::new().unwrap();
        let store = FsRegistry::new(store_dir.path());
        let id = install_remote(&store, &src, cache.path(), &hits[0], Some("my-pdf")).unwrap();
        assert_eq!(id, "my-pdf");
        let entry = store.skill("my-pdf").unwrap().unwrap();
        let SkillOrigin::Installed {
            remote: Some(origin),
        } = entry.origin
        else {
            panic!("expected a remote origin");
        };
        assert_eq!(origin.path, "skills/pdf");
        assert_eq!(origin.commit.as_deref().map(str::len), Some(40));
        assert!(
            store_dir
                .path()
                .join("skills/my-pdf/templates/inner/SKILL.md")
                .is_file()
        );
        assert!(!store_dir.path().join("skills/my-pdf/.git").exists());

        // A second refresh picks up new upstream commits.
        skill(repo.path(), "skills/new", "new", "added later");
        git(
            Some(repo.path()),
            &["-c", "user.name=t", "-c", "user.email=t@t", "add", "-A"],
        )
        .unwrap();
        git(
            Some(repo.path()),
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "more",
            ],
        )
        .unwrap();
        refresh_source(&src, cache.path()).unwrap();
        assert_eq!(list_source(&src, cache.path()).unwrap().len(), 3);
    }

    #[test]
    fn failing_source_is_a_problem_and_bad_paths_are_refused() {
        let cache = TempDir::new().unwrap();
        let bad = source("gone", Path::new("/nonexistent/repo"), None);
        let (hits, problems) = search(std::slice::from_ref(&bad), cache.path(), "");
        assert!(hits.is_empty());
        assert_eq!(problems.len(), 1);
        assert!(problems[0].starts_with("gone:"));

        let store = FsRegistry::new(TempDir::new().unwrap().path());
        let evil = RemoteSkill {
            source: "gone".into(),
            id: "x".into(),
            name: None,
            description: None,
            path: "../etc".into(),
            url: None,
            rev: None,
        };
        assert!(install_remote(&store, &bad, cache.path(), &evil, None).is_err());
    }

    #[test]
    fn index_listing_shapes() {
        let list = r#"[{"name":"pdf","description":"d","url":"https://x/r.git","path":"skills/pdf","rev":"v1"},
                       {"description":"no url"},
                       {"name":"Nice Name","url":"https://x/r.git","path":"a/b/tool"}]"#;
        let got = parse_index("idx", list).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].rev.as_deref(), Some("v1"));
        assert_eq!(got[1].id, "tool");
        assert_eq!(got[1].name.as_deref(), Some("Nice Name"));

        let wrapped = r#"{"skills":[{"name":"a","url":"u","path":""}]}"#;
        assert_eq!(parse_index("idx", wrapped).unwrap()[0].id, "a");
        assert!(parse_index("idx", "{}").is_err());
    }

    #[test]
    fn option_looking_urls_and_revs_are_refused() {
        let cache = TempDir::new().unwrap();
        let mut src = source("evil", Path::new("--upload-pack=touch /tmp/x"), None);
        assert!(refresh_source(&src, cache.path()).is_err());
        let repo = repo();
        for rev in ["--foo", "a b", "a..b", ""] {
            src.kind = SkillSourceKind::Git {
                url: repo.path().to_string_lossy().into_owned(),
                rev: Some(rev.into()),
                subpath: None,
            };
            assert!(refresh_source(&src, cache.path()).is_err(), "rev {rev:?}");
        }
        let bad_id = source("../x", repo.path(), None);
        assert!(refresh_source(&bad_id, cache.path()).is_err());
        assert!(!cache.path().join("evil").exists());

        let json = r#"[{"name":"a","url":"-oProxyCommand=x","path":"a"},
                       {"name":"b","url":"https://x/r.git","path":"b","rev":"--x"},
                       {"name":"c","url":"https://x/r.git","path":"c","rev":"v1"}]"#;
        let got = parse_index("idx", json).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "c");
    }

    #[test]
    fn cache_cloned_for_another_rev_is_recloned() {
        let repo = repo();
        let cache = TempDir::new().unwrap();
        let mut src = source("local", repo.path(), None);
        let dir = ensure_cached(&src, cache.path()).unwrap();
        assert!(cache_matches(&dir, &repo.path().to_string_lossy(), None));
        src.kind = SkillSourceKind::Git {
            url: repo.path().to_string_lossy().into_owned(),
            rev: Some("main".into()),
            subpath: None,
        };
        assert!(!cache_matches(
            &dir,
            &repo.path().to_string_lossy(),
            Some("main")
        ));
        // Fresh by age, yet re-cloned because it was cloned for no rev.
        let dir = ensure_cached(&src, cache.path()).unwrap();
        assert!(cache_matches(
            &dir,
            &repo.path().to_string_lossy(),
            Some("main")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn skill_folder_symlinked_out_of_the_repo_is_refused() {
        let outside = TempDir::new().unwrap();
        skill(outside.path(), "s", "s", "outside");
        let repo = repo();
        std::os::unix::fs::symlink(outside.path().join("s"), repo.path().join("skills/link"))
            .unwrap();
        git(
            Some(repo.path()),
            &["-c", "user.name=t", "-c", "user.email=t@t", "add", "-A"],
        )
        .unwrap();
        git(
            Some(repo.path()),
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "link",
            ],
        )
        .unwrap();
        let cache = TempDir::new().unwrap();
        let src = source("local", repo.path(), None);
        let store = FsRegistry::new(TempDir::new().unwrap().path());
        let sk = RemoteSkill {
            source: "local".into(),
            id: "link".into(),
            name: None,
            description: None,
            path: "skills/link".into(),
            url: None,
            rev: None,
        };
        assert!(install_remote(&store, &src, cache.path(), &sk, None).is_err());
    }

    #[test]
    fn defaults_are_the_two_github_repos() {
        let d = default_skill_sources();
        assert_eq!(
            d.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["anthropics", "huggingface"]
        );
    }
}

//! The writable side of a catalog: [`CatalogStore`], implemented for [`FsRegistry`].
//!
//! Every write to `catalog.toml` reads it into a `toml::Table`, edits one array and writes it
//! back through a temp file + rename, so the other tables survive (comments do not).

use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow, bail};

use crate::Result;
use crate::registry::fs::{FsRegistry, ORIGIN_FILE};
use crate::registry::remote::default_skill_sources;
use crate::registry::{
    McpEntry, McpExpose, Registry, RemoteOrigin, SkillOrigin, SkillSource, SkillSourceKind,
    valid_id,
};
use crate::source::{LinkMode, Source};

/// A catalog an embedder can write to. [`FsRegistry`] implements it; Ubiq uses one `FsRegistry`
/// per layer but may supply its own impl (database, remote service) — that is the override point.
pub trait CatalogStore: Registry {
    /// Reference the skill folder at `path` in place (`[[skill]]`). Validates `SKILL.md`; the id
    /// defaults to the folder name. Returns the id.
    fn link_skill(&self, path: &Path, id: Option<&str>) -> Result<String>;
    /// Scan `path` for `<sub>/SKILL.md` skills (`[[skill_dir]]`); adding it twice is a no-op.
    fn add_skill_dir(&self, path: &Path) -> Result<()>;
    /// Stop scanning `path`.
    fn remove_skill_dir(&self, path: &Path) -> Result<()>;
    /// The scanned folders.
    fn skill_dirs(&self) -> Result<Vec<PathBuf>>;
    /// Copy a skill into the catalog as `skills/<id>/`, replacing any previous copy atomically.
    /// `remote` is recorded beside it in `.am-origin.toml`.
    fn install_skill(&self, id: &str, from: &Source, remote: Option<&RemoteOrigin>) -> Result<()>;
    /// Installed: delete the folder. Linked: drop the `[[skill]]` entry. Scanned: an error
    /// (remove the folder instead).
    fn remove_skill(&self, id: &str) -> Result<()>;
    /// Write `mcp/<id>.json`; when the id lives inline in `catalog.toml`, rewrite that entry instead.
    fn put_mcp(&self, entry: &McpEntry) -> Result<()>;
    /// Remove an MCP, wherever it is declared.
    fn remove_mcp(&self, id: &str) -> Result<()>;
    /// The declared skill sources, or [`default_skill_sources`] when none are declared.
    fn skill_sources(&self) -> Result<Vec<SkillSource>>;
    /// Replace the declared skill sources (an empty slice falls back to the defaults).
    fn set_skill_sources(&self, sources: &[SkillSource]) -> Result<()>;
}

fn check_id(id: &str) -> Result<()> {
    if !valid_id(id) {
        bail!("invalid id '{id}': use letters, digits, '.', '_' and '-'");
    }
    Ok(())
}

impl FsRegistry {
    fn catalog_path(&self) -> PathBuf {
        self.root().join("catalog.toml")
    }

    /// `catalog.toml` as an editable table (empty when the file is missing).
    fn read_table(&self) -> Result<toml::Table> {
        let path = self.catalog_path();
        if !path.exists() {
            return Ok(toml::Table::new());
        }
        let text = std::fs::read_to_string(&path)?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Write the table back atomically (temp file + rename), creating the root if needed.
    fn write_table(&self, table: &toml::Table) -> Result<()> {
        std::fs::create_dir_all(self.root())?;
        let path = self.catalog_path();
        let tmp = self.root().join(".catalog.toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(table)?)?;
        std::fs::rename(&tmp, &path).with_context(|| format!("writing {}", path.display()))
    }

    /// Edit the array-of-tables `key` in place; an emptied array removes the key.
    fn edit_array(
        &self,
        key: &str,
        f: impl FnOnce(&mut Vec<toml::Value>) -> Result<()>,
    ) -> Result<()> {
        let mut table = self.read_table()?;
        let mut items = match table.remove(key) {
            Some(toml::Value::Array(a)) => a,
            Some(_) => bail!("catalog.toml: '{key}' is not an array of tables"),
            None => Vec::new(),
        };
        f(&mut items)?;
        if !items.is_empty() {
            table.insert(key.to_string(), toml::Value::Array(items));
        }
        self.write_table(&table)
    }
}

/// Text of `v` as a string field of a table entry.
fn str_field<'a>(v: &'a toml::Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(toml::Value::as_str)
}

fn table_of(pairs: &[(&str, &str)]) -> toml::Value {
    let mut t = toml::Table::new();
    for (k, v) in pairs {
        t.insert((*k).to_string(), toml::Value::String((*v).to_string()));
    }
    toml::Value::Table(t)
}

/// The JSON shape of an MCP entry: the server fields (without `id` unless asked) plus the
/// catalog-only keys, empty collections left out.
fn mcp_json(entry: &McpEntry, with_id: bool) -> Result<serde_json::Value> {
    let mut v = serde_json::to_value(&entry.def)?;
    let obj = v
        .as_object_mut()
        .ok_or_else(|| anyhow!("MCP definition is not an object"))?;
    obj.remove("id");
    for key in ["args", "env", "headers"] {
        let empty = match obj.get(key) {
            Some(serde_json::Value::Array(a)) => a.is_empty(),
            Some(serde_json::Value::Object(o)) => o.is_empty(),
            _ => false,
        };
        if empty {
            obj.remove(key);
        }
    }
    if with_id {
        obj.insert("id".into(), entry.id.clone().into());
    }
    if entry.expose == McpExpose::Skill {
        obj.insert("expose".into(), "skill".into());
    }
    if let Some(s) = entry.summary.as_ref().filter(|s| !s.is_empty()) {
        obj.insert("summary".into(), s.clone().into());
    }
    if let Some(d) = entry.description.as_ref().filter(|s| !s.is_empty()) {
        obj.insert("description".into(), d.clone().into());
    }
    Ok(v)
}

/// Replace a skill folder with `tmp` (already complete): move the old one aside, swap, delete it.
fn swap_dir(tmp: &Path, dest: &Path) -> Result<()> {
    let old = dest.with_file_name(format!(
        ".{}.old",
        dest.file_name().and_then(|n| n.to_str()).unwrap_or("skill")
    ));
    if old.exists() {
        std::fs::remove_dir_all(&old)?;
    }
    let had_old = dest.exists();
    if had_old {
        std::fs::rename(dest, &old)?;
    }
    if let Err(e) = std::fs::rename(tmp, dest) {
        if had_old {
            let _ = std::fs::rename(&old, dest);
        }
        return Err(e).with_context(|| format!("installing {}", dest.display()));
    }
    if had_old {
        let _ = std::fs::remove_dir_all(&old);
    }
    Ok(())
}

impl CatalogStore for FsRegistry {
    fn link_skill(&self, path: &Path, id: Option<&str>) -> Result<String> {
        let path = std::path::absolute(path)?;
        if !path.join("SKILL.md").is_file() {
            bail!("no SKILL.md in {}", path.display());
        }
        let id = match id {
            Some(id) => id.to_string(),
            None => path
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
                .ok_or_else(|| anyhow!("cannot derive an id from {}", path.display()))?,
        };
        check_id(&id)?;
        let path_str = path.to_string_lossy().into_owned();
        self.edit_array("skill", |items| {
            items.retain(|v| str_field(v, "id") != Some(id.as_str()));
            items.push(table_of(&[("id", &id), ("path", &path_str)]));
            Ok(())
        })?;
        Ok(id)
    }

    fn add_skill_dir(&self, path: &Path) -> Result<()> {
        let path = std::path::absolute(path)?;
        if !path.is_dir() {
            bail!("not a folder: {}", path.display());
        }
        let path_str = path.to_string_lossy().into_owned();
        self.edit_array("skill_dir", |items| {
            if !items
                .iter()
                .any(|v| str_field(v, "path") == Some(path_str.as_str()))
            {
                items.push(table_of(&[("path", &path_str)]));
            }
            Ok(())
        })
    }

    fn remove_skill_dir(&self, path: &Path) -> Result<()> {
        let path = std::path::absolute(path)?.to_string_lossy().into_owned();
        self.edit_array("skill_dir", |items| {
            items.retain(|v| str_field(v, "path") != Some(path.as_str()));
            Ok(())
        })
    }

    fn skill_dirs(&self) -> Result<Vec<PathBuf>> {
        Ok(self
            .catalog_toml()?
            .skill_dir
            .into_iter()
            .map(|d| PathBuf::from(d.path))
            .collect())
    }

    fn install_skill(&self, id: &str, from: &Source, remote: Option<&RemoteOrigin>) -> Result<()> {
        check_id(id)?;
        let skills = self.root().join("skills");
        std::fs::create_dir_all(&skills)?;
        let tmp = skills.join(format!(".{id}.tmp"));
        if tmp.exists() {
            std::fs::remove_dir_all(&tmp)?;
        }
        std::fs::create_dir_all(&tmp)?;
        let result = (|| -> Result<()> {
            from.materialize(&tmp, LinkMode::Copy, true)?;
            // A skill at a repository's root brings its checkout metadata along.
            let git = tmp.join(".git");
            if git.exists() {
                std::fs::remove_dir_all(&git)?;
            }
            if !tmp.join("SKILL.md").is_file() {
                bail!("the source has no SKILL.md at its top level");
            }
            if let Some(origin) = remote {
                std::fs::write(tmp.join(ORIGIN_FILE), toml::to_string_pretty(origin)?)?;
            }
            swap_dir(&tmp, &skills.join(id))
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&tmp);
        }
        result
    }

    fn remove_skill(&self, id: &str) -> Result<()> {
        check_id(id)?;
        let installed = self.root().join("skills").join(id);
        if installed.join("SKILL.md").is_file() {
            return std::fs::remove_dir_all(&installed).map_err(Into::into);
        }
        let is_link = |id_field: Option<&str>, path: &str| match id_field {
            Some(i) => i == id,
            None => folder_is(path, id),
        };
        if self
            .catalog_toml()?
            .skill
            .iter()
            .any(|l| is_link(l.id.as_deref(), &l.path))
        {
            return self.edit_array("skill", |items| {
                items.retain(|v| {
                    !is_link(str_field(v, "id"), str_field(v, "path").unwrap_or_default())
                });
                Ok(())
            });
        }
        match self.skill(id)? {
            Some(e) if matches!(e.origin, SkillOrigin::Dir(_)) => {
                bail!("'{id}' comes from a scanned folder: remove the folder instead")
            }
            _ => bail!("no skill '{id}' in this catalog"),
        }
    }

    fn put_mcp(&self, entry: &McpEntry) -> Result<()> {
        check_id(&entry.id)?;
        let inline = self
            .catalog_toml()?
            .mcp
            .iter()
            .any(|m| m.server.id == entry.id);
        if inline {
            let value = toml::Value::try_from(mcp_json(entry, true)?)?;
            return self.edit_array("mcp", |items| {
                for item in items.iter_mut() {
                    if str_field(item, "id") == Some(entry.id.as_str()) {
                        *item = value.clone();
                    }
                }
                Ok(())
            });
        }
        let dir = self.root().join("mcp");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.json", entry.id));
        let tmp = dir.join(format!(".{}.json.tmp", entry.id));
        std::fs::write(
            &tmp,
            serde_json::to_string_pretty(&mcp_json(entry, false)?)?,
        )?;
        std::fs::rename(&tmp, &path).with_context(|| format!("writing {}", path.display()))
    }

    fn remove_mcp(&self, id: &str) -> Result<()> {
        check_id(id)?;
        let file = self.root().join("mcp").join(format!("{id}.json"));
        let mut removed = false;
        if file.is_file() {
            std::fs::remove_file(&file)?;
            removed = true;
        }
        if self.catalog_toml()?.mcp.iter().any(|m| m.server.id == id) {
            self.edit_array("mcp", |items| {
                items.retain(|v| str_field(v, "id") != Some(id));
                Ok(())
            })?;
            removed = true;
        }
        if !removed {
            bail!("no MCP '{id}' in this catalog");
        }
        Ok(())
    }

    fn skill_sources(&self) -> Result<Vec<SkillSource>> {
        let declared = self.catalog_toml()?.skill_source;
        if declared.is_empty() {
            return Ok(default_skill_sources());
        }
        declared.into_iter().map(|s| s.into_source()).collect()
    }

    fn set_skill_sources(&self, sources: &[SkillSource]) -> Result<()> {
        for s in sources {
            check_id(&s.id)?;
        }
        let items: Vec<toml::Value> = sources
            .iter()
            .map(|s| {
                let mut t = toml::Table::new();
                t.insert("id".into(), s.id.clone().into());
                if let Some(label) = &s.label {
                    t.insert("label".into(), label.clone().into());
                }
                match &s.kind {
                    SkillSourceKind::Git { url, rev, subpath } => {
                        t.insert("kind".into(), "git".into());
                        t.insert("url".into(), url.clone().into());
                        if let Some(rev) = rev {
                            t.insert("rev".into(), rev.clone().into());
                        }
                        if let Some(subpath) = subpath {
                            t.insert("subpath".into(), subpath.clone().into());
                        }
                    }
                    SkillSourceKind::Index { url } => {
                        t.insert("kind".into(), "index".into());
                        t.insert("url".into(), url.clone().into());
                    }
                }
                toml::Value::Table(t)
            })
            .collect();
        self.edit_array("skill_source", |old| {
            *old = items;
            Ok(())
        })
    }
}

/// Whether the last component of `path` is `name`.
fn folder_is(path: &str, name: &str) -> bool {
    Path::new(path).file_name().and_then(|n| n.to_str()) == Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{McpServer, McpTransport};
    use tempfile::TempDir;

    fn skill_folder(parent: &Path, name: &str, description: &str) -> PathBuf {
        let dir = parent.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\nbody\n"),
        )
        .unwrap();
        dir
    }

    fn mcp(id: &str) -> McpEntry {
        McpEntry {
            id: id.into(),
            def: McpServer {
                id: id.into(),
                transport: McpTransport::Stdio,
                command: Some("npx".into()),
                args: vec!["-y".into(), "pkg".into()],
                env: Default::default(),
                url: None,
                headers: Default::default(),
            },
            expose: McpExpose::Tools,
            summary: None,
            description: Some("a server".into()),
        }
    }

    #[test]
    fn link_scan_and_precedence() {
        let cat = TempDir::new().unwrap();
        let ext = TempDir::new().unwrap();
        let reg = FsRegistry::new(cat.path().join("catalog"));

        let pdf = skill_folder(ext.path(), "pdf", "reads pdfs");
        assert_eq!(reg.link_skill(&pdf, None).unwrap(), "pdf");
        assert_eq!(reg.link_skill(&pdf, Some("pdf2")).unwrap(), "pdf2");
        let scanned = ext.path().join("scanned");
        skill_folder(&scanned, "pdf", "scanned pdf");
        skill_folder(&scanned, "other", "other");
        reg.add_skill_dir(&scanned).unwrap();
        reg.add_skill_dir(&scanned).unwrap();
        assert_eq!(reg.skill_dirs().unwrap().len(), 1);

        let skills = reg.skills().unwrap();
        let ids: Vec<_> = skills.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["other", "pdf", "pdf2"]);
        // [[skill]] beats [[skill_dir]] on the clash.
        assert!(matches!(
            reg.skill("pdf").unwrap().unwrap().origin,
            SkillOrigin::Linked(_)
        ));
        assert!(matches!(
            reg.skill("other").unwrap().unwrap().origin,
            SkillOrigin::Dir(_)
        ));

        // Installed beats linked.
        let src = skill_folder(ext.path(), "src", "installed");
        reg.install_skill("pdf", &Source::Dir(src), None).unwrap();
        let e = reg.skill("pdf").unwrap().unwrap();
        assert_eq!(e.origin, SkillOrigin::Installed { remote: None });
        assert_eq!(e.meta.description.as_deref(), Some("installed"));

        reg.remove_skill("pdf").unwrap(); // installed copy
        assert!(matches!(
            reg.skill("pdf").unwrap().unwrap().origin,
            SkillOrigin::Linked(_)
        ));
        reg.remove_skill("pdf").unwrap(); // link
        assert!(matches!(
            reg.skill("pdf").unwrap().unwrap().origin,
            SkillOrigin::Dir(_)
        ));
        let err = reg.remove_skill("pdf").unwrap_err().to_string();
        assert!(err.contains("remove the folder"), "{err}");
        assert!(reg.remove_skill("nope").is_err());
        reg.remove_skill_dir(&scanned).unwrap();
        assert!(reg.skills().unwrap().iter().all(|s| s.id == "pdf2"));
    }

    #[test]
    fn install_records_origin_and_skips_it_on_materialize() {
        let cat = TempDir::new().unwrap();
        let ext = TempDir::new().unwrap();
        let reg = FsRegistry::new(cat.path());
        let src = skill_folder(ext.path(), "s", "d");
        std::fs::write(src.join("extra.txt"), "x").unwrap();
        let origin = RemoteOrigin {
            url: "https://example.com/r".into(),
            rev: Some("main".into()),
            path: "skills/s".into(),
            commit: Some("abc".into()),
        };
        reg.install_skill("s", &Source::Dir(src.clone()), Some(&origin))
            .unwrap();
        reg.install_skill("s", &Source::Dir(src), Some(&origin))
            .unwrap(); // atomic re-install
        let e = reg.skill("s").unwrap().unwrap();
        assert_eq!(
            e.origin,
            SkillOrigin::Installed {
                remote: Some(origin)
            }
        );
        assert!(cat.path().join("skills/s/extra.txt").is_file());
        assert!(!cat.path().join("skills/.s.tmp").exists());

        let run = TempDir::new().unwrap();
        e.source
            .materialize(run.path(), LinkMode::Copy, true)
            .unwrap();
        assert!(run.path().join("SKILL.md").is_file());
        assert!(!run.path().join(ORIGIN_FILE).exists());

        assert!(
            reg.install_skill("bad/id", &Source::Files(vec![]), None)
                .is_err()
        );
        assert!(
            reg.install_skill("empty", &Source::Files(vec![]), None)
                .is_err()
        );
        assert!(!cat.path().join("skills/empty").exists());
    }

    #[test]
    fn link_rejects_missing_skill_md_and_bad_id() {
        let cat = TempDir::new().unwrap();
        let reg = FsRegistry::new(cat.path());
        assert!(reg.link_skill(cat.path(), None).is_err());
        let s = skill_folder(cat.path(), "ok", "d");
        assert!(reg.link_skill(&s, Some("a b")).is_err());
    }

    #[test]
    fn mcp_json_and_inline_round_trip() {
        let cat = TempDir::new().unwrap();
        let reg = FsRegistry::new(cat.path());
        std::fs::write(
            cat.path().join("catalog.toml"),
            "[registry]\nname = \"x\"\n\n[[mcp]]\nid = \"inline\"\ntransport = \"stdio\"\ncommand = \"a\"\n",
        )
        .unwrap();

        reg.put_mcp(&mcp("fresh")).unwrap();
        assert!(cat.path().join("mcp/fresh.json").is_file());
        let got = reg.mcp("fresh").unwrap().unwrap();
        assert_eq!(got.def.args, ["-y", "pkg"]);
        assert_eq!(got.description.as_deref(), Some("a server"));

        let mut inline = mcp("inline");
        inline.expose = McpExpose::Skill;
        inline.summary = Some("s".into());
        reg.put_mcp(&inline).unwrap();
        assert!(!cat.path().join("mcp/inline.json").exists());
        let got = reg.mcp("inline").unwrap().unwrap();
        assert_eq!(got.def.command.as_deref(), Some("npx"));
        assert_eq!(got.expose, McpExpose::Skill);
        assert!(
            std::fs::read_to_string(cat.path().join("catalog.toml"))
                .unwrap()
                .contains("[registry]")
        );

        reg.remove_mcp("fresh").unwrap();
        reg.remove_mcp("inline").unwrap();
        assert!(reg.mcps().unwrap().is_empty());
        assert!(reg.remove_mcp("inline").is_err());
        assert!(reg.put_mcp(&mcp("bad id")).is_err());
    }

    #[test]
    fn skill_sources_default_then_declared() {
        let cat = TempDir::new().unwrap();
        let reg = FsRegistry::new(cat.path());
        assert_eq!(reg.skill_sources().unwrap(), default_skill_sources());
        let mine = vec![
            SkillSource {
                id: "mine".into(),
                label: Some("Mine".into()),
                kind: SkillSourceKind::Git {
                    url: "https://example.com/r".into(),
                    rev: Some("dev".into()),
                    subpath: Some("skills".into()),
                },
            },
            SkillSource {
                id: "idx".into(),
                label: None,
                kind: SkillSourceKind::Index {
                    url: "https://example.com/i.json".into(),
                },
            },
        ];
        reg.set_skill_sources(&mine).unwrap();
        assert_eq!(reg.skill_sources().unwrap(), mine);
        reg.set_skill_sources(&[]).unwrap();
        assert_eq!(reg.skill_sources().unwrap(), default_skill_sources());
    }
}

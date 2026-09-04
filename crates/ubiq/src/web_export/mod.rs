//! A local, on-demand HTTP server that serves a project's files read-only, for browsing
//! markdown docs and source code in a real web browser. Entirely self-contained in the UI
//! crate: no proto messages, no host involvement — the process already has the file on disk.

mod assets;
mod routes;
mod server;

pub use server::ensure_started_and_registered;

#[cfg(test)]
mod tests {
    use super::ensure_started_and_registered;
    use std::fs;

    /// One shared server for the whole process (`ensure_started_and_registered`'s contract), so
    /// each test registers its own project under a name unique to it rather than assuming a fresh
    /// server — this is the smoke test for the routing and path-safety logic in `routes.rs`.
    fn serve_temp_project(name: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("README.md"), "# Hello\n\nSome *text*.\n").unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".git").join("config"), "secret").unwrap();
        let base = ensure_started_and_registered(name, name, dir.path()).unwrap();
        (dir, base)
    }

    #[test]
    fn serves_markdown_as_html() {
        let (_dir, base) = serve_temp_project("web-export-test-markdown");
        let body = ureq::get(&format!("{base}README.md"))
            .call()
            .unwrap()
            .into_string()
            .unwrap();
        assert!(body.contains("Hello"), "missing rendered heading: {body}");
        assert!(
            body.contains("<em>text</em>"),
            "markdown not rendered: {body}"
        );
    }

    #[test]
    fn refuses_dotfile_and_traversal_paths() {
        let (_dir, base) = serve_temp_project("web-export-test-safety");
        let dotfile = ureq::get(&format!("{base}.git/config")).call();
        assert!(
            matches!(dotfile, Err(ureq::Error::Status(404, _))),
            "dotfile path should 404, got {dotfile:?}"
        );
        let traversal = ureq::get(&format!("{base}../../../etc/passwd")).call();
        assert!(
            matches!(traversal, Err(ureq::Error::Status(404, _))),
            "traversal path should 404, got {traversal:?}"
        );
    }

    #[test]
    fn search_finds_a_term_and_skips_the_dotfile() {
        let (_dir, base) = serve_temp_project("web-export-test-search");
        let get_json = |url: &str| -> serde_json::Value {
            let text = ureq::get(url).call().unwrap().into_string().unwrap();
            serde_json::from_str(&text).unwrap()
        };

        let body = get_json(&format!("{base}_search?q=hello"));
        let results = body["results"].as_array().unwrap();
        assert_eq!(results.len(), 1, "only README.md has the term: {body}");
        assert_eq!(results[0]["path"], "README.md");
        assert_eq!(results[0]["lines"][0]["line"], 1);

        let empty = get_json(&format!("{base}_search?q="));
        assert!(
            empty["results"].as_array().unwrap().is_empty(),
            "an empty query finds nothing rather than everything: {empty}"
        );

        let secret = get_json(&format!("{base}_search?q=secret"));
        assert!(
            secret["results"].as_array().unwrap().is_empty(),
            "the .git dotfile is outside the walk, same as the nav tree: {secret}"
        );
    }

    #[test]
    fn an_unrecognised_extension_is_sniffed_by_size() {
        let (dir, base) = serve_temp_project("web-export-test-unknown");
        fs::write(dir.path().join("notes.xyz"), "plain text notes").unwrap();
        fs::write(dir.path().join("big.xyz"), vec![b'a'; 300 * 1024]).unwrap();

        let small = ureq::get(&format!("{base}notes.xyz"))
            .call()
            .unwrap()
            .into_string()
            .unwrap();
        assert!(
            small.contains("plain text notes"),
            "a small, text-sniffed unknown extension renders as text: {small}"
        );

        let big = ureq::get(&format!("{base}big.xyz"))
            .call()
            .unwrap()
            .into_string()
            .unwrap();
        assert!(
            big.len() < 300 * 1024,
            "a file past the sniff ceiling is never read into the page: {} bytes",
            big.len()
        );
        assert!(
            big.contains("Download") && big.contains("KB"),
            "the oversized case shows a size and a download link: {big}"
        );
    }

    #[test]
    fn raw_passthrough_serves_bytes_and_download_forces_disposition() {
        let (_dir, base) = serve_temp_project("web-export-test-raw");

        let raw = ureq::get(&format!("{base}README.md?raw=1")).call().unwrap();
        assert_eq!(
            raw.header("Content-Type"),
            Some("text/plain; charset=utf-8")
        );
        assert!(raw.into_string().unwrap().contains("# Hello"));

        let dl = ureq::get(&format!("{base}README.md?raw=1&dl=1"))
            .call()
            .unwrap();
        assert!(
            dl.header("Content-Disposition")
                .is_some_and(|h| h.starts_with("attachment")),
            "the download flag forces an attachment disposition"
        );
    }

    #[test]
    fn a_known_image_extension_is_embedded_not_downloaded() {
        let (dir, base) = serve_temp_project("web-export-test-media");
        fs::write(dir.path().join("shot.png"), [0u8, 1, 2, 3]).unwrap();

        let body = ureq::get(&format!("{base}shot.png"))
            .call()
            .unwrap()
            .into_string()
            .unwrap();
        assert!(
            body.contains("<img src=\"?raw=1\""),
            "a recognised image is embedded in the page shell, not served bare: {body}"
        );
    }
}

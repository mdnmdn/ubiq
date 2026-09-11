//! A local, on-demand HTTP server that serves a project's files read-only, for browsing
//! markdown docs and source code in a real web browser. Entirely self-contained in the UI
//! crate: no proto messages, no host involvement — the process already has the file on disk.

pub mod bridge;

mod assets;
mod routes;
mod server;

pub use server::ensure_started_and_registered;
pub use server::{WebSession, close_session, open_session, receive, send, set_vendor_root};

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

    // --- Web panels ---

    use super::bridge::{self, FromWeb, ToWeb};
    use super::{close_session, open_session, receive, send, set_vendor_root};

    /// Posts one bridge envelope and answers the status code. A refusal here is a 404 rather than
    /// a transport error, so the status is the whole of what a caller asserts on.
    fn post_frame(url: &str, body: serde_json::Value) -> u16 {
        match ureq::post(url)
            .set("Content-Type", "application/json")
            .send_string(&body.to_string())
        {
            Ok(response) => response.status(),
            Err(ureq::Error::Status(code, _)) => code,
            Err(err) => panic!("web bridge post failed to reach the server: {err}"),
        }
    }

    #[test]
    fn a_web_session_serves_its_chrome_and_round_trips_both_directions() {
        let session = open_session(bridge::DEMO_APP).unwrap();

        let chrome = ureq::get(&session.url).call().unwrap();
        assert_eq!(
            chrome.header("Content-Type"),
            Some("text/html; charset=utf-8")
        );
        let csp = chrome
            .header("Content-Security-Policy")
            .expect("the chrome names a policy")
            .to_string();
        assert!(
            csp.contains("default-src 'self'") && csp.contains("frame-ancestors 'none'"),
            "the chrome's policy names only this origin: {csp}"
        );
        assert!(chrome.into_string().unwrap().contains("bridge.js"));

        let shim = ureq::get(&format!("{}bridge.js", session.url))
            .call()
            .unwrap();
        assert_eq!(
            shim.header("Content-Type"),
            Some("text/javascript; charset=utf-8")
        );
        assert!(
            ureq::get(&format!("{}app.js", session.url))
                .call()
                .is_ok_and(|r| r.status() == 200)
        );

        // Interface -> web: queued first, so the long-poll answers at once rather than parking.
        send(
            &session.token,
            bridge::to_value(&ToWeb::Open {
                document: "{}".to_string(),
                palette: "dark".to_string(),
            }),
        );
        let polled: serde_json::Value = serde_json::from_str(
            &ureq::get(&format!("{}bridge", session.url))
                .call()
                .unwrap()
                .into_string()
                .unwrap(),
        )
        .unwrap();
        let frames = polled.as_array().unwrap();
        assert_eq!(
            frames.len(),
            1,
            "the queued frame comes back once: {polled}"
        );
        assert_eq!(frames[0]["type"], "open");
        assert_eq!(frames[0]["palette"], "dark");

        // Web -> interface: posted with the token, drained by the panel.
        assert_eq!(
            post_frame(
                &format!("{}bridge", session.url),
                serde_json::json!({ "token": session.token, "frame": { "type": "ready" } }),
            ),
            200
        );
        let received = receive(&session.token);
        assert_eq!(received.len(), 1, "one posted frame drains once");
        assert_eq!(
            bridge::decode_from_web(&received[0]),
            Some(FromWeb::Ready),
            "the typed decode round-trips"
        );
        assert!(
            receive(&session.token).is_empty(),
            "a drained queue drains empty"
        );

        close_session(&session.token);
    }

    #[test]
    fn a_wrong_token_and_a_closed_session_both_404() {
        let session = open_session(bridge::DEMO_APP).unwrap();
        let base = session.url.trim_end_matches(&format!("{}/", session.token));

        let wrong_url = format!("{base}deadbeef/index.html");
        assert!(
            matches!(
                ureq::get(&wrong_url).call(),
                Err(ureq::Error::Status(404, _))
            ),
            "a token naming no session is dropped, never a 200"
        );

        // A well-formed frame on the wrong token is refused too — the URL is not the credential.
        let bad_frame = post_frame(
            &format!("{}bridge", session.url),
            serde_json::json!({ "token": "deadbeef", "frame": { "type": "ready" } }),
        );
        assert_eq!(
            bad_frame, 404,
            "a frame carrying the wrong token is dropped"
        );
        assert!(receive(&session.token).is_empty());

        close_session(&session.token);
        assert!(
            matches!(
                ureq::get(&session.url).call(),
                Err(ureq::Error::Status(404, _))
            ),
            "a closed session's chrome is gone"
        );
    }

    #[test]
    fn an_unknown_variant_is_dropped_rather_than_erroring() {
        let session = open_session(bridge::DEMO_APP).unwrap();
        assert_eq!(
            post_frame(
                &format!("{}bridge", session.url),
                serde_json::json!({ "token": session.token, "frame": { "type": "from_a_newer_chrome" } }),
            ),
            200,
            "an unknown variant is accepted on the wire, not rejected"
        );

        let received = receive(&session.token);
        assert_eq!(received.len(), 1);
        assert_eq!(
            bridge::decode_from_web(&received[0]),
            None,
            "an unknown variant decodes to nothing rather than an error"
        );
        assert_eq!(
            bridge::decode_from_web(&serde_json::json!({ "nothing": true })),
            None,
            "so does a frame with no tag at all"
        );

        close_session(&session.token);
    }

    /// The three things the Excalidraw chrome cannot boot without, asserted where they are cheap
    /// to assert: the import map is reachable at the name the chrome fetches, the page carries a
    /// nonce matching its own policy (an inline `<script type="importmap">` is blocked without
    /// one), and the policy still names no remote origin — a missing font must fail rather than
    /// leak to `esm.sh`.
    #[test]
    fn the_excalidraw_chrome_carries_a_nonce_and_an_origin_locked_policy() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("importmap.json"), r#"{"imports":{}}"#).unwrap();

        let session = open_session(bridge::EXCALIDRAW_APP).unwrap();
        set_vendor_root(&session.token, Some(dir.path().to_path_buf()));

        let page = ureq::get(&session.url).call().unwrap();
        let csp = page
            .header("Content-Security-Policy")
            .expect("the chrome names a policy")
            .to_string();
        let body = page.into_string().unwrap();

        assert!(
            !body.contains("__UBIQ_NONCE__"),
            "the placeholder is substituted, not served: {body}"
        );
        let nonce = body
            .split("data-nonce=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("the page carries its nonce")
            .to_string();
        assert_eq!(nonce.len(), 48, "192 bits, hex-encoded: {nonce}");
        assert!(
            csp.contains(&format!("'nonce-{nonce}'")),
            "the policy names the page's own nonce: {csp}"
        );
        assert!(
            !csp.contains("http") && !csp.contains("esm.sh"),
            "no remote origin is permitted, so a hole in the mirror fails loudly: {csp}"
        );

        // Two more nonces, two more responses: a nonce reused across responses is not one.
        let second = ureq::get(&session.url)
            .call()
            .unwrap()
            .into_string()
            .unwrap();
        assert!(!second.contains(&nonce), "a nonce is minted per response");

        // `importmap.json` is what the chrome fetches first; without it nothing else loads.
        let map = ureq::get(&format!("{}vendor/importmap.json", session.url))
            .call()
            .unwrap();
        assert_eq!(map.into_string().unwrap(), r#"{"imports":{}}"#);

        assert!(
            ureq::get(&format!("{}app.js", session.url))
                .call()
                .unwrap()
                .into_string()
                .unwrap()
                .contains("EXCALIDRAW_ASSET_PATH"),
            "the chrome module is the Excalidraw one, not the demo's"
        );

        close_session(&session.token);
    }

    #[test]
    fn the_vendor_route_refuses_traversal_and_types_an_extensionless_module() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("+esm"), "export default 1;\n").unwrap();
        fs::write(dir.path().join("index.mjs"), "export default 2;\n").unwrap();
        fs::create_dir_all(dir.path().join(".hidden")).unwrap();
        fs::write(dir.path().join(".hidden").join("secret"), "no").unwrap();

        let session = open_session(bridge::DEMO_APP).unwrap();
        set_vendor_root(&session.token, Some(dir.path().to_path_buf()));

        // Extensionless is the jsDelivr `+esm` case: `application/octet-stream` here is a blank
        // panel, not a degradation.
        let esm = ureq::get(&format!("{}vendor/+esm", session.url))
            .call()
            .unwrap();
        assert_eq!(
            esm.header("Content-Type"),
            Some("text/javascript; charset=utf-8")
        );
        let mjs = ureq::get(&format!("{}vendor/index.mjs", session.url))
            .call()
            .unwrap();
        assert_eq!(
            mjs.header("Content-Type"),
            Some("text/javascript; charset=utf-8")
        );

        let dotfile = ureq::get(&format!("{}vendor/.hidden/secret", session.url)).call();
        assert!(
            matches!(dotfile, Err(ureq::Error::Status(404, _))),
            "the vendor route refuses a dotfile: {dotfile:?}"
        );
        let traversal = ureq::get(&format!("{}vendor/../../etc/passwd", session.url)).call();
        assert!(
            matches!(traversal, Err(ureq::Error::Status(404, _))),
            "the vendor route refuses traversal: {traversal:?}"
        );

        set_vendor_root(&session.token, None);
        let gone = ureq::get(&format!("{}vendor/+esm", session.url)).call();
        assert!(
            matches!(gone, Err(ureq::Error::Status(404, _))),
            "with no vendor root the route answers nothing: {gone:?}"
        );

        close_session(&session.token);
    }
}

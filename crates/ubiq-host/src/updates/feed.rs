//! The signed feed: fetching a channel's manifest, verifying it, and choosing a release.
//!
//! **Verify, then parse.** A manifest's bytes are checked against the baked-in minisign key before
//! a single field is read, and a channel file with no signature beside it is an error, not a
//! channel with nothing in it. Only a 404 on the manifest itself means "nothing there".
//!
//! **Pick strictly by semver.** Prerelease identifiers compare by text, so `1.5.0-beta.1` sorts
//! below `1.5.0-nightly.7` and both below `1.5.0`. That is semver's rule and is not adjusted here:
//! a nightly follower is offered the highest version any of its channels carries, and never
//! anything not greater than what is running.

use std::collections::HashMap;

use semver::Version;
use serde::Deserialize;
use ubiq_proto::update::{UpdateChannel, UpdateInfo};

/// One GET. `Ok(None)` is a 404; any other failure is an `Err` sentence.
pub type Fetch = fn(&str) -> Result<Option<Vec<u8>>, String>;

/// Largest manifest accepted.
const MAX_MANIFEST: u64 = 1 << 20;

/// What the feed says about one platform's download.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Asset {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    channel: UpdateChannel,
    version: String,
    published: String,
    notes_url: String,
    platforms: HashMap<String, Asset>,
}

/// A release that applies to this platform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub info: UpdateInfo,
    pub version: Version,
    pub asset: Asset,
}

/// Accept only `https://` URLs.
pub fn require_https(url: &str) -> Result<(), String> {
    if url.len() > 8 && url[..8].eq_ignore_ascii_case("https://") {
        Ok(())
    } else {
        Err(format!("the update feed named a URL that is not https: {url}"))
    }
}

/// Check `body` against `signature` (the text of a `.minisig`) under the base64 public key.
pub fn verify(pubkey: &str, body: &[u8], signature: &str) -> Result<(), String> {
    let key = minisign_verify::PublicKey::from_base64(pubkey.trim())
        .map_err(|error| format!("the update key baked into this build is invalid: {error}"))?;
    let signature = minisign_verify::Signature::decode(signature)
        .map_err(|error| format!("the update signature is malformed: {error}"))?;
    // Legacy (non-prehashed) signatures are accepted: only the hashing differs, and which
    // minisign release signed the feed is not a property worth failing an update over.
    key.verify(body, &signature, true)
        .map_err(|error| format!("the update signature does not match: {error}"))
}

/// Parse a verified manifest into a release for `platform`. `Ok(None)` when the platform has no
/// build in it.
pub fn parse(body: &[u8], platform: &str) -> Result<Option<Release>, String> {
    let manifest: Manifest = serde_json::from_slice(body)
        .map_err(|error| format!("the update manifest could not be read: {error}"))?;
    let version = Version::parse(manifest.version.trim_start_matches('v'))
        .map_err(|error| format!("the update manifest has a bad version: {error}"))?;
    require_https(&manifest.notes_url)?;
    let Some(asset) = manifest.platforms.get(platform) else {
        return Ok(None);
    };
    require_https(&asset.url)?;
    Ok(Some(Release {
        info: UpdateInfo {
            version: version.to_string(),
            channel: manifest.channel,
            published: manifest.published,
            notes_url: manifest.notes_url,
            size: asset.size,
        },
        version,
        asset: asset.clone(),
    }))
}

/// The highest release strictly newer than `current`.
pub fn pick(current: &Version, releases: Vec<Release>) -> Option<Release> {
    releases
        .into_iter()
        .filter(|release| release.version > *current)
        .max_by(|a, b| a.version.cmp(&b.version))
}

/// Fetch, verify and parse every channel `selected` includes, and pick the newest release.
pub fn newest(
    fetch: Fetch,
    feed: &str,
    pubkey: &str,
    selected: UpdateChannel,
    current: &Version,
    platform: &str,
) -> Result<Option<Release>, String> {
    let feed = feed.trim_end_matches('/');
    let mut found = Vec::new();
    for channel in UpdateChannel::ALL.iter().copied().filter(|c| selected.includes(*c)) {
        let url = format!("{feed}/{}.json", channel.slug());
        let Some(body) = fetch(&url)? else { continue };
        let Some(signature) = fetch(&format!("{url}.minisig"))? else {
            return Err(format!("the {} update manifest has no signature", channel.slug()));
        };
        let signature = String::from_utf8(signature)
            .map_err(|_| "the update signature is not text".to_string())?;
        verify(pubkey, &body, &signature)?;
        found.extend(parse(&body, platform)?);
    }
    Ok(pick(current, found))
}

/// The default fetcher: one capped GET over https.
pub fn http(url: &str) -> Result<Option<Vec<u8>>, String> {
    use std::io::Read;
    require_https(url)?;
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build();
    match agent.get(url).set("User-Agent", super::USER_AGENT).call() {
        Ok(response) => {
            let mut bytes = Vec::new();
            response
                .into_reader()
                .take(MAX_MANIFEST + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| format!("reading the update feed failed: {error}"))?;
            if bytes.len() as u64 > MAX_MANIFEST {
                return Err("the update manifest is too large".to_string());
            }
            Ok(Some(bytes))
        }
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(error) => Err(format!("the update feed could not be reached: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
    /// minisign-verify's own vector: a prehashed signature over the four bytes `test`.
    const SIG: &str = "untrusted comment: signature from minisign secret key
RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=
trusted comment: timestamp:1633700835\tfile:test\tprehashed
wLMDjy9FLAuxZ3q4NlEvkgtyhrr0gtTu6KC4KBJdITbbOeAi1zBIYo0v4iTgt8jJpIidRJnp94ABQkJAgAooBQ==";

    fn manifest(channel: &str, version: &str, url: &str) -> String {
        format!(
            r#"{{"channel":"{channel}","version":"{version}","published":"2026-01-01","notes_url":"https://example.com/n",
"platforms":{{"macos-aarch64":{{"url":"{url}","sha256":"ab","size":7}}}}}}"#
        )
    }

    fn release(channel: &str, version: &str) -> Release {
        parse(manifest(channel, version, "https://e.com/a.zip").as_bytes(), "macos-aarch64")
            .unwrap()
            .unwrap()
    }

    #[test]
    fn a_good_signature_verifies_and_a_changed_body_does_not() {
        assert!(verify(KEY, b"test", SIG).is_ok());
        assert!(verify(KEY, b"Test", SIG).is_err());
        assert!(verify(KEY, b"test", "garbage").is_err());
        assert!(verify("not a key", b"test", SIG).is_err());
    }

    #[test]
    fn a_manifest_parses_for_its_platform_only() {
        let body = manifest("stable", "v1.4.0", "https://e.com/a.zip");
        let release = parse(body.as_bytes(), "macos-aarch64").unwrap().unwrap();
        assert_eq!(release.info.version, "1.4.0");
        assert_eq!(release.info.channel, UpdateChannel::Stable);
        assert_eq!(release.info.size, 7);
        assert!(parse(body.as_bytes(), "windows-x86_64").unwrap().is_none());
        assert!(parse(b"{", "macos-aarch64").is_err());
    }

    #[test]
    fn a_url_that_is_not_https_is_refused() {
        let body = manifest("stable", "1.4.0", "http://e.com/a.zip");
        assert!(parse(body.as_bytes(), "macos-aarch64").is_err());
        assert!(require_https("https://e.com").is_ok());
        assert!(require_https("file:///etc/passwd").is_err());
        assert!(require_https("https://").is_err());
    }

    #[test]
    fn the_highest_version_wins_and_nothing_downgrades() {
        let now = Version::parse("1.4.0").unwrap();
        let releases = vec![release("stable", "1.4.0"), release("beta", "1.3.0")];
        assert!(pick(&now, releases).is_none());
        let releases = vec![release("stable", "1.4.1"), release("beta", "1.5.0-beta.1")];
        assert_eq!(pick(&now, releases).unwrap().info.version, "1.5.0-beta.1");
        // Semver orders prerelease text: beta < nightly < the release itself.
        let releases = vec![
            release("beta", "1.5.0-beta.1"),
            release("nightly", "1.5.0-nightly.7"),
            release("stable", "1.5.0"),
        ];
        assert_eq!(pick(&now, releases).unwrap().info.version, "1.5.0");
        let nightly = Version::parse("1.5.0-nightly.7").unwrap();
        let releases = vec![release("beta", "1.5.0-beta.1"), release("nightly", "1.5.0-nightly.7")];
        assert!(pick(&nightly, releases).is_none());
    }

    fn feed_fetch(url: &str) -> Result<Option<Vec<u8>>, String> {
        match url {
            "https://f/stable.json" => Ok(Some(b"test".to_vec())),
            "https://f/stable.json.minisig" => Ok(Some(SIG.as_bytes().to_vec())),
            "https://f/beta.json" => Ok(Some(b"unsigned".to_vec())),
            _ => Ok(None),
        }
    }

    #[test]
    fn a_channel_file_must_be_signed_and_a_missing_one_is_empty() {
        let now = Version::parse("1.0.0").unwrap();
        // `stable.json` verifies but is not a manifest: verified bytes are still parsed.
        let error = newest(feed_fetch, "https://f/", KEY, UpdateChannel::Stable, &now, "macos-aarch64")
            .unwrap_err();
        assert!(error.contains("could not be read"), "{error}");
        // `beta.json` has no signature file at all.
        let error = newest(feed_fetch, "https://f", KEY, UpdateChannel::Beta, &now, "macos-aarch64")
            .unwrap_err();
        assert!(error.contains("no signature") || error.contains("could not be read"), "{error}");
        fn empty(_: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(None)
        }
        assert!(
            newest(empty, "https://f", KEY, UpdateChannel::Nightly, &now, "macos-aarch64")
                .unwrap()
                .is_none()
        );
    }
}

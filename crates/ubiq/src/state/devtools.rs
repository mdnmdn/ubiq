//! The Tools dialog: which section is up, every field it types into, and what each computed.
//!
//! The fields are `Entity<InputState>`/`Entity<TextareaState>` because the widget's state *is* the
//! model here — the dialog holds nothing a field does not — and they live and die with the dialog:
//! made when it opens, dropped with their subscriptions when it closes. The arithmetic is
//! `crate::devtools`; the outputs below are its answers, recomputed when a field changes rather
//! than on every paint.

use gpui::{Entity, Subscription};
use gpui_component::input::{InputState, TextareaState};
use ubiq_proto::files::FileDiff;

use crate::devtools::codec::B64Alphabet;
use crate::devtools::detect::Kind;
use crate::devtools::jwt::Jwt;
use crate::devtools::lorem::LoremUnit;
use crate::devtools::time::ReadAs;
use crate::devtools::uuids::{Namespace, UuidFormat, UuidVersion};

/// The dialog's sections, in nav order. `Smart` is first and where it opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ToolSection {
    #[default]
    Smart,
    Base64,
    Jwt,
    Time,
    Url,
    Hash,
    Uuid,
    Lorem,
    Diff,
}

impl ToolSection {
    pub const ALL: [ToolSection; 9] = [
        Self::Smart,
        Self::Base64,
        Self::Jwt,
        Self::Time,
        Self::Url,
        Self::Hash,
        Self::Uuid,
        Self::Lorem,
        Self::Diff,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Smart => "Smart",
            Self::Base64 => "Base64",
            Self::Jwt => "JWT",
            Self::Time => "Date & time",
            Self::Url => "URL",
            Self::Hash => "Hash",
            Self::Uuid => "UUID",
            Self::Lorem => "Lorem ipsum",
            Self::Diff => "Diff",
        }
    }

    /// What a suggestion chip calls it: "open in *the JWT decoder*".
    pub fn destination(self) -> &'static str {
        match self {
            Self::Smart => "Smart",
            Self::Base64 => "Base64 decoder",
            Self::Jwt => "JWT decoder",
            Self::Time => "date converter",
            Self::Url => "URL decoder",
            Self::Hash => "hash checker",
            Self::Uuid => "UUID inspector",
            Self::Lorem => "Lorem ipsum",
            Self::Diff => "diff",
        }
    }
}

/// Where a detected kind is worked on, and whether that section should start out decoding it.
pub fn section_for(kind: Kind) -> (ToolSection, bool) {
    match kind {
        Kind::Jwt => (ToolSection::Jwt, true),
        Kind::Uuid => (ToolSection::Uuid, true),
        Kind::HexDigest(_) => (ToolSection::Hash, true),
        Kind::UnixSeconds | Kind::UnixMillis | Kind::Date => (ToolSection::Time, true),
        Kind::EncodedUrl | Kind::UrlEncoded => (ToolSection::Url, true),
        Kind::Url => (ToolSection::Url, false),
        Kind::Base64 => (ToolSection::Base64, true),
        Kind::Json => (ToolSection::Base64, false),
    }
}

/// Whether a paste that matches only this kind may open its section without a click. Only kinds
/// the section *decodes* — JSON and a plain URL would land in an encoder, which is a guess about
/// intent rather than a reading, so they stay chips.
pub fn auto_follows(kind: Kind) -> bool {
    !matches!(kind, Kind::Json | Kind::Url)
}

/// One offer to open a section on a text: from the Smart field, or from the clipboard.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub kind: Kind,
    pub section: ToolSection,
    pub decode: bool,
    pub text: String,
}

impl Suggestion {
    pub fn new(kind: Kind, text: &str) -> Self {
        let (section, decode) = section_for(kind);
        Self {
            kind,
            section,
            decode,
            text: text.trim().to_string(),
        }
    }

    /// The chip's words: "JWT decoder" for a decode, "Base64 encoder" when the text is to be
    /// encoded rather than read.
    pub fn chip(&self) -> String {
        if self.decode {
            return self.section.destination().to_string();
        }
        format!("{} encoder", self.section.label())
    }
}

/// The fields a file can be dropped on or loaded into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileTarget {
    Smart,
    DiffLeft,
    DiffRight,
}

/// Every field the dialog types into.
pub struct DevtoolsInputs {
    pub smart: Entity<TextareaState>,
    pub base64: Entity<TextareaState>,
    pub jwt: Entity<TextareaState>,
    pub time: Entity<InputState>,
    pub url: Entity<TextareaState>,
    pub hash: Entity<TextareaState>,
    /// A digest to compare every computed one against.
    pub hash_expect: Entity<InputState>,
    pub uuid_name: Entity<InputState>,
    pub uuid_inspect: Entity<InputState>,
    pub diff_left: Entity<TextareaState>,
    pub diff_right: Entity<TextareaState>,
}

/// The time section's answer: how the input was read, and every shape of it.
pub type TimeOut = Result<(ReadAs, Vec<(&'static str, String)>), String>;

pub struct DevtoolsState {
    pub section: ToolSection,
    pub inputs: DevtoolsInputs,

    pub b64_decode: bool,
    pub b64_alphabet: B64Alphabet,
    pub b64_out: Result<String, String>,

    pub jwt_out: Option<Result<Jwt, String>>,

    pub time_out: Option<TimeOut>,

    pub url_decode: bool,
    pub url_full: bool,
    pub url_plus: bool,
    pub url_out: Result<String, String>,

    pub hash_out: Vec<(&'static str, String)>,

    pub uuid_version: UuidVersion,
    pub uuid_count: usize,
    pub uuid_format: UuidFormat,
    pub uuid_namespace: Namespace,
    /// Generated values, kept unformatted so a case or hyphen toggle rewrites the same ones.
    pub uuid_out: Vec<::uuid::Uuid>,
    pub uuid_inspect_out: Option<Result<Vec<(&'static str, String)>, String>>,

    pub lorem_unit: LoremUnit,
    pub lorem_count: usize,
    pub lorem_seed: u64,
    pub lorem_out: String,

    /// Side by side, or unified.
    pub diff_split: bool,
    pub diff: Option<FileDiff>,
    /// Bumped per diff request; a background answer lands only if it is still the latest.
    pub diff_gen: u64,

    /// What the Smart field's text looks like, most specific first.
    pub smart_found: Vec<Kind>,
    /// The Smart field's length in characters (not bytes) before the last change — a jump of more than one character is a
    /// paste or a drop, and only that jumps to a section on its own.
    pub smart_len: usize,
    /// What the clipboard looked like when the dialog opened.
    pub clipboard: Option<Suggestion>,
    /// A file that could not be loaded, or was cut short.
    pub note: Option<String>,

    pub subscriptions: Vec<Subscription>,
}

impl DevtoolsState {
    pub fn new(inputs: DevtoolsInputs) -> Self {
        Self {
            section: ToolSection::Smart,
            inputs,
            b64_decode: false,
            b64_alphabet: B64Alphabet::Standard,
            b64_out: Ok(String::new()),
            jwt_out: None,
            time_out: None,
            url_decode: true,
            url_full: false,
            url_plus: false,
            url_out: Ok(String::new()),
            hash_out: Vec::new(),
            uuid_version: UuidVersion::V4,
            uuid_count: 5,
            uuid_format: UuidFormat::default(),
            uuid_namespace: Namespace::Dns,
            uuid_out: Vec::new(),
            uuid_inspect_out: None,
            lorem_unit: LoremUnit::Paragraphs,
            lorem_count: 3,
            lorem_seed: 1,
            lorem_out: String::new(),
            diff_split: true,
            diff: None,
            diff_gen: 0,
            smart_found: Vec::new(),
            smart_len: 0,
            clipboard: None,
            note: None,
            subscriptions: Vec::new(),
        }
    }

    /// The Smart field's offers, one per kind found.
    pub fn smart_suggestions(&self, text: &str) -> Vec<Suggestion> {
        self.smart_found
            .iter()
            .map(|kind| Suggestion::new(*kind, text))
            .collect()
    }
}

/// The most UUIDs one press generates.
pub const UUID_MAX: usize = 100;
/// The most units of placeholder text one press generates.
pub const LOREM_MAX: usize = 50;
/// The most clipboard text the dialog looks at on open; past this it is not a token.
pub const CLIPBOARD_SCAN_MAX: usize = 64 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_home() {
        assert_eq!(section_for(Kind::Jwt), (ToolSection::Jwt, true));
        assert_eq!(section_for(Kind::HexDigest("MD5")).0, ToolSection::Hash);
        assert_eq!(section_for(Kind::UnixMillis).0, ToolSection::Time);
        assert_eq!(section_for(Kind::Json), (ToolSection::Base64, false));
        assert_eq!(Suggestion::new(Kind::Jwt, " x ").text, "x");
        assert_eq!(Suggestion::new(Kind::Jwt, "x").chip(), "JWT decoder");
        assert_eq!(Suggestion::new(Kind::Json, "{}").chip(), "Base64 encoder");
    }

    #[test]
    fn only_decodable_kinds_auto_follow() {
        for kind in [
            Kind::Jwt,
            Kind::Base64,
            Kind::UnixSeconds,
            Kind::UnixMillis,
            Kind::Date,
            Kind::UrlEncoded,
            Kind::EncodedUrl,
            Kind::Uuid,
            Kind::HexDigest("MD5"),
        ] {
            assert!(auto_follows(kind), "{kind:?}");
        }
        assert!(!auto_follows(Kind::Json));
        assert!(!auto_follows(Kind::Url));
    }
}

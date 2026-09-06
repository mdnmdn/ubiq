//! The full-text index: which files could match a query.
//!
//! **It selects candidates. It never produces hits** — see `D75`. Each document is a file's path
//! and its trigrams; nothing else is stored, and the content is not stored at all. A query
//! resolves to a set of paths, and [`crate::search::hits::scan_file`] reads those files back to
//! produce the `LineHit`s the interface draws, exactly as it does for the walk. So a hit is
//! byte-identical whichever path chose the file, and a stale entry costs one wasted read rather
//! than a wrong answer.
//!
//! The tokenizer is trigrams rather than words (`D76`), which is what makes the candidate set a
//! *superset* of what a substring search would find: a word index would never return the file
//! holding `needles` for the query `needl`, and no amount of re-reading recovers a file the index
//! did not name.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tantivy::collector::DocSetCollector;
use tantivy::query::{BooleanQuery, Occur, Query, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions, Value,
};
use tantivy::tokenizer::{LowerCaser, NgramTokenizer, TextAnalyzer};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term};

use super::ceiling;

/// The name the trigram analyser is registered under, in the index's own tokenizer manager.
const TRIGRAM: &str = "tri";

/// The file the stamp lives in, beside the index directory rather than inside it — tantivy owns
/// what is inside, and a foreign file there is a file it may one day garbage-collect.
const STAMP_FILE: &str = "text.stamp";

/// The index directory's name under the project's host-owned workarea.
const DIR: &str = "text";

/// The two fields, resolved once so a lookup is not a string comparison per document.
#[derive(Clone, Copy)]
pub struct Fields {
    /// The project-relative path, forward-slashed. Stored, because it is the whole answer, and
    /// indexed as a single term so it can be the delete key for an incremental update.
    pub path: Field,
    /// The file's trigrams. Indexed, never stored — the content is read back from disk.
    pub body: Field,
}

fn schema() -> (Schema, Fields) {
    let mut builder = Schema::builder();
    let path = builder.add_text_field("path", STRING | STORED);
    let body = builder.add_text_field(
        "body",
        TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(TRIGRAM)
                // `Basic` because no phrase query is ever run: an AND of trigrams needs no
                // positions, and positions are the larger half of a postings list.
                .set_index_option(IndexRecordOption::Basic),
        ),
    );
    (builder.build(), Fields { path, body })
}

/// A read handle on one project's index, cloneable and cheap.
///
/// This is what the search worker is given. It carries no writer, so a search can never block the
/// thread that is indexing, and reading is lock-free and mmap-backed. `ready` is false until a
/// cold build has finished and while a rebuild is in flight — a search that finds it false takes
/// the walk, which is always the answer when the index cannot be one.
#[derive(Clone)]
pub struct Reader {
    reader: IndexReader,
    fields: Fields,
    ready: Arc<AtomicBool>,
}

/// One project's index, open for reading and writing.
pub struct Text {
    index: Index,
    fields: Fields,
    writer: IndexWriter,
    reader: IndexReader,
    ready: Arc<AtomicBool>,
}

impl Text {
    /// Open the index under `workarea`, or create it.
    ///
    /// **Any doubt deletes it.** A missing, unreadable or mismatched stamp, or a directory tantivy
    /// refuses, means the directory is removed and built again from nothing. It is derived data;
    /// repairing it would be a second implementation of building it.
    pub fn open(workarea: &Path) -> tantivy::Result<Self> {
        let dir = workarea.join(DIR);
        if !stamp_matches(workarea) {
            let _ = std::fs::remove_dir_all(&dir);
        }
        std::fs::create_dir_all(&dir)?;

        let (schema, fields) = schema();
        let index = match Index::open_in_dir(&dir) {
            Ok(index) => index,
            Err(_) => {
                // Either it was never there, or it is from a tantivy that wrote a format this one
                // will not read. Both answer the same way.
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(&dir)?;
                Index::create_in_dir(&dir, schema.clone())?
            }
        };
        register_trigrams(&index)?;
        write_stamp(workarea);

        let writer = index.writer_with_num_threads(1, ceiling::WRITER_HEAP)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        Ok(Self {
            index,
            fields,
            writer,
            reader,
            ready: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Add or replace one file's document. Delete-then-add, which is one code path for creating,
    /// modifying and (with `text: None`) deleting.
    pub fn put(&mut self, rel_path: &str, text: Option<&str>) {
        self.writer
            .delete_term(Term::from_field_text(self.fields.path, rel_path));
        if let Some(text) = text {
            let mut doc = TantivyDocument::default();
            doc.add_text(self.fields.path, rel_path);
            doc.add_text(self.fields.body, text);
            let _ = self.writer.add_document(doc);
        }
    }

    /// Throw the whole index away, in place. What a truncated watcher burst falls back to.
    pub fn clear(&mut self) -> tantivy::Result<()> {
        self.writer.delete_all_documents()?;
        self.commit()
    }

    /// Make everything written since the last commit visible to readers.
    ///
    /// The reload is explicit rather than left to [`ReloadPolicy::OnCommitWithDelay`]: the policy
    /// refreshes on its own schedule, so a caller that commits and immediately asks a question
    /// would be answered from the searcher it had before it wrote. Committing is already the
    /// expensive part; the reload beside it is not what makes it so.
    pub fn commit(&mut self) -> tantivy::Result<()> {
        self.writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    /// How many documents the index holds. For tests and for a log line, never for a decision.
    pub fn len(&self) -> usize {
        self.reader.searcher().num_docs() as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A read handle, so a searcher can be given one without the writer coming along.
    pub fn reader(&self) -> Reader {
        Reader {
            reader: self.reader.clone(),
            fields: self.fields,
            ready: self.ready.clone(),
        }
    }

    /// Say whether this index may be consulted. Set true when a cold build finishes, false while
    /// one is in flight — a search seeing false walks instead, and sees no error.
    pub fn set_ready(&self, ready: bool) {
        self.ready.store(ready, Ordering::Relaxed);
    }

    pub fn index(&self) -> &Index {
        &self.index
    }

    /// Ask this index directly, ignoring `ready`. The search path goes through [`Reader`]; this is
    /// for the builder itself and for tests, which have just written what they are asking about.
    pub fn candidates(&self, needle: &str) -> tantivy::Result<Option<(Vec<String>, bool)>> {
        Reader {
            reader: self.reader.clone(),
            fields: self.fields,
            ready: Arc::new(AtomicBool::new(true)),
        }
        .candidates(needle)
    }
}

impl Reader {
    /// Whether the index is built and may be consulted.
    pub fn ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    /// The files that could contain `needle`, as project-relative paths.
    ///
    /// `Ok(None)` means the index cannot answer this query and the caller must walk — a query too
    /// short to have a trigram. It is not an error, and it is not an empty result: an empty `Vec`
    /// means the index looked and found nothing, which is a real answer.
    ///
    /// The set is a superset of the files that actually match: trigrams are case-folded and
    /// unordered, so `abc` and `cba` share none but `abcd` matches the query `bcd` and also the
    /// query `abd` spuriously. Every candidate is re-read and re-matched, which is what makes the
    /// looseness free.
    pub fn candidates(&self, needle: &str) -> tantivy::Result<Option<(Vec<String>, bool)>> {
        let lowered = needle.to_lowercase();
        let grams = trigrams(&lowered);
        if grams.is_empty() {
            return Ok(None);
        }

        let clauses: Vec<(Occur, Box<dyn Query>)> = grams
            .into_iter()
            .map(|gram| {
                let term = Term::from_field_text(self.fields.body, &gram);
                let query: Box<dyn Query> =
                    Box::new(TermQuery::new(term, IndexRecordOption::Basic));
                (Occur::Must, query)
            })
            .collect();
        let query = BooleanQuery::new(clauses);

        let searcher = self.reader.searcher();
        // The whole matching set, not a ranked top-N: the body is indexed without positions or
        // frequencies, so there is no score to rank by and a "top" would be an arbitrary order
        // dressed up as a relevant one. Sorting by path instead makes the truncation deterministic
        // — the same query cuts the same files every time.
        let found = searcher.search(&query, &DocSetCollector)?;

        let mut paths = Vec::with_capacity(found.len());
        for address in found {
            let doc: TantivyDocument = searcher.doc(address)?;
            if let Some(path) = doc
                .get_first(self.fields.path)
                .and_then(|value| value.as_str())
            {
                paths.push(path.to_string());
            }
        }
        paths.sort();
        let truncated = paths.len() > ceiling::CANDIDATES;
        paths.truncate(ceiling::CANDIDATES);
        Ok(Some((paths, truncated)))
    }
}

fn register_trigrams(index: &Index) -> tantivy::Result<()> {
    // `all_ngrams` rather than prefix-only: a query's trigrams come from the middle of words as
    // often as the start, which is the whole point of not using a word tokenizer.
    let ngram = NgramTokenizer::all_ngrams(3, 3)?;
    let analyzer = TextAnalyzer::builder(ngram).filter(LowerCaser).build();
    index.tokenizers().register(TRIGRAM, analyzer);
    Ok(())
}

/// The distinct trigrams of a string, in order, without repeats.
///
/// Over `chars`, not bytes: a byte window would cut a multi-byte character in half and produce
/// terms the tokenizer never emitted, which silently answers "no candidates" for any query
/// containing a non-ASCII character.
fn trigrams(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() < ceiling::MIN_QUERY_CHARS {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    for window in chars.windows(3) {
        let gram: String = window.iter().collect();
        if !out.contains(&gram) {
            out.push(gram);
        }
    }
    out
}

/// Whether a file is worth indexing: small enough, and not binary.
///
/// The size test is the cheap one and comes first. The binary test reads only the head, and looks
/// for a NUL — the same signal `grep-searcher` uses, so the index and the read agree about what a
/// text file is.
pub fn indexable(path: &Path, len: u64) -> bool {
    if len > ceiling::MAX_FILE_BYTES {
        return false;
    }
    match std::fs::File::open(path) {
        Ok(mut file) => {
            use std::io::Read;
            let mut head = vec![0u8; ceiling::BINARY_SNIFF_BYTES.min(len as usize).max(1)];
            match file.read(&mut head) {
                Ok(read) => !head[..read].contains(&0),
                Err(_) => false,
            }
        }
        Err(_) => false,
    }
}

fn stamp_path(workarea: &Path) -> PathBuf {
    workarea.join(STAMP_FILE)
}

fn stamp_matches(workarea: &Path) -> bool {
    std::fs::read_to_string(stamp_path(workarea))
        .ok()
        .and_then(|text| text.trim().parse::<u32>().ok())
        .is_some_and(|found| found == ceiling::STAMP)
}

fn write_stamp(workarea: &Path) {
    // Best effort: a stamp that failed to write means the next open rebuilds, which is the safe
    // direction and costs a walk rather than a wrong answer.
    let _ =
        crate::atomic::write_atomic(&stamp_path(workarea), ceiling::STAMP.to_string().as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn built(files: &[(&str, &str)]) -> (TempDir, Text) {
        let dir = TempDir::new().unwrap();
        let mut text = Text::open(dir.path()).unwrap();
        for (path, body) in files {
            text.put(path, Some(body));
        }
        text.commit().unwrap();
        (dir, text)
    }

    #[test]
    fn a_candidate_is_found_mid_word() {
        let (_dir, text) = built(&[
            ("a.rs", "let needles = 1;"),
            ("b.rs", "fn needle() {}"),
            ("c.rs", "nothing here at all"),
        ]);

        // The whole reason for trigrams: `needl` must return the file holding `needles`, which a
        // word tokenizer would never do.
        let (paths, truncated) = text.candidates("needl").unwrap().unwrap();
        assert!(!truncated);
        let mut paths = paths;
        paths.sort();
        assert_eq!(paths, vec!["a.rs".to_string(), "b.rs".to_string()]);
    }

    #[test]
    fn case_is_folded_because_the_read_re_checks_it() {
        let (_dir, text) = built(&[("a.rs", "let NEEDLE = 1;")]);
        let (paths, _) = text.candidates("needle").unwrap().unwrap();
        assert_eq!(paths, vec!["a.rs".to_string()]);
    }

    #[test]
    fn a_query_under_three_characters_has_no_answer_and_says_so() {
        let (_dir, text) = built(&[("a.rs", "ab")]);
        // `None` is "walk it", which is not the same as an empty candidate set.
        assert!(text.candidates("ab").unwrap().is_none());
        assert!(text.candidates("").unwrap().is_none());
        assert!(text.candidates("abc").unwrap().is_some());
    }

    #[test]
    fn a_query_that_matches_nothing_answers_an_empty_set() {
        let (_dir, text) = built(&[("a.rs", "let needle = 1;")]);
        let (paths, _) = text.candidates("zzzqqq").unwrap().unwrap();
        assert!(paths.is_empty());
    }

    #[test]
    fn re_putting_a_path_replaces_it_rather_than_duplicating_it() {
        let (_dir, mut text) = built(&[("a.rs", "let needle = 1;")]);
        assert_eq!(text.len(), 1);

        text.put("a.rs", Some("let haystack = 2;"));
        text.commit().unwrap();
        assert_eq!(text.len(), 1);

        assert!(
            text.candidates("needle").unwrap().unwrap().0.is_empty(),
            "the old body should be gone"
        );
        assert_eq!(
            text.candidates("haystack").unwrap().unwrap().0,
            vec!["a.rs".to_string()]
        );
    }

    #[test]
    fn putting_none_deletes_the_file() {
        let (_dir, mut text) = built(&[("a.rs", "let needle = 1;")]);
        text.put("a.rs", None);
        text.commit().unwrap();
        assert_eq!(text.len(), 0);
        assert!(text.candidates("needle").unwrap().unwrap().0.is_empty());
    }

    #[test]
    fn a_wrong_stamp_throws_the_directory_away() {
        let dir = TempDir::new().unwrap();
        {
            let mut text = Text::open(dir.path()).unwrap();
            text.put("a.rs", Some("let needle = 1;"));
            text.commit().unwrap();
            assert_eq!(text.len(), 1);
        }
        // Re-opening on the same stamp keeps what was built.
        {
            let text = Text::open(dir.path()).unwrap();
            assert_eq!(text.len(), 1);
        }
        // A stamp from another build is not migrated, it is deleted.
        std::fs::write(dir.path().join(STAMP_FILE), "99999").unwrap();
        let text = Text::open(dir.path()).unwrap();
        assert_eq!(text.len(), 0);
    }

    #[test]
    fn a_missing_stamp_is_the_same_as_a_wrong_one() {
        let dir = TempDir::new().unwrap();
        {
            let mut text = Text::open(dir.path()).unwrap();
            text.put("a.rs", Some("let needle = 1;"));
            text.commit().unwrap();
        }
        std::fs::remove_file(dir.path().join(STAMP_FILE)).unwrap();
        let text = Text::open(dir.path()).unwrap();
        assert_eq!(text.len(), 0);
    }

    #[test]
    fn trigrams_are_over_characters_not_bytes() {
        // Three characters, six bytes: a byte window would emit terms the tokenizer never did.
        assert_eq!(trigrams("héllo").len(), 3);
        assert_eq!(trigrams("ab"), Vec::<String>::new());
        assert_eq!(trigrams("abcd"), vec!["abc".to_string(), "bcd".to_string()]);
        // Repeats collapse: `aaaa` is one distinct trigram, not two clauses of the same term.
        assert_eq!(trigrams("aaaa"), vec!["aaa".to_string()]);
    }

    #[test]
    fn a_non_ascii_query_finds_its_file() {
        let (_dir, text) = built(&[("a.rs", "let café = 1;")]);
        let (paths, _) = text.candidates("café").unwrap().unwrap();
        assert_eq!(paths, vec!["a.rs".to_string()]);
    }

    #[test]
    fn a_binary_or_oversized_file_is_not_indexable() {
        let dir = TempDir::new().unwrap();
        let text_file = dir.path().join("a.rs");
        std::fs::write(&text_file, "fn main() {}").unwrap();
        assert!(indexable(&text_file, 12));
        // Above the ceiling, whatever it contains.
        assert!(!indexable(&text_file, ceiling::MAX_FILE_BYTES + 1));

        let binary = dir.path().join("b.bin");
        std::fs::write(&binary, [0x7f, 0x45, 0x4c, 0x00, 0x01]).unwrap();
        assert!(!indexable(&binary, 5));

        assert!(!indexable(&dir.path().join("missing"), 1));
    }
}

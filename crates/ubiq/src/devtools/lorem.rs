//! Placeholder text: the classic opening, then words drawn from the usual pool.
//!
//! Deterministic for a seed, so the section redraws the same text until the user asks for another.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoremUnit {
    #[default]
    Paragraphs,
    Sentences,
    Words,
}

impl LoremUnit {
    pub const ALL: [LoremUnit; 3] = [Self::Paragraphs, Self::Sentences, Self::Words];

    pub fn label(self) -> &'static str {
        match self {
            Self::Paragraphs => "Paragraphs",
            Self::Sentences => "Sentences",
            Self::Words => "Words",
        }
    }
}

const OPENING: &[&str] = &[
    "lorem",
    "ipsum",
    "dolor",
    "sit",
    "amet",
    "consectetur",
    "adipiscing",
    "elit",
];

const POOL: &[&str] = &[
    "sed",
    "do",
    "eiusmod",
    "tempor",
    "incididunt",
    "ut",
    "labore",
    "et",
    "dolore",
    "magna",
    "aliqua",
    "enim",
    "ad",
    "minim",
    "veniam",
    "quis",
    "nostrud",
    "exercitation",
    "ullamco",
    "laboris",
    "nisi",
    "aliquip",
    "ex",
    "ea",
    "commodo",
    "consequat",
    "duis",
    "aute",
    "irure",
    "in",
    "reprehenderit",
    "voluptate",
    "velit",
    "esse",
    "cillum",
    "fugiat",
    "nulla",
    "pariatur",
    "excepteur",
    "sint",
    "occaecat",
    "cupidatat",
    "non",
    "proident",
    "sunt",
    "culpa",
    "qui",
    "officia",
    "deserunt",
    "mollit",
    "anim",
    "id",
    "est",
    "laborum",
    "lorem",
    "ipsum",
    "dolor",
    "amet",
    "elit",
    "porta",
    "nibh",
    "venenatis",
    "cras",
    "pulvinar",
    "mattis",
    "nunc",
    "vitae",
    "turpis",
    "massa",
    "tincidunt",
    "integer",
    "feugiat",
];

/// A small xorshift: enough to vary word choice, and no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn range(&mut self, low: usize, high: usize) -> usize {
        low + (self.next() % (high - low + 1) as u64) as usize
    }
}

/// An endless stream of words, the opening first.
struct Words {
    rng: Rng,
    emitted: usize,
}

impl Words {
    fn new(seed: u64) -> Self {
        Self {
            // Spread the seed so neighbouring seeds start far apart; xorshift needs a non-zero state.
            rng: Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1),
            emitted: 0,
        }
    }

    fn word(&mut self) -> &'static str {
        let word = OPENING
            .get(self.emitted)
            .copied()
            .unwrap_or_else(|| POOL[self.rng.next() as usize % POOL.len()]);
        self.emitted += 1;
        word
    }

    fn sentence(&mut self) -> String {
        let len = if self.emitted == 0 {
            OPENING.len()
        } else {
            self.rng.range(6, 14)
        };
        let comma = (len > 8).then(|| self.rng.range(3, len - 3));
        let mut out = String::new();
        for i in 0..len {
            let word = self.word();
            if i == 0 {
                let mut chars = word.chars();
                if let Some(first) = chars.next() {
                    out.extend(first.to_uppercase());
                    out.push_str(chars.as_str());
                }
            } else {
                out.push(' ');
                out.push_str(word);
            }
            if Some(i) == comma || (self.emitted == 5 && i == 4) {
                out.push(',');
            }
        }
        out.push('.');
        out
    }

    fn paragraph(&mut self) -> String {
        let count = self.rng.range(4, 7);
        (0..count)
            .map(|_| self.sentence())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// `count` units of placeholder text. Paragraphs are separated by a blank line.
pub fn generate(unit: LoremUnit, count: usize, seed: u64) -> String {
    let mut words = Words::new(seed);
    let count = count.max(1);
    match unit {
        LoremUnit::Words => (0..count)
            .map(|_| words.word())
            .collect::<Vec<_>>()
            .join(" "),
        LoremUnit::Sentences => (0..count)
            .map(|_| words.sentence())
            .collect::<Vec<_>>()
            .join(" "),
        LoremUnit::Paragraphs => (0..count)
            .map(|_| words.paragraph())
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_with_the_classic() {
        assert!(
            generate(LoremUnit::Sentences, 1, 7)
                .starts_with("Lorem ipsum dolor sit amet, consectetur adipiscing elit.")
        );
        assert_eq!(generate(LoremUnit::Words, 3, 7), "lorem ipsum dolor");
    }

    #[test]
    fn counts_units() {
        assert_eq!(generate(LoremUnit::Words, 20, 1).split(' ').count(), 20);
        assert_eq!(
            generate(LoremUnit::Paragraphs, 3, 1).split("\n\n").count(),
            3
        );
        assert_eq!(generate(LoremUnit::Sentences, 4, 1).matches('.').count(), 4);
    }

    #[test]
    fn same_seed_same_text() {
        assert_eq!(
            generate(LoremUnit::Paragraphs, 2, 42),
            generate(LoremUnit::Paragraphs, 2, 42)
        );
        assert_ne!(
            generate(LoremUnit::Paragraphs, 2, 42),
            generate(LoremUnit::Paragraphs, 2, 43)
        );
    }
}

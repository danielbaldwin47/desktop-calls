//! Which letters of a name matched what was typed, for the Dialer to
//! underline: found the way the name's match was ranked.

use std::ops::Range;

use super::{Tier, Typed, fold_char};
use unicode_normalization::char::is_combining_mark;

/// Where each of `typed` starts one of `words`, in order: the index of the
/// word each starts.
pub(super) fn starts_in_order(typed: &[String], words: &[String]) -> Option<Vec<usize>> {
    let mut next = 0;
    let mut found = Vec::with_capacity(typed.len());
    for part in typed {
        let at = next
            + words
                .get(next..)?
                .iter()
                .position(|word| word.starts_with(part.as_str()))?;
        found.push(at);
        next = at + 1;
    }
    Some(found)
}

/// Whether each of `typed` starts a different one of `words`, in any order:
/// the index of the word each starts.
pub(super) fn starts_any_order(typed: &[String], words: &[String]) -> Option<Vec<usize>> {
    let mut used = vec![false; words.len()];
    let mut found = Vec::with_capacity(typed.len());
    for part in typed {
        let at = words
            .iter()
            .enumerate()
            .position(|(index, word)| !used[index] && word.starts_with(part.as_str()))?;
        used[at] = true;
        found.push(at);
    }
    Some(found)
}

/// The byte ranges of `name` whose letters matched `typed`, in order, for a
/// name whose match is of `tier`.
pub(super) fn matched(typed: &Typed, name: &str, tier: Tier) -> Vec<Range<usize>> {
    let letters = Letters::new(name);
    let words: Vec<String> = letters
        .words
        .iter()
        .map(|word| word.iter().map(|(letter, _)| *letter).collect())
        .collect();
    // Each matched letter as (word, letter in the word).
    let mut found: Vec<(usize, usize)> = Vec::new();
    let mut starts = |at: Vec<usize>| {
        for (part, word) in typed.words.iter().zip(at) {
            found.extend((0..part.chars().count()).map(|letter| (word, letter)));
        }
    };
    match tier {
        Tier::Whole | Tier::Start => {
            if let Some(at) = starts_in_order(&typed.words, &words) {
                starts(at);
            }
        }
        Tier::WordsAnyOrder => {
            if let Some(at) = starts_any_order(&typed.words, &words) {
                starts(at);
            }
        }
        Tier::Initials => {
            let typed_letters = typed.compact.chars().count();
            found.extend((0..typed_letters.min(words.len())).map(|word| (word, 0)));
        }
        Tier::Inside => {
            // The words joined by single spaces, as search compares them,
            // each character with the letter it is, if not a space.
            let mut joined = String::new();
            let mut at = Vec::new();
            for (index, word) in words.iter().enumerate() {
                if index > 0 {
                    joined.push(' ');
                    at.push(None);
                }
                joined.push_str(word);
                at.extend((0..word.chars().count()).map(|letter| Some((index, letter))));
            }
            for part in &typed.words {
                if let Some(start) = joined.find(part.as_str()) {
                    let start = joined[..start].chars().count();
                    let end = start + part.chars().count();
                    found.extend(at[start..end].iter().flatten());
                }
            }
        }
        Tier::Fuzzy => {
            // The typed letters in order, each where it first comes, as
            // search ranks it.
            let mut left = typed.compact.chars().peekable();
            for (word, letters) in words.iter().enumerate() {
                for (letter, character) in letters.chars().enumerate() {
                    if left.peek() == Some(&character) {
                        left.next();
                        found.push((word, letter));
                    }
                }
            }
        }
    }
    letters.ranges(&found)
}

/// A name's words as search folds them, each letter with the bytes of the
/// name's character it came from.
struct Letters {
    words: Vec<Vec<(char, Range<usize>)>>,
}

impl Letters {
    fn new(name: &str) -> Self {
        let mut words: Vec<Vec<(char, Range<usize>)>> = vec![Vec::new()];
        for (at, character) in name.char_indices() {
            let source = at..at + character.len_utf8();
            // A combining mark written on its own belongs to the letter
            // before it, so "Jose\u{301}" underlines its accent too.
            if is_combining_mark(character) {
                if let Some(last) = words.last_mut().and_then(|word| word.last_mut()) {
                    last.1.end = source.end;
                }
                continue;
            }
            fold_char(character, |letter| {
                if letter.is_whitespace() {
                    if words.last().is_some_and(|word| !word.is_empty()) {
                        words.push(Vec::new());
                    }
                } else if let Some(word) = words.last_mut() {
                    word.push((letter, source.clone()));
                }
            });
        }
        words.retain(|word| !word.is_empty());
        Self { words }
    }

    /// The name's bytes the letters at `found` came from, in order, those
    /// that touch joined.
    fn ranges(&self, found: &[(usize, usize)]) -> Vec<Range<usize>> {
        let mut sources: Vec<Range<usize>> = found
            .iter()
            .filter_map(|&(word, letter)| Some(self.words.get(word)?.get(letter)?.1.clone()))
            .collect();
        sources.sort_by_key(|source| source.start);
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for source in sources {
            match ranges.last_mut() {
                Some(last) if source.start <= last.end => last.end = last.end.max(source.end),
                _ => ranges.push(source),
            }
        }
        ranges
    }
}

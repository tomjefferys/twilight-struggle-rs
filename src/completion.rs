//! Tab completion for the REPL's line editor (`main.rs`'s `rustyline`
//! integration — see that module's own doc for why). [`candidates`] is
//! the pure matching logic — line and cursor position in, a replacement
//! start offset and the matching strings out — kept separate from
//! [`TsHelper`]'s `rustyline::completion::Completer` impl so it can be
//! unit-tested with no terminal, no [`CardCatalog`], and no file I/O:
//! `TsHelper` only adds the one impure step (re-reading
//! [`StateLibrary::list`] on every keystroke, so a state saved moments
//! ago completes right away).
//!
//! What completes depends on the command word already typed: the first
//! word always completes against [`crate::COMMANDS`]; `load`/`save`
//! complete a `file/name` state reference (`load` also offers `demo`);
//! `play`/`card`/`discard`/`exile` (and, after its `us`/`ussr` side
//! argument, `give`) complete a card name against the *rest* of the
//! line, not just the current word, since a card name is routinely
//! several words ("Duck and Cover"); `country`/`place`/`roll` do the same
//! against country names. Anything else offers no candidates.

use rustyline::completion::Completer;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Context, Helper};

use twilight_struggle::StateLibrary;

/// Byte-offset `(start, end)` ranges of every whitespace-delimited token
/// in `text`.
fn tokenize(text: &str) -> Vec<(usize, usize)> {
    let mut tokens = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some(s) = start.take() {
                tokens.push((s, i));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        tokens.push((s, text.len()));
    }
    tokens
}

fn filter_prefix<'a>(options: impl IntoIterator<Item = &'a str>, current: &str) -> Vec<String> {
    let current_lower = current.to_lowercase();
    options.into_iter().filter(|o| o.to_lowercase().starts_with(&current_lower)).map(str::to_string).collect()
}

/// The pure matching logic — see this module's own doc for the per-
/// command rules. Returns the byte offset in `line` the match starts
/// at (what a selected candidate replaces up to `pos`) and every
/// candidate that currently matches.
pub fn candidates(line: &str, pos: usize, commands: &[String], cards: &[String], countries: &[String], state_refs: &[String]) -> (usize, Vec<String>) {
    let prefix = &line[..pos];
    let tokens = tokenize(prefix);
    let ends_with_space = prefix.chars().last().is_none_or(char::is_whitespace);
    let (word_index, current_start) = if ends_with_space || tokens.is_empty() {
        (tokens.len(), pos)
    } else {
        (tokens.len() - 1, tokens[tokens.len() - 1].0)
    };

    if word_index == 0 {
        let current = &prefix[current_start..pos];
        return (current_start, filter_prefix(commands.iter().map(String::as_str), current));
    }

    let cmd = prefix[tokens[0].0..tokens[0].1].to_lowercase();
    match cmd.as_str() {
        "load" | "save" if word_index == 1 => {
            let current = &prefix[current_start..pos];
            let demo = (cmd == "load").then_some("demo");
            let options = demo.into_iter().chain(state_refs.iter().map(String::as_str));
            (current_start, filter_prefix(options, current))
        }
        "play" | "card" | "discard" | "exile" => {
            let arg_start = tokens.get(1).map_or(current_start, |t| t.0);
            (arg_start, filter_prefix(cards.iter().map(String::as_str), &prefix[arg_start..pos]))
        }
        "give" => {
            if word_index == 1 {
                let current = &prefix[current_start..pos];
                (current_start, filter_prefix(["us", "ussr"], current))
            } else {
                let arg_start = tokens.get(2).map_or(current_start, |t| t.0);
                (arg_start, filter_prefix(cards.iter().map(String::as_str), &prefix[arg_start..pos]))
            }
        }
        "country" | "place" | "roll" | "take" | "+" | "-" => {
            let arg_start = tokens.get(1).map_or(current_start, |t| t.0);
            (arg_start, filter_prefix(countries.iter().map(String::as_str), &prefix[arg_start..pos]))
        }
        _ => (pos, Vec::new()),
    }
}

/// The `rustyline` glue: owns the fixed candidate lists (commands, card
/// names, country names — computed once at startup) plus the
/// [`StateLibrary`] re-read fresh on every completion (see this module's
/// own doc). Every trait but [`Completer`] uses its default (no-op) impl
/// — `rustyline`'s `derive` feature would generate these, but it also
/// pulls in `rustyline-derive` for one tiny macro, so they're spelled out
/// by hand instead.
pub struct TsHelper {
    pub commands: Vec<String>,
    pub cards: Vec<String>,
    pub countries: Vec<String>,
    pub states: StateLibrary,
}

impl Completer for TsHelper {
    type Candidate = String;

    fn complete(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> rustyline::Result<(usize, Vec<String>)> {
        let state_refs: Vec<String> = self.states.list().map(|entries| entries.iter().map(|e| e.reference()).collect()).unwrap_or_default();
        Ok(candidates(line, pos, &self.commands, &self.cards, &self.countries, &state_refs))
    }
}

impl Hinter for TsHelper {
    type Hint = String;
}

impl Highlighter for TsHelper {}

impl Validator for TsHelper {}

impl Helper for TsHelper {}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands() -> Vec<String> {
        ["play", "card", "country", "load", "save", "give", "place"].iter().map(|s| s.to_string()).collect()
    }
    fn cards() -> Vec<String> {
        ["Duck and Cover", "Fidel", "Five Year Plan"].iter().map(|s| s.to_string()).collect()
    }
    fn countries() -> Vec<String> {
        ["Poland", "Portugal", "East Germany"].iter().map(|s| s.to_string()).collect()
    }
    fn states() -> Vec<String> {
        ["scoring/europe-ussr-control-wins", "scoring/empty-region"].iter().map(|s| s.to_string()).collect()
    }

    fn complete(line: &str) -> (usize, Vec<String>) {
        candidates(line, line.len(), &commands(), &cards(), &countries(), &states())
    }

    #[test]
    fn completes_the_command_word_itself() {
        let (start, matches) = complete("pl");
        assert_eq!(start, 0);
        assert_eq!(matches, vec!["play", "place"]);
    }

    #[test]
    fn completes_a_multi_word_card_name() {
        let (start, matches) = complete("play duck and");
        assert_eq!(start, 5);
        assert_eq!(matches, vec!["Duck and Cover"]);
    }

    #[test]
    fn completes_an_empty_card_argument_to_every_card() {
        let (start, matches) = complete("card ");
        assert_eq!(start, 5);
        assert_eq!(matches.len(), 3);
    }

    #[test]
    fn completes_a_country_name_for_place() {
        let (start, matches) = complete("place por");
        assert_eq!(start, 6);
        assert_eq!(matches, vec!["Portugal"]);
    }

    #[test]
    fn give_completes_the_side_first() {
        let (start, matches) = complete("give u");
        assert_eq!(start, 5);
        assert_eq!(matches, vec!["us", "ussr"]);
    }

    #[test]
    fn give_completes_the_card_after_the_side() {
        let (start, matches) = complete("give us fid");
        assert_eq!(start, 8);
        assert_eq!(matches, vec!["Fidel"]);
    }

    #[test]
    fn load_offers_demo_and_state_references() {
        let (start, matches) = complete("load sc");
        assert_eq!(start, 5);
        assert_eq!(matches, vec!["scoring/europe-ussr-control-wins", "scoring/empty-region"]);
    }

    #[test]
    fn save_does_not_offer_demo() {
        let (_, matches) = complete("save de");
        assert!(matches.is_empty());
    }

    #[test]
    fn load_does_not_complete_a_second_argument() {
        let (_, matches) = complete("load demo extra");
        assert!(matches.is_empty());
    }

    #[test]
    fn an_unrecognised_command_offers_nothing_for_its_argument() {
        let (_, matches) = complete("status fo");
        assert!(matches.is_empty());
    }
}

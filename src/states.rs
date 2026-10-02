//! The named test-state library (`data/states/<topic>.json`): small,
//! hand- or debug-mode-authored game snapshots used to exercise a card's
//! event (or an ops action) without re-creating the board by hand every
//! time. Deliberately a different shape from an in-progress game's own
//! save (not built yet, and likely its own module when it is): a test
//! state is a bare [`Scenario`] — status, board, hands, discard, and
//! removed piles — with **no log**, since it's a starting point to jump
//! *to*, not a sequence of moves to resume. Read from disk at runtime
//! under `data/states/`, not `include_str!`-embedded like every other
//! data file in the crate (see `CLAUDE.md`'s own data-files section) —
//! the one deliberate exception, made so a state just `save`d from the
//! REPL is loadable by name immediately, with no rebuild.
//!
//! A state is referred to as `"<file>/<name>"` (e.g.
//! `"scoring/europe-ussr-control-wins"`) — `file` names one JSON document
//! under the library's directory (`<file>.json`, holding a
//! `{"states": [...]}` array so several related states share one file
//! rather than scattering one-state-per-file across the directory),
//! `name` one entry inside it. Convention: a new card implementation adds
//! its own named states here (and matching cases in `tests/states.rs`),
//! the same way it's expected to add a snapshot test.

use std::fmt;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cards::CardCatalog;
use crate::map::WorldMap;
use crate::scenario::{RawScenario, Scenario, ScenarioError};

/// Where [`StateLibrary::standard`] looks — a *path*, fixed at compile
/// time, not the files' own contents (unlike every other data file) —
/// see this module's own doc for why.
const STANDARD_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/data/states");

#[derive(Debug)]
pub enum StateError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Scenario(ScenarioError),
    /// A reference wasn't shaped like `file/name` — e.g. no `/` at all,
    /// or an empty half.
    BadReference(String),
    UnknownFile(String),
    UnknownState { file: String, name: String },
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateError::Io(e) => write!(f, "{e}"),
            StateError::Json(e) => write!(f, "invalid state JSON: {e}"),
            StateError::Scenario(e) => write!(f, "{e}"),
            StateError::BadReference(s) => write!(f, "{s:?} isn't shaped like file/name"),
            StateError::UnknownFile(file) => write!(f, "no state file named {file:?} (try `states` to list what's there)"),
            StateError::UnknownState { file, name } => write!(f, "{file} has no state named {name:?}"),
        }
    }
}

impl std::error::Error for StateError {}

impl From<std::io::Error> for StateError {
    fn from(e: std::io::Error) -> Self {
        StateError::Io(e)
    }
}

impl From<serde_json::Error> for StateError {
    fn from(e: serde_json::Error) -> Self {
        StateError::Json(e)
    }
}

impl From<ScenarioError> for StateError {
    fn from(e: ScenarioError) -> Self {
        StateError::Scenario(e)
    }
}

/// One named state's own record inside a `<file>.json` document — a
/// [`RawScenario`] (flattened, so its fields sit alongside `name`/
/// `description` rather than nested under a `scenario` key) plus the two
/// fields that only make sense for a *named* entry in a shared file.
#[derive(Debug, Serialize, Deserialize)]
struct StateRecord {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(flatten)]
    scenario: RawScenario,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct StateFile {
    states: Vec<StateRecord>,
}

/// One entry in [`StateLibrary::list`] — a state's reference and its own
/// description, with no board/hands actually loaded yet (that's
/// [`StateLibrary::load`]'s job, once something picks one). Cheap enough
/// to rebuild the whole list fresh on every call — see this module's own
/// doc for why that's the point, not a shortcut.
#[derive(Debug, Clone)]
pub struct StateEntry {
    pub file: String,
    pub name: String,
    pub description: String,
}

impl StateEntry {
    pub fn reference(&self) -> String {
        format!("{}/{}", self.file, self.name)
    }
}

/// The directory of named test states — see this module's own doc for
/// the file format and why it's read from disk rather than embedded.
pub struct StateLibrary {
    dir: PathBuf,
}

impl StateLibrary {
    /// The bundled library at `data/states/`, read from the source tree
    /// (via `CARGO_MANIFEST_DIR`) rather than wherever the binary
    /// happens to run from — so `cargo run` finds it regardless of the
    /// current directory, the same way every `include_str!`ed data file
    /// already does, just resolved at runtime instead of compile time.
    pub fn standard() -> Self {
        Self::new(STANDARD_DIR)
    }

    /// A library rooted at any directory — what `tests/states.rs` uses
    /// to exercise `save`/`load` against a throwaway temp directory
    /// without touching the real one.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        StateLibrary { dir: dir.into() }
    }

    fn file_path(&self, file: &str) -> PathBuf {
        self.dir.join(format!("{file}.json"))
    }

    fn read_file(&self, file: &str) -> Result<StateFile, StateError> {
        let text = match fs::read_to_string(self.file_path(file)) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(StateError::UnknownFile(file.to_string())),
            Err(e) => return Err(e.into()),
        };
        Ok(serde_json::from_str(&text)?)
    }

    fn split_reference(reference: &str) -> Result<(&str, &str), StateError> {
        reference
            .split_once('/')
            .filter(|(file, name)| !file.is_empty() && !name.is_empty())
            .ok_or_else(|| StateError::BadReference(reference.to_string()))
    }

    /// Every state across every `<file>.json` in the library, read fresh
    /// from disk — so a state `save`d earlier in this same session shows
    /// up immediately, and a file hand-edited outside the game is picked
    /// up with no restart needed. Files are walked in name order, states
    /// within a file in the order they're stored. Anything in the
    /// directory that isn't a `.json` file is ignored; a `.json` file
    /// that fails to parse is skipped here (rather than failing the
    /// whole listing) — `load`ing it directly still surfaces the real
    /// error.
    pub fn list(&self) -> Result<Vec<StateEntry>, StateError> {
        let mut entries = Vec::new();
        let read_dir = match fs::read_dir(&self.dir) {
            Ok(read_dir) => read_dir,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(entries),
            Err(e) => return Err(e.into()),
        };
        let mut files = Vec::new();
        for entry in read_dir {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                files.push(stem.to_string());
            }
        }
        files.sort();
        for file in files {
            let Ok(state_file) = self.read_file(&file) else { continue };
            for record in state_file.states {
                entries.push(StateEntry { file: file.clone(), name: record.name, description: record.description });
            }
        }
        Ok(entries)
    }

    /// Loads one named state, resolving its country and card *names*
    /// against `map`/`cards` into a real [`Scenario`] — the same
    /// resolution [`Scenario::from_json`] does, via
    /// [`Scenario::from_raw`]. Returns the state's own description
    /// alongside it, for the caller to print.
    pub fn load(&self, map: &WorldMap, cards: &CardCatalog, reference: &str) -> Result<(Scenario, String), StateError> {
        let (file, name) = Self::split_reference(reference)?;
        let state_file = self.read_file(file)?;
        let record = state_file
            .states
            .into_iter()
            .find(|r| r.name == name)
            .ok_or_else(|| StateError::UnknownState { file: file.to_string(), name: name.to_string() })?;
        let scenario = Scenario::from_raw(map, cards, record.scenario)?;
        Ok((scenario, record.description))
    }

    /// Saves `scenario` as a named state, creating `<file>.json` if it
    /// doesn't exist yet. Replaces a same-named entry in place (so
    /// re-saving over a mistake doesn't pile up duplicates); otherwise
    /// appends the new one at the end, so listing order stays stable —
    /// the reason `states` is kept as an array rather than a map that
    /// would have to pick its own (e.g. alphabetical) order on every
    /// write.
    pub fn save(
        &self,
        map: &WorldMap,
        cards: &CardCatalog,
        reference: &str,
        description: &str,
        scenario: &Scenario,
    ) -> Result<(), StateError> {
        let (file, name) = Self::split_reference(reference)?;
        let mut state_file = match self.read_file(file) {
            Ok(f) => f,
            Err(StateError::UnknownFile(_)) => StateFile::default(),
            Err(e) => return Err(e),
        };
        let record = StateRecord { name: name.to_string(), description: description.to_string(), scenario: scenario.to_raw(map, cards) };
        match state_file.states.iter_mut().find(|r| r.name == name) {
            Some(existing) => *existing = record,
            None => state_file.states.push(record),
        }
        fs::create_dir_all(&self.dir)?;
        fs::write(self.file_path(file), render_state_file(&state_file)?)?;
        Ok(())
    }
}

/// Turns a [`StateFile`] into pretty JSON text, via [`serde_json::Value`]
/// so the actual field layout stays driven by [`StateRecord`]/
/// [`RawScenario`]'s own `Serialize` impls rather than hand-duplicated
/// here. The one thing worth a hand-written formatter for (per this
/// module's own doc): `serde_json`'s own pretty printer puts every array
/// element on its own line, which is right for a hand's card list but
/// turns every country's `[us, ussr]` influence pair into three lines
/// apiece — [`render_value`] special-cases exactly that shape (a 2-element
/// all-numeric array) back onto one line, matching `demo_state.json`'s
/// own hand-authored style.
fn render_state_file(file: &StateFile) -> Result<String, serde_json::Error> {
    let value = serde_json::to_value(file)?;
    let mut out = String::new();
    render_value(&value, 0, &mut out);
    out.push('\n');
    Ok(out)
}

fn render_value(value: &Value, indent: usize, out: &mut String) {
    match value {
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{\n");
            let last = map.len() - 1;
            for (i, (key, v)) in map.iter().enumerate() {
                push_indent(out, indent + 1);
                out.push_str(&serde_json::to_string(key).expect("a JSON object key is always a valid JSON string"));
                out.push_str(": ");
                render_value(v, indent + 1, out);
                if i != last {
                    out.push(',');
                }
                out.push('\n');
            }
            push_indent(out, indent);
            out.push('}');
        }
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
            } else if items.len() == 2 && items.iter().all(Value::is_number) {
                // The one shape kept inline — see this function's own doc.
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&v.to_string());
                }
                out.push(']');
            } else {
                out.push_str("[\n");
                let last = items.len() - 1;
                for (i, v) in items.iter().enumerate() {
                    push_indent(out, indent + 1);
                    render_value(v, indent + 1, out);
                    if i != last {
                        out.push(',');
                    }
                    out.push('\n');
                }
                push_indent(out, indent);
                out.push(']');
            }
        }
        // Strings/numbers/bools/null already round-trip through
        // `serde_json`'s own `Display`/`to_string` exactly as JSON wants.
        _ => out.push_str(&value.to_string()),
    }
}

fn push_indent(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push_str("  ");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::country::Superpower;

    fn fixtures() -> (WorldMap, CardCatalog) {
        (WorldMap::standard().unwrap(), CardCatalog::standard().unwrap())
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ts-states-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn save_then_load_round_trips_a_scenario() {
        let (map, cards) = fixtures();
        let dir = temp_dir("round-trip");
        let lib = StateLibrary::new(&dir);
        let mut scenario = Scenario::from_raw(&map, &cards, RawScenario::default()).unwrap();
        let poland = map.id_by_name("Poland").unwrap();
        scenario.board.set_influence(poland, Superpower::Ussr, 4);
        scenario.status.vp = 7;

        lib.save(&map, &cards, "demo/first", "a test state", &scenario).unwrap();
        let (loaded, description) = lib.load(&map, &cards, "demo/first").unwrap();
        assert_eq!(description, "a test state");
        assert_eq!(loaded.status.vp, 7);
        assert_eq!(loaded.board.influence(poland, Superpower::Ussr), 4);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_the_same_name_twice_replaces_rather_than_duplicates() {
        let (map, cards) = fixtures();
        let dir = temp_dir("replace");
        let lib = StateLibrary::new(&dir);
        let mut scenario = Scenario::from_raw(&map, &cards, RawScenario::default()).unwrap();
        scenario.status.vp = 1;
        lib.save(&map, &cards, "demo/x", "first", &scenario).unwrap();
        scenario.status.vp = 2;
        lib.save(&map, &cards, "demo/x", "second", &scenario).unwrap();

        let entries = lib.list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].description, "second");
        let (loaded, _) = lib.load(&map, &cards, "demo/x").unwrap();
        assert_eq!(loaded.status.vp, 2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_file_is_reported() {
        let (map, cards) = fixtures();
        let lib = StateLibrary::new(temp_dir("missing"));
        assert!(matches!(lib.load(&map, &cards, "nope/nothing"), Err(StateError::UnknownFile(_))));
    }

    #[test]
    fn a_reference_with_no_slash_is_rejected() {
        let (map, cards) = fixtures();
        let lib = StateLibrary::new(temp_dir("bad-ref"));
        assert!(matches!(lib.load(&map, &cards, "nothing"), Err(StateError::BadReference(_))));
    }

    #[test]
    fn the_standard_library_states_all_load_cleanly() {
        let (map, cards) = fixtures();
        let lib = StateLibrary::standard();
        for entry in lib.list().unwrap() {
            lib.load(&map, &cards, &entry.reference()).unwrap_or_else(|e| panic!("{}: {e}", entry.reference()));
        }
    }
}

//! Saving and restoring games.
//!
//! A [`SaveGame`] is a [`Checkpoint`] (status, board, hands, headline picks, winner and the whole
//! log) plus the dice state and the session's AI settings. Only a *resting point* — no card in
//! play, no operation open, nothing waiting on `settle` — can be saved, so an open operation or
//! choice event never needs serialising; resuming replays at most the round that was in progress.
//! [`SaveStore`] keeps them on disk (`autosave.json` and `saves/<name>.json`); [`Autosaver`]
//! rewrites the autosave once per resting point.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::ai::AiKind;
use crate::cards::CardCatalog;
use crate::country::Superpower;
use crate::dice::Dice;
use crate::game::{Checkpoint, Game, HeadlineState, Victory};
use crate::log::GameLog;
use crate::map::WorldMap;
use crate::scenario::{RawScenario, Scenario, ScenarioError};

/// Bumped whenever the file layout changes incompatibly.
pub const SAVE_VERSION: u32 = 1;

#[derive(Debug)]
pub enum SaveError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Scenario(ScenarioError),
    /// The file was written by an incompatible version.
    Version(u32),
    /// The game is mid-action: finish the card in play first.
    NotResting,
    /// A save name with characters that don't belong in a file name.
    BadName(String),
    NotFound(String),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::Io(e) => write!(f, "{e}"),
            SaveError::Json(e) => write!(f, "invalid save file: {e}"),
            SaveError::Scenario(e) => write!(f, "{e}"),
            SaveError::Version(v) => write!(f, "save file version {v} isn't supported (expected {SAVE_VERSION})"),
            SaveError::NotResting => write!(f, "finish the current action first — a game can only be saved between actions"),
            SaveError::BadName(n) => write!(f, "{n:?} isn't a usable save name (letters, digits, - _ . only)"),
            SaveError::NotFound(n) => write!(f, "no saved game named {n:?} (try `saves` to list them)"),
        }
    }
}

impl std::error::Error for SaveError {}

impl From<std::io::Error> for SaveError {
    fn from(e: std::io::Error) -> Self {
        SaveError::Io(e)
    }
}

impl From<serde_json::Error> for SaveError {
    fn from(e: serde_json::Error) -> Self {
        SaveError::Json(e)
    }
}

impl From<ScenarioError> for SaveError {
    fn from(e: ScenarioError) -> Self {
        SaveError::Scenario(e)
    }
}

/// The session settings a save carries alongside the game itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub ai_side: Option<Superpower>,
    pub ai_kind: AiKind,
    pub ai_fast: bool,
    pub debug: bool,
}

/// A restored game with its dice and settings.
pub struct Restored {
    pub game: Game,
    pub dice: Dice,
    pub settings: Settings,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SaveGame {
    version: u32,
    /// Seconds since the Unix epoch when it was written.
    saved: u64,
    scenario: RawScenario,
    headline: HeadlineState,
    winner: Option<Victory>,
    log: GameLog,
    dice: u64,
    ai_side: Option<Superpower>,
    ai_kind: String,
    ai_fast: bool,
    debug: bool,
}

impl SaveGame {
    /// Captures `game`, or `None` when it isn't at a resting point (see [`Game::checkpoint`]).
    pub fn capture(map: &WorldMap, cards: &CardCatalog, game: &Game, dice: &Dice, settings: Settings) -> Option<SaveGame> {
        let cp = game.checkpoint()?;
        Some(SaveGame {
            version: SAVE_VERSION,
            saved: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            scenario: cp.scenario.to_raw(map, cards),
            headline: cp.headline,
            winner: cp.winner,
            log: cp.log,
            dice: dice.state(),
            ai_side: settings.ai_side,
            ai_kind: settings.ai_kind.name().to_string(),
            ai_fast: settings.ai_fast,
            debug: settings.debug,
        })
    }

    pub fn restore(self, map: &WorldMap, cards: &CardCatalog) -> Result<Restored, SaveError> {
        if self.version != SAVE_VERSION {
            return Err(SaveError::Version(self.version));
        }
        let scenario = Scenario::from_raw(map, cards, self.scenario)?;
        let game = Game::from_checkpoint(Checkpoint { scenario, headline: self.headline, winner: self.winner, log: self.log });
        Ok(Restored {
            game,
            dice: Dice::from_seed(self.dice),
            settings: Settings {
                ai_side: self.ai_side,
                ai_kind: AiKind::parse(&self.ai_kind).unwrap_or_default(),
                ai_fast: self.ai_fast,
                debug: self.debug,
            },
        })
    }

    /// Seconds since the Unix epoch when this was captured.
    pub fn saved_at(&self) -> u64 {
        self.saved
    }

    /// `turn3-ar2-1735689600`: where the game stands, made unique by the time.
    pub fn default_name(game: &Game) -> String {
        let s = game.status();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        format!("turn{}-ar{}-{now}", s.turn, s.action_round)
    }
}

/// One entry of [`SaveStore::list`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveInfo {
    pub name: String,
    /// Seconds since the Unix epoch the file was last written.
    pub modified: u64,
}

/// A directory of saved games: `autosave.json` plus `saves/<name>.json`.
#[derive(Debug, Clone)]
pub struct SaveStore {
    dir: PathBuf,
}

impl SaveStore {
    /// `$XDG_DATA_HOME/twilight-struggle`, else `~/.local/share/twilight-struggle`; `None` when
    /// neither a data directory nor a home directory is known.
    pub fn standard() -> Option<SaveStore> {
        let base = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").filter(|v| !v.is_empty()).map(|h| PathBuf::from(h).join(".local/share")))?;
        Some(SaveStore::new(base.join("twilight-struggle")))
    }

    pub fn new(dir: impl Into<PathBuf>) -> SaveStore {
        SaveStore { dir: dir.into() }
    }

    fn autosave_path(&self) -> PathBuf {
        self.dir.join("autosave.json")
    }

    fn saves_dir(&self) -> PathBuf {
        self.dir.join("saves")
    }

    fn named_path(&self, name: &str) -> Result<PathBuf, SaveError> {
        let ok = !name.is_empty()
            && !name.starts_with('.')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if !ok {
            return Err(SaveError::BadName(name.to_string()));
        }
        Ok(self.saves_dir().join(format!("{name}.json")))
    }

    pub fn has_autosave(&self) -> bool {
        self.autosave_path().is_file()
    }

    pub fn autosave(&self, save: &SaveGame) -> Result<(), SaveError> {
        write_atomic(&self.autosave_path(), save)
    }

    pub fn load_autosave(&self) -> Result<SaveGame, SaveError> {
        read_save(&self.autosave_path(), "autosave")
    }

    pub fn save(&self, name: &str, save: &SaveGame) -> Result<(), SaveError> {
        write_atomic(&self.named_path(name)?, save)
    }

    pub fn load(&self, name: &str) -> Result<SaveGame, SaveError> {
        read_save(&self.named_path(name)?, name)
    }

    /// Named saves, newest first.
    pub fn list(&self) -> Vec<SaveInfo> {
        let Ok(entries) = fs::read_dir(self.saves_dir()) else { return Vec::new() };
        let mut out: Vec<SaveInfo> = entries
            .filter_map(Result::ok)
            .filter_map(|e| {
                let path = e.path();
                if path.extension()? != "json" {
                    return None;
                }
                let name = path.file_stem()?.to_str()?.to_string();
                let modified = e.metadata().ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
                Some(SaveInfo { name, modified })
            })
            .collect();
        out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
        out
    }
}

fn read_save(path: &Path, what: &str) -> Result<SaveGame, SaveError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(serde_json::from_str(&text)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(SaveError::NotFound(what.to_string())),
        Err(e) => Err(e.into()),
    }
}

/// Writes to a sibling `.tmp` file, then renames it over `path`, so a crash never leaves half a save.
fn write_atomic(path: &Path, save: &SaveGame) -> Result<(), SaveError> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string(save)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Rewrites the autosave once per resting point — not on every keypress.
#[derive(Debug)]
pub struct Autosaver {
    store: SaveStore,
    last: Option<(usize, u8, u8, Superpower)>,
}

impl Autosaver {
    pub fn new(store: SaveStore) -> Autosaver {
        Autosaver { store, last: None }
    }

    pub fn store(&self) -> &SaveStore {
        &self.store
    }

    /// Forget what was last written (a different game was loaded or started).
    pub fn reset(&mut self) {
        self.last = None;
    }

    /// Saves if `game` is at a resting point it hasn't saved yet. `Ok(true)` when it wrote.
    pub fn tick(&mut self, map: &WorldMap, cards: &CardCatalog, game: &Game, dice: &Dice, settings: Settings) -> Result<bool, SaveError> {
        let s = game.status();
        let key = (game.log().entries().len(), s.turn, s.action_round, s.active);
        if self.last == Some(key) {
            return Ok(false);
        }
        let Some(save) = SaveGame::capture(map, cards, game, dice, settings) else { return Ok(false) };
        self.store.autosave(&save)?;
        self.last = Some(key);
        Ok(true)
    }
}

/// `serde(with = ...)` for an `Option<&'static str>` field: written as a plain string, read back
/// by leaking it. The few titles involved are tiny and a load runs once, so the leak is bounded.
/// An alias so serde's derive doesn't see a `&'static str` and demand `'de: 'static`.
pub type StaticStr = &'static str;

pub mod serde_static_opt {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<&'static str>, s: S) -> Result<S::Ok, S::Error> {
        v.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<&'static str>, D::Error> {
        Ok(Option::<String>::deserialize(d)?.map(|s| &*Box::leak(s.into_boxed_str())))
    }
}

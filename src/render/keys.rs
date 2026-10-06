//! The two key-hint rows `interactive.rs` pins under every map screen: the
//! *global* row (keys that work whatever is happening — map navigation, the
//! hand, the track and pile views) and the *context* row (the extra keys
//! that do something right now). Pure string producers like the rest of
//! `render/`; nothing here touches a terminal.

use crate::cards::CardCatalog;
use crate::events::PlayAs;
use crate::game::{Game, Trap};
use crate::ops::Operation;


/// Which map screen the keys are for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyScreen {
    World,
    Region,
    Country,
}

/// What only the interactive loop knows about the moment the row is drawn.
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyUi {
    pub zoomed: bool,
    /// The strip is showing the opponent's revealed hand.
    pub peeking: bool,
    pub modal_open: bool,
    /// Whether `p` plays the selected card (rather than passing).
    pub p_plays: bool,
}

/// The always-available keys for a screen.
pub fn global_keys(screen: KeyScreen) -> String {
    let nav = match screen {
        KeyScreen::World => "←→↑↓ move · Enter open region",
        KeyScreen::Region => "←→↑↓ move · Enter open country · Esc world",
        KeyScreen::Country => "←→↑↓ move · Esc region",
    };
    format!("{nav} · [ ] hand · z zoom · t tracks · D piles · q quit")
}

/// The keys that act right now, first match wins. Empty while a modal has
/// the keyboard (each modal carries its own hint).
pub fn context_keys(game: &Game, cards: &CardCatalog, screen: KeyScreen, ui: KeyUi) -> String {
    if ui.modal_open {
        return String::new();
    }
    let mut keys = primary(game, cards, screen, ui);
    if game.status().effects.hand_revealed(game.active().opponent()) && game.operation().is_none() {
        keys.push_str(if ui.peeking { " · v back to your hand" } else { " · v opponent's hand" });
    }
    if screen != KeyScreen::World && game.operation().is_none() && game.status().effects.coup_forbidden(game.active()) {
        keys.push_str(" · d defuse");
    }
    keys
}

fn primary(game: &Game, cards: &CardCatalog, screen: KeyScreen, ui: KeyUi) -> String {
    if game.winner().is_some() {
        return "game over".to_string();
    }
    if ui.zoomed {
        return "z/Esc close · [ ] browse · Space play".to_string();
    }
    if let Some(side) = game.awaiting_discard() {
        return format!("{side} may discard a card: [ ] select · Space discard it · p keep them all");
    }
    if game.picking_headline() {
        return "[ ] select · Space choose headline (its event only, can't be taken back)".to_string();
    }
    match game.operation() {
        Some(Operation::Influence(_)) => "+ place · - take back · u undo · c confirm · X cancel · ⌫ abandon".to_string(),
        Some(op @ (Operation::Realign(_) | Operation::Coup(_) | Operation::War(_))) => {
            let roll = match screen {
                KeyScreen::World => "Enter open region".to_string(),
                _ => "Enter/r choose target and roll".to_string(),
            };
            match op {
                Operation::War(_) if op.ops_spent() == 0 => format!("{roll} · ⌫ abandon"),
                Operation::War(_) => roll,
                _ if op.ops_spent() == 0 => format!("{roll} · c done · X cancel · ⌫ abandon"),
                _ => format!("{roll} · c done · X cancel"),
            }
        }
        Some(Operation::Event(e)) => event_keys(game, e),
        None => idle_keys(game, cards, ui),
    }
}

fn event_keys(game: &Game, e: &crate::events::EventChoice) -> String {
    let n = e.modes().len().min(9);
    let abandon = if game.event_abandonable() { " · ⌫ abandon" } else { "" };
    if !e.gate_cards().is_empty() {
        return match e.mode() {
            None if e.gate_offset() == 0 => "[ ] pick a card · Space discard it · c confirm".to_string(),
            None => "[ ] pick a card · Space discard it · 1 keep your cards".to_string(),
            Some(_) => "c confirm · ⌫ undo · [ ] Space change".to_string(),
        };
    }
    if e.is_multi() {
        return "↑↓ move · Enter mark/unmark · c confirm · ⌫ clear marks".to_string();
    }
    if e.is_pile_pick() {
        return if e.pile().is_empty() { "c confirm · ⌫ take the card back".to_string() } else { "↑↓ move · Enter choose · c done · ⌫ clear".to_string() };
    }
    if e.needs_roll() {
        return if e.is_participation() { format!("r roll the dice · 1-{n} change · ⌫ clear") } else { "r roll the dice · ⌫ cancel the event".to_string() };
    }
    if e.is_designation() {
        return match e.mode() {
            None => format!("Enter on the world map or 1-{n} to choose a region{abandon}"),
            Some(_) => format!("Enter/1-{n} change region · c done · ⌫ clear"),
        };
    }
    if e.is_mode_only() || !e.picks_countries() {
        return match e.mode() {
            None => format!("1-{n} choose{abandon}"),
            Some(_) => format!("c confirm · 1-{n} change · ⌫ undo"),
        };
    }
    match e.mode() {
        None if n > 1 => format!("1-{n} choose mode{abandon}"),
        _ => format!("+ add · - remove · u undo · {}c done{abandon}", if n > 1 { format!("1-{n} mode · ") } else { String::new() }),
    }
}

fn idle_keys(game: &Game, cards: &CardCatalog, ui: KeyUi) -> String {
    let status = game.status();
    let play = if ui.p_plays { "Space/p play selected".to_string() } else { "p pass".to_string() };
    if let Some((_, trap)) = game.trap() {
        return match trap {
            Trap::Escape(_) => "Space on a 2+ ops card: discard it and roll to escape".to_string(),
            Trap::PlayScoring => "Space play a scoring card · p pass when none".to_string(),
            Trap::Skip => "p pass".to_string(),
        };
    }
    if status.forced_play.is_some_and(|(side, _)| side == game.active()) && game.card_in_play().is_none() {
        return "Space/p play Missile Envy (for operations)".to_string();
    }
    let Some(id) = game.card_in_play() else { return play };
    if let Some(grant) = game.ops_after_event() {
        let ops: Vec<&str> = [(grant.influence, "i influence"), (grant.realign, "a realign"), (grant.coup, "o coup")].into_iter().filter_map(|(on, k)| on.then_some(k)).collect();
        return format!("{} · p skip the ops", ops.join(" · "));
    }
    if let (Some(_), Some(how)) = (game.forced_by(), game.forced_how()) {
        return match how {
            PlayAs::Event => "e play its event (can't be skipped)".to_string(),
            PlayAs::Either => "e event · i/a/o operations".to_string(),
            PlayAs::Ops => "i influence · a realign · o coup".to_string(),
        };
    }
    card_keys(game, cards, id)
}

/// What a card offers once it is the card in play: its event, its operations, the space race
/// and the way to return it to the hand.
fn card_keys(game: &Game, cards: &CardCatalog, id: crate::cards::CardId) -> String {
    let card = cards.card(id);
    let mut keys: Vec<&str> = Vec::new();
    if game.event_playable(cards, id) {
        keys.push(if card.scoring { "e score" } else { "e event" });
    }
    if !card.scoring {
        keys.extend(["i influence", "a realign", "o coup", "s space race"]);
    }
    keys.push("⌫ return card");
    keys.join(" · ")
}

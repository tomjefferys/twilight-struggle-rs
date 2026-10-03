//! Keeps `CARDS.md`'s ✅ column in step with `events::is_implemented`, the
//! same way `main.rs`'s `every_command_is_mentioned_in_help` keeps the
//! REPL's command list and help text from drifting.

use twilight_struggle::{events, CardCatalog};

#[test]
fn cards_md_marks_exactly_the_implemented_events() {
    let cards = CardCatalog::standard().unwrap();
    let doc = include_str!("../CARDS.md");

    let mut rows = 0;
    for line in doc.lines().filter(|l| l.starts_with("| ") && !l.starts_with("| #") && !l.starts_with("|---")) {
        let cols: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        let number: u8 = cols[0].parse().unwrap_or_else(|_| panic!("bad card number in row: {line}"));
        let id = match cards.find(&number.to_string()) {
            twilight_struggle::CardFound::One(id) => id,
            other => panic!("card #{number} should resolve to one card, got {other:?}"),
        };
        assert_eq!(cols[1], cards.card(id).name, "card #{number}'s name in CARDS.md");
        assert_eq!(cols[3] == "✅", events::is_implemented(id), "card #{number} ({}): CARDS.md disagrees with is_implemented", cols[1]);
        rows += 1;
    }
    assert_eq!(rows, cards.iter().count(), "CARDS.md should list every card exactly once");
}

# Card event progress

Which of the 110 cards have their **event** implemented (`events::is_implemented`).
Every card can already be played for its ops value; this tracks the event text only.
`tests/cards_progress.rs` fails if a ✅ here disagrees with the code, so update both together.

**Implemented: 102 / 110**

- `events::scoring` — the seven scoring cards.
- `events::effects` — fixed influence / VP / DEFCON effects with no choices or die rolls.
- `events::ongoing` effects (in `events::effects`, plus #94 in `events::choice`) — cards whose event lasts for the rest of the turn; see `src/ongoing.rs`.
- `events::war` — the five war cards (die roll against a target; Military Ops tracked in `GameStatus`; see `src/events/war.rs`).
- Game-long effects (`LastingEffects` in `src/ongoing.rs`, a field of `GameStatus`) — #21 NATO, #27, #35, #50, #55, #59, #73 (plus #17's De Gaulle exemption); never cleared by a turn rolling over.
- Action-round triggers (`Game::trap`, `Game::settle`, `Game::defuse_crisis`) — #42 Quagmire / #44 Bear Trap (the trapped side's rounds become escape attempts), #106 NORAD (an end-of-round trigger) and #40 Cuban Missile Crisis (a turn-long coup ban with an anytime cancel).
- `events::choice` — cards where a player picks the countries (the card's own side chooses, whoever is phasing).

| # | Card | Side | Event | Notes |
|---|---|---|---|---|
| 1 | Asia Scoring | Both | ✅ |  |
| 2 | Europe Scoring | Both | ✅ | Control tier is an outright win |
| 3 | Middle East Scoring | Both | ✅ |  |
| 4 | Duck and Cover | US | ✅ |  |
| 5 | Five Year Plan | US |  |  |
| 6 | The China Card | Both |  | Ops only; +1 op if all spent in Asia (`OpsBonus`); passes face down; end-of-Turn-10 VP pending (no final scoring yet) |
| 7 | Socialist Governments | USSR | ✅ | Choice; prevented by #83 (modelled) |
| 8 | Fidel | USSR | ✅ |  |
| 9 | Vietnam Revolts | USSR | ✅ | Turn-long: +1 ops for a card spent wholly in Southeast Asia |
| 10 | Blockade | USSR | ✅ | The US discards a 3+ ops card or loses all its West Germany influence (`EventChoice::discard_gate`; no such card → applies at once) |
| 11 | Korean War | USSR | ✅ | War: roll 4+ (−1 per US-controlled neighbour); +2 VP, replaces US influence |
| 12 | Romanian Abdication | USSR | ✅ |  |
| 13 | Arab-Israeli War | USSR | ✅ | War; can't be played after #65 (modelled) |
| 14 | Comecon | USSR | ✅ | Choice |
| 15 | Nasser | USSR | ✅ |  |
| 16 | Warsaw Pact Formed | USSR | ✅ | Choice (2 modes); enables #21 (modelled) |
| 17 | De Gaulle Leads France | USSR | ✅ | Exempts France from NATO (#21) |
| 18 | Captured Nazi Scientist | Both | ✅ | Advances the space race 1 box |
| 19 | Truman Doctrine | US | ✅ | Choice |
| 20 | Olympic Games | Both | ✅ | The opponent participates (roll-off, sponsor +2, ties re-rolled, winner 2 VP) or boycotts (DEFCON −1, sponsor conducts ops as a 4-ops card); played in the same modal as Summit (choose, `r` roll, confirm) |
| 21 | NATO | US | ✅ | Lasting: USSR can't coup/realign US-controlled Europe; needs #16 or #23 first; Brush War clause modelled |
| 22 | Independent Reds | US | ✅ | Choice |
| 23 | Marshall Plan | US | ✅ | Choice; enables #21 (modelled) |
| 24 | Indo-Pakistani War | Both | ✅ | War (choose India/Pakistan) |
| 25 | Containment | US | ✅ | Turn-long: US ops +1 (max 4) |
| 26 | CIA Created | US | ✅ | Reveals the USSR hand for the turn (`v` in the map), then the US may use the card's ops for any operation (`Game::ops_after_event`) |
| 27 | US/Japan Mutual Defense Pact | US | ✅ | Lasting: US takes control of Japan; USSR can't coup/realign it |
| 28 | Suez Crisis | USSR | ✅ | Choice |
| 29 | East European Unrest | US | ✅ | Choice; 2 per country from turn 8 (Late War) |
| 30 | Decolonization | USSR | ✅ | Choice |
| 31 | Red Scare/Purge | Both | ✅ | Turn-long: opponent's ops -1 (min 1) |
| 32 | UN Intervention | Both |  |  |
| 33 | De-Stalinization | USSR | ✅ | Choice ('may'); balanced reallocation |
| 34 | Nuclear Test Ban | Both | ✅ |  |
| 35 | Formosan Resolution | US | ✅ | Lasting: US-controlled Taiwan scores as an Asia battleground until the US plays the China Card |
| 36 | Brush War | Both | ✅ | War (stability ≤ 2 target, 3+ wins, 1 VP); can't hit NATO-protected Europe (modelled) |
| 37 | Central America Scoring | Both | ✅ |  |
| 38 | Southeast Asia Scoring | Both | ✅ |  |
| 39 | Arms Race | Both | ✅ | Player ahead on Military Ops: 1 VP, or 3 if they also meet the required amount (= DEFCON) |
| 40 | Cuban Missile Crisis | Both | ✅ | DEFCON 2; the opponent's coup this turn loses them the game (`VictoryReason::CubanMissileCrisis`), unless it defuses: `d` / `defuse <country>` removes 2 of its own influence from Cuba (USSR) or West Germany/Turkey (US) |
| 41 | Nuclear Subs | US | ✅ | Turn-long: US battleground coups keep DEFCON (the coup DEFCON drop itself is new) |
| 42 | Quagmire | USSR | ✅ | The US's action rounds become escape attempts until it rolls 1-4; cancels #106. Each scoring card played while it has no 2+ ops card to discard takes one round |
| 43 | SALT Negotiations | Both | ✅ | DEFCON +2 and −1 on all coup rolls this turn (`TurnEffects::salt`); then the player may take one non-scoring discard into their hand (`EventChoice::pick_from_pile`, revealed) |
| 44 | Bear Trap | US | ✅ | As Quagmire, against the USSR: discard a 2+ ops card (`space`/`escape <card>`) and roll 1-4; with none, play scoring cards, then skip rounds |
| 45 | Summit | Both | ✅ | Roll-off (+1 per region dominated/controlled); the winner gets 2 VP and picks DEFCON ±1 or no change; a tie does nothing; played in a modal: odds, `r` roll, then choose (`Game::roll_contest`) |
| 46 | How I Learned to Stop Worrying | Both | ✅ | Choice (mode = DEFCON level 1-5); +5 Military Ops (max 5) |
| 47 | Junta | Both | ✅ | Choice (+2 influence in one Central/South America country), then a coup or realignment there with the card's ops |
| 48 | Kitchen Debates | US | ✅ |  |
| 49 | Missile Envy | Both |  |  |
| 50 | “We Will Bury You” | USSR | ✅ | DEFCON −1; USSR +3 VP after the US's next round; UN Intervention escape pending #32 |
| 51 | Brezhnev Doctrine | USSR | ✅ | Turn-long: USSR ops +1 (max 4) |
| 52 | Portuguese Empire Crumbles | USSR | ✅ |  |
| 53 | South African Unrest | USSR | ✅ | Choice (2 modes) |
| 54 | Allende | USSR | ✅ |  |
| 55 | Willy Brandt | USSR | ✅ | USSR +1 VP, +1 West Germany; NATO exempts West Germany; cancelled by #96 (modelled) |
| 56 | Muslim Revolution | USSR | ✅ | Choice; prevented by #110 (modelled) |
| 57 | ABM Treaty | Both | ✅ | DEFCON +1, then the player may conduct any operation with the card |
| 58 | Cultural Revolution | USSR | ✅ | US holds China Card → USSR gets it face up; else +1 VP (`EffectResult::china`) |
| 59 | Flower Power | USSR | ✅ | Lasting: USSR +2 VP per US war card (ops or event); cancelled by #97 |
| 60 | U2 Incident | USSR | ✅ | USSR +1 VP; the extra VP if #32 follows pending #32 |
| 61 | OPEC | USSR | ✅ | USSR +1 VP per controlled oil producer; barred after #86 (modelled) |
| 62 | “Lone Gunman” | USSR | ✅ | Reveals the US hand for the turn (`v` in the map), then the USSR may use the card's ops for any operation |
| 63 | Colonial Rear Guards | US | ✅ | Choice |
| 64 | Panama Canal Returned | US | ✅ |  |
| 65 | Camp David Accords | US | ✅ | 'prevents #13' clause modelled |
| 66 | Puppet Governments | US | ✅ | Choice ('may') |
| 67 | Grain Sales to Soviets | US |  |  |
| 68 | John Paul II Elected Pope | US | ✅ | 'allows #101' clause pending #101 |
| 69 | Latin American Death Squads | Both | ✅ | Turn-long: coup roll ±1 in Central/South America |
| 70 | OAS Founded | US | ✅ | Choice |
| 71 | Nixon Plays the China Card | US | ✅ | USSR holds China Card → US gets it face down; else +2 VP |
| 72 | Sadat Expels Soviets | US | ✅ |  |
| 73 | Shuttle Diplomacy | US | ✅ | Lasting: −1 USSR battleground at the next Asia/Middle East scoring, then discarded |
| 74 | The Voice of America | US | ✅ | Choice; 4 USSR influence outside Europe, max 2 per country |
| 75 | Liberation Theology | USSR | ✅ | Choice |
| 76 | Ussuri River Skirmish | US | ✅ | Choice: USSR holds China Card → US gets it face up; else +4 US in Asia (max 2 each) |
| 77 | “Ask Not What Your Country…” | US |  |  |
| 78 | Alliance for Progress | US | ✅ |  |
| 79 | Africa Scoring | Both | ✅ |  |
| 80 | “One Small Step…” | Both | ✅ | If behind on the space race: 2 boxes, VP only from the last |
| 81 | South America Scoring | Both | ✅ |  |
| 82 | Iranian Hostage Crisis | USSR | ✅ |  |
| 83 | The Iron Lady | US | ✅ | Prevents #7 (modelled) |
| 84 | Reagan Bombs Libya | US | ✅ |  |
| 85 | Star Wars | US |  |  |
| 86 | North Sea Oil | US | ✅ | Turn-long: US plays an 8th action round; prevents #61 (modelled) |
| 87 | The Reformer | USSR | ✅ | Choice; USSR can't coup in Europe afterwards (modelled) |
| 88 | Marine Barracks Bombing | USSR | ✅ | Choice; Lebanon cleared up front |
| 89 | Soviets Shoot Down KAL-007 | US | ✅ | DEFCON −1, US +2 VP; then, if South Korea is US-controlled, the US may place influence or realign |
| 90 | Glasnost | USSR | ✅ | DEFCON +1, USSR +2 VP; then, if #87 has been played, the USSR may place influence or realign |
| 91 | Ortega Elected in Nicaragua | USSR | ✅ | Clears Nicaragua's US influence, then a coup only in a neighbouring country |
| 92 | Terrorism | Both | ✅ | The opponent discards 1 card at random (the US 2 once #82 is played); needs `Game::play_event_with` dice |
| 93 | Iran-Contra Scandal | USSR | ✅ | Turn-long: US realignment rolls -1 |
| 94 | Chernobyl | US | ✅ | Choice (region, modes 1-6); turn-long: USSR can't add influence there with ops |
| 95 | Latin American Debt Crisis | USSR | ✅ | The US discards a 3+ ops card, or the USSR may double its influence in 2 South American countries (a second session) |
| 96 | Tear Down this Wall | US | ✅ | +3 US East Germany; cancels #55 (and bars it later); then a coup or realignment in Europe |
| 97 | “An Evil Empire” | US | ✅ | Cancels #59 (modelled) |
| 98 | Aldrich Ames Remix | USSR | ✅ | The USSR discards a card of its choice from the US hand (`EventChoice::pick_from_hand`); the US hand is open to it for the turn (`TurnEffects::us_hand_revealed`, `v` in the map) |
| 99 | Pershing II Deployed | USSR | ✅ | Choice; USSR +1 VP, 1 US influence from each of 3 Western European countries |
| 100 | Wargames | Both | ✅ | Choice (end the game / play on) at DEFCON 2: the opponent gets 6 VP, the VP leader wins (a tie goes to the opponent); nothing at other levels |
| 101 | Solidarity | US | ✅ | Needs #68 first |
| 102 | Iran-Iraq War | Both | ✅ | War (choose Iran/Iraq) |
| 103 | Defectors | US | ✅ | USSR playing it gives the US 1 VP; the headline half pending a headline phase |
| 104 | The Cambridge Five | USSR | ✅ | Reveals the US scoring cards; the USSR may add 1 influence to one country in a region they name (Southeast Asia isn't a region); not in the Late War |
| 105 | Special Relationship | US | ✅ | Choice; adjacent-to-UK branch, or with NATO in effect +2 influence in Western Europe and +2 VP |
| 106 | NORAD | US | ✅ | After an action round that moved DEFCON to 2, with Canada US-controlled: +1 US influence where it has some (`Game::settle` opens it as a triggered `EventChoice`) |
| 107 | Che | USSR | ✅ | No direct effect; a coup in a non-battleground in Central/South America or Africa, plus a second (different country) if the first removed US influence |
| 108 | Our Man in Tehran | US |  |  |
| 109 | Yuri and Samantha | USSR | ✅ | Turn-long: USSR +1 VP per US coup |
| 110 | AWACS Sale to Saudis | US | ✅ | Prevents #56 (modelled) |

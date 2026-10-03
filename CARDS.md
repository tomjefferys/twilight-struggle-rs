# Card event progress

Which of the 110 cards have their **event** implemented (`events::is_implemented`).
Every card can already be played for its ops value; this tracks the event text only.
`tests/cards_progress.rs` fails if a ✅ here disagrees with the code, so update both together.

**Implemented: 26 / 110**

- `events::scoring` — the seven scoring cards.
- `events::effects` — fixed influence / VP / DEFCON effects with no choices or die rolls.

| # | Card | Side | Event | Notes |
|---|---|---|---|---|
| 1 | Asia Scoring | Both | ✅ |  |
| 2 | Europe Scoring | Both | ✅ | Control tier is an outright win |
| 3 | Middle East Scoring | Both | ✅ |  |
| 4 | Duck and Cover | US | ✅ |  |
| 5 | Five Year Plan | US |  |  |
| 6 | The China Card | Both |  | Ops only; passes to the opponent |
| 7 | Socialist Governments | USSR |  |  |
| 8 | Fidel | USSR | ✅ |  |
| 9 | Vietnam Revolts | USSR |  |  |
| 10 | Blockade | USSR |  |  |
| 11 | Korean War | USSR |  |  |
| 12 | Romanian Abdication | USSR | ✅ |  |
| 13 | Arab-Israeli War | USSR |  |  |
| 14 | Comecon | USSR |  |  |
| 15 | Nasser | USSR | ✅ |  |
| 16 | Warsaw Pact Formed | USSR |  |  |
| 17 | De Gaulle Leads France | USSR | ✅ | NATO-cancel clause pending #21 |
| 18 | Captured Nazi Scientist | Both |  |  |
| 19 | Truman Doctrine | US |  |  |
| 20 | Olympic Games | Both |  |  |
| 21 | NATO | US |  |  |
| 22 | Independent Reds | US |  |  |
| 23 | Marshall Plan | US |  |  |
| 24 | Indo-Pakistani War | Both |  |  |
| 25 | Containment | US |  |  |
| 26 | CIA Created | US |  |  |
| 27 | US/Japan Mutual Defense Pact | US |  |  |
| 28 | Suez Crisis | USSR |  |  |
| 29 | East European Unrest | US |  |  |
| 30 | Decolonization | USSR |  |  |
| 31 | Red Scare/Purge | Both |  |  |
| 32 | UN Intervention | Both |  |  |
| 33 | De-Stalinization | USSR |  |  |
| 34 | Nuclear Test Ban | Both | ✅ |  |
| 35 | Formosan Resolution | US |  |  |
| 36 | Brush War | Both |  |  |
| 37 | Central America Scoring | Both | ✅ |  |
| 38 | Southeast Asia Scoring | Both | ✅ |  |
| 39 | Arms Race | Both |  |  |
| 40 | Cuban Missile Crisis | Both |  |  |
| 41 | Nuclear Subs | US |  |  |
| 42 | Quagmire | USSR |  |  |
| 43 | SALT Negotiations | Both |  |  |
| 44 | Bear Trap | US |  |  |
| 45 | Summit | Both |  |  |
| 46 | How I Learned to Stop Worrying | Both |  |  |
| 47 | Junta | Both |  |  |
| 48 | Kitchen Debates | US | ✅ |  |
| 49 | Missile Envy | Both |  |  |
| 50 | “We Will Bury You” | USSR |  |  |
| 51 | Brezhnev Doctrine | USSR |  |  |
| 52 | Portuguese Empire Crumbles | USSR | ✅ |  |
| 53 | South African Unrest | USSR |  |  |
| 54 | Allende | USSR | ✅ |  |
| 55 | Willy Brandt | USSR |  |  |
| 56 | Muslim Revolution | USSR |  |  |
| 57 | ABM Treaty | Both |  |  |
| 58 | Cultural Revolution | USSR |  |  |
| 59 | Flower Power | USSR |  |  |
| 60 | U2 Incident | USSR |  |  |
| 61 | OPEC | USSR |  |  |
| 62 | “Lone Gunman” | USSR |  |  |
| 63 | Colonial Rear Guards | US |  |  |
| 64 | Panama Canal Returned | US | ✅ |  |
| 65 | Camp David Accords | US | ✅ | 'prevents #13' clause pending #13 |
| 66 | Puppet Governments | US |  |  |
| 67 | Grain Sales to Soviets | US |  |  |
| 68 | John Paul II Elected Pope | US | ✅ | 'allows #101' clause pending #101 |
| 69 | Latin American Death Squads | Both |  |  |
| 70 | OAS Founded | US |  |  |
| 71 | Nixon Plays the China Card | US |  |  |
| 72 | Sadat Expels Soviets | US | ✅ |  |
| 73 | Shuttle Diplomacy | US |  |  |
| 74 | The Voice of America | US |  |  |
| 75 | Liberation Theology | USSR |  |  |
| 76 | Ussuri River Skirmish | US |  |  |
| 77 | “Ask Not What Your Country…” | US |  |  |
| 78 | Alliance for Progress | US | ✅ |  |
| 79 | Africa Scoring | Both | ✅ |  |
| 80 | “One Small Step…” | Both |  |  |
| 81 | South America Scoring | Both | ✅ |  |
| 82 | Iranian Hostage Crisis | USSR | ✅ |  |
| 83 | The Iron Lady | US | ✅ | 'prevents #7' clause pending #7 |
| 84 | Reagan Bombs Libya | US | ✅ |  |
| 85 | Star Wars | US |  |  |
| 86 | North Sea Oil | US |  |  |
| 87 | The Reformer | USSR |  |  |
| 88 | Marine Barracks Bombing | USSR |  |  |
| 89 | Soviets Shoot Down KAL-007 | US |  |  |
| 90 | Glasnost | USSR |  |  |
| 91 | Ortega Elected in Nicaragua | USSR |  |  |
| 92 | Terrorism | Both |  |  |
| 93 | Iran-Contra Scandal | USSR |  |  |
| 94 | Chernobyl | US |  |  |
| 95 | Latin American Debt Crisis | USSR |  |  |
| 96 | Tear Down this Wall | US |  |  |
| 97 | “An Evil Empire” | US | ✅ | 'cancels #59' clause pending #59 |
| 98 | Aldrich Ames Remix | USSR |  |  |
| 99 | Pershing II Deployed | USSR |  |  |
| 100 | Wargames | Both |  |  |
| 101 | Solidarity | US |  |  |
| 102 | Iran-Iraq War | Both |  |  |
| 103 | Defectors | US |  |  |
| 104 | The Cambridge Five | USSR |  |  |
| 105 | Special Relationship | US |  |  |
| 106 | NORAD | US |  |  |
| 107 | Che | USSR |  |  |
| 108 | Our Man in Tehran | US |  |  |
| 109 | Yuri and Samantha | USSR |  |  |
| 110 | AWACS Sale to Saudis | US | ✅ | 'prevents #56' clause pending #56 |

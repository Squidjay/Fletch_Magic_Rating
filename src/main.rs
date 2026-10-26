//To anyone reading this code, make sure to run it in powershell with cargo run

//MTG Fletch rating system
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use strsim::jaro_winkler;
use std::collections::HashMap;

//---
//Glicko / FLETCH constants
//---

// Rating scale: a 400-point gap is a 10:1 expected-score ratio
const RATING_SCALE: f64 = 400.0;
const Q: f64 = std::f64::consts::LN_10 / RATING_SCALE;

// The 3 in g(RD) = 1 / sqrt(1 + 3q²RD²/π²)
const G_COEFFICIENT: f64 = 3.0;

const DEFAULT_RATING: f64 = 1500.0;
const DEFAULT_RD: f64 = 350.0;
const MAX_RD: f64 = DEFAULT_RD;
const MIN_RD: f64 = 30.0;

// How a rating change is divided between player and deck
const DECK_SHARE: f64 = 0.30;
const PLAYER_SHARE: f64 = 1.0 - DECK_SHARE;

// Surprise adjustment to RD
const SURPRISE_THRESHOLD: f64 = 0.5;
const SURPRISE_RD_INCREASE_SCALE: f64 = 40.0;
const SURPRISE_RD_DECREASE_SCALE: f64 = 10.0;

// Match scores
const WIN_SCORE: f64 = 1.0;
const LOSS_SCORE: f64 = 0.0;

// Starting deck rating by bracket (index 0 = bracket 1)
const MIN_BRACKET: i16 = 1;
const MAX_BRACKET: i16 = 5;
const BRACKET_STARTING_RATINGS: [f64; 5] = [1300.0, 1450.0, 1600.0, 1750.0, 1900.0];

//---
//Input / UI constants
//---
const MIN_PLAYERS: usize = 2;
const MAX_PLAYERS: usize = 5;
const FUZZY_MATCH_THRESHOLD: f64 = 0.80;
const CLEAR_SCREEN: &str = "\x1b[2J";

//Structs
#[derive(Debug, Serialize, Deserialize, Clone)]
struct Player {
    name: String,
    rating: f64,
    rd: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct MatchPlayer {
    player: String,
    commander: String,
}

struct Opponent {
    effective: f64,
    deck_rd: f64,
    score: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct MatchRecord {
    players: Vec<MatchPlayer>,
    winner: String,
    match_note: String,
    turn_win: i16,
}

#[derive(Default)]
struct CommanderStats {
    games: u32,
    wins: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Decks {
    name: String,
    bracket: i16,
    type_of_deck: String,
    rating: f64,
    rd: f64,
}

//Player methods
impl Player {
    //Creates a new player with default value
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            rating: DEFAULT_RATING,
            rd: DEFAULT_RD,
        }
    }
}

//Commander methods
impl Decks {
    fn new(
        name: String,
        bracket: i16,
        type_of_deck: String,
    ) -> Self {
        Self {
            name,
            bracket,
            type_of_deck,
            rating: starting_rating_for_bracket(bracket),
            rd: DEFAULT_RD,
        }
    }
}

//---
//Fletch functions
//---

//Calculates g(RD) - reduces impacts of players with a big difference
fn g(rd: f64) -> f64 {
    1.0 / (1.0 + (G_COEFFICIENT * Q.powi(2) * rd.powi(2)) / std::f64::consts::PI.powi(2)).sqrt()
}

// Calculates the effective rating of a player using a deck.
//
// A deck rated 1500 has no effect.
// A deck rated above 1500 increases the effective rating.
// A deck rated below 1500 decreases the effective rating.
fn effective_rating(player: &Player, deck: &Decks) -> f64 {
    player.rating + (deck.rating - DEFAULT_RATING)
}

fn starting_rating_for_bracket(bracket: i16) -> f64 {
    if (MIN_BRACKET..=MAX_BRACKET).contains(&bracket) {
        BRACKET_STARTING_RATINGS[(bracket - MIN_BRACKET) as usize]
    } else {
        DEFAULT_RATING
    }
}

// Returns (rating change already multiplied by `share`, new RD)
fn glicko_update(
    own_effective: f64,
    own_rd: f64,
    share: f64,
    opponents: &[Opponent],
) -> (f64, f64) {
    let mut varience_sum = 0.0;
    let mut score_sum = 0.0;
    let mut surpise_sum = 0.0;

    for opp in opponents {
        let g_rd = g(opp.deck_rd);
        let e = 1.0 / 
            (1.0 / 10_f64.powf(-g_rd * (own_effective - opp.effective) / RATING_SCALE));
        
        varience_sum += g_rd.powi(2) * e * (1.0 - e);
        score_sum += g_rd * (opp.score - e);
        surpise_sum += (opp.score - e).abs();
    }

    let d2 = 1.0 / (Q.powi(2) * varience_sum);
    let precision = 1.0 / own_rd.powi(2) + 1.0 / d2;

    let rating_change = Q / precision * score_sum * share;

    let mut new_rd = precision.powf(-0.5).min(MAX_RD);

    // Surprise adjustment, avereged over the opponents faced
    let surprise  = surpise_sum / opponents.len() as f64;
    if surprise > SURPRISE_THRESHOLD {
        new_rd += (SURPRISE_THRESHOLD - surprise) * SURPRISE_RD_INCREASE_SCALE;
    } else {
        new_rd -= (SURPRISE_THRESHOLD - surprise) * SURPRISE_RD_DECREASE_SCALE;
    }

    (rating_change, new_rd.clamp(MIN_RD, MAX_RD))
}

//---
//File function
//---

fn load_json<T>(path: &str) -> Vec<T>
where
    T: for<'de> Deserialize<'de>,
{
    //
    // FILE DOES NOT EXIST
    //

    if !Path::new(path).exists() {
        println!("{} not found. Creating new file.", path);

        return Vec::new();
    }

    //
    // READ FILE
    //

    let data =
        fs::read_to_string(path)
            .expect("Unable to read file");

    //
    // PARSE JSON
    //

    match serde_json::from_str(&data) {

        Ok(result) => result,

        Err(error) => {

            println!(
                "Failed to parse {}: {}",
                path,
                error
            );

            Vec::new()
        }
    }
}

fn save_json<T>(path: &str, data: &[T]) where T: Serialize, {
    let json = serde_json::to_string_pretty(data).expect("Unable to write file");

    //Write
    fs::write(path, json).expect("Unable to write to file");
}

//---
//Input functions - Basic input function
//---

fn input(prompt: &str) -> String {
    println!("{}", prompt);

    //Ensure prompt appears immediatley
    io::stdout().flush().unwrap();
    //buffer to store imput
    let mut buffer = String::new();

    //Read input line
    io::stdin().read_line(&mut buffer).unwrap();

    buffer.trim().to_string()
}

//Clear the terminal to keep things tidy
fn clear_terminal() {
    print!("{}", CLEAR_SCREEN);
    std::io::stdout().flush().unwrap();
}

//---
//fuzzy serach
//---
fn find_player(players: &[Player], query: &str) -> Option<usize> {
    let query = query.to_lowercase();

     players.iter().enumerate().map(|(i, p)| {
            (
                i,
                jaro_winkler(
                    &query,
                    &p.name.to_lowercase(),
                ),
            )
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .and_then(|(index, score)| {
            if score > FUZZY_MATCH_THRESHOLD {
                Some(index)
            } else {
                None
            }
        })
}

fn find_deck(decks: &[Decks], query: &str) -> Option<usize> {
    let query = query.trim().to_lowercase();

    // First: exact match
    if let Some((index, _)) = decks.iter().enumerate()
        .find(|(_, deck)| deck.name.trim().to_lowercase() == query)
    {
        return Some(index);
    }

    // Second: fuzzy match
    decks.iter()
        .enumerate()
        .map(|(i, deck)| {
            (
                i,
                jaro_winkler(
                    &query,
                    &deck.name.to_lowercase(),
                ),
            )
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .and_then(|(index, score)| {
            if score > FUZZY_MATCH_THRESHOLD {
                Some(index)
            } else {
                None
            }
        })
}

//---
//Add match
//---
fn add_match(players: &mut Vec<Player>, matches: &mut Vec<MatchRecord>, decks: &mut Vec<Decks>,) {
    println!("");
    println!("\n=== ADD MATCH ===");

    //
    // PLAYER COUNT
    //

    let player_count: usize =
    input(&format!("How many players? ({}-{}): ", MIN_PLAYERS, MAX_PLAYERS))
        .parse()
        .unwrap_or(MIN_PLAYERS);

    if !(MIN_PLAYERS..=MAX_PLAYERS).contains(&player_count) {
        println!("Invalid player count.");
        return;
    }

    // SELECT PLAYERS
    let mut selected_indexes = Vec::new();
    let mut selected_deck_indexes: Vec<usize> = Vec::new();
    let mut selected_players: Vec<MatchPlayer> = Vec::new();

    for i in 0..player_count {

        println!("\nSelect player {}", i + 1);

        // Ask for a player's name instead of a number.
        let name = input("Player name: ");

        // Try to find an existing player using fuzzy search.
        let player_index = match find_player(players, &name) {

            // A matching player was found.
            Some(index) => {
                println!("Matched with '{}'.", players[index].name);
                index
            }

            // No suitable match exists.
            None => {

                println!("No player matched '{}'.", name);

                let create = input("Create new player? (y/n): ");

                if create.to_lowercase() != "y" {
                    println!("Player selection cancelled.");
                    return;
                }

                // Create a brand-new player.
                players.push(Player::new(&name));

                println!("Created player '{}'.", name);

                players.len() - 1
            }
        };

        //
        // PREVENT DUPLICATE PLAYERS
        //

        if selected_indexes.contains(&player_index) {
            println!("Player already selected.");
            return;
        }

        selected_indexes.push(player_index);

        //
        // RECORD THE COMMANDER USED
        //

        let commander_name = input("Commander or deck name: ");

        let commander_index = match find_deck(decks, &commander_name) {
        Some(index) => {
            println!("Matched with '{}'.", decks[index].name);
            index
        }

        None => {
            println!("No commander matched '{}'.", commander_name);

            let create = input("Create new commander? (y/n): ");

            if create.to_lowercase() != "y" {
                println!("Cancelled.");
                return;
            }

            println!("=== Add Deck ===");

            println!("Enter deck name:");
            let name = input("");

            println!("Enter deck bracket ({}-{}):", MIN_BRACKET, MAX_BRACKET);
            let bracket: i16 = loop {
                let value = input("");

                match value.parse::<i16>() {
                    Ok(bracket) if (MIN_BRACKET..=MAX_BRACKET).contains(&bracket) => break bracket,
                    _ => println!("Please enter a bracket between {} and {}:", MIN_BRACKET, MAX_BRACKET),
                }
            };

            println!("Enter deck type:");
            let type_of_deck = input("");

            let deck = Decks::new(
                name,
                bracket,
                type_of_deck,
            );

            decks.push(deck);

            println!("Deck added successfully!");

            println!("Created commander '{}'.", commander_name);
            decks.len() - 1

            
        }
    };

    selected_deck_indexes.push(commander_index);

    let commander = decks[commander_index].name.clone();
        selected_players.push(MatchPlayer {player: players[player_index].name.clone(),commander,});
    }

    // SELECT WINNER
    println!("\nWho won?");

    for (i, player_index) in selected_indexes.iter().enumerate() {
        println!("{}. {}", i + 1, players[*player_index].name);
    }

    let winner_choice: usize =
        input("Winner number: ").parse().expect("Invalid number");

    if winner_choice == 0 || winner_choice > selected_indexes.len() {
        println!("Invalid winner.");
        return;
    }

    // Turn that the game was won on
    let winning_turn: i16 = loop {
        match input("Turn that it was won on: ").parse::<i16>() {
            Ok(turn) if turn > 0 => break turn,
            _ => println!("Please enter a whole number greater than 0."),
        }
    };

    // Any notes
    let note = input("Match note: ");

    let winner_index = selected_indexes[winner_choice - 1];

    // CLONE ORIGINAL RATINGS
        // Snapshot ratings so every calculation uses pre-match values.
    let snap_players = players.clone();
    let snap_decks = decks.clone();

    let winner_pos = winner_choice - 1;
    let count = selected_indexes.len();

    let effective_at = |pos: usize| {
        effective_rating(
            &snap_players[selected_indexes[pos]],
            &snap_decks[selected_deck_indexes[pos]],
        )
    };

    // (player idx, deck idx, player change, player RD, deck change, deck RD)
    let mut updates: Vec<(usize, usize, f64, f64, f64, f64)> = Vec::new();

    for pos in 0..count {
        let is_winner = pos == winner_pos;
        let score = if is_winner { WIN_SCORE } else { LOSS_SCORE };

        // The winner faces everyone; each loser faces only the winner.
        let opponents: Vec<Opponent> = (0..count)
            .filter(|&o| o != pos && (is_winner || o == winner_pos))
            .map(|o| Opponent {
                effective: effective_at(o),
                deck_rd: snap_decks[selected_deck_indexes[o]].rd,
                score,
            })
            .collect();

        let player_idx = selected_indexes[pos];
        let deck_idx = selected_deck_indexes[pos];
        let own_effective = effective_at(pos);

        let (p_change, p_rd) = glicko_update(
            own_effective, snap_players[player_idx].rd, PLAYER_SHARE, &opponents);
        let (d_change, d_rd) = glicko_update(
            own_effective, snap_decks[deck_idx].rd, DECK_SHARE, &opponents);

        updates.push((player_idx, deck_idx, p_change, p_rd, d_change, d_rd));
    }

    // Apply everything at once.
    for (pi, di, p_change, p_rd, d_change, d_rd) in updates {
        players[pi].rating += p_change;
        players[pi].rd = p_rd;
        decks[di].rating += d_change;
        decks[di].rd = d_rd;
    }

    // Save the match so history and winrates can see it.
    matches.push(MatchRecord {
        players: selected_players,
        winner: players[winner_index].name.clone(),
        match_note: note,
        turn_win: winning_turn,
    });
}

//---
//Show Rankings
//---

fn show_rankings(
    players: &[Player],
    matches: &[MatchRecord],
    decks: &[Decks],
) {
    //Show menu
    println!();
    println!("=== Player Rankings Menu ===");

    println!("1. Player Rankings");
    println!("2. Deck Rankings");
    println!("3. Commander winrates");

    let choice = input("Select option: ");

    match choice.as_str() {
    "1" => {
        clear_terminal();

        println!("=== Player Rankings ===");
        println!("1. Rank by Score");
        println!("2. Rank by Conservative Score");
        println!("3. Rank by Supreme Score");
        println!("4. Rank by Lowest RD");

        let ranking_choice = input("Select option: ");

        match ranking_choice.as_str() {
            "1" => rank_by_score(players),
            "2" => rank_by_rd_substract(players),
            "3" => rank_by_rd_addition(players),
            "4" => rank_by_rd(players),
            _ => println!("Invalid input."),
        }
    }

    "2" => {
        clear_terminal();
        deck_ranking_menu(decks);
    }

    "3" => {
        clear_terminal();
        commander_winrate_menu(players, matches);
    }

    _ => {
        clear_terminal();
        println!("Invalid input.");
    }
}

    println!();
}

fn rank_decks(decks: &[Decks]) {
    println!();
    println!("=== Deck Rankings ===");

    let mut sorted = decks.to_vec();

    sorted.sort_by(|a, b| {
        b.rating
            .partial_cmp(&a.rating)
            .unwrap()
    });

    for (index, deck) in sorted.iter().enumerate() {
        println!(
        "{}. {} - Rating: {:.2}, RD: {:.2}, Bracket: {}, Type: {}",
        index + 1,
        deck.name,
        deck.rating,
        deck.rd,
        deck.bracket,
        deck.type_of_deck
        );
    }

    println!();
}

fn commander_winrate_menu(players: &[Player], matches: &[MatchRecord]) {
    println!("");
    println!("=== Commander Menu ===");

    println!("1. All commander winrates");
    println!("2. Player specific commander winrate");

    let choice = input("Select option: ");

    match choice.as_str() {
        "1" => {
            clear_terminal();
            show_commander_winrates(matches);
        }
        "2" => {
            clear_terminal();
            show_player_commanders(players, matches);
        }
        _ => {
            clear_terminal();
            println!("Invalid input.");
        }
    }
}

fn rank_by_score(players: &[Player]) {    
    //Print
    println!("");
    println!("=== Rank by Score ===");
    
    //Clone players
    let mut sorted = players.to_vec();

    //sort by ranking accending
    sorted.sort_by(|a, b| {b.rating.partial_cmp(&a.rating).unwrap()});

    //Display rankings based on score
    for (index, player) in sorted.iter().enumerate() {
        println!("{}. {} - Rating: {:.2}, RD: {:.2}", index + 1, player.name, player.rating, player.rd); 
    }
    println!("");
}

fn rank_by_rd_substract(players: &[Player]) {
    println!("");
    println!("=== Rank by Lowest Score ===");

    //Clone players
    let mut sorted = players.to_vec();

    sorted.sort_by(|a, b| {(b.rating - b.rd).partial_cmp(&(a.rating - a.rd)).unwrap()});

    for (index, player) in sorted.iter().enumerate() {
        println!("{}. {} - Conservative Rating: {:.2} (Rating: {:.2}, RD: {:.2})",index + 1,player.name,player.rating - player.rd,player.rating,player.rd);
    }
    println!();
}

fn rank_by_rd_addition(players: &[Player]) {
    println!("");
    println!("=== Rank by Highest Score ===");

    //Clone players
    let mut sorted = players.to_vec();

    sorted.sort_by(|a, b| {(b.rating + b.rd).partial_cmp(&(a.rating + a.rd)).unwrap()});

    for (index, player) in sorted.iter().enumerate() {
        println!("{}. {} - Supreme Rating: {:.2} (Rating: {:.2}, RD: {:.2})",index + 1,player.name,player.rating + player.rd,player.rating,player.rd);
    }
}

fn rank_by_rd(players: &[Player]) {
    println!();
    println!("=== Rank by Lowest RD ===");

    let mut sorted = players.to_vec();

    sorted.sort_by(|a, b| {a.rd.partial_cmp(&b.rd).unwrap()});

    for (index, player) in sorted.iter().enumerate() {
        println!("{}. {} - RD: {:.2}, Rating: {:.2}", index + 1, player.name, player.rd, player.rating);
    }

    println!();
}

fn rank_decks_by_conservative(decks: &[Decks]) {
    println!();
    println!("=== Deck Conservative Ranking ===");

    let mut sorted = decks.to_vec();

    sorted.sort_by(|a, b| {
        (b.rating - b.rd)
            .partial_cmp(&(a.rating - a.rd))
            .unwrap()
    });

    for (index, deck) in sorted.iter().enumerate() {
        println!(
            "{}. {} - Conservative Rating: {:.2} (Rating: {:.2}, RD: {:.2})",
            index + 1,
            deck.name,
            deck.rating - deck.rd,
            deck.rating,
            deck.rd
        );
    }

    println!();
}

fn rank_decks_by_supreme(decks: &[Decks]) {
    println!();
    println!("=== Deck Supreme Ranking ===");

    let mut sorted = decks.to_vec();

    sorted.sort_by(|a, b| {
        (b.rating + b.rd)
            .partial_cmp(&(a.rating + a.rd))
            .unwrap()
    });

    for (index, deck) in sorted.iter().enumerate() {
        println!(
            "{}. {} - Supreme Rating: {:.2} (Rating: {:.2}, RD: {:.2})",
            index + 1,
            deck.name,
            deck.rating + deck.rd,
            deck.rating,
            deck.rd
        );
    }

    println!();
}

//View commander winrates
fn show_commander_winrates(matches: &[MatchRecord]) {
    println!("");
    println!("=== Commander winrates");

    let mut stats: HashMap<(String, String), CommanderStats> = HashMap::new();

    //Read each match
    for m in matches {
        //Check each player
        for p in &m.players {
            //Get or create entry
            let key = (p.player.clone(), p.commander.clone(),);
            let entry = stats.entry(key).or_default();

            //Record game players
            entry.games += 1;
            //Record win
            if p.player == m.winner {
                entry.wins += 1;
            }
        }
    }

    //Convert to vector
    let mut results: Vec<_> = stats.into_iter().collect();

    //Sort by winrate
    results.sort_by(|a, b| {
        let a_rate = a.1.wins as f64 / a.1.games as f64;
        let b_rate = b.1.wins as f64 / b.1.games as f64;

        b_rate.partial_cmp(&a_rate).unwrap()
    });

    //Display results
    for ((player, commander), stat) in results {
        let win_rate = stat.wins as f64 / stat.games as f64 * 100.0;

        println!(
            "{} - {} : {:.1}% ({}/{})", player, commander, win_rate, stat.wins, stat.games,);
    }
    println!();
}

// Show commander win rates for a specific player
fn show_player_commanders(players: &[Player],matches: &[MatchRecord],) {
    println!();
    println!("=== Player Commander Statistics ===");

    // Find player with fuzzy search
    let name = input("Enter player name: ");

    let player_index = match find_player(players, &name) {
        // Player found
        Some(index) => {
            println!("Matched with '{}'.", players[index].name);index
        }

        // Player not found
        None => {
            println!("No player matched '{}'.", name);
            return;
        }
    };

    // Store the correct player name
    let player_name = &players[player_index].name;

    // Commander stats
    let mut stats: HashMap<String, CommanderStats> =
        HashMap::new();

    //Read through every match
    for m in matches {
        //Check players
        for p in &m.players {
            //Ignore other players
            if p.player != *player_name {
                continue;
            }
            // Get or create commander entrance
            let entry = stats.entry(p.commander.clone()).or_default();

            //Record game
            entry.games += 1;

            // RECORD WIN
            if p.player == m.winner {
                entry.wins += 1;
            }
        }
    }

    //If the player has 0 games
    if stats.is_empty() {
        println!("No commander data found.");
        return;
    }

    //Sort results by wins
    let mut results: Vec<_> = stats.into_iter().collect();

    results.sort_by(|a, b| {
        let a_rate = a.1.wins as f64 / a.1.games as f64;

        let b_rate = b.1.wins as f64 / b.1.games as f64;

        b_rate.partial_cmp(&a_rate).unwrap()
    });

    //Display results
    println!();
    println!("=== {}'s Commanders ===", player_name);

    for (commander, stat) in results {

        let win_rate = stat.wins as f64 / stat.games as f64 * 100.0;

        println!("{} - {:.1}% ({}/{})", commander, win_rate, stat.wins, stat.games,);
    }
}

//---
//Show Match History
//---

//Displays the match history menu
fn show_match_menu(matches: &[MatchRecord]) {
    //Show menu
    println!("");
    println!("=== MATCH HISTORY ===");

    println!("1. Show all matches");
    println!("2. Search specific player results");
    

    let choice = input("Select option: ");

    match choice.as_str() {
        "1" => {
            clear_terminal();
            show_matches(matches);
        }
        "2" => {
            clear_terminal();
            search_player_results(matches);
        }
        _ => {
            println!("Invalid input.");
        }
    }
}

//Full match history
fn show_matches(matches: &[MatchRecord]) {
    println!("");
    println!("=== MATCH HISTORY ===");

    for (i, m) in matches.iter().enumerate() {

        println!("\nMatch {}", i + 1);

        println!("Players:");

        for player in &m.players {
            println!("- {} ({})", player.player, player.commander);
        }

        println!("Winner: {}", m.winner);
        println!("Turn won on: {}", m.turn_win);
        println!("Match note: {}", m.match_note);
    }
}

//Show WINS and LOSSES for chosen player
fn search_player_results(matches: &[MatchRecord]) {
    println!("\n=== Search Player ===");

    let name = input("Enter player name: ");

    let mut wins = 0;
    let mut losses = 0;
    let mut found = false;

    for m in matches {
        if m.players.iter().any(|p| p.player.eq_ignore_ascii_case(&name)) {
            found = true;

            if m.winner.eq_ignore_ascii_case(&name) {
                wins += 1;
            } else {
                losses += 1;
            }
        }
    }

    if !found {
        println!("\nPlayer '{}' was not found.", name);
        return;
    }

    let total = wins + losses;

    println!("\n=== Summary ===");
    println!("Total:  {}", total);
    println!("Wins:   {}", wins);
    println!("Losses: {}", losses);
}

//Deck ranking menu
fn deck_ranking_menu(decks: &[Decks]) {
    println!();
    println!("=== Deck Rankings Menu ===");

    println!("1. Rank by Score");
    println!("2. Rank by Conservative Score");
    println!("3. Rank by Supreme Score");
    println!("4. Rank by Lowest RD");

    let choice = input("Select option: ");

    match choice.as_str() {
        "1" => {
            clear_terminal();
            rank_decks(decks);
        }

        "2" => {
            clear_terminal();
            rank_decks_by_conservative(decks);
        }

        "3" => {
            clear_terminal();
            rank_decks_by_supreme(decks);
        }

        "4" => {
            clear_terminal();

            let mut sorted = decks.to_vec();

            sorted.sort_by(|a, b| {
                a.rd.partial_cmp(&b.rd).unwrap()
            });

            println!("=== Decks by Lowest RD ===");

            for (index, deck) in sorted.iter().enumerate() {
                println!(
                    "{}. {} - RD: {:.2}, Rating: {:.2}",
                    index + 1,
                    deck.name,
                    deck.rd,
                    deck.rating
                );
            }
        }

        _ => {
            clear_terminal();
            println!("Invalid input.");
        }
    }

    println!();
}

fn main() {
    //File paths
    let players_file = "players.json";
    let matches_file = "matches.json";
    let decks_file = "decks.json";

    //Load save data
    let mut players: Vec<Player> = load_json(players_file);
    let mut matches: Vec<MatchRecord> = load_json(matches_file);
    let mut decks: Vec<Decks> = load_json(decks_file);

    //Main menu - loop
    loop {
        println!("");
        println!("=== MTG Fletch Rating System ===");

        println!("1. Add Match");
        println!("2. Show Ranking");
        println!("3. Show Match History");
        println!("4. Save and Exit");

        let choice = input("Select option: ");

        //Menu handling
        match choice.as_str() {
            "1" => {
                add_match(&mut players, &mut matches, &mut decks); 
            }
            "2" => {
                clear_terminal();
                show_rankings(&players, &matches, &decks);
            }
            "3" => {
                show_match_menu(&matches);
            }
            "4" => {
                save_json(players_file, &players);
                save_json(matches_file, &matches);
                save_json(decks_file, &decks);

                println!("Data saved");

                break;
            }
            _ => {
                println!("Invalid input.");
            }
        }
    }
}
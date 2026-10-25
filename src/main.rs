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

// Updates a deck's rating.
//
// Decks receive a smaller portion of the rating change
// because the main rating is intended to represent the player.
fn update_deck_rating(
    deck: &mut Decks,
    opponent_deck: &Decks,
    player: &Player,
    opponent_player: &Player,
    score: f64,
) {
    // Effective ratings include the player using each deck.
    let player_effective = effective_rating(player, deck);
    let opponent_effective = effective_rating(opponent_player, opponent_deck);

    // Use the opponent deck's RD.
    let g_rd = g(opponent_deck.rd);

    // Expected result based on player + deck strength.
    let e = 1.0 / (
        1.0
        + 10_f64.powf(
            -g_rd * (player_effective - opponent_effective) / RATING_SCALE
        )
    );

    // Variance.
    let d2 = 1.0 / (
        Q.powi(2)
        * g_rd.powi(2)
        * e
        * (1.0 - e)
    );

    // Rating adjustment factor.
    let pre_factor =
        Q / ((1.0 / deck.rd.powi(2)) + (1.0 / d2));

    // Calculate the normal rating change.
    let rating_change =
        pre_factor * g_rd * (score - e);

    // Only 30% of the total change goes to the deck.
    deck.rating += rating_change * DECK_SHARE;

    // Update RD.
    deck.rd = (
        (1.0 / deck.rd.powi(2))
        + (1.0 / d2)
    )
    .powf(-0.5)
    .min(MAX_RD);

    // Surprise factor.
    let surprise = (score - e).abs();

    if surprise > SURPRISE_THRESHOLD {
        deck.rd += (surprise - SURPRISE_THRESHOLD) * SURPRISE_RD_INCREASE_SCALE;
    } else {
        deck.rd -= (SURPRISE_THRESHOLD - surprise) * SURPRISE_RD_DECREASE_SCALE;
    }

    // Clamp RD.
    deck.rd = deck.rd.clamp(MIN_RD, MAX_RD);
}


fn starting_rating_for_bracket(bracket: i16) -> f64 {
    if (MIN_BRACKET..=MAX_BRACKET).contains(&bracket) {
        BRACKET_STARTING_RATINGS[(bracket - MIN_BRACKET) as usize]
    } else {
        DEFAULT_RATING
    }
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
            if score > 0.80 {
                Some(index)
            } else {
                None
            }
        })
}

//---
//Add match
//---
fn add_match(players: &mut Vec<Player>, _matches: &mut Vec<MatchRecord>, decks: &mut Vec<Decks>,) {
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

    //Turn that the game was won on
    let _winning_turn: i16 = input("Turn that it was won on: ").parse().expect("N/A");

    //Any notes
    let _note = input("Match note: ");

    let winner_index = selected_indexes[winner_choice - 1];

   // CLONE ORIGINAL RATINGS
    //
    // We MUST use the original ratings for every calculation.
    // Otherwise the first opponent could change the rating
    // used against the second opponent.
    let original_players = players.clone();
    let original_decks = decks.clone();

    // WINNER BEATS EVERYONE
    for loser_index in &selected_indexes {
        if *loser_index == winner_index {
        continue;
    }

    // -----------------------------
    // FIND WINNER'S DECK
    // -----------------------------

    let winner_match_player =
        selected_players
            .iter()
            .find(|p| {
                p.player == original_players[winner_index].name
            })
            .unwrap();

    // -----------------------------
    // FIND LOSER'S DECK
    // -----------------------------

    let loser_match_player =
        selected_players
            .iter()
            .find(|p| {
                p.player == original_players[*loser_index].name
            })
            .unwrap();

    // Find deck indexes.
    let winner_deck_index =
        find_deck(
            &original_decks,
            &winner_match_player.commander,
        )
        .unwrap();

    let loser_deck_index =
        find_deck(
            &original_decks,
            &loser_match_player.commander,
        )
        .unwrap();

    let winner_original_player =
        original_players[winner_index].clone();

    let loser_original_player =
        original_players[*loser_index].clone();

    let winner_original_deck =
        original_decks[winner_deck_index].clone();

    let loser_original_deck =
        original_decks[loser_deck_index].clone();

    // -----------------------------
    // UPDATE PLAYER RATINGS
    // -----------------------------

    update_player_rating_with_decks(
        &mut players[winner_index],
        &loser_original_player,
        &winner_original_deck,
        &loser_original_deck,
        WIN_SCORE,
    );

    update_player_rating_with_decks(
        &mut players[*loser_index],
        &winner_original_player,
        &loser_original_deck,
        &winner_original_deck,
        LOSS_SCORE,
    );

    // -----------------------------
    // UPDATE DECK RATINGS
    // -----------------------------

    update_deck_rating(
        &mut decks[winner_deck_index],
        &loser_original_deck,
        &winner_original_player,
        &loser_original_player,
        WIN_SCORE,
    );

    update_deck_rating(
        &mut decks[loser_deck_index],
        &winner_original_deck,
        &loser_original_player,
        &winner_original_player,
        LOSS_SCORE,
        );
    }
}

// Updates a player's rating using the effective rating
// of their opponent's player + deck combination.
fn update_player_rating_with_decks(
    player: &mut Player,
    opponent: &Player,
    player_deck: &Decks,
    opponent_deck: &Decks,
    score: f64,
) {
    // Effective ratings.
    let player_effective =
        effective_rating(player, player_deck);

    let opponent_effective =
        effective_rating(opponent, opponent_deck);

    let g_rd = g(opponent_deck.rd);

    // Expected score.
    let e = 1.0 / (
        1.0
        + 10_f64.powf(
            -g_rd
                * (player_effective - opponent_effective)
                / RATING_SCALE
        )
    );

    // Variance.
    let d2 = 1.0 / (
        Q.powi(2)
        * g_rd.powi(2)
        * e
        * (1.0 - e)
    );

    // Rating adjustment factor.
    let pre_factor =
        Q / ((1.0 / player.rd.powi(2)) + (1.0 / d2));

    // Actual rating change.
    let rating_change =
        pre_factor * g_rd * (score - e);

    // Player receives 70%.
    player.rating += rating_change * PLAYER_SHARE;

    // Update RD.
    player.rd = (
        (1.0 / player.rd.powi(2))
        + (1.0 / d2)
    )
    .powf(-0.5)
    .min(350.0);

    // Surprise factor.
    let surprise = (score - e).abs();

    if surprise > 0.5 {
        let increase = (surprise - 0.5) * 40.0;
        player.rd += increase;
    } else {
        let decrease = (0.5 - surprise) * 10.0;
        player.rd -= decrease;
    }

    // Clamp RD.
    player.rd = player.rd.clamp(MIN_RD, MAX_RD);
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
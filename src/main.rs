//To anyone reading this code, make sure to run it in powershell with cargo run

//MTG Fletch rating system
use serde::{Deserialize, Serialize};
use std::fs;
use std::hash::Hash;
use std::io::{self, Write};
use std::path::Path;
use strsim::jaro_winkler;
use std::collections::HashMap;

//FLETCH consts
const Q: f64 = std::f64::consts::LN_10 / 400.0;
const DEFAULT_RATING: f64 = 1500.0;
const DEFAULT_RD: f64 = 350.0;

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
}

#[derive(Default)]
struct CommanderStats {
    games: u32,
    wins: u32,
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

//---
//Fletch functions
//---

//Calculates g(RD) - reduces impacts of players with a big difference
fn g(rd: f64) -> f64 {
    1.0 / (1.0 + (3.0 * Q.powi(2) * rd.powi(2)) / std::f64::consts::PI.powi(2)).sqrt()
}

//Calculates expected score
fn expected_score(player_rating: f64, opponent_rating: f64, opponent_rd: f64) -> f64 {
    1.0 / (1.0 + 10_f64.powf(-g(opponent_rd) * (player_rating - opponent_rating) / 400.0,))
}

//Updates a player's rating
//1.0 is a win
//0.5 is a Draw
//0.0 is a loss
fn update_rating(player: &mut Player, opponent: &Player, score: f64) {
    let g_rd = g(opponent.rd);

    //expected score
    let e = expected_score(player.rating, opponent.rating, opponent.rd);

    //variance
    let d2 = 1.0 / (Q.powi(2) * g_rd.powi(2) * e * (1.0 - e));

    //Rating adjustment factor
    let pre_factor = Q / ((1.0 / player.rd.powi(2)) + (1.0 / d2));

    //Actual rating change
    let rating_change = pre_factor * g_rd * (score - e);
    //Apply rating change
    player.rating += rating_change;

    //Update rating deviation
    player.rd = ((1.0 / player.rd.powi(2)) + (1.0 / d2)).powf(-0.5).min(350.0);

    //Surprise factor
    
    let surprise = (score - e).abs();

    // EXPECTED RESULTS: LOWER RD
    // SURPRISING RESULTS: RAISE RD

    if surprise > 0.5 {

        // Increase uncertainty

        let increase = (surprise - 0.5) * 40.0;

        player.rd += increase;

    } else {

        // Increase confidence

        let decrease = (0.5 - surprise) * 10.0;

        player.rd -= decrease;
    }

    // CLAMP RD

    player.rd = player.rd.clamp(
        30.0,   // Minimum RD
        350.0,  // Maximum RD
    );
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
    print!("\x1b[2J");
    std::io::stdout().flush().unwrap();
}

//---
//find player with fuzzy search
//---
fn find_player(players: &[Player], query: &str) -> Option<usize> {
    let query = query.to_lowercase();

     players
        .iter()
        .enumerate()
        .map(|(i, p)| {
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
fn add_match(players: &mut Vec<Player>, matches: &mut Vec<MatchRecord>,) {
    println!("");
    println!("\n=== ADD MATCH ===");

    //
    // PLAYER COUNT
    //

    let player_count: usize =
        input("How many players? (2-4): ")
            .parse()
            .unwrap_or(2);

    if player_count < 2 || player_count > 4 {
        println!("Invalid player count.");
        return;
    }

    //
    // SELECT PLAYERS
    //

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

        let commander = input("Commander or deck name: ");

        selected_players.push(MatchPlayer {
            player: players[player_index].name.clone(),
            commander,
        });
    }

    //
    // SELECT WINNER
    //

    println!("\nWho won?");

    for (i, player_index) in
        selected_indexes.iter().enumerate()
    {
        println!(
            "{}. {}",
            i + 1,
            players[*player_index].name
        );
    }

    let winner_choice: usize =
        input("Winner number: ")
            .parse()
            .expect("Invalid number");

    if winner_choice == 0
        || winner_choice > selected_indexes.len()
    {
        println!("Invalid winner.");
        return;
    }

    let winner_index =
        selected_indexes[winner_choice - 1];

    //
    // CLONE ORIGINAL RATINGS
    //

    let originals = players.clone();

    //
    // WINNER BEATS EVERYONE
    //

    for loser_index in &selected_indexes {

        if *loser_index == winner_index {
            continue;
        }

        let winner_original =
            originals[winner_index].clone();

        let loser_original =
            originals[*loser_index].clone();

        update_rating(
            &mut players[winner_index],
            &loser_original,
            1.0,
        );

        update_rating(
            &mut players[*loser_index],
            &winner_original,
            0.0,
        );
    }

    //
    // STORE MATCH
    //

    matches.push(MatchRecord {
        players: selected_players,
        winner: players[winner_index]
            .name
            .clone(),
    });

    println!("Match recorded.");
    println!("");
}

//---
//Show Rankings
//---

fn show_rankings(players: &[Player], matches: &[MatchRecord]) {
    //Show menu
    println!();
    println!("=== Player Rankings Menu ===");

    println!("1. Rank by Score");
    println!("2. Rank by Conservative Score (Rating - RD)");
    println!("3. Rank by Supreme Score (Rating + RD)");
    println!("4. Rank by Lowest RD");
    println!("5. Commander winrates");

    let choice = input("Select option: ");

    match choice.as_str() {
        "1" => {
            clear_terminal();
            rank_by_score(players);
        }
        "2" => {
            clear_terminal();
            rank_by_rd_substract(players);
        }
        "3" => {
            clear_terminal();
            rank_by_rd_addition(players);
        }
        "4" => {
            clear_terminal();
            rank_by_rd(players);
        }
        "5" => {
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

fn main() {
    //File paths
    let players_file = "players.json";
    let matches_file = "matches.json";

    //Load save data
    let mut players: Vec<Player> = load_json(players_file);
    let mut matches: Vec<MatchRecord> = load_json(matches_file);

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
                add_match(&mut players, &mut matches);
            }
            "2" => {
                clear_terminal();
                show_rankings(&players, &matches);
            }
            "3" => {
                show_match_menu(&matches);
            }
            "4" => {
                save_json(players_file, &players);
                save_json(matches_file, &matches);

                println!("Data saved");

                break;
            }
            _ => {
                println!("Invalid input.");
            }
        }
    }
}
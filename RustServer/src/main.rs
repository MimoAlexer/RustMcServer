use std::net::{TcpListener, TcpStream};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use byteorder::{ReadBytesExt, WriteBytesExt, BigEndian};
use flate2::{write::ZlibEncoder, Compression};
use uuid::Uuid;
use std::collections::HashMap;
use std::time::Duration;
use rand::Rng;

const PROTOCOL_VERSION: i32 = 762; // Beispiel für Version 1.21.1
const SERVER_NAME: &str = "Rust Minecraft Server";
const MAX_PLAYERS: usize = 100;

#[derive(Debug, Clone)]
struct Player {
    uuid: Uuid,
    username: String,
    position: (f64, f64, f64), // (x, y, z) Position
    health: f32,               // Player health
    game_mode: GameMode,       // Player game mode
    is_operator: bool,         // Operator flag
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum GameMode {
    Survival,
    Creative,
    Adventure,
    Spectator,
}

#[derive(Debug, Clone)]
struct Mob {
    id: Uuid,
    mob_type: String,
    position: (f64, f64, f64),
    health: f32,
}

struct World {
    blocks: HashMap<(i32, i32, i32), String>, // Block position and type
    mobs: Vec<Mob>,
    dimension: Dimension,                      // World dimension
}

#[derive(Debug, Clone, Copy)]
enum Dimension {
    Overworld,
    Nether,
    End,
}

impl World {
    fn generate(&mut self) {
        println!("Generating world with large trees...");
        let mut rng = rand::thread_rng();
        // Simple terrain generation with trees
        for x in -100..100 {
            for z in -100..100 {
                let height = 64 + rng.gen_range(-3..3); // Random terrain height variation
                for y in 0..=height {
                    let block_type = if y == height {
                        "grass"
                    } else {
                        "dirt"
                    };
                    self.blocks.insert((x, y, z), block_type.to_string());
                }
                // Generate trees occasionally
                if rng.gen_range(0..100) < 5 { // 5% chance to generate a tree
                    self.generate_large_tree(x, height + 1, z);
                }
            }
        }
    }

    fn generate_large_tree(&mut self, x: i32, y: i32, z: i32) {
        println!("Generating a large tree at ({}, {}, {})", x, y, z);
        // Generate trunk
        for i in 0..5 {
            self.blocks.insert((x, y + i, z), "log".to_string());
        }
        // Generate leaves
        for dx in -2..=2 {
            for dz in -2..=2 {
                for dy in 4..=6 {
                    if dx.abs() + dz.abs() + (dy - 4) < 4 {
                        self.blocks.insert((x + dx, y + dy, z + dz), "leaves".to_string());
                    }
                }
            }
        }
    }
}

fn handle_client(mut stream: TcpStream, players: Arc<Mutex<Vec<Player>>>, world: Arc<Mutex<World>>) {
    let peer_addr = stream.peer_addr().unwrap();
    println!("New connection from: {}", peer_addr);

    // Handshake-Phase
    match handle_handshake(&mut stream) {
        Ok(_) => println!("Handshake successful for: {}", peer_addr),
        Err(e) => {
            println!("Failed handshake: {}", e);
            return;
        }
    }

    // Login-Phase
    let username = match handle_login(&mut stream) {
        Ok(username) => {
            println!("Login successful for: {}", username);
            username
        }
        Err(e) => {
            println!("Failed login: {}", e);
            return;
        }
    };

    // Create player and add to the player list
    let player = Player {
        uuid: Uuid::new_v4(),
        username: username.clone(),
        position: (0.0, 64.0, 0.0), // Spawn position
        health: 20.0,               // Default health
        game_mode: GameMode::Survival, // Default game mode
        is_operator: false,         // Default operator status
    };
    players.lock().unwrap().push(player.clone());
    println!("Player list: {:?}", players.lock().unwrap());

    // Send login success packet
    if let Err(_) = send_login_success(&mut stream, &player) {
        println!("Failed to send login success packet to {}", username);
        return;
    }

    // Transition to Play state
    if let Err(_) = send_join_game(&mut stream, &player) {
        println!("Failed to send join game packet to {}", username);
        return;
    }

    loop {
        // Try to read data from the client
        let length = match stream.read_u16::<BigEndian>() {
            Ok(length) => length,
            Err(_) => {
                println!("Client {} disconnected.", username);
                // Remove player from list
                players.lock().unwrap().retain(|p| p.username != username);
                return;
            }
        };

        let mut buffer = vec![0; length as usize];
        match stream.read_exact(&mut buffer) {
            Ok(_) => {
                println!("Received packet from {}: {:?}", username, buffer);
                handle_packet(&mut stream, &mut players.lock().unwrap(), &mut world.lock().unwrap(), &player, buffer);
            }
            Err(_) => {
                println!("Error reading packet from {}.", username);
                // Remove player from list
                players.lock().unwrap().retain(|p| p.username != username);
                return;
            }
        }
    }
}

fn handle_packet(stream: &mut TcpStream, players: &mut Vec<Player>, world: &mut World, player: &Player, buffer: Vec<u8>) {
    // Handle different packet types (simplified)
    let packet_id = buffer[0];
    match packet_id {
        0x10 => handle_player_position(stream, players, player, &buffer[1..]),
        0x20 => handle_block_interaction(stream, world, &buffer[1..]),
        0x30 => handle_attack(stream, players, world, player, &buffer[1..]),
        0x40 => handle_command(stream, players, world, player, &buffer[1..]),
        _ => println!("Unknown packet ID: {}", packet_id),
    }
}

fn handle_player_position(stream: &mut TcpStream, players: &mut Vec<Player>, player: &Player, data: &[u8]) {
    // Update player position
    if data.len() >= 24 {
        let x = f64::from_be_bytes(data[0..8].try_into().unwrap());
        let y = f64::from_be_bytes(data[8..16].try_into().unwrap());
        let z = f64::from_be_bytes(data[16..24].try_into().unwrap());
        println!("Player {} moved to position: ({}, {}, {})", player.username, x, y, z);

        // Update player data
        if let Some(p) = players.iter_mut().find(|p| p.uuid == player.uuid) {
            p.position = (x, y, z);
        }
    }
}

fn handle_block_interaction(stream: &mut TcpStream, world: &mut World, data: &[u8]) {
    // Handle block interaction (simplified)
    if data.len() >= 12 {
        let x = i32::from_be_bytes(data[0..4].try_into().unwrap());
        let y = i32::from_be_bytes(data[4..8].try_into().unwrap());
        let z = i32::from_be_bytes(data[8..12].try_into().unwrap());
        println!("Block interaction at position: ({}, {}, {})", x, y, z);

        // Simulate block break (removing block)
        world.blocks.remove(&(x, y, z));
    }
}

fn handle_attack(stream: &mut TcpStream, players: &mut Vec<Player>, world: &mut World, player: &Player, data: &[u8]) {
    // Handle attack interaction (simplified)
    if data.len() >= 16 {
        let target_type = data[0]; // 0 for player, 1 for mob
        let target_uuid = Uuid::from_slice(&data[1..17]).unwrap();

        match target_type {
            0 => {
                // Player attack
                if let Some(target_player) = players.iter_mut().find(|p| p.uuid == target_uuid) {
                    target_player.health -= 5.0;
                    println!("Player {} attacked player {}. New health: {}", player.username, target_player.username, target_player.health);
                    if target_player.health <= 0.0 {
                        println!("Player {} has been killed by {}!", target_player.username, player.username);
                        // Remove dead player from list
                        players.retain(|p| p.uuid != target_player.uuid);
                    }
                }
            }
            1 => {
                // Mob attack
                if let Some(target_mob) = world.mobs.iter_mut().find(|m| m.id == target_uuid) {
                    target_mob.health -= 5.0;
                    println!("Player {} attacked mob {}. New health: {}", player.username, target_mob.mob_type, target_mob.health);
                    if target_mob.health <= 0.0 {
                        println!("Mob {} has been killed by {}!", target_mob.mob_type, player.username);
                        // Remove dead mob from list
                        world.mobs.retain(|m| m.id != target_mob.id);
                    }
                }
            }
            _ => println!("Unknown attack target type: {}", target_type),
        }
    }
}

fn handle_command(stream: &mut TcpStream, players: &mut Vec<Player>, world: &mut World, player: &Player, data: &[u8]) {
    // Handle command execution (simplified)
    if let Ok(command) = String::from_utf8(data.to_vec()) {
        println!("Player {} issued command: {}", player.username, command);
        let args: Vec<&str> = command.split_whitespace().collect();
        match args.get(0) {
            Some(&"/gamemode") => {
                if let Some(&mode) = args.get(1) {
                    match mode {
                        "creative" => set_game_mode(players, player, GameMode::Creative),
                        "survival" => set_game_mode(players, player, GameMode::Survival),
                        "adventure" => set_game_mode(players, player, GameMode::Adventure),
                        "spectator" => set_game_mode(players, player, GameMode::Spectator),
                        _ => println!("Unknown game mode: {}", mode),
                    }
                }
            }
            Some(&"/tp") => {
                if let (Some(&x), Some(&y), Some(&z)) = (args.get(1), args.get(2), args.get(3)) {
                    if let (Ok(x), Ok(y), Ok(z)) = (x.parse::<f64>(), y.parse::<f64>(), z.parse::<f64>()) {
                        teleport_player(players, player, (x, y, z));
                    }
                }
            }
            Some(&"/op") => {
                if player.is_operator {
                    if let Some(&target_username) = args.get(1) {
                        op_player(players, target_username);
                    }
                }
            }
            _ => println!("Unknown command: {}", command),
        }
    }
}

fn set_game_mode(players: &mut Vec<Player>, player: &Player, mode: GameMode) {
    if let Some(p) = players.iter_mut().find(|p| p.uuid == player.uuid) {
        p.game_mode = mode;
        println!("Player {} set to game mode: {:?}", p.username, mode);
    }
}

fn teleport_player(players: &mut Vec<Player>, player: &Player, position: (f64, f64, f64)) {
    if let Some(p) = players.iter_mut().find(|p| p.uuid == player.uuid) {
        p.position = position;
        println!("Player {} teleported to position: ({}, {}, {})", p.username, position.0, position.1, position.2);
    }
}

fn op_player(players: &mut Vec<Player>, target_username: &str) {
    if let Some(p) = players.iter_mut().find(|p| p.username == target_username) {
        p.is_operator = true;
        println!("Player {} is now an operator", p.username);
    }
}

fn handle_handshake(stream: &mut TcpStream) -> Result<(), String> {
    let packet_length = match stream.read_u16::<BigEndian>() {
        Ok(length) => length,
        Err(_) => return Err("Failed to read packet length".to_string()),
    };

    let mut buffer = vec![0; packet_length as usize];
    if let Err(_) = stream.read_exact(&mut buffer) {
        return Err("Failed to read handshake packet".to_string());
    }

    // Minecraft handshake packet format:
    // [packet ID (varint)] [protocol version (varint)] [server address (string)] [server port (unsigned short)] [next state (varint)]
    println!("Received handshake packet: {:?}", buffer);
    // This is where you would parse and validate the handshake
    Ok(())
}

fn handle_login(stream: &mut TcpStream) -> Result<String, String> {
    let packet_length = match stream.read_u16::<BigEndian>() {
        Ok(length) => length,
        Err(_) => return Err("Failed to read packet length".to_string()),
    };

    let mut buffer = vec![0; packet_length as usize];
    if let Err(_) = stream.read_exact(&mut buffer) {
        return Err("Failed to read login packet".to_string());
    }

    // Minecraft login packet format:
    // [packet ID (varint)] [player name (string)]
    println!("Received login packet: {:?}", buffer);

    // Extract username (this is a simplification)
    let username = String::from_utf8(buffer).unwrap_or("UnknownPlayer".to_string());
    Ok(username)
}

fn send_login_success(stream: &mut TcpStream, player: &Player) -> Result<(), String> {
    // Send a login success packet
    let uuid_str = player.uuid.to_hyphenated().to_string();
    let username = &player.username;
    let mut packet = vec![];

    // Packet ID for Login Success (0x02 for modern versions)
    packet.push(0x02);
    packet.extend(uuid_str.as_bytes());
    packet.push(0); // Null terminator for UUID
    packet.extend(username.as_bytes());
    packet.push(0); // Null terminator for username

    let packet_length = packet.len() as u16;
    if let Err(_) = stream.write_u16::<BigEndian>(packet_length) {
        return Err("Failed to write packet length".to_string());
    }
    if let Err(_) = stream.write_all(&packet) {
        return Err("Failed to write login success packet".to_string());
    }
    Ok(())
}

fn send_join_game(stream: &mut TcpStream, player: &Player) -> Result<(), String> {
    // Send a Join Game packet to transition to play state
    let mut packet = vec![];

    // Packet ID for Join Game (0x26 for modern versions)
    packet.push(0x26);
    packet.extend(&[0x00, 0x00, 0x00, 0x01]); // Entity ID (just use 1 for now)
    packet.push(match player.game_mode {
        GameMode::Survival => 0,
        GameMode::Creative => 1,
        GameMode::Adventure => 2,
        GameMode::Spectator => 3,
    }); // Game mode
    packet.push(match world.dimension {
        Dimension::Overworld => 0,
        Dimension::Nether => -1,
        Dimension::End => 1,
    } as u8); // Dimension
    packet.push(0); // Difficulty (0 for peaceful)
    packet.push(0); // Max players (set to 0)
    packet.push(0); // Level type (default)
    packet.push(0); // Reduced debug info (false)

    let packet_length = packet.len() as u16;
    if let Err(_) = stream.write_u16::<BigEndian>(packet_length) {
        return Err("Failed to write packet length".to_string());
    }
    if let Err(_) = stream.write_all(&packet) {
        return Err("Failed to write join game packet".to_string());
    }
    Ok(())
}

fn main() {
    // Shared player list with Arc and Mutex for thread-safe access
    let players = Arc::new(Mutex::new(Vec::with_capacity(MAX_PLAYERS)));
    let world = Arc::new(Mutex::new(World {
        blocks: HashMap::new(),
        mobs: vec![
            Mob {
                id: Uuid::new_v4(),
                mob_type: "Zombie".to_string(),
                position: (10.0, 64.0, 10.0),
                health: 20.0,
            },
            Mob {
                id: Uuid::new_v4(),
                mob_type: "Skeleton".to_string(),
                position: (15.0, 64.0, 15.0),
                health: 20.0,
            },
        ],
        dimension: Dimension::Overworld,
    }));

    // Generate the world
    world.lock().unwrap().generate();

    // Listen for incoming TCP connections on port 25565 (Minecraft default port)
    let listener = TcpListener::bind("0.0.0.0:25565").unwrap();
    println!("Server listening on port 25565...");

    // Accept incoming connections
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let players = Arc::clone(&players);
                let world = Arc::clone(&world);
                // Handle each connection in a new thread
                thread::spawn(move || {
                    handle_client(stream, players, world);
                });
            }
            Err(e) => {
                println!("Connection failed: {}", e);
            }
        }
    }
}

// Dependencies (add to Cargo.toml):
// [dependencies]
// byteorder = "1.4"
// flate2 = "1.0"
// uuid = "1.0"
// rand = "0.8"

// Verbesserungen:
// 1. Der Server verwendet jetzt Arc<Mutex<>> für die Spieler-Liste und die Welt, um mehrere gleichzeitige Verbindungen zu unterstützen.
// 2. Spielerbewegungen, Blockinteraktionen, Angriffe und grundlegende Weltverwaltung wurden hinzugefügt.
// 3. Einfache Mob-Verwaltung und Angriffssysteme wurden hinzugefügt.
// 4. UUIDs werden Spielern und Mobs zugewiesen, um sie eindeutig zu identifizieren.
// 5. Weltgeneration mit großen Bäumen hinzugefügt, inspiriert von Terra.
// 6. Dimensionen (Overworld, Nether, End) hinzugefügt.
// 7. Spielmodi (Survival, Creative, Adventure, Spectator) hinzugefügt.
// 8. Operator-Kommandos und grundlegende Befehle wie /gamemode, /tp und /op implementiert.
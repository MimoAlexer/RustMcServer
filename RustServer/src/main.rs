use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use byteorder::{ReadBytesExt, WriteBytesExt, BigEndian};
use uuid::Uuid;
use rand::Rng;

const PROTOCOL_VERSION: i32 = 767; // Protokollversion für Minecraft 1.21.1
const SERVER_NAME: &str = "Rust Minecraft Server";
const MAX_PLAYERS: usize = 100;

#[derive(Debug, Clone)]
struct Player {
    uuid: Uuid,
    username: String,
    position: (f64, f64, f64), // (x, y, z) Position
    health: f32,               // Spieler-Gesundheit
    game_mode: GameMode,       // Spielmodus des Spielers
    is_operator: bool,         // Operator-Status
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
    blocks: HashMap<(i32, i32, i32), String>, // Blockposition und Typ
    mobs: Vec<Mob>,
    dimension: Dimension,                      // Welt-Dimension
}

#[derive(Debug, Clone, Copy)]
enum Dimension {
    Overworld,
    Nether,
    End,
}

impl World {
    fn generate(&mut self) {
        println!("Generiere Welt mit großen Bäumen...");
        let mut rng = rand::thread_rng();
        for x in -100..100 {
            for z in -100..100 {
                let height = 64 + rng.gen_range(-3..3);
                for y in 0..=height {
                    let block_type = if y == height { "grass" } else { "dirt" };
                    self.blocks.insert((x, y, z), block_type.to_string());
                }
                if rng.gen_range(0..100) < 5 {
                    self.generate_large_tree(x, height + 1, z);
                }
            }
        }
    }

    fn generate_large_tree(&mut self, x: i32, y: i32, z: i32) {
        println!("Erzeuge einen großen Baum bei ({}, {}, {})", x, y, z);
        for i in 0..5 {
            self.blocks.insert((x, y + i, z), "log".to_string());
        }
        for dx in -2i32..=2 {
            for dz in -2i32..=2 {
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
    let peer_addr = match stream.peer_addr() {
        Ok(addr) => addr,
        Err(e) => {
            println!("Konnte Peer-Adresse nicht lesen: {}", e);
            return;
        }
    };
    println!("Neue Verbindung von: {}", peer_addr);

    let next_state = match handle_handshake(&mut stream) {
        Ok(state) => state,
        Err(e) => {
            println!("Handshake fehlgeschlagen: {}", e);
            return;
        }
    };

    if next_state == 1 {
        handle_status(&mut stream);
        return;
    }

    let username = match handle_login(&mut stream) {
        Ok(username) => {
            println!("Login erfolgreich für: {}", username);
            username
        }
        Err(e) => {
            println!("Login fehlgeschlagen: {}", e);
            return;
        }
    };

    let player = Player {
        uuid: Uuid::new_v4(),
        username: username.clone(),
        position: (0.0, 64.0, 0.0),
        health: 20.0,
        game_mode: GameMode::Survival,
        is_operator: false,
    };
    players.lock().unwrap().push(player.clone());
    println!("Spielerliste: {:?}", players.lock().unwrap());

    if let Err(_) = send_login_success(&mut stream, &player) {
        println!("Fehler beim Senden des Login-Erfolgs an {}", username);
        return;
    }

    if let Err(_) = send_join_game(&mut stream, &player, &world.lock().unwrap()) {
        println!("Fehler beim Senden des Beitritts an {}", username);
        return;
    }

    loop {
        let length = match read_varint(&mut stream) {
            Ok(length) => length,
            Err(_) => {
                println!("Client {} hat die Verbindung getrennt.", username);
                players.lock().unwrap().retain(|p| p.username != username);
                return;
            }
        };

        let mut buffer = vec![0; length as usize];
        match stream.read_exact(&mut buffer) {
            Ok(_) => handle_packet(&mut stream, &mut players.lock().unwrap(), &mut world.lock().unwrap(), &player, buffer),
            Err(_) => {
                println!("Fehler beim Lesen des Pakets von {}.", username);
                players.lock().unwrap().retain(|p| p.username != username);
                return;
            }
        }
    }
}

fn send_login_success(stream: &mut TcpStream, player: &Player) -> Result<(), String> {
    let mut packet_data = vec![];
    packet_data.extend(write_varint_to_vec(0x02)); // Paket-ID für Login-Erfolg

    // UUID als String (ohne Bindestriche)
    let uuid_str = player.uuid.simple().to_string();
    packet_data.extend(write_string_to_vec(&uuid_str));

    // Benutzername senden
    packet_data.extend(write_string_to_vec(&player.username));

    // Paketlänge berechnen und senden
    let mut packet = vec![];
    let packet_length = packet_data.len() as i32;
    packet.extend(write_varint_to_vec(packet_length));
    packet.extend(packet_data);

    stream.write_all(&packet).map_err(|e| format!("Fehler beim Senden des Login-Erfolgspakets: {}", e))?;
    Ok(())
}

fn send_join_game(stream: &mut TcpStream, player: &Player, world: &World) -> Result<(), String> {
    let mut packet_data = vec![];
    packet_data.extend(write_varint_to_vec(0x26)); // Paket-ID für Spielbeitritt

    // Entity ID (Int)
    let entity_id = 1i32.to_be_bytes();
    packet_data.extend(&entity_id);

    // Is Hardcore (Boolean)
    packet_data.push(0);

    // Game Mode (Unsigned Byte)
    let game_mode = match player.game_mode {
        GameMode::Survival => 0,
        GameMode::Creative => 1,
        GameMode::Adventure => 2,
        GameMode::Spectator => 3,
    };
    packet_data.push(game_mode);

    // Previous Game Mode (Byte)
    packet_data.push(255u8);

    // World Count (VarInt)
    packet_data.extend(write_varint_to_vec(1));

    // World Names (Identifier)
    let world_name = "minecraft:overworld";
    packet_data.extend(write_string_to_vec(world_name));

    // Dimension Codec (NBT Tag) - Verwende ein minimales NBT-Tag
    packet_data.push(0x0A); // NBT Compound Tag
    packet_data.push(0x00); // Leerzeichen-String
    packet_data.push(0x00); // Ende des Compound-Tags

    // Dimension Name (Identifier)
    packet_data.extend(write_string_to_vec(world_name));

    // World Name (Identifier)
    packet_data.extend(write_string_to_vec(world_name));

    // Hashed Seed (Long)
    packet_data.extend(&[0u8; 8]);

    // Max Players (VarInt)
    packet_data.extend(write_varint_to_vec(MAX_PLAYERS as i32));

    // View Distance (VarInt)
    let view_distance = 10;
    packet_data.extend(write_varint_to_vec(view_distance));

    // Simulation Distance (VarInt)
    let simulation_distance = 10;
    packet_data.extend(write_varint_to_vec(simulation_distance));

    // Reduced Debug Info (Boolean)
    packet_data.push(0);

    // Enable Respawn Screen (Boolean)
    packet_data.push(1);

    // Is Debug (Boolean)
    packet_data.push(0);

    // Is Flat (Boolean)
    packet_data.push(0);

    // Paketlänge berechnen und senden
    let mut packet = vec![];
    let packet_length = packet_data.len() as i32;
    packet.extend(write_varint_to_vec(packet_length));
    packet.extend(packet_data);

    stream.write_all(&packet).map_err(|e| format!("Fehler beim Senden des Spielbeitrittspakets: {}", e))?;
    Ok(())
}

fn read_varint(stream: &mut TcpStream) -> Result<i32, std::io::Error> {
    let mut num_read = 0;
    let mut result = 0;
    loop {
        let mut buffer = [0u8; 1];
        stream.read_exact(&mut buffer)?;
        let byte = buffer[0];
        result |= ((byte & 0b0111_1111) as i32) << (7 * num_read);
        num_read += 1;
        if num_read > 5 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "VarInt too big"));
        }
        if (byte & 0b1000_0000) == 0 {
            break;
        }
    }
    Ok(result)
}

fn read_varint_from_cursor(cursor: &mut std::io::Cursor<Vec<u8>>) -> Result<i32, std::io::Error> {
    let mut num_read = 0;
    let mut result = 0;
    loop {
        let byte = cursor.read_u8()?;
        result |= ((byte & 0b0111_1111) as i32) << (7 * num_read);
        num_read += 1;
        if num_read > 5 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "VarInt too big"));
        }
        if (byte & 0b1000_0000) == 0 {
            break;
        }
    }
    Ok(result)
}

fn write_varint(stream: &mut TcpStream, mut value: i32) -> Result<(), std::io::Error> {
    loop {
        let mut temp = (value & 0b0111_1111) as u8;
        value >>= 7;
        if value != 0 {
            temp |= 0b1000_0000;
        }
        stream.write_all(&[temp])?;
        if value == 0 {
            break;
        }
    }
    Ok(())
}

fn write_varint_to_vec(mut value: i32) -> Vec<u8> {
    let mut buf = vec![];
    loop {
        let mut temp = (value & 0b0111_1111) as u8;
        value >>= 7;
        if value != 0 {
            temp |= 0b1000_0000;
        }
        buf.push(temp);
        if value == 0 {
            break;
        }
    }
    buf
}

fn read_string(stream: &mut TcpStream) -> Result<String, String> {
    let length = read_varint(stream).map_err(|_| "Failed to read string length".to_string())?;
    let mut buffer = vec![0u8; length as usize];
    stream.read_exact(&mut buffer).map_err(|_| "Failed to read string data".to_string())?;
    String::from_utf8(buffer).map_err(|_| "Invalid UTF-8 string".to_string())
}

fn read_string_from_cursor(cursor: &mut std::io::Cursor<Vec<u8>>) -> Result<String, String> {
    let length = read_varint_from_cursor(cursor).map_err(|_| "Failed to read string length".to_string())?;
    let mut buffer = vec![0u8; length as usize];
    cursor.read_exact(&mut buffer).map_err(|_| "Failed to read string data".to_string())?;
    String::from_utf8(buffer).map_err(|_| "Invalid UTF-8 string".to_string())
}

fn write_string_to_vec(s: &str) -> Vec<u8> {
    let mut buf = vec![];
    buf.extend(write_varint_to_vec(s.len() as i32));
    buf.extend(s.as_bytes());
    buf
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

/*Login attempt from username: sd
Login successful for: sd
Player list: [Player { uuid: 0fc44095-4196-403c-af0e-b043448e3628, username: "sd", position: (0.0, 64.0, 0.0), health: 20.0, game_mode: Survival, is_operator: false }]
Sending UUID: 0fc44095-4196-403c-af0e-b043448e3628
Sending username: sd
Login success packet length: 41
Login success packet sent successfully.
Sending entity ID: [0, 0, 0, 1]
Sending is hardcore: false
Sending game mode: 0
Sending previous game mode: -1
Sending world count: 1
Sending world name: minecraft:overworld
Sending empty NBT tag
Sending dimension name: minecraft:overworld
Sending world name again: minecraft:overworld
Sending hashed seed: [0, 0, 0, 0, 0, 0, 0, 0]
Sending max players: 10
Sending view distance: 10
Sending simulation distance: 10
Sending reduced debug info: false
Sending enable respawn screen: true
Sending is debug: false
Sending is flat: false
Join game packet length: 85
Join game packet sent successfully.
Client sd disconnected.*/

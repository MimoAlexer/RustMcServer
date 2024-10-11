use std::net::{TcpListener, TcpStream};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use byteorder::{ReadBytesExt, WriteBytesExt, BigEndian};
use uuid::Uuid;
use std::collections::HashMap;
use rand::Rng;

// Updated protocol version for Minecraft 1.21.1
const PROTOCOL_VERSION: i32 = 767; // For Minecraft 1.21.1
const SERVER_NAME: &str = "Rust Minecraft Server";
const MAX_PLAYERS: usize = 100;

#[derive(Debug, Clone)]
struct Player {
    uuid: Uuid,
    username: String,
    position: (f64, f64, f64), // (x, y, z) position
    health: f32,               // Player health
    game_mode: GameMode,       // Player's game mode
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
                // Occasionally generate trees
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
    let peer_addr = stream.peer_addr().unwrap();
    println!("New connection from: {}", peer_addr);

    // Handshake phase
    let next_state = match handle_handshake(&mut stream) {
        Ok(state) => state,
        Err(e) => {
            println!("Handshake failed: {}", e);
            return;
        }
    };

    if next_state == 1 {
        // Status request (ping)
        handle_status(&mut stream);
        return;
    }

    // Login phase
    let username = match handle_login(&mut stream) {
        Ok(username) => {
            println!("Login successful for: {}", username);
            username
        }
        Err(e) => {
            println!("Login failed: {}", e);
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

    // Transition to play state
    if let Err(_) = send_join_game(&mut stream, &player, &world.lock().unwrap()) {
        println!("Failed to send join game packet to {}", username);
        return;
    }

    loop {
        // Read the length of the packet
        let length = match read_varint(&mut stream) {
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
                // Handle packet
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
    let mut cursor = std::io::Cursor::new(buffer);
    let packet_id = match read_varint_from_cursor(&mut cursor) {
        Ok(id) => id,
        Err(_) => return,
    };

    match packet_id {
        0x12 => handle_player_position(stream, players, player, &mut cursor), // Updated packet ID
        0x13 => handle_player_position_and_rotation(stream, players, player, &mut cursor), // Updated packet ID
        _ => println!("Unknown packet ID: {}", packet_id),
    }
}

fn handle_player_position(stream: &mut TcpStream, players: &mut Vec<Player>, player: &Player, cursor: &mut std::io::Cursor<Vec<u8>>) {
    if cursor.get_ref().len() >= 24 {
        let x = cursor.read_f64::<BigEndian>().map_err(|e| println!("Error reading x position: {}", e)).unwrap();
        let y = cursor.read_f64::<BigEndian>().map_err(|e| println!("Error reading y position: {}", e)).unwrap();
        let z = cursor.read_f64::<BigEndian>().map_err(|e| println!("Error reading z position: {}", e)).unwrap();
        println!("Player {} moved to position: ({}, {}, {})", player.username, x, y, z);

        // Update player position
        if let Some(p) = players.iter_mut().find(|p| p.uuid == player.uuid) {
            p.position = (x, y, z);
        }
    }
}

fn handle_player_position_and_rotation(stream: &mut TcpStream, players: &mut Vec<Player>, player: &Player, cursor: &mut std::io::Cursor<Vec<u8>>) {
    if cursor.get_ref().len() >= 32 {
        let x = cursor.read_f64::<BigEndian>().unwrap();
        let y = cursor.read_f64::<BigEndian>().unwrap();
        let z = cursor.read_f64::<BigEndian>().unwrap();
        let _yaw = cursor.read_f32::<BigEndian>().unwrap();
        let _pitch = cursor.read_f32::<BigEndian>().unwrap();
        println!("Player {} moved to position: ({}, {}, {})", player.username, x, y, z);

        // Update player position
        if let Some(p) = players.iter_mut().find(|p| p.uuid == player.uuid) {
            p.position = (x, y, z);
        }
    }
}

fn handle_handshake(stream: &mut TcpStream) -> Result<i32, String> {
    // Read packet length
    let packet_length = read_varint(stream).map_err(|e| format!("Failed to read packet length: {}", e))?;

    // Read the entire packet based on the packet length
    let mut packet_data = vec![0u8; packet_length as usize];
    stream.read_exact(&mut packet_data).map_err(|e| format!("Failed to read handshake packet: {}", e))?;

    // Use a cursor to read from the packet data buffer
    let mut cursor = std::io::Cursor::new(packet_data);

    // Read packet ID
    let packet_id = read_varint_from_cursor(&mut cursor).map_err(|e| format!("Failed to read packet ID: {}", e))?;

    if packet_id != 0x00 {
        return Err(format!("Invalid packet ID for handshake: {}", packet_id));
    }

    // Read protocol version
    let protocol_version = read_varint_from_cursor(&mut cursor).map_err(|e| format!("Failed to read protocol version: {}", e))?;

    // Read server address (string)
    let server_address = read_string_from_cursor(&mut cursor).map_err(|e| format!("Failed to read server address: {}", e))?;

    // Read server port (unsigned short)
    let server_port = cursor.read_u16::<BigEndian>().map_err(|e| format!("Failed to read server port: {}", e))?;

    // Read next state (VarInt)
    let next_state = read_varint_from_cursor(&mut cursor).map_err(|e| format!("Failed to read next state: {}", e))?;

    println!("Received handshake: packet_id={}, protocol_version={}, server_address={}, server_port={}, next_state={}",
             packet_id, protocol_version, server_address, server_port, next_state);

    Ok(next_state)
}

fn handle_status(stream: &mut TcpStream) {
    // Status handling is not implemented for simplicity
}

fn handle_login(stream: &mut TcpStream) -> Result<String, String> {
    // Read packet length
    let packet_length = read_varint(stream).map_err(|_| "Failed to read packet length".to_string())?;

    // Read the entire packet based on the packet length
    let mut packet_data = vec![0u8; packet_length as usize];
    stream.read_exact(&mut packet_data).map_err(|_| "Failed to read login packet".to_string())?;

    // Use a cursor to read from the packet data buffer
    let mut cursor = std::io::Cursor::new(packet_data);

    // Read packet ID
    let packet_id = read_varint_from_cursor(&mut cursor).map_err(|_| "Failed to read packet ID".to_string())?;

    if packet_id != 0x00 {
        return Err(format!("Invalid packet ID for login start: {}", packet_id));
    }

    // Read username (string)
    let username = read_string_from_cursor(&mut cursor)?;

    println!("Login attempt from username: {}", username);

    Ok(username)
}

fn send_login_success(stream: &mut TcpStream, player: &Player) -> Result<(), String> {
    let mut packet_data = vec![];
    packet_data.extend(write_varint_to_vec(0x02)); // Packet ID for Login Success

    // Write UUID as a string
    packet_data.extend(write_string_to_vec(&player.uuid.to_string()));

    // Write username
    packet_data.extend(write_string_to_vec(&player.username));

    // Write packet length
    let mut packet = vec![];
    packet.extend(write_varint_to_vec(packet_data.len() as i32));
    packet.extend(packet_data);

    stream.write_all(&packet).map_err(|_| "Failed to send login success packet".to_string())?;
    Ok(())
}

fn send_join_game(stream: &mut TcpStream, player: &Player, world: &World) -> Result<(), String> {
    let mut packet_data = vec![];
    packet_data.extend(write_varint_to_vec(0x26)); // Packet ID for Join Game (verify if this ID is correct for the protocol version)

    // Entity ID (Int)
    packet_data.extend(&(1i32.to_be_bytes()));

    // Is Hardcore (Boolean)
    packet_data.push(0); // False

    // Game Mode (Unsigned Byte)
    packet_data.push(match player.game_mode {
        GameMode::Survival => 0,
        GameMode::Creative => 1,
        GameMode::Adventure => 2,
        GameMode::Spectator => 3,
    });

    // Previous Game Mode (Byte)
    packet_data.push(255u8); // -1 for none

    // World Count (VarInt)
    packet_data.extend(write_varint_to_vec(1));

    // World Names (Identifier)
    packet_data.extend(write_string_to_vec("minecraft:overworld"));

    // Dimension Codec (NBT Tag) - We'll send an empty NBT for simplicity
    packet_data.push(0x00); // NBT End Tag

    // Dimension Name (Identifier)
    packet_data.extend(write_string_to_vec("minecraft:overworld"));

    // World Name (Identifier)
    packet_data.extend(write_string_to_vec("minecraft:overworld"));

    // Hashed Seed (Long)
    packet_data.extend(&0i64.to_be_bytes());

    // Max Players (VarInt)
    packet_data.extend(write_varint_to_vec(10)); // Example value

    // View Distance (VarInt)
    packet_data.extend(write_varint_to_vec(10));

    // Simulation Distance (VarInt)
    packet_data.extend(write_varint_to_vec(10));

    // Reduced Debug Info (Boolean)
    packet_data.push(0);

    // Enable Respawn Screen (Boolean)
    packet_data.push(1);

    // Is Debug (Boolean)
    packet_data.push(0);

    // Is Flat (Boolean)
    packet_data.push(0);

    // Write packet length
    let mut packet = vec![];
    packet.extend(write_varint_to_vec(packet_data.len() as i32));
    packet.extend(packet_data);

    stream.write_all(&packet).map_err(|_| "Failed to send join game packet".to_string())?;
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

/*Received handshake: packet_id=0, protocol_version=767, server_address=localhost, server_port=25565, next_state=2
Login attempt from username: sd
Login successful for: sd
Player list: [Player { uuid: 9b55de80-ddf0-4dfe-a447-6df0e83cdb4e, username: "sd", position: (0.0, 64.0, 0.0), health: 20.0, game_mode: Survival, is_operator: false }]
Client sd disconnected.*/

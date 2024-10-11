use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use byteorder::{ReadBytesExt, WriteBytesExt, BigEndian};
use uuid::Uuid;
use rand::Rng;

// Konstanten und Serverinformationen
const PROTOCOL_VERSION: i32 = 767; // Protokollversion für Minecraft 1.21.1
const SERVER_NAME: &str = "Rust Minecraft Server";
const MAX_PLAYERS: usize = 100;

#[derive(Debug, Clone)]
struct Player {
    uuid: Uuid,
    username: String,
    position: (f64, f64, f64),
    health: f32,
    game_mode: GameMode,
    is_operator: bool,
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
    blocks: HashMap<(i32, i32, i32), String>,
    mobs: Vec<Mob>,
    dimension: Dimension,
}

#[derive(Debug, Clone, Copy)]
enum Dimension {
    Overworld,
    Nether,
    End,
}

impl World {
    fn generate(&mut self) {
        println!("Generiere Welt...");
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
        println!("Erzeuge großen Baum bei ({}, {}, {})", x, y, z);
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

    fn send_login_success(stream: &mut TcpStream, player: &Player) -> Result<(), String> {
        let mut packet_data = vec![];
        packet_data.extend(write_varint_to_vec(0x02)); // Packet ID für Login Success

        // UUID als String ohne Bindestriche
        let uuid_str = player.uuid.simple().to_string(); // UUID ohne Bindestriche, z.B. "0fc440954196403caf0eb043448e3628"
        packet_data.extend(write_string_to_vec(&uuid_str));
        println!("Sende UUID: {}", uuid_str);

        // Benutzernamen hinzufügen
        packet_data.extend(write_string_to_vec(&player.username));
        println!("Sende Benutzernamen: {}", player.username);

        // Paketlänge berechnen und senden
        let mut packet = vec![];
        let packet_length = packet_data.len() as i32;
        packet.extend(write_varint_to_vec(packet_length)); // Länge des Pakets
        packet.extend(packet_data);
        println!("Login-Erfolgs-Paketlänge: {}", packet_length);

        // Paket an den Stream senden
        stream.write_all(&packet).map_err(|e| format!("Fehler beim Senden des Login-Erfolgspakets: {}", e))?;
        println!("Login-Erfolgs-Paket erfolgreich gesendet.");
        Ok(())
    }

    fn send_join_game(stream: &mut TcpStream, player: &Player, world: &World) -> Result<(), String> {
        let mut packet_data = vec![];
        packet_data.extend(write_varint_to_vec(0x26)); // Packet ID für Join Game

        // Entity ID als Integer
        let entity_id = 1i32.to_be_bytes(); // Beispiel-ID für den Spieler, normalerweise dynamisch
        packet_data.extend(&entity_id);
        println!("Sende Entity ID: {:?}", entity_id);

        // Ist Hardcore (Boolean)
        packet_data.push(0); // False für nicht-hardcore

        // Spielmodus (Unsigned Byte)
        let game_mode = match player.game_mode {
            GameMode::Survival => 0,
            GameMode::Creative => 1,
            GameMode::Adventure => 2,
            GameMode::Spectator => 3,
        };
        packet_data.push(game_mode);
        println!("Sende Spielmodus: {}", game_mode);

        // Vorheriger Spielmodus (Byte)
        packet_data.push(255u8); // -1 für keinen vorherigen Spielmodus
        println!("Sende vorherigen Spielmodus: -1");

        // Anzahl der Welten (VarInt)
        packet_data.extend(write_varint_to_vec(1));
        println!("Sende Weltanzahl: 1");

        // Weltname (Identifier)
        let world_name = "minecraft:overworld";
        packet_data.extend(write_string_to_vec(world_name));
        println!("Sende Weltname: {}", world_name);

        // Dimension Codec (NBT Tag) - Minimaler NBT-Tag
        packet_data.push(0x0A); // NBT Compound Tag
        packet_data.push(0x00); // Leerzeichen-String
        packet_data.push(0x00); // Ende des Compound-Tags
        println!("Sende leeren NBT-Tag");

        // Dimension Name (Identifier)
        packet_data.extend(write_string_to_vec(world_name));
        println!("Sende Dimension Name: {}", world_name);

        // Weltname erneut senden
        packet_data.extend(write_string_to_vec(world_name));
        println!("Sende Weltname erneut: {}", world_name);

        // Gehashter Seed (Long)
        let hashed_seed = 0i64.to_be_bytes();
        packet_data.extend(&hashed_seed);
        println!("Sende gehashten Seed: {:?}", hashed_seed);

        // Maximale Spieleranzahl (VarInt)
        packet_data.extend(write_varint_to_vec(MAX_PLAYERS as i32));
        println!("Sende maximale Spieleranzahl: {}", MAX_PLAYERS);

        // Sichtweite (VarInt)
        let view_distance = 10;
        packet_data.extend(write_varint_to_vec(view_distance));
        println!("Sende Sichtweite: {}", view_distance);

        // Simulations-Distanz (VarInt)
        let simulation_distance = 10;
        packet_data.extend(write_varint_to_vec(simulation_distance));
        println!("Sende Simulations-Distanz: {}", simulation_distance);

        // Reduzierte Debug-Info (Boolean)
        packet_data.push(0); // False für nicht reduziert
        println!("Sende reduzierte Debug-Info: false");

        // Respawn-Bildschirm aktivieren (Boolean)
        packet_data.push(1); // True für aktiviert
        println!("Sende Respawn-Bildschirm: true");

        // Ist Debug-Welt (Boolean)
        packet_data.push(0); // False für keine Debug-Welt
        println!("Sende Debug-Welt: false");

        // Ist flache Welt (Boolean)
        packet_data.push(0); // False für nicht flach
        println!("Sende flache Welt: false");

        // Paketlänge berechnen und senden
        let mut packet = vec![];
        let packet_length = packet_data.len() as i32;
        packet.extend(write_varint_to_vec(packet_length)); // Länge des Pakets
        packet.extend(packet_data);
        println!("Beitrittspaket-Länge: {}", packet_length);

        // Paket an den Stream senden
        stream.write_all(&packet).map_err(|e| format!("Fehler beim Senden des Beitrittspakets: {}", e))?;
        println!("Beitrittspaket erfolgreich gesendet.");
        Ok(())
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

fn handle_handshake(stream: &mut TcpStream) -> Result<i32, String> {
    let packet_length = read_varint(stream).map_err(|e| format!("Failed to read packet length: {}", e))?;
    let mut packet_data = vec![0u8; packet_length as usize];
    stream.read_exact(&mut packet_data).map_err(|e| format!("Failed to read handshake packet: {}", e))?;
    let mut cursor = std::io::Cursor::new(packet_data);
    let packet_id = read_varint_from_cursor(&mut cursor).map_err(|e| format!("Failed to read packet ID: {}", e))?;
    if packet_id != 0x00 {
        return Err(format!("Invalid packet ID for handshake: {}", packet_id));
    }
    let protocol_version = read_varint_from_cursor(&mut cursor).map_err(|e| format!("Failed to read protocol version: {}", e))?;
    let server_address = read_string_from_cursor(&mut cursor).map_err(|e| format!("Failed to read server address: {}", e))?;
    let server_port = cursor.read_u16::<BigEndian>().map_err(|e| format!("Failed to read server port: {}", e))?;
    let next_state = read_varint_from_cursor(&mut cursor).map_err(|e| format!("Failed to read next state: {}", e))?;
    println!("Handshake erhalten: packet_id={}, protocol_version={}, server_address={}, server_port={}, next_state={}",
             packet_id, protocol_version, server_address, server_port, next_state);
    Ok(next_state)
}

fn handle_status(stream: &mut TcpStream) {
    println!("Status-Anfrage erhalten.");
}

fn handle_login(stream: &mut TcpStream) -> Result<String, String> {
    let packet_length = read_varint(stream).map_err(|_| "Failed to read packet length".to_string())?;
    let mut packet_data = vec![0u8; packet_length as usize];
    stream.read_exact(&mut packet_data).map_err(|_| "Failed to read login packet".to_string())?;
    let mut cursor = std::io::Cursor::new(packet_data);
    let packet_id = read_varint_from_cursor(&mut cursor).map_err(|_| "Failed to read packet ID".to_string())?;
    if packet_id != 0x00 {
        return Err(format!("Invalid packet ID for login start: {}", packet_id));
    }
    let username = read_string_from_cursor(&mut cursor)?;
    println!("Login-Versuch von Benutzername: {}", username);
    Ok(username)
}

fn handle_packet(stream: &mut TcpStream, players: &mut Vec<Player>, world: &mut World, player: &Player, buffer: Vec<u8>) {
    let mut cursor = std::io::Cursor::new(buffer);
    let packet_id = match read_varint_from_cursor(&mut cursor) {
        Ok(id) => id,
        Err(_) => return,
    };
    match packet_id {
        0x12 => handle_player_position(stream, players, player, &mut cursor),
        0x13 => handle_player_position_and_rotation(stream, players, player, &mut cursor),
        _ => println!("Unbekannte Paket-ID: {}", packet_id),
    }
}

fn handle_player_position(stream: &mut TcpStream, players: &mut Vec<Player>, player: &Player, cursor: &mut std::io::Cursor<Vec<u8>>) {
    if cursor.get_ref().len() >= 24 {
        let x = cursor.read_f64::<BigEndian>().unwrap();
        let y = cursor.read_f64::<BigEndian>().unwrap();
        let z = cursor.read_f64::<BigEndian>().unwrap();
        println!("Spieler {} bewegte sich zu Position: ({}, {}, {})", player.username, x, y, z);
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
        println!("Spieler {} bewegte sich zu Position: ({}, {}, {})", player.username, x, y, z);
        if let Some(p) = players.iter_mut().find(|p| p.uuid == player.uuid) {
            p.position = (x, y, z);
        }
    }
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

fn read_string_from_cursor(cursor: &mut std::io::Cursor<Vec<u8>>) -> Result<String, String> {
    let length = read_varint_from_cursor(cursor).map_err(|_| "Failed to read string length".to_string())?;
    let mut buffer = vec![0u8; length as usize];
    cursor.read_exact(&mut buffer).map_err(|_| "Failed to read string data".to_string())?;
    String::from_utf8(buffer).map_err(|_| "Invalid UTF-8 string".to_string())
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

fn write_string_to_vec(s: &str) -> Vec<u8> {
    let mut buf = vec![];
    buf.extend(write_varint_to_vec(s.len() as i32));
    buf.extend(s.as_bytes());
    buf
}

fn main() {
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
    world.lock().unwrap().generate();
    let listener = TcpListener::bind("0.0.0.0:25565").unwrap();
    println!("Server hört auf Port 25565...");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let players = Arc::clone(&players);
                let world = Arc::clone(&world);
                thread::spawn(move || {
                    handle_client(stream, players, world);
                });
            }
            Err(e) => println!("Verbindung fehlgeschlagen: {}", e),
        }
    }
}

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use byteorder::{ReadBytesExt, BigEndian}; // `WriteBytesExt` entfernt
use uuid::Uuid;
use rand::Rng;

const MAX_PLAYERS: usize = 100;

#[derive(Debug, Clone)]
struct Player {
    uuid: Uuid,
    username: String,
    position: (f64, f64, f64),
    _health: f32,
    game_mode: GameMode,
    _is_operator: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum GameMode {
    Survival,
}

#[derive(Debug, Clone)]
struct Mob {
    id: Uuid,
    _mob_type: String,
    _position: (f64, f64, f64),
    _health: f32,
}

struct World {
    blocks: HashMap<(i32, i32, i32), String>,
    _mobs: Vec<Mob>,
    _dimension: Dimension,
}

#[derive(Debug, Clone, Copy)]
enum Dimension {
    Overworld,
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
        _health: 20.0,
        game_mode: GameMode::Survival,
        _is_operator: false,
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
    packet_data.extend(write_varint_to_vec(0x02)); // Packet ID für Login Success

    let uuid_str = player.uuid.to_string();
    packet_data.extend(write_string_to_vec(&uuid_str));
    println!("Sende UUID: {}", uuid_str);

    packet_data.extend(write_string_to_vec(&player.username));
    println!("Sende Benutzernamen: {}", player.username);

    let mut packet = vec![];
    let packet_length = packet_data.len() as i32;
    packet.extend(write_varint_to_vec(packet_length));
    packet.extend(packet_data);
    println!("Login-Erfolgs-Paketlänge: {}", packet_length);

    stream.write_all(&packet).map_err(|e| format!("Fehler beim Senden des Login-Erfolgspakets: {}", e))?;
    println!("Login-Erfolgs-Paket erfolgreich gesendet.");
    Ok(())
}

fn send_join_game(stream: &mut TcpStream, player: &Player, _world: &World) -> Result<(), String> {
    let mut packet_data = vec![];
    packet_data.extend(write_varint_to_vec(0x26)); // Packet ID für Join Game

    let entity_id = 1i32.to_be_bytes();
    packet_data.extend(&entity_id);
    println!("Sende Entity ID: {:?}", entity_id);

    packet_data.push(0); // Is Hardcore (Boolean)

    let game_mode = match player.game_mode {
        GameMode::Survival => 0,
    };
    packet_data.push(game_mode);
    println!("Sende Spielmodus: {}", game_mode);

    packet_data.push(255u8); // Vorheriger Spielmodus (Byte)
    println!("Sende vorherigen Spielmodus: -1");

    packet_data.extend(write_varint_to_vec(1)); // Anzahl der Welten
    println!("Sende Weltanzahl: 1");

    let world_name = "minecraft:overworld";
    packet_data.extend(write_string_to_vec(world_name)); // Name der Welt
    println!("Sende Weltname: {}", world_name);

    // Beispielhafter Dimension Codec - Detaillierterer NBT-Tag (in einfachem Format)
    let dimension_codec = vec![
        0x0A, 0x00, 0x0A, 0x00, 0x00, // Compound Tag (Root Level)
        0x00,                         // Ende des Tags
    ];
    packet_data.extend(dimension_codec);
    println!("Sende Dimension Codec NBT");

    packet_data.extend(write_string_to_vec(world_name)); // Dimension Name
    println!("Sende Dimension Name: {}", world_name);

    packet_data.extend(write_string_to_vec(world_name)); // Weltname erneut senden
    println!("Sende Weltname erneut: {}", world_name);

    let hashed_seed = 0i64.to_be_bytes();
    packet_data.extend(&hashed_seed); // Gehashter Seed
    println!("Sende gehashten Seed: {:?}", hashed_seed);

    packet_data.extend(write_varint_to_vec(MAX_PLAYERS as i32)); // Maximale Spieleranzahl
    println!("Sende maximale Spieleranzahl: {}", MAX_PLAYERS);

    let view_distance = 10;
    packet_data.extend(write_varint_to_vec(view_distance));
    println!("Sende Sichtweite: {}", view_distance);

    let simulation_distance = 10;
    packet_data.extend(write_varint_to_vec(simulation_distance));
    println!("Sende Simulations-Distanz: {}", simulation_distance);

    packet_data.push(0); // Reduzierte Debug-Info (Boolean)
    println!("Sende reduzierte Debug-Info: false");

    packet_data.push(1); // Respawn-Bildschirm aktiviert (Boolean)
    println!("Sende Respawn-Bildschirm: true");

    packet_data.push(0); // Ist Debug-Welt (Boolean)
    println!("Sende Debug-Welt: false");

    packet_data.push(0); // Ist flache Welt (Boolean)
    println!("Sende flache Welt: false");

    let mut packet = vec![];
    let packet_length = packet_data.len() as i32;
    packet.extend(write_varint_to_vec(packet_length));
    packet.extend(packet_data);
    println!("Beitrittspaket-Länge: {}", packet_length);

    stream.write_all(&packet).map_err(|e| format!("Fehler beim Senden des Beitrittspakets: {}", e))?;
    println!("Beitrittspaket erfolgreich gesendet.");
    Ok(())
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

fn handle_status(_stream: &mut TcpStream) {
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

fn handle_player_position(_stream: &mut TcpStream, players: &mut Vec<Player>, player: &Player, cursor: &mut std::io::Cursor<Vec<u8>>) {
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

fn handle_player_position_and_rotation(_stream: &mut TcpStream, players: &mut Vec<Player>, player: &Player, cursor: &mut std::io::Cursor<Vec<u8>>) {
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
        _mobs: vec![
            Mob {
                id: Uuid::new_v4(),
                _mob_type: "Zombie".to_string(),
                _position: (10.0, 64.0, 10.0),
                _health: 20.0,
            },
            Mob {
                id: Uuid::new_v4(),
                _mob_type: "Skeleton".to_string(),
                _position: (15.0, 64.0, 15.0),
                _health: 20.0,
            },
        ],
        _dimension: Dimension::Overworld,
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

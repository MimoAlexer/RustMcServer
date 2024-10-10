use std::net::{TcpListener, TcpStream};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use byteorder::{ReadBytesExt, WriteBytesExt, BigEndian};
use uuid::Uuid;
use std::collections::HashMap;
use rand::Rng;

const PROTOCOL_VERSION: i32 = 800; // Beispiel für Version 1.21.1
const SERVER_NAME: &str = "Rust Minecraft Server";
const MAX_PLAYERS: usize = 100;

#[derive(Debug, Clone)]
struct Player {
    uuid: Uuid,
    username: String,
    position: (f64, f64, f64), // (x, y, z) Position
    health: f32,               // Spieler Gesundheit
    game_mode: GameMode,       // Spielmodus des Spielers
    is_operator: bool,         // Operator-Flag
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
    dimension: Dimension,                      // Weltdimension
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
        // Einfache Terrain-Generierung mit Bäumen
        for x in -100..100 {
            for z in -100..100 {
                let height = 64 + rng.gen_range(-3..3); // Zufällige Höhenvariation
                for y in 0..=height {
                    let block_type = if y == height {
                        "grass"
                    } else {
                        "dirt"
                    };
                    self.blocks.insert((x, y, z), block_type.to_string());
                }
                // Gelegentlich Bäume generieren
                if rng.gen_range(0..100) < 5 { // 5% Chance, einen Baum zu generieren
                    self.generate_large_tree(x, height + 1, z);
                }
            }
        }
    }

    fn generate_large_tree(&mut self, x: i32, y: i32, z: i32) {
        println!("Generiere einen großen Baum bei ({}, {}, {})", x, y, z);
        // Stamm generieren
        for i in 0..5 {
            self.blocks.insert((x, y + i, z), "log".to_string());
        }
        // Blätter generieren
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
    println!("Neue Verbindung von: {}", peer_addr);

    // Handshake-Phase
    let next_state = match handle_handshake(&mut stream) {
        Ok(state) => state,
        Err(e) => {
            println!("Handshake fehlgeschlagen: {}", e);
            return;
        }
    };

    if next_state == 1 {
        // Statusanfrage (Ping)
        handle_status(&mut stream);
        return;
    }

    // Login-Phase
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

    // Spieler erstellen und zur Spielerliste hinzufügen
    let player = Player {
        uuid: Uuid::new_v4(),
        username: username.clone(),
        position: (0.0, 64.0, 0.0), // Spawn-Position
        health: 20.0,               // Standardgesundheit
        game_mode: GameMode::Survival, // Standard-Spielmodus
        is_operator: false,         // Standard-Operator-Status
    };
    players.lock().unwrap().push(player.clone());
    println!("Spielerliste: {:?}", players.lock().unwrap());

    // Login-Erfolgspaket senden
    if let Err(_) = send_login_success(&mut stream, &player) {
        println!("Fehler beim Senden des Login-Erfolgspakets an {}", username);
        return;
    }

    // Übergang zum Spielzustand
    if let Err(_) = send_join_game(&mut stream, &player, &world.lock().unwrap()) {
        println!("Fehler beim Senden des Join-Game-Pakets an {}", username);
        return;
    }

    loop {
        // Lesen der Paketlänge
        let length = match read_varint(&mut stream) {
            Ok(length) => length,
            Err(_) => {
                println!("Client {} getrennt.", username);
                // Spieler aus der Liste entfernen
                players.lock().unwrap().retain(|p| p.username != username);
                return;
            }
        };

        let mut buffer = vec![0; length as usize];
        match stream.read_exact(&mut buffer) {
            Ok(_) => {
                // Paket verarbeiten
                handle_packet(&mut stream, &mut players.lock().unwrap(), &mut world.lock().unwrap(), &player, buffer);
            }
            Err(_) => {
                println!("Fehler beim Lesen des Pakets von {}.", username);
                // Spieler aus der Liste entfernen
                players.lock().unwrap().retain(|p| p.username != username);
                return;
            }
        }
    }
}

fn handle_packet(stream: &mut TcpStream, players: &mut Vec<Player>, world: &mut World, player: &Player, buffer: Vec<u8>) {
    // Verschiedene Pakettypen verarbeiten (vereinfacht)
    let mut cursor = std::io::Cursor::new(buffer);
    let packet_id = match read_varint_from_cursor(&mut cursor) {
        Ok(id) => id,
        Err(_) => return,
    };

    match packet_id {
        0x11 => handle_player_position(stream, players, player, &mut cursor),
        0x1A => handle_player_position_and_rotation(stream, players, player, &mut cursor),
        _ => println!("Unbekannte Paket-ID: {}", packet_id),
    }
}

fn handle_player_position(stream: &mut TcpStream, players: &mut Vec<Player>, player: &Player, cursor: &mut std::io::Cursor<Vec<u8>>) {
    if cursor.get_ref().len() >= 24 {
        let x = cursor.read_f64::<BigEndian>().unwrap();
        let y = cursor.read_f64::<BigEndian>().unwrap();
        let z = cursor.read_f64::<BigEndian>().unwrap();
        println!("Spieler {} bewegte sich zu Position: ({}, {}, {})", player.username, x, y, z);

        // Spielerposition aktualisieren
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

        // Spielerposition aktualisieren
        if let Some(p) = players.iter_mut().find(|p| p.uuid == player.uuid) {
            p.position = (x, y, z);
        }
    }
}

fn handle_handshake(stream: &mut TcpStream) -> Result<i32, String> {
    let _packet_length = read_varint(stream).map_err(|_| "Fehler beim Lesen der Paketlänge".to_string())?;
    let packet_id = read_varint(stream).map_err(|_| "Fehler beim Lesen der Paket-ID".to_string())?;

    if packet_id != 0x00 {
        return Err("Ungültige Paket-ID für Handshake".to_string());
    }

    let _protocol_version = read_varint(stream).map_err(|_| "Fehler beim Lesen der Protokollversion".to_string())?;
    let _server_address = read_string(stream)?;
    let _server_port = stream.read_u16::<BigEndian>().map_err(|_| "Fehler beim Lesen des Serverports".to_string())?;
    let next_state = read_varint(stream).map_err(|_| "Fehler beim Lesen des nächsten Zustands".to_string())?;

    Ok(next_state)
}

fn handle_status(stream: &mut TcpStream) {
    // Statusverarbeitung wird der Einfachheit halber nicht implementiert
}

fn handle_login(stream: &mut TcpStream) -> Result<String, String> {
    let _packet_length = read_varint(stream).map_err(|_| "Fehler beim Lesen der Paketlänge".to_string())?;
    let packet_id = read_varint(stream).map_err(|_| "Fehler beim Lesen der Paket-ID".to_string())?;

    if packet_id != 0x00 {
        return Err("Ungültige Paket-ID für Login Start".to_string());
    }

    // Spielernamen lesen (String)
    let username = read_string(stream)?;
    Ok(username)
}

fn send_login_success(stream: &mut TcpStream, player: &Player) -> Result<(), String> {
    let mut packet_data = vec![];
    packet_data.extend(write_varint_to_vec(0x02)); // Paket-ID für Login Success

    // UUID als 16-Byte-Array schreiben
    packet_data.extend(player.uuid.as_bytes());

    // Benutzernamen schreiben
    packet_data.extend(write_string_to_vec(&player.username));

    // Paketlänge schreiben
    let mut packet = vec![];
    packet.extend(write_varint_to_vec(packet_data.len() as i32));
    packet.extend(packet_data);

    stream.write_all(&packet).map_err(|_| "Fehler beim Senden des Login-Erfolgspakets".to_string())?;
    Ok(())
}

fn send_join_game(stream: &mut TcpStream, player: &Player, world: &World) -> Result<(), String> {
    let mut packet_data = vec![];
    packet_data.extend(write_varint_to_vec(0x26)); // Paket-ID für Join Game

    // Entity ID (Int)
    packet_data.extend(&(1i32.to_be_bytes()));

    // Ist Hardcore (Boolean)
    packet_data.push(0); // Falsch

    // Spielmodus (Unsigned Byte)
    packet_data.push(match player.game_mode {
        GameMode::Survival => 0,
        GameMode::Creative => 1,
        GameMode::Adventure => 2,
        GameMode::Spectator => 3,
    });

    // Vorheriger Spielmodus (Byte)
    packet_data.push(255u8); // -1 für keinen

    // Anzahl der Welten (VarInt)
    packet_data.extend(write_varint_to_vec(1));

    // Weltnamen (Identifier)
    packet_data.extend(write_string_to_vec("minecraft:overworld"));

    // Dimension Codec (NBT Tag) - Wir senden der Einfachheit halber ein leeres NBT
    packet_data.extend(write_varint_to_vec(0)); // Länge der NBT-Daten

    // Dimension (Identifier)
    packet_data.extend(write_string_to_vec("minecraft:overworld"));

    // Weltname (Identifier)
    packet_data.extend(write_string_to_vec("minecraft:overworld"));

    // Gehashter Seed (Long)
    packet_data.extend(&0i64.to_be_bytes());

    // Maximale Spieleranzahl (VarInt)
    packet_data.extend(write_varint_to_vec(0)); // Veraltet

    // Sichtweite (VarInt)
    packet_data.extend(write_varint_to_vec(10));

    // Simulationsdistanz (VarInt)
    packet_data.extend(write_varint_to_vec(10));

    // Reduzierte Debug-Info (Boolean)
    packet_data.push(0);

    // Respawn-Bildschirm aktivieren (Boolean)
    packet_data.push(1);

    // Ist Debug (Boolean)
    packet_data.push(0);

    // Ist Flach (Boolean)
    packet_data.push(0);

    // Paketlänge schreiben
    let mut packet = vec![];
    packet.extend(write_varint_to_vec(packet_data.len() as i32));
    packet.extend(packet_data);

    stream.write_all(&packet).map_err(|_| "Fehler beim Senden des Join-Game-Pakets".to_string())?;
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
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "VarInt zu groß"));
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
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "VarInt zu groß"));
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
    let length = read_varint(stream).map_err(|_| "Fehler beim Lesen der String-Länge".to_string())?;
    let mut buffer = vec![0u8; length as usize];
    stream.read_exact(&mut buffer).map_err(|_| "Fehler beim Lesen der String-Daten".to_string())?;
    String::from_utf8(buffer).map_err(|_| "Ungültiger UTF-8-String".to_string())
}

fn write_string_to_vec(s: &str) -> Vec<u8> {
    let mut buf = vec![];
    buf.extend(write_varint_to_vec(s.len() as i32));
    buf.extend(s.as_bytes());
    buf
}

fn main() {
    // Gemeinsame Spielerliste mit Arc und Mutex für Thread-sicheren Zugriff
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
                mob_type: "Skelett".to_string(),
                position: (15.0, 64.0, 15.0),
                health: 20.0,
            },
        ],
        dimension: Dimension::Overworld,
    }));

    // Welt generieren
    world.lock().unwrap().generate();

    // Hören auf eingehende TCP-Verbindungen auf Port 25565 (Minecraft-Standardport)
    let listener = TcpListener::bind("0.0.0.0:25565").unwrap();
    println!("Server hört auf Port 25565...");

    // Eingehende Verbindungen akzeptieren
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let players = Arc::clone(&players);
                let world = Arc::clone(&world);
                // Jede Verbindung in einem neuen Thread bearbeiten
                thread::spawn(move || {
                    handle_client(stream, players, world);
                });
            }
            Err(e) => {
                println!("Verbindung fehlgeschlagen: {}", e);
            }
        }
    }
}

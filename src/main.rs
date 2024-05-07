use std::error::Error;
use std::future::Future;
use std::net::SocketAddr;
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast::{channel, Sender};
use tokio_tungstenite::{accept_async, tungstenite::protocol::Message};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use serde::{Serialize, Deserialize};
use lazy_static::lazy_static;
use tokio::sync::Mutex;

// Date.now(), in millis
fn get_epoch_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

#[derive(Debug, Clone)]
struct User {
    nick: String,
    is_alive: bool,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatMessage {
    message_type: String,
    data: Option<String>,
    data_array: Option<Vec<String>>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessagePacket {
    from: String,
    message: String,
    time: u128,
}

lazy_static! {
    static ref USERS: Mutex<Vec<User>> = Mutex::new(Vec::new());
}

async fn handle_connection(
    addr: SocketAddr,
    stream: TcpStream,
    broadcast_tx: Sender<String>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let ws_stream= accept_async(stream)
        .await
        .expect("Failed to connect");
    let mut broadcast_rx = broadcast_tx.subscribe();
    let mut user: Option<User> = None;
    let (mut write, mut read) = ws_stream.split();
    println!("Finished handshake with client {addr:?}.");

    loop {
        tokio::select! {
            incoming = read.next() => {
                match incoming {
                Some(Ok(msg)) => {
                    if let Ok(text) = msg.to_text() {
                        println!("From client {addr:?}:");
                        let parsed =
                            serde_json::from_str::<ChatMessage>(text)
                            .expect("Can't parse to JSON");
                        println!("JSON: {parsed:#?}");
                        match parsed.message_type.clone().as_str() {
                        "register" => {
                            user = Some(User {
                                nick: parsed.data.unwrap(),
                                is_alive: true,
                            });
                            println!("User: {user:#?}");
                            USERS.lock().await.push(user.clone().unwrap());
                            let send_message = serde_json::to_string(&ChatMessage {
                                message_type: "users".to_string(),
                                data: None,
                                data_array: Some(USERS.lock().await
                                    .iter()
                                    .map(|user| {user.nick.clone()})
                                    .collect()),
                            }).unwrap();
                            println!("Send message: {send_message}");
                            broadcast_tx.send(send_message).expect("Failed to send message");
                        },
                        "message" => {
                            match user {
                            Some(ref u) => {
                                let message_packet = serde_json::to_string(&MessagePacket {
                                    from: u.nick.clone(),
                                    message: parsed.data.unwrap(),
                                    time: get_epoch_ms(),
                                });
                                let send_message = serde_json::to_string(&ChatMessage {
                                    message_type: "message".to_string(),
                                    data: Some(message_packet.unwrap()),
                                    data_array: None,
                                }).unwrap();
                                println!("Send message: {send_message}");
                                broadcast_tx.send(send_message).expect("Failed to send message");
                            }
                            None => {}
                            }
                        },
                        _ => {
                            println!("Unrecognized message type");
                        }
                        }
                    }
                },
                Some(Err(err)) => {
                    return Err(err.into())
                },
                None => return Ok(()),
                }
            }
            msg = broadcast_rx.recv() => {
                write.send(Message::text(msg?)).await?;
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let (broadcast_tx, _) = channel(32);

    let listener = TcpListener::bind("127.0.0.1:8080").await?;

    println!("listening on port 8080");

    // Update users every 5 seconds using interval stream
    let mut interval = tokio::time::interval(Duration::from_millis(5000));
    loop {
        tokio::select! {
            Ok((socket, addr)) = listener.accept() => {
                let broadcast_tx = broadcast_tx.clone();
                println!("New connection from {addr:?}");
                tokio::spawn({
                    handle_connection(addr, socket, broadcast_tx)
                });
            }
            _ = interval.tick() => {
                let send_message = serde_json::to_string(&ChatMessage {
                    message_type: "users".to_string(),
                    data: None,
                    data_array: Some(USERS.lock().await.iter()
                        .map(|user| {user.nick.clone()})
                        .collect()),
                }).unwrap();
                let res = broadcast_tx.send(send_message);
                if res.is_err() {
                    println!("Failed to send update");
                }
            }
        }
    }
}
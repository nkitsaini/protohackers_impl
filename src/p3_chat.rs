use std::collections::HashMap;

use tokio::join;

use crate::prelude::*;

type UserName = String;
type Message = String;

#[derive(Debug, Clone)]
struct User {
    id: u64,
    name: String,
}

#[derive(Debug, Clone)]
enum RoomEventType {
    Joined,
    Message(Message), // TODO: use &str
    Left,
}

#[derive(Debug, Clone)]
struct RoomEvents {
    user: &'static User,
    ev_type: RoomEventType,
}

impl RoomEvents {
    pub fn as_msg(&self) -> String {
        let name = &self.user.name;
        match &self.ev_type {
            RoomEventType::Joined => format!("* {name} has joined!"),
            RoomEventType::Message(msg) => format!("[{name}] {msg}"),
            RoomEventType::Left => format!("* {name} has left us!"),
        }
    }
}

async fn listen_to_messages<'a>(
    user: &'static User,
    mut reader: tokio::io::BufReader<tokio::net::tcp::OwnedReadHalf>,
    event_tx: tokio::sync::broadcast::Sender<RoomEvents>,
) -> anyhow::Result<()> {
    loop {
        let mut message = String::new();
        dbg!("Waiting for message");
        match reader.read_line(&mut message).await {
            Ok(count) => {
                if count == 0 {
                    break;
                }
                dbg!("got msg", &message);
                event_tx
                    .send(RoomEvents {
                        user,
                        ev_type: RoomEventType::Message(message.trim().to_string()),
                    })
                    .ok();
                dbg!("Sent message", user, message);
                // RoomEvents::Message(name.clone(), message.trim().to_string()))?;
            }
            Err(_) => break,
        };
    }
    // event_tx.send(RoomEvents {
    //     user,
    //     ev_type: RoomEventType::Left,
    // })?;
    // RoomEvents::Left)?;
    Ok(())
}

async fn inform_user<'a>(
    user: &User,
    mut stream: tokio::net::tcp::OwnedWriteHalf,
    mut event_rx: tokio::sync::broadcast::Receiver<RoomEvents>,
) -> anyhow::Result<()> {
    loop {
        let event = event_rx.recv().await?;
        if event.user.id == user.id {
            // same user
            continue;
        }
        let msg = event.as_msg();
        stream.write(msg.as_bytes()).await?;
        stream.write("\n".as_bytes()).await?;
        stream.flush().await?;
    }
}

// impl
async fn handle_user(
    mut stream: tokio::net::TcpStream,
    room: Arc<tokio::sync::Mutex<Room>>,
    event_tx: tokio::sync::broadcast::Sender<RoomEvents>,
) -> anyhow::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    write_half
        .write("Hey what's your name\n".as_bytes())
        .await?;

    dbg!("Connection Recvd");

    let mut reader = tokio::io::BufReader::new(read_half);
    let mut name = String::new();
    reader.read_line(&mut name).await?;
    let name = name.trim().to_string();
    if !is_valid_name(&name) {
        write_half
            .write("This name is invalid booo!".to_string().as_bytes())
            .await
            .ok();
        write_half.shutdown().await?;
        return Ok(());
    }
    dbg!("Got name", &name);

    dbg!("Waiting mutex lock");
    let mut rm = room.lock().await;
    dbg!("Got mutex lock");
    let users: Vec<String> = rm.users.values().cloned().collect();
    let user = rm.add_user(name.clone());
    drop(rm);
    dbg!("Dropped mutex");

    event_tx
        .send(RoomEvents {
            user,
            ev_type: RoomEventType::Joined,
        })
        .ok();
    dbg!("Created Join Event", &name);

    let users_str = users.join(", ");
    let user_list = format!("* Current users: {users_str}\n");

    dbg!("Sent users list", &user_list);

    write_half.write(user_list.as_bytes()).await.ok(); // listen_to_messages will disconnect if this write fails
    let rx = event_tx.subscribe();

    dbg!("Listening for events", user);
    tokio::select!(
        _ = inform_user(user, write_half, rx) => { },
        _ = listen_to_messages(user, reader, event_tx.clone()) => { }
    );

    dbg!("Can't listen anymore", user);
    let mut rm = room.lock().await;
    rm.remove_user(user.id);
    drop(rm);
    dbg!("Removed user", user);

    event_tx.send(RoomEvents {
        user,
        ev_type: RoomEventType::Left,
    })?;

    // let mut data = [0; 9];
    // stream.read_exact(&mut data)?;
    // // let input = Message::try_parse(&data).context("Input not valid")?;
    // stream.write(&[])?;
    // stream.flush()?;
    Ok(())
}

fn is_valid_name(name: &str) -> bool {
    if name.len() == 0 {
        return false;
    }
    for char in name.chars() {
        if !char.is_alphanumeric() {
            return false;
        }
    }
    true
}

#[derive(Debug, Default)]
struct Room {
    users: HashMap<u64, UserName>,
    total_users: u64,
}
impl Room {
    fn add_user(&mut self, name: String) -> &'static User {
        let user_id = self.total_users;
        self.users.insert(user_id, name.clone());
        self.total_users += 1;
        let user = User { id: user_id, name };
        return Box::leak(Box::new(user));
    }
    fn remove_user(&mut self, id: u64) {
        self.users.remove(&id).unwrap();
    }
}

async fn run() -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3007").await?;

    let room = Arc::new(tokio::sync::Mutex::new(Room::default()));

    let (tx, _) = tokio::sync::broadcast::channel(128);

    loop {
        let (stream, _) = listener.accept().await?;
        let room_clone = room.clone();
        let txc = tx.clone();
        tokio::task::spawn(async move { dbg!(handle_user(stream, room_clone, txc).await) });
    }
}

pub fn main() -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { run().await })
}

#[cfg(test)]
mod tests {
    use super::*;
}

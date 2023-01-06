use crate::prelude::*;

// impl
async fn handle_user(
    mut stream: tokio::net::TcpStream,
    // room: Arc<tokio::sync::Mutex<Room>>,
    // event_tx: tokio::sync::broadcast::Sender<RoomEvents>
) -> anyhow::Result<()> {
    Ok(())
}

async fn run() -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3007").await?;

    let (tx, _) = tokio::sync::broadcast::channel(128);

    loop {
        let (stream, _) = listener.accept().await?;
        // let room_clone = room.clone();
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

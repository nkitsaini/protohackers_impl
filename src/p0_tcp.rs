use std::io::prelude::*;
use std::net::{TcpListener, TcpStream};
use std::thread;

fn handle_client(stream: &mut TcpStream) -> anyhow::Result<()> {
    let mut data = vec![];
    stream.read_to_end(&mut data)?;
    stream.write(&data)?;
    stream.flush()?;
    stream.shutdown(std::net::Shutdown::Both)?;
    Ok(())
}

pub fn main() -> anyhow::Result<()> {
    let listener = TcpListener::bind("0.0.0.0:3007")?;

    for stream in listener.incoming() {
        thread::spawn(|| -> anyhow::Result<()> { dbg!(handle_client(&mut stream?)) });
    }

    Ok(())
}

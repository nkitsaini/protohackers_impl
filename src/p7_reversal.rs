
/// L1: (Given by os)
/// 	-> Addr, Packet   
/// 	<- Addr, Packet
/// L2
/// 	-> Message, Packet
/// 	<- Message, Packet
/// L3
/// 	-> conn, String
/// 	<- conn, String
/// 

use std::{collections::VecDeque, net::SocketAddr, any};

use once_cell::sync::Lazy;
use parking_lot::Mutex;

use crate::prelude::*;


type Packet = [u8; 1024];
type Session = u64;

#[derive(Debug, Clone)]
enum MessageType {
    Connect,
    Data { pos: u64, data: String },
    Ack { length: u64 },
    Close,
}

#[derive(Debug, Clone)]
struct Message {
    session: u64,
    message_type: MessageType,
}

impl Message {
	const CONNECT_PAT: Lazy<Regex> = Lazy::new(|| {
		Regex::new(r#"^/connect/(\d+)/$"#).unwrap()
	});
	const DATA_PAT: Lazy<Regex> = Lazy::new(|| {
		Regex::new(r#"^/data/(\d+)/(\d+)/.*/$"#).unwrap()
	});
	const ACK_PAT: Lazy<Regex> = Lazy::new(|| {
		Regex::new(r#"^/ack/(\d+)/(\d+)/$"#).unwrap()
	});
	const CLOSE_PAT: Lazy<Regex> = Lazy::new(|| {
		Regex::new(r#"^/close/(\d+)/$"#).unwrap()
	});

	fn unescape_data(data: &str) -> String {
		// todo!()
		return data.to_string();
	}
	fn escape_data(data: &str) -> String {
		// todo!()
		return data.to_string();
	}

	pub fn parse(data: Vec<u8>) -> Option<Self> {
		let data = String::from_utf8(data).ok()?;

		if !data.starts_with('/')  || !data.ends_with('/') {
			return None
		}
		let second_slash_loc = data[1..].find('/')? +1;

		// Escape only for /data/ message
		let message_type = &data[1..second_slash_loc];
		Some(match message_type {
			"connect" => {
				let mt = Self::CONNECT_PAT.captures(&data)?;
				let session: u64 = mt.get(1).unwrap().as_str().parse().ok()?;
				Self {
					session,
					message_type: MessageType::Connect
				}
			},
			"data" => {
				let mt = Self::DATA_PAT.captures(&data)?;
				let session: u64 = mt.get(1).unwrap().as_str().parse().ok()?;
				let pos: u64 = mt.get(2).unwrap().as_str().parse().ok()?;
				let content = mt.get(3).unwrap().as_str();
				Self {
					session,
					message_type: MessageType::Data { pos, data: Self::unescape_data(content) }
				}

			},
			"ack" => {
				let mt = Self::ACK_PAT.captures(&data)?;
				let session: u64 = mt.get(1).unwrap().as_str().parse().ok()?;
				let length: u64 = mt.get(2).unwrap().as_str().parse().ok()?;
				Self {
					session,
					message_type: MessageType::Ack { length }
				}
			},
			"close" => {
				let mt = Self::CLOSE_PAT.captures(&data)?;
				let session: u64 = mt.get(1).unwrap().as_str().parse().ok()?;
				Self {
					session,
					message_type: MessageType::Close
				}
			},
			_ => {
				return None
			}
		})
	}

	fn serialize(&self) -> Vec<u8> {
		let session = self.session;
		let ser_str = match self.message_type {
			MessageType::Connect => {
				format!("/connect/{session}/")
			},
			MessageType::Data { pos, data } => {
				// TODO split into multiple?
				format!("/data/{session}/{pos}/{data}", data=Self::escape_data(&data))
			},
			MessageType::Ack { length } => {
				format!("/ack/{session}/{length}")
			},
			MessageType::Close => {
				format!("/close/{session}/")
			},
		};

		ser_str.as_bytes().to_vec()
	}

}

/// Used by application layer
struct LRStream {
	write_half: LRStreamWriteHalf,
	read_half: LRStreamReadHalf,
}
impl LRStream {
	fn new(addr: SocketAddr, recv_q: tokio::sync::mpsc::UnboundedReceiver<Message>, send_q: tokio::sync::mpsc::UnboundedSender<(Message, SocketAddr)>) -> Self {
		Self {
			write_half: LRStreamWriteHalf { addr, send_q },

			read_half: LRStreamReadHalf {recv_q}
		}
	}
	async fn recv(&mut self) -> anyhow::Result<Message> {
		self.read_half.recv().await
	}
	fn send(&self, message: Message) -> anyhow::Result<()> {
		self.write_half.send(message)
	}
}
struct LRStreamWriteHalf {
	addr: SocketAddr,
	send_q: tokio::sync::mpsc::UnboundedSender<(Message, SocketAddr)>,
}

struct LRStreamReadHalf {
	recv_q: tokio::sync::mpsc::UnboundedReceiver<Message>,
}
impl LRStreamReadHalf {
	async fn recv(&mut self) -> anyhow::Result<Message> {
		Ok(self.recv_q.recv().await.context("Closed Connection")?)
	}
}
impl LRStreamWriteHalf {
	fn send(&self, message: Message) -> anyhow::Result<()> {
		self.send_q.send((message, self.addr))?;
		Ok(())
	}
}

async fn handle_stream(stream: LRStream) -> anyhow::Result<()> {
	// LRStream.
	// send ack first
	Ok(())
}

/// Used by protocol layer
struct Connection {
	addr: SocketAddr,
	message_box: tokio::sync::mpsc::UnboundedSender<Message>
}


async fn run() -> anyhow::Result<()> {
    let sock = tokio::net::UdpSocket::bind("0.0.0.0:3007").await?;
	let sock = Arc::new(sock);
	let sock_sender = sock.clone();
	let mut connections: HashMap<Session, Connection> = HashMap::new();
	let connections = Arc::new(Mutex::new(connections));
	let connections_sender = connections.clone();
	let (message_send_tx, mut message_send_rx) = tokio::sync::mpsc::unbounded_channel::<(Message, SocketAddr)>();

	// Recie
    let reciever = || async move {
		let mut active_connections: HashMap<Session, Connection> = Default::default();

		loop {
			let mut buf = [0u8; 1024];
			let (len, addr) = sock.recv_from(&mut buf).await?;
			let buf = buf[..len].to_vec();

			let message = match Message::parse(buf) {
				Some(x) => x,
				None => continue
			};

			match message.message_type {
				MessageType::Connect => {
					let conn = &mut active_connections;
					if !conn.contains_key(&message.session) {
						let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
						conn.insert(message.session, Connection { addr, message_box: tx });
						let lsend_tx = message_send_tx.clone();
						tokio::spawn(async move {
							handle_stream(LRStream::new(
								addr,
								rx,
								lsend_tx,
							)).await;
						});
					}
				},
				_ => unimplemented!()
			}
		}
		Ok(()) as anyhow::Result<()>
    };

	let sender = || async move {
		loop {
			let msg = message_send_rx.recv().await.unwrap();
			sock_sender.send_to(&msg.0.serialize(), msg.1).await?;
		}
		Ok(()) as anyhow::Result<()>
	};
	select! {
		_ = sender() => {},
		_ = reciever() => {},
	};
	Ok(())
}


pub fn main() -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { run().await })?;
	Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
}
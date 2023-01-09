
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

impl MessageType {
	fn split_data(pos: u64, data: String) -> Vec<(u64, String)> {
		// \ counts as 2 and / also counts as 2
		// max_size is 800 let's say
		let max_size = 800;
		let mut consumed_pos = pos;
		let mut curr_string = String::new();
		let mut curr_size = 0;
		let mut rv = vec![];

		let char_size = |c: char| -> u64 {
			match c {
				'\\' => 2, 
				'/' => 2,
				_ => 1
			}
		};

		for (i, char) in data.chars().enumerate() {
			if curr_size + char_size(char) > max_size {
				let curr_pos = consumed_pos;
				consumed_pos += curr_string.len() as u64; 
				rv.push((curr_pos, curr_string.clone()));

				curr_string.clear();
				curr_string.push(char);
				curr_size = char_size(char);
			} else {
				curr_string.push(char);
				curr_size += char_size(char);
			}
		}
		rv.push((consumed_pos, curr_string));
		rv
	}
}

#[derive(Debug, Clone)]
struct ClientMessage {
    session: u64,
    message_type: MessageType,
}

impl ClientMessage {
	const CONNECT_PAT: Lazy<Regex> = Lazy::new(|| {
		Regex::new(r#"^/connect/(\d+)/$"#).unwrap()
	});
	const DATA_PAT: Lazy<Regex> = Lazy::new(|| {
		Regex::new(r#"^/data/(\d+)/(\d+)/((?:.|\n)*)/$"#).unwrap()
	});
	const ACK_PAT: Lazy<Regex> = Lazy::new(|| {
		Regex::new(r#"^/ack/(\d+)/(\d+)/$"#).unwrap()
	});
	const CLOSE_PAT: Lazy<Regex> = Lazy::new(|| {
		Regex::new(r#"^/close/(\d+)/$"#).unwrap()
	});

	fn unescape_data(data: &str) -> String {
		let data = data.replace(r#"\/"#, r#"/"#);
		let data = data.replace(r#"\\"#, r#"\"#);
		data
	}
	fn escape_data(data: &str) -> String {
		let data = data.replace('\\', r#"\\"#);
		let data = data.replace('/', r#"\/"#);
		data
	}

	pub fn parse(data: Vec<u8>) -> Option<Self> {
		let data = String::from_utf8(data).ok()?;

		// data.pop(); // \n
		dbg!("Valid utf8", &data);
		if !data.starts_with('/')  || !data.ends_with('/') {
			return None
		}
		dbg!("Data", &data);
		let second_slash_loc = data[1..].find('/')? +1;

		// Escape only for /data/ message
		let message_type = &data[1..second_slash_loc];
		dbg!("Matching", message_type);
		Some(match message_type {
			"connect" => {
				dbg!("Matchedwith connect");
				let mt = Self::CONNECT_PAT.captures(&data)?;
				let session: u64 = mt.get(1).unwrap().as_str().parse().ok()?;
				Self {
					session,
					message_type: MessageType::Connect
				}
			},
			"data" => {
				let mt = Self::DATA_PAT.captures(&data)?;
				dbg!(&mt);
				let session: u64 = mt.get(1).unwrap().as_str().parse().ok()?;
				let pos: u64 = mt.get(2).unwrap().as_str().parse().ok()?;
				let content = mt.get(3).unwrap().as_str();
				let unescaped_content = Self::unescape_data(content);
				let total_size = unescaped_content.len() + unescaped_content.chars().filter(|x| *x == '\\' || *x == '/').count();
				if total_size != content.len() {
					return None // Invalid escape
				}
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
		let ser_str = match &self.message_type {
			MessageType::Connect => {
				format!("/connect/{session}/")
			},
			MessageType::Data { pos, data } => {
				// TODO split into multiple?
				format!("/data/{session}/{pos}/{data}/", data=Self::escape_data(&data))
			},
			MessageType::Ack { length } => {
				format!("/ack/{session}/{length}/")
			},
			MessageType::Close => {
				format!("/close/{session}/")
			},
		};

		ser_str.as_bytes().to_vec()
	}

	fn get_ack(&self) -> Option<ClientMessage> {
		let t = match &self.message_type {
			MessageType::Data { pos, data } => {
				Some(MessageType::Ack { length: pos + data.len() as u64 })
			},
			MessageType::Connect => {
				Some(MessageType::Ack { length: 0 })
			},
			MessageType::Close => {
				Some(MessageType::Close)
			},
			MessageType::Ack { length } => {
				None
			}
		};
		Some(ClientMessage { session: self.session, message_type: t? })
	}
}


#[derive(Debug)]
struct InternalMessage {
	send_msg: (u64, String),
	last_sends: Vec<chrono::DateTime<chrono::Utc>>
}

impl InternalMessage {
	fn ack_until(&self) -> u64 {
		return self.send_msg.0 + self.send_msg.1.len() as u64
	}
	fn is_expired(&self) -> bool {
		let total_duration = chrono::Duration::seconds(60);
		if let Some(x) = self.last_sends.first() {
			return (chrono::Utc::now() - *x) > total_duration
		};
		return false;
	}
	fn tick(&mut self) -> chrono::DateTime<chrono::Utc> {
		let tick_gaps = chrono::Duration::seconds(3);
		let curr_run = chrono::Utc::now();
		self.last_sends.push(curr_run);
		curr_run + tick_gaps
	}

	fn to_client_msg(&self) -> MessageType {
		return MessageType::Data { pos: self.send_msg.0, data: self.send_msg.1.clone() }
	}
}

#[derive(Debug)]
enum Message {
	Internal(InternalMessage),
	Client(ClientMessage)
}

struct ActiveConnection {
	sent_ack_until: u64,
	sent_until: u64,
	consumed_until: u64,
	buffered: String,
	send_buffered: Vec<(u64, String)>,
	msg_send: tokio::sync::mpsc::Sender<Message>,
	udp_conn: Arc<tokio::net::UdpSocket>, 
	session: Session,
	peer_addr: SocketAddr
}
impl ActiveConnection {
	fn recv_until(&self) -> u64 {
		self.buffered.len() as u64 + self.consumed_until
	}

	fn new(send: tokio::sync::mpsc::Sender<Message>, conn: Arc<tokio::net::UdpSocket>, session: Session, peer_addr: SocketAddr) -> Self {
		ActiveConnection {
			sent_ack_until: 0,
			consumed_until: 0,
			buffered: String::new(),
			send_buffered: vec![],
			sent_until: 0,
			msg_send: send, udp_conn: conn,
			session,
			peer_addr
		}
	}

	async fn send_msg(&mut self, msg: MessageType) -> anyhow::Result<()> {
		let cm = ClientMessage {session: self.session, message_type: msg};
		self.udp_conn.send_to(&cm.serialize(), self.peer_addr).await?;
		Ok(())
	}

	async fn process_buffers(&mut self) -> anyhow::Result<()> {
		// Pick from send buffer if something is remaining
		// Otherwise see from recieved buffer is newline is present

		// Recieved buffer
		if dbg!(self.send_buffered.is_empty()) {
			if let Some(i) = self.buffered.find('\n') {
				// let buf_clone = self.buffered.clone();
				let (curr, rest) = self.buffered.split_at(i+1);
				let mut curr = curr.to_string();
				self.consumed_until += curr.len() as u64;
				self.buffered = rest.to_string();
				curr.pop(); // reomove `/n`
				dbg!(curr.clone());
				curr = curr.chars().rev().collect();
				curr.push('\n');
				let mut splits = dbg!(MessageType::split_data(self.sent_until, curr));
				splits.reverse();
				self.send_buffered = splits;
			}
		}

		if let Some((pos, content)) = self.send_buffered.pop() {
			self.sent_until = pos + content.len() as u64;
			self.msg_send.send(Message::Internal(InternalMessage { send_msg: (pos, content.clone()), last_sends: vec![] })).await?;
			// self.send_msg(MessageType::Data { pos, data: content }).await?;
		}

		Ok(())
	}

	/// Returns true if connection should be closed
	async fn handle_msg(&mut self, msg: Message) -> anyhow::Result<bool> {
		match msg {
			Message::Internal(mut msg) => {
				if msg.ack_until() <= self.sent_ack_until {
					// Already ack
					return Ok(false);
				}
				if msg.is_expired() {
					return Ok(true);
				}
				let client_msg = msg.to_client_msg();
				self.send_msg(client_msg).await?;
				let next_run = msg.tick();

				let msg_sender = self.msg_send.clone();
				tokio::spawn(async move {
					let sleep_time = next_run - chrono::Utc::now();
					tokio::time::sleep(sleep_time.to_std().expect("non zero wait")).await;
					msg_sender.send(Message::Internal(msg)).await;
				});

			},
			Message::Client(cm) => {
				match cm.message_type {
					MessageType::Connect => {
						self.send_msg(MessageType::Ack { length: 0 }).await?;
					},
					MessageType::Data { pos, data } => {
						// Got new data
						if pos <= self.recv_until() && pos + data.len() as u64 > self.recv_until() { 
							let new_data = pos + data.len() as u64 - self.recv_until();
							self.buffered += &data[((data.len() as u64 - new_data) as usize)..];
						} 
						self.send_msg(MessageType::Ack { length: self.recv_until() }).await?;
						self.process_buffers().await?;
					},
					MessageType::Ack { length } => {
						if length > self.sent_until {
							self.send_msg(MessageType::Close).await?;
							return Ok(true);
						};
						self.sent_ack_until = length.max(self.sent_ack_until); 
						self.process_buffers().await?;
					},
					MessageType::Close => {
						self.send_msg(MessageType::Close).await?;
						return Ok(true)
					}
				}
			}
		};

		Ok(false)
	}
}

enum LRConnectionState {
	Closed,
	Active(ActiveConnection)
}
struct LRConnection {
	state: LRConnectionState,
	/// To only be used for sending packets
	udp_conn: Arc<tokio::net::UdpSocket>, 
	peer_addr: SocketAddr,
	session: Session,
	msg_recv: tokio::sync::mpsc::Receiver<Message>,
	msg_send: tokio::sync::mpsc::Sender<Message>
}

impl LRConnection {
	pub fn new(conn: Arc<tokio::net::UdpSocket>, peer_addr: SocketAddr, session: Session) -> Self {
		let (tx, rx) = tokio::sync::mpsc::channel(1024);
		Self {
			state: LRConnectionState::Closed,
			udp_conn: conn,
			peer_addr,
			session,
			msg_recv: rx,
			msg_send: tx
		}
	}
	async fn send_msg(&mut self, msg: MessageType) -> anyhow::Result<()> {
		let cm = ClientMessage {session: self.session, message_type: msg};
		self.udp_conn.send_to(&cm.serialize(), self.peer_addr).await?;
		Ok(())
	}
	async fn run(&mut self) -> anyhow::Result<()> {
		loop {
			let msg = self.msg_recv.recv().await.context("queue closed")?;
			self.handle_msg(msg).await?;
		}
	}
	async fn handle_msg(&mut self, msg: Message) -> anyhow::Result<()> {
		match &mut self.state {
			LRConnectionState::Closed => {
				match msg {
					Message::Client(ClientMessage { session, message_type: MessageType::Connect }) => {
						self.state = LRConnectionState::Active(ActiveConnection::new(self.msg_send.clone(), self.udp_conn.clone(), self.session, self.peer_addr));
						self.send_msg(MessageType::Ack { length: 0 }).await?;
					},
					_ => { self.send_msg(MessageType::Close).await?  }
				}
			}
			LRConnectionState::Active(ac) => {
				if ac.handle_msg(msg).await? {
					self.state = LRConnectionState::Closed
				}
			}
		}
		Ok(())
	}
}

async fn run() -> anyhow::Result<()> {
	let connection = Arc::new(tokio::net::UdpSocket::bind("0.0.0.0:3007").await?);
	let mut connections : HashMap<u64, tokio::sync::mpsc::Sender<Message>>= HashMap::new();
	loop {
		let mut buf = [0u8; 1024];
		let (len, addr) = connection.recv_from(&mut buf).await?;
		let packet = buf[..len].to_vec();
		let p = match dbg!(ClientMessage::parse(packet)) {
			Some(x) => x,
			None => continue
		};
		if connections.get(&p.session).is_none() {
			// let (tx, rx) = tokio::sync::mpsc::channel(1024);
			// connections.insert(p.session, tx);
			let mut conn = LRConnection::new(connection.clone(), addr, p.session);
			let tx = conn.msg_send.clone();
			tokio::spawn(async move {
				conn.run().await?;
				Ok(()) as anyhow::Result<()>
			});
			connections.insert(p.session, tx);
		}
		connections.get(&p.session).unwrap().send(dbg!(Message::Client(p))).await?;
		
		// connection.send_to(buf, target);
	}
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
    use std::sync::Mutex;

    use super::*;

	#[tokio::test]
	async fn test_p7() -> anyhow::Result<()> {
		tokio::spawn(run());
		tokio::time::sleep(Duration::from_millis(10)).await;
		let sock = Arc::new(tokio::net::UdpSocket::bind("0.0.0.0:8080").await?);
		sock.connect("0.0.0.0:3007".parse::<SocketAddr>()?).await?;
		let sock_pri = sock.clone();
		let msg_history: Arc<Mutex<Vec<(bool, String)>>> = Arc::new(Mutex::new(Default::default()));
		let msg_hist2 = msg_history.clone();
		tokio::spawn(async move {
			loop {
				let mut buf =  [0u8; 1024];
				let len = sock_pri.recv(&mut buf).await?;
				let msg = String::from_utf8(buf[..len].to_vec())?;
				msg_hist2.lock().unwrap().push((true, msg.clone()));
				println!("=== Msg Recvd: {}",  msg);
			}
			Ok(()) as anyhow::Result<()>
		});
		let sends = vec![
			("/connect/12345/", Duration::from_millis(10)),
			("/data/12345/0/hello\n/", Duration::from_millis(10)),
			("/ack/12345/6/", Duration::from_millis(10)),
			("/data/12345/6/Hello, world!\n/", Duration::from_millis(10)),
			("/ack/12345/20/", Duration::from_millis(10)),
			("/close/12345/", Duration::from_millis(10)),
		];
		for (msg, wait) in sends {
			sock.send(msg.as_bytes()).await?;
			msg_history.lock().unwrap().push((false, msg.to_string()));
			println!("=== Msg Sent: {}",  msg);
			tokio::time::sleep(wait).await;
		}
		dbg!(msg_history.lock().unwrap().clone());
		Ok(())
		// let client = tokio::net::UdpSocket::sen
	}
}
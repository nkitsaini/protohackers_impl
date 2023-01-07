use std::net::{SocketAddr, ToSocketAddrs};

use anyhow::{bail, Ok};
use once_cell::sync::Lazy;

use crate::prelude::*;

type Timestamp = u32;
type CarId = String;
type Mile = u16;
type Speed = u16;
type RoadId = u16;
type Limit = u16;
type Interval = u32;

struct Ticket {
    plate: String,
    road: RoadId,
    mile1: Mile,
    timestamp1: Timestamp,
    mile2: Mile,
    timestamp2: Timestamp,
    speed: Speed,
}

impl Ticket {
    pub const MESSAGE_NO: u8 = 0x21;
}

enum Message {
	// Server -> Client
    Error {
        msg: String,
    },
	// Client (Camera) -> Server 
    Plate {
        plate: String,
        timestamp: Timestamp,
    },
	// Server -> Client (Dispatcher)
    Ticket(Ticket),

	// Client -> Server 
    WantHeartBeat {
        interval: Interval,
    }, // deciseconds
	// Server -> Client 
    HeartBeat,
	// Client(camera) -> Server 
    IAmCamera {
        road: RoadId,
        mile: Mile,
        limit: Limit,
    }, // limit = mile/hour
	// Client(Dispatcher) -> Server 
    IAmDispatcher {
        roads: Vec<RoadId>,
    },
}

impl Message {
    async fn parse_next(
        reader: &mut tokio::io::BufReader<tokio::net::tcp::OwnedReadHalf>,
    ) -> anyhow::Result<Self> {
        let u = reader.read_u8().await?;
        Ok(match u {
            0x20 => {
                let plate = Self::read_str(reader).await?;
                let timestamp = reader.read_u32().await?;
                Self::Plate { plate, timestamp }
            }
            0x40 => Self::WantHeartBeat {
                interval: reader.read_u32().await?,
            },
            0x80 => {
                let road = reader.read_u16().await?;
                let mile = reader.read_u16().await?;
                let limit = reader.read_u16().await?;
                Self::IAmCamera { road, mile, limit }
            }
            0x81 => {
                let numroads = reader.read_u8().await?;
                let mut roads = vec![];
                for _ in 0..numroads {
                    roads.push(reader.read_u16().await?);
                }
                Self::IAmDispatcher { roads }
            }
            _ => bail!("Unknown Message type"),
        })
    }

    async fn read_str(
        reader: &mut tokio::io::BufReader<tokio::net::tcp::OwnedReadHalf>,
    ) -> anyhow::Result<String> {
        let length = reader.read_u8().await?;
        let mut a = Vec::with_capacity(length as usize);
        reader.read_exact(&mut a).await?;
        Ok(String::from_utf8(a)?)
    }

    fn serialize_str(val: &str) -> Vec<u8> {
        let mut rv = vec![];
        rv.push(val.len() as u8);
        rv.append(&mut val.as_bytes().to_vec());
        rv
    }

    async fn serialize(&self) -> Vec<u8> {
        let mut rv = vec![];
        match self {
            Self::Error { msg } => {
                rv.push(0x10);
                rv.append(&mut Self::serialize_str(msg));
            }
            Self::Ticket(Ticket {
                plate,
                road,
                mile1,
                timestamp1,
                mile2,
                timestamp2,
                speed,
            }) => {
                rv.push(Ticket::MESSAGE_NO);
                rv.append(&mut Self::serialize_str(plate));
                rv.append(&mut road.to_be_bytes().to_vec());
                rv.append(&mut mile1.to_be_bytes().to_vec());
                rv.append(&mut timestamp1.to_be_bytes().to_vec());
                rv.append(&mut mile2.to_be_bytes().to_vec());
                rv.append(&mut timestamp2.to_be_bytes().to_vec());
                rv.append(&mut speed.to_be_bytes().to_vec());
            }
            Self::HeartBeat => {
                rv.push(0x41);
            }
            _ => unreachable!(),
        }
        rv
    }
}

// struct MessageParser { }
// impl MessageParser {
// 	async fn parse_msg(val: &mut tokio::io::BufReader<tokio::net::tcp::OwnedReadHalf>) -> anyhow::Result<Message> {
// 		loop {
// 			yield Message::parse_next(val).await;
// 		}

// 	}
// }

enum ClientType {
    Camera,
    Dispatcher,
}

#[derive(Debug, Default)]
struct RunningCar {
    history: HashMap<RoadId, BTreeMap<Timestamp, Mile>>,
	plate: String
}

impl RunningCar {
	fn new(plate: String) -> Self {
		Self {
			history: Default::default(),
			plate
		}
	}
    fn new_entry(
        &mut self,
        road: RoadId,
        mile: Mile,
        ts: Timestamp,
        road_limit: Limit,
    ) -> Vec<Ticket> {
        let road_history = self.history.entry(road).or_default();
		road_history.insert(ts, mile);

		// See road_history to find if user has crossed the limits and create ticket(s) for same
		// See immediate previous `ts`, and immediate next `ts`
		// Assume no duplicate ts for single car
        todo!()
    }
}

#[derive(Debug, Default)]
struct State {
    cars: HashMap<CarId, RunningCar>,
	dispatchers: HashMap<RoadId, Vec<tokio::sync::broadcast::Sender<Message>>>
}

impl State {
	// pub async fn process_event(&mut self, message: Message)
}

struct Client {
	client_type: ClientType,
	reader: tokio::io::BufReader<tokio::net::tcp::OwnedReadHalf>,
	write_half: tokio::net::tcp::OwnedWriteHalf,
}

impl Client {
	async fn run(mut self) -> anyhow::Result<()> {
		// let mut reader = tokio::io::BufReader::new(self.read_half);
		match self._run().await {
			Err(x) => {
				self.write_half.write(&Message::Error { msg: "Something went wrong".to_string() }.serialize().await).await?
			},
			_ => unreachable!()
		};
		Ok(())
	}


	async fn _run(&mut self) -> anyhow::Result<()> {
		// let (cread_half, mut cwrite_half) = self.connection.into_split();
		// let mut reader = tokio::io::BufReader::new(cread_half);
		let mut heartbeat_active = false;
		// let message_task = tokio::task::spawn_local(async move {
		// 	Message::parse_next(&mut self.reader).await?;
		// 	Ok(())
		// });
		loop {
			let message = Message::parse_next(&mut self.reader).await?;
			match message {
				Message::WantHeartBeat { interval } => {
					if heartbeat_active {
						bail!("Heartbeat already active");
					}
					heartbeat_active = true;
					tokio::spawn( async move {

					});
				}
				_ => unreachable!()
			}
		}
		Ok(())
	}
}

async fn handle_client(stream: tokio::net::TcpStream) -> anyhow::Result<()> {
    let (cread_half, mut cwrite_half) = stream.into_split();
    let client: Option<Client> = None;

    Ok(())
}

async fn run() -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3007").await?;

    loop {
        let (stream, _) = listener.accept().await?;
        tokio::task::spawn(async move { dbg!(handle_client(stream).await) });
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
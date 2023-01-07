use std::{net::{SocketAddr, ToSocketAddrs}, ops::Bound};

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

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
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
                // let timestamp = reader.read_u32().await?;
                let timestamp = reader.read_u32().await? as u32;
                Self::Plate { plate, timestamp}
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
        let mut a = vec![0u8; length as usize];
        reader.read_exact(&mut a).await?;
        Ok(String::from_utf8(a)?)
    }

    fn serialize_str(val: &str) -> Vec<u8> {
        let mut rv = vec![];
        rv.push(val.len() as u8);
        rv.append(&mut val.as_bytes().to_vec());
        rv
    }

    fn serialize(&self) -> Vec<u8> {
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

#[derive(Debug, Clone)]
enum ClientType {
    Camera(RoadId, Mile, Limit),
    Dispatcher(Vec<RoadId>),
}

#[derive(Debug, Default)]
struct RunningCar {
    history: HashMap<RoadId, BTreeMap<Timestamp, Mile>>,
    plate: String,
	days_charged: HashSet<u64>
}

impl RunningCar {
    fn new(plate: String) -> Self {
        Self {
            history: Default::default(),
            plate,
			days_charged: Default::default()
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
		let prev = road_history.range(..ts).next_back();
		let next = road_history.range((Bound::Excluded(ts), Bound::Unbounded)).next_back();
        for val in vec![prev, next] {
            if let Some(b_data) = val {
                let mut data2 = (*b_data.0, *b_data.1);
                let mut data1 = (ts, mile);

                if data1.0 > data2.0 {
                    (data1, data2) = (data2, data1);
                }
                // let avg_speed = 
                todo!()

            }
        }

        // See road_history to find if user has crossed the limits and create ticket(s) for same
        // See immediate previous `ts`, and immediate next `ts`
        // Assume no duplicate ts for single car
        // todo!()
		let t  = Ticket{plate: "UN1X".to_string(), road: 123, mile1: 8, timestamp1: 0, mile2: 9, timestamp2: 45, speed: 8000};
		dbg!("Created Ticket", &road_history);

		vec![]
    }
}

#[derive(Debug, Default)]
struct State {
    cars: HashMap<CarId, RunningCar>,
    dispatchers: HashMap<RoadId, Vec<tokio::sync::mpsc::Sender<Message>>>,
    unsend_tickets: HashMap<RoadId, Vec<Ticket>>
}

impl State {
	pub async fn add_dispatcher(&mut self, roads: Vec<RoadId>, tx: tokio::sync::mpsc::Sender<Message>) {
		for road in roads.iter() {
			self.dispatchers.entry(*road).or_default().push(tx.clone());

            let ticket_vec = self.unsend_tickets.entry(*road).or_default();
            for ticket_i in (0..ticket_vec.len()).rev() {
                let a= ticket_vec.get_mut(ticket_i).unwrap();
                tx.send(Message::Ticket(a.clone())).await;
            }
		}
	}

	pub async fn new_entry(&mut self, road: RoadId, mile: Mile, ts: Timestamp, road_limit: Limit, plate: CarId) {
		let tickets = self.cars.entry(plate).or_default().new_entry(road, mile, ts, road_limit);
		for ticket in tickets {
			self.send_ticket(ticket).await;
		}
		dbg!("Done with ticket");
	}

	pub async fn send_ticket(&mut self, ticket: Ticket) {
		let mut new_i = None;
		for (i, disp) in self.dispatchers.entry(ticket.road).or_default().iter().enumerate() {
			if disp.send(Message::Ticket(ticket.clone())).await.is_ok() {
				new_i = Some(i);
				break
			}
		}
		match new_i {
			Some(i) => {
				dbg!("Cleaning dispatchers", i);
				let disps = self.dispatchers.entry(ticket.road).or_default();
				*disps = disps.split_at(i).1.to_vec();

			},
			None => {
				dbg!("No dispatcher");
				self.dispatchers.entry(ticket.road).or_default().clear();
			}
		}
	}
}

struct Client {
    client_type: Option<ClientType>,
    reader: tokio::io::BufReader<tokio::net::tcp::OwnedReadHalf>,
    write_half: tokio::net::tcp::OwnedWriteHalf,
    events_rx: tokio::sync::mpsc::Receiver<Message>,
    events_tx: tokio::sync::mpsc::Sender<Message>,
}

impl Client {
	pub fn new(stream: tokio::net::TcpStream) -> Self {
		let (cread_half, mut cwrite_half) = stream.into_split();
		let (events_tx, events_rx) = tokio::sync::mpsc::channel(128);
		Self {
			client_type: None,
			reader: tokio::io::BufReader::new(cread_half),
			write_half: cwrite_half,
			events_tx,
			events_rx
		}
	}
    async fn run(mut self, state: Arc<tokio::sync::Mutex<State>>) -> anyhow::Result<()> {
        // let mut reader = tokio::io::BufReader::new(self.read_half);
        match self._run(state).await {
            Err(x) => {
				dbg!(x);
                self.write_half
                    .write(
                        &Message::Error {
                            msg: "Something went wrong".to_string(),
                        }
                        .serialize()
                        .await,
                    )
                    .await?
            }
            _ => unreachable!(),
        };
        Ok(())
    }

    async fn _run(&mut self, state: Arc<tokio::sync::Mutex<State>>) -> anyhow::Result<()> {
        let ev_tx1 = self.events_tx.clone();
        let ev_tx2 = self.events_tx.clone();
        let reader = &mut self.reader;
        let mut heartbeat_task = None;
		let client = &mut self.client_type;
        let task1 = || async move {
            let ev_tx1 = ev_tx1;
            loop {
                let message = Message::parse_next(reader).await?;
                match dbg!(message) {
                    Message::WantHeartBeat { interval } => {
                        match heartbeat_task {
                            Some(_) => bail!("Two heartbeat req"),
                            None => {
                                let ev_tx_int = ev_tx1.clone();
                                heartbeat_task = Some(tokio::spawn(async move {
                                    let mut tick_interval = tokio::time::interval(
                                        Duration::from_micros(interval as u64 * 100),
                                    );
                                    loop {
                                        tick_interval.tick().await;
										if interval != 0 {
											ev_tx_int.send(Message::HeartBeat).await?;
										} else {
											// No heartbeats if interval=0
											tokio::time::sleep(Duration::from_secs(10000)).await;
										}
                                    }
                                    Ok(())
                                }));
                            }
                        }
                    },
					Message::Plate { plate, timestamp } => {
						match client.clone() {
							Some(ClientType::Camera(road, mile, limit)) => {
								state.lock().await.new_entry(road, mile, timestamp, limit, plate).await;
							},
							_ => bail!("plate info by non-camera client")
						}

					},
					Message::IAmCamera { road, mile, limit } => {
						match client {
							None => {
								*client = Some(ClientType::Camera(road, mile, limit))
							},
							Some(_) => {
								bail!("Already declared type");
							}

						}
					}
					Message::IAmDispatcher { roads } => {
						match client {
							None => {
								*client = Some(ClientType::Dispatcher(roads.clone()));
								let ev_tx_state = ev_tx2.clone();
								state.lock().await.add_dispatcher(roads, ev_tx_state).await;
							},
							Some(_) => {
								bail!("Already declared type");
							}

						}
					}
                    _ => unreachable!() //Message parse only returns client->server events
                }
            }
            Ok(())
        };
        let event_mut = &mut self.events_rx;
        let writer = &mut self.write_half;
        let task2 = || async move {
            loop {
                let ev = event_mut.recv().await.context("closed")?;
                writer.write(&dbg!(ev).serialize().await).await?;
            }
            Ok(())
        };
        select! {
            x = task1() => { x? },
            y = task2() => { y? }
        }

        Ok(())
    }
}

async fn handle_client(stream: tokio::net::TcpStream, state: Arc<tokio::sync::Mutex<State>>) -> anyhow::Result<()> {
    Client::new(stream).run(state).await?;

    Ok(())
}

async fn run() -> anyhow::Result<()> {
	dbg!("Starting listener");
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3009").await?;
	dbg!("Started listener");
	let state = Arc::new(tokio::sync::Mutex::new(State::default()));

    loop {
        let (stream, _) = listener.accept().await?;
		let st2 = state.clone();
        tokio::task::spawn(async move { dbg!(handle_client(stream, st2).await) });
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
	#[tokio::test]
	async fn test_p6_flow() -> anyhow::Result<()> {
		tokio::spawn(run());
		tokio::time::sleep(Duration::from_micros(5)).await; // Let the server start

		let mut camera1 = tokio::net::TcpStream::connect("0.0.0.0:3009".parse::<SocketAddr>().unwrap()).await?;
		let mut camera2 = tokio::net::TcpStream::connect("0.0.0.0:3009".parse::<SocketAddr>().unwrap()).await?;
		let mut dispatcher1 = tokio::net::TcpStream::connect("0.0.0.0:3009".parse::<SocketAddr>().unwrap()).await?;
		// 
		// Hexadecimal:
		// <-- 80 00 7b 00 08 00 3c
		// <-- 20 04 55 4e 31 58 00 00 00 00
		// 
		// Decoded:
		// <-- IAmCamera{road: 123, mile: 8, limit: 60}
		// <-- Plate{plate: "UN1X", timestamp: 0}
		camera1.write(&hex::decode("80 00 7b 00 08 00 3c".replace(" ", "")).unwrap()).await?;
		camera1.write(&hex::decode("20 04 55 4e 31 58 00 00 00 00".replace(" ", "")).unwrap()).await?;

		// 
		// Client 2: camera at mile 9
		// 
		// Hexadecimal:
		// <-- 80 00 7b 00 09 00 3c
		// <-- 20 04 55 4e 31 58 00 00 00 2d
		// 
		// Decoded:
		// <-- IAmCamera{road: 123, mile: 9, limit: 60}
		// <-- Plate{plate: "UN1X", timestamp: 45}
		// 
		dispatcher1.write(&hex::decode("81 01 00 7b".replace(" ", "")).unwrap()).await?;
		tokio::time::sleep(Duration::from_micros(5)).await; // let dispatcher register to capture ticket


		camera2.write(&hex::decode("80 00 7b 00 09 00 3c".replace(" ", "")).unwrap()).await?;
		camera2.write(&hex::decode("20 04 55 4e 31 58 00 00 00 2d".replace(" ", "")).unwrap()).await?;

		// tokio::time::sleep(Duration::from_micros(500)).await;
		// Client 3: ticket dispatcher
		// 
		// Hexadecimal:
		// <-- 81 01 00 7b
		// --> 21 04 55 4e 31 58 00 7b 00 08 00 00 00 00 00 09 00 00 00 2d 1f 40
		// 
		// Decoded:
		// <-- IAmDispatcher{roads: [123]}
		// --> Ticket{plate: "UN1X", road: 123, mile1: 8, timestamp1: 0, mile2: 9, timestamp2: 45, speed: 8000}

		let mut v = vec![0u8; 22];
		dbg!("Reading");
		select! {
			_ = dispatcher1.read_exact(&mut v) => {
				assert_eq!(
					&v, 
					&hex::decode("21 04 55 4e 31 58 00 7b 00 08 00 00 00 00 00 09 00 00 00 2d 1f 40".replace(" ", "")).unwrap()
				);
			},
			_ = tokio::time::sleep(Duration::from_secs(1)) => {
				bail!("Took too long to read")
			}
		}

		// assert!(false);
		Ok(())

	}

}

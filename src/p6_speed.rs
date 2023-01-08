use std::{
    net::{SocketAddr, ToSocketAddrs},
    ops::Bound,
};

use anyhow::{bail, Ok};
use once_cell::sync::Lazy;
use parking_lot::lock_api::RawRwLockDowngrade;

use crate::prelude::*;

type Timestamp = u32;
type CarId = String;
type Mile = u16;
type Speed = u16;
type RoadId = u16;
type Limit = u16;
type Interval = u32;

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
            // For testing
            // Ticket::MESSAGE_NO => {

            //     Self::Ticket(Ticket {
            //         plate: Self::read_str(reader).await?,
            //         road: reader.read_u16().await?,
            //         mile1: reader.read_u16().await?,
            //         timestamp1: reader.read_u32().await?,
            //         mile2: reader.read_u16().await?,
            //         timestamp2: reader.read_u32().await?,
            //         speed: reader.read_u16().await?,
            //     })
            // },
            // 0x10 => {
            //     Self::Error { msg: Self::read_str(reader).await? }
            // }
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
            // For testing
            Self::IAmCamera { road, mile, limit } => {
                rv.push(0x80);
                rv.append(&mut road.to_be_bytes().to_vec());
                rv.append(&mut mile.to_be_bytes().to_vec());
                rv.append(&mut limit.to_be_bytes().to_vec());
            }
            Self::Plate { plate, timestamp } => {
                rv.push(0x20);
                rv.append(&mut Self::serialize_str(plate));
                rv.append(&mut timestamp.to_be_bytes().to_vec());
            }
            Self::IAmDispatcher { roads } => {
                rv.push(0x81);
                rv.push(roads.len() as u8);
                for road in roads {
                    rv.append(&mut road.to_be_bytes().to_vec());
                }
            }

            _ => unreachable!(),
        }
        rv
    }
}

#[derive(Debug, Clone)]
enum ClientType {
    Camera(RoadId, Mile, Limit),
    Dispatcher(Vec<RoadId>),
}

#[derive(Debug, Default)]
struct RunningCar {
    history: HashMap<RoadId, BTreeMap<Timestamp, Mile>>,
    plate: String,
    days_charged: HashSet<u32>,
}

impl RunningCar {
    fn new(plate: String) -> Self {
        Self {
            history: Default::default(),
            plate,
            days_charged: Default::default(),
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
        let next = road_history
            .range((Bound::Excluded(ts), Bound::Unbounded))
            .next_back();
        let mut tickets = vec![];
        for val in vec![prev, next] {
            if let Some(b_data) = val {
                let mut data2 = (*b_data.0, *b_data.1);
                let mut data1 = (ts, mile);

                if data1.0 > data2.0 {
                    (data1, data2) = (data2, data1);
                }
                assert!(data1.0 != data2.0);
                let avg_speed =
                    data1.1.abs_diff(data2.1) as f64 / ((data2.0 - data1.0) as f64 / 3600.);
                let avg_speed_100 = (avg_speed * 100 as f64).floor();
                // dbg!(avg_speed, road_limit);
                let first_day = data1.0 / (3600 * 24);
                let last_day = data2.0 / (3600 * 24);
                if (first_day..(last_day + 1)).any(|x| self.days_charged.contains(&x)) {
                    continue; // Already ticketed for the day
                }

                // {
                //     if self.days_charged.contains(&day) {
                //         continue
                //     }
                // }
                if avg_speed > road_limit as f64 {
                    for day in first_day..(last_day + 1) {
                        self.days_charged.insert(day);
                    }
                    tickets.push(Ticket {
                        plate: self.plate.clone(),
                        road: road,
                        mile1: data1.1,
                        timestamp1: data1.0,
                        mile2: data2.1,
                        timestamp2: data2.0,
                        speed: avg_speed_100 as u16,
                    });
                }
            }
        }

        // See road_history to find if user has crossed the limits and create ticket(s) for same
        // See immediate previous `ts`, and immediate next `ts`
        // Assume no duplicate ts for single car
        // todo!()
        tickets
        // let t  = Ticket{plate: "UN1X".to_string(), road: 123, mile1: 8, timestamp1: 0, mile2: 9, timestamp2: 45, speed: 8000};
        // dbg!("Created Ticket", &road_history);

        // vec![t]
    }
}

#[derive(Debug, Default)]
struct State {
    cars: HashMap<CarId, RunningCar>,
    ticket_queues: HashMap<
        RoadId,
        (
            async_channel::Sender<Message>,
            async_channel::Receiver<Message>,
        ),
    >,
}

impl State {
    pub async fn new() {
        let (s, r) = async_channel::unbounded::<Message>();
        let k = s.clone();
    }

    pub fn road_queues(
        &mut self,
        road_id: RoadId,
    ) -> (
        async_channel::Sender<Message>,
        async_channel::Receiver<Message>,
    ) {
        let et = self
            .ticket_queues
            .entry(road_id)
            .or_insert_with(|| async_channel::unbounded::<Message>());
        et.clone()
    }

    pub async fn new_entry(
        &mut self,
        road: RoadId,
        mile: Mile,
        ts: Timestamp,
        road_limit: Limit,
        plate: CarId,
    ) {
        let tickets = self
            .cars
            .entry(plate)
            .or_insert_with_key(|plate| RunningCar::new(plate.clone()))
            .new_entry(road, mile, ts, road_limit);
        for ticket in tickets {
            // dbg!("Creating ticket", &ticket);
            let (w, _) = self.road_queues(road);
            w.try_send(Message::Ticket(ticket)).unwrap();
            // dbg!(w.len(), w.receiver_count(), w.sender_count());
        }
    }
}

#[derive(Debug)]
struct Client {
    client_type: Option<ClientType>,
    reader: tokio::io::BufReader<tokio::net::tcp::OwnedReadHalf>,
    write_half: tokio::net::tcp::OwnedWriteHalf,
    events_rx: tokio::sync::mpsc::Receiver<(Message, Option<async_channel::Sender<Message>>)>,
    events_tx: tokio::sync::mpsc::Sender<(Message, Option<async_channel::Sender<Message>>)>,
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
            events_rx,
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
                        .serialize(),
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
                dbg!(&message, &client);
                match message {
                    Message::WantHeartBeat { interval } => {
                        match heartbeat_task {
                            Some(_) => bail!("Two heartbeat req"),
                            None => {
                                let ev_tx_int = ev_tx1.clone();
                                heartbeat_task = Some(tokio::spawn(async move {
                                    let my_interval = if interval == 0 { 100 } else { interval };
                                    let mut tick_interval = tokio::time::interval(
                                        Duration::from_millis(my_interval as u64 * 100),
                                    );
                                    loop {
                                        tick_interval.tick().await;
                                        if interval != 0 {
                                            // dbg!("Sending heartbeat");
                                            ev_tx_int.send((Message::HeartBeat, None)).await?;
                                        } else {
                                            // No heartbeats if interval=0
                                            tokio::time::sleep(Duration::from_secs(10000)).await;
                                        }
                                    }
                                    Ok(())
                                }));
                            }
                        }
                    }
                    Message::Plate { plate, timestamp } => match client.clone() {
                        Some(ClientType::Camera(road, mile, limit)) => {
                            state
                                .lock()
                                .await
                                .new_entry(road, mile, timestamp, limit, plate)
                                .await;
                        }
                        _ => bail!("plate info by non-camera client"),
                    },
                    Message::IAmCamera { road, mile, limit } => match client {
                        None => *client = Some(ClientType::Camera(road, mile, limit)),
                        Some(_) => {
                            bail!("Already declared type");
                        }
                    },
                    Message::IAmDispatcher { roads } => match client {
                        None => {
                            *client = Some(ClientType::Dispatcher(roads.clone()));
                            for road in roads {
                                let ev_tx_c = ev_tx1.clone();
                                let state_c = state.clone();
                                tokio::spawn(async move {
                                    let (t, r) = state_c.lock().await.road_queues(road);
                                    loop {
                                        let m = r.recv().await.unwrap();
                                        match ev_tx_c.send((m.clone(), Some(t.clone()))).await {
                                            Err(x) => {
                                                t.try_send(m).unwrap();
                                                bail!(x);
                                            }
                                            Result::Ok(_) => {}
                                        };
                                    }
                                    Ok(())
                                });
                            }
                        }
                        Some(_) => {
                            bail!("Already declared type");
                        }
                    },
                    _ => unreachable!(), //Message parse only returns client->server events
                }
            }
            Ok(())
        };
        let event_mut = &mut self.events_rx;
        let writer = &mut self.write_half;
        let task2 = || async move {
            loop {
                let (ev, revert_q) = event_mut.recv().await.context("closed")?;
                match writer.write_all(&ev.clone().serialize()).await {
                    Result::Ok(_) => {}
                    Err(x) => {
                        // if let Some(q) = revert_q {
                        //     q.try_send(ev).unwrap();
                        // }
                        bail!(x);
                    }
                };
            }
            Ok(())
        };
        // self.events_tx.closed()
        select! {
            x = task1() => {
                while let Result::Ok((ev, revert_q)) = self.events_rx.try_recv() {
                    if let Some(q) = revert_q {
                        q.try_send(ev).unwrap();
                    }
                }

                x?
             },
            y = task2() => {
                while let Result::Ok((ev, revert_q)) = self.events_rx.try_recv() {
                    if let Some(q) = revert_q {
                        q.try_send(ev).unwrap();
                    }
                }

                y?
            }
        }

        Ok(())
    }
}

async fn handle_client(
    stream: tokio::net::TcpStream,
    state: Arc<tokio::sync::Mutex<State>>,
) -> anyhow::Result<()> {
    Client::new(stream).run(state).await?;

    Ok(())
}

async fn run(port_no: u16) -> anyhow::Result<()> {
    dbg!("Starting listener");
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port_no}")).await?;
    dbg!("Started listener");
    let state = Arc::new(tokio::sync::Mutex::new(State::default()));

    let state_c = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        // let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            interval.tick().await;
            let mut total_messages = 0;
            for v in state_c.lock().await.ticket_queues.values() {
                total_messages += v.0.len();
            }
            dbg!("Messages active", total_messages);
        }
    });

    loop {
        let (stream, _) = listener.accept().await?;
        let st2 = state.clone();
        tokio::task::spawn(async move {
            match handle_client(stream, st2).await {
                Err(x) => {
                    dbg!("Client disconnected", x);
                }

                Result::Ok(_) => {}
            }
        });
    }
}

pub fn main() -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { run(3008).await })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn test_p6_flow() -> anyhow::Result<()> {
        let PORT_NO = 3008;
        tokio::spawn(run(PORT_NO));
        tokio::time::sleep(Duration::from_millis(5)).await; // Let the server start

        let socket_addr: SocketAddr = format!("0.0.0.0:{PORT_NO}").parse().unwrap();
        let mut camera1 = tokio::net::TcpStream::connect(socket_addr).await?;
        let mut camera2 = tokio::net::TcpStream::connect(socket_addr).await?;
        let mut dispatcher1 = tokio::net::TcpStream::connect(socket_addr).await?;
        //
        // Hexadecimal:
        // <-- 80 00 7b 00 08 00 3c
        // <-- 20 04 55 4e 31 58 00 00 00 00
        //
        // Decoded:
        // <-- IAmCamera{road: 123, mile: 8, limit: 60}
        // <-- Plate{plate: "UN1X", timestamp: 0}
        camera1
            .write(&hex::decode("80 00 7b 00 08 00 3c".replace(" ", "")).unwrap())
            .await?;
        camera1
            .write(&hex::decode("20 04 55 4e 31 58 00 00 00 00".replace(" ", "")).unwrap())
            .await?;

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

        dispatcher1
            .write(&hex::decode("81 01 00 7b".replace(" ", "")).unwrap())
            .await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        // dispatcher1.shutdown().await;

        camera2
            .write(&hex::decode("80 00 7b 00 09 00 3c".replace(" ", "")).unwrap())
            .await?;
        camera2
            .write(&hex::decode("20 04 55 4e 31 58 00 00 00 2d".replace(" ", "")).unwrap())
            .await?;

        // tokio::time::sleep(Duration::from_millis(30000)).await;
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
                dbg!(String::from_utf8_lossy(&v));
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

    #[tokio::test]
    async fn test_p6_flow_custom() -> anyhow::Result<()> {
        let port_no = 3009;
        tokio::spawn(run(port_no));
        tokio::time::sleep(Duration::from_millis(5)).await; // Let the server start

        let socket_addr: SocketAddr = format!("0.0.0.0:{port_no}").parse().unwrap();
        let mut camera1 = tokio::net::TcpStream::connect(socket_addr).await?;
        let mut camera2 = tokio::net::TcpStream::connect(socket_addr).await?;
        let mut dispatcher1 = tokio::net::TcpStream::connect(socket_addr).await?;

        camera1
            .write(
                &Message::IAmCamera {
                    road: 123,
                    mile: 8,
                    limit: 60,
                }
                .serialize(),
            )
            .await?;
        camera1
            .write(
                &Message::Plate {
                    plate: "UN1X".into(),
                    timestamp: 0,
                }
                .serialize(),
            )
            .await?;

        dispatcher1
            .write(&Message::IAmDispatcher { roads: vec![123] }.serialize())
            .await?;
        tokio::time::sleep(Duration::from_millis(500)).await;

        camera2
            .write(
                &Message::IAmCamera {
                    road: 123,
                    mile: 9,
                    limit: 60,
                }
                .serialize(),
            )
            .await?;
        camera2
            .write(
                &Message::Plate {
                    plate: "UN1X".into(),
                    timestamp: 45,
                }
                .serialize(),
            )
            .await?;

        let ticket = Ticket {
            plate: "UN1X".into(),
            road: 123,
            mile1: 8,
            timestamp1: 0,
            mile2: 9,
            timestamp2: 45,
            speed: 8000,
        };
        dbg!("Reading");
        let (read_half, _write_half) = dispatcher1.into_split();
        let mut disp1_reader = tokio::io::BufReader::new(read_half);
        select! {
            l = Message::parse_next(&mut disp1_reader) => {
                assert_eq!(
                    l?,
                    Message::Ticket(ticket)
                );
            },
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                bail!("Took too long to read")
            }
        }

        // assert!(false);
        Ok(())
    }

    #[tokio::test]
    async fn test_p6_failure() -> anyhow::Result<()> {
        let port_no = 3010;
        let road: RoadId = 1;
        let limit: Limit = 90;
        let plate: String = "UN".into();
        tokio::spawn(run(port_no));
        tokio::time::sleep(Duration::from_millis(5)).await; // Let the server start

        let socket_addr: SocketAddr = format!("0.0.0.0:{port_no}").parse().unwrap();
        let mut camera1 = tokio::net::TcpStream::connect(socket_addr).await?;
        let mut camera2 = tokio::net::TcpStream::connect(socket_addr).await?;
        let mut camera3 = tokio::net::TcpStream::connect(socket_addr).await?;
        let mut camera4 = tokio::net::TcpStream::connect(socket_addr).await?;
        let mut camera5 = tokio::net::TcpStream::connect(socket_addr).await?;
        let mut dispatcher1 = tokio::net::TcpStream::connect(socket_addr).await?;

        camera1
            .write(
                &Message::IAmCamera {
                    road,
                    mile: 68,
                    limit,
                }
                .serialize(),
            )
            .await?;
        camera1
            .write(
                &Message::Plate {
                    plate: plate.clone(),
                    timestamp: 44851675,
                }
                .serialize(),
            )
            .await?;

        camera2
            .write(
                &Message::IAmCamera {
                    road,
                    mile: 194,
                    limit,
                }
                .serialize(),
            )
            .await?;
        camera2
            .write(
                &Message::Plate {
                    plate: plate.clone(),
                    timestamp: 44846713,
                }
                .serialize(),
            )
            .await?;

        camera3
            .write(
                &Message::IAmCamera {
                    road,
                    mile: 5,
                    limit,
                }
                .serialize(),
            )
            .await?;
        camera3
            .write(
                &Message::Plate {
                    plate: plate.clone(),
                    timestamp: 44854156,
                }
                .serialize(),
            )
            .await?;

        camera4
            .write(
                &Message::IAmCamera {
                    road,
                    mile: 398,
                    limit,
                }
                .serialize(),
            )
            .await?;
        camera4
            .write(
                &Message::Plate {
                    plate: plate.clone(),
                    timestamp: 44838681,
                }
                .serialize(),
            )
            .await?;

        camera5
            .write(
                &Message::IAmCamera {
                    road,
                    mile: 596,
                    limit,
                }
                .serialize(),
            )
            .await?;
        camera5
            .write(
                &Message::Plate {
                    plate: plate.clone(),
                    timestamp: 44830884,
                }
                .serialize(),
            )
            .await?;

        dispatcher1
            .write(&Message::IAmDispatcher { roads: vec![road] }.serialize())
            .await?;
        // tokio::time::sleep(Duration::from_millis(500)).await;

        // camera2.write(&Message::IAmCamera { road: 123, mile: 9, limit: 60 }.serialize()).await?;
        // camera2.write(&Message::Plate { plate: "UN1X".into(), timestamp: 45}.serialize()).await?;

        let ticket = Ticket {
            plate: "UN1X".into(),
            road: 123,
            mile1: 8,
            timestamp1: 0,
            mile2: 9,
            timestamp2: 45,
            speed: 8000,
        };
        dbg!("Reading");
        let (read_half, _write_half) = dispatcher1.into_split();
        let mut disp1_reader = tokio::io::BufReader::new(read_half);
        select! {
            l = Message::parse_next(&mut disp1_reader) => {
                dbg!(l?);
                // assert_eq!(
                // 	l?,
                // 	Message::Ticket(ticket)
                // );
            },
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                bail!("Took too long to read")
            }
        }
        select! {
            l = Message::parse_next(&mut disp1_reader) => {
                dbg!(l?);
                // assert_eq!(
                // 	l?,
                // 	Message::Ticket(ticket)
                // );
            },
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                bail!("Took too long to read")
            }
        }
        select! {
            l = Message::parse_next(&mut disp1_reader) => {
                dbg!(l?);
                // assert_eq!(
                // 	l?,
                // 	Message::Ticket(ticket)
                // );
            },
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                bail!("Took too long to read")
            }
        }

        assert!(false);
        Ok(())
    }
}

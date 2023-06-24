use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::bail;
use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio::{
    io::{AsyncReadExt, BufReader},
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
};

use self::data_types::{
    Action, Message, PolicyId, PopulationObservation, PopulationTarget, SiteId, Species,
};

mod data_types;
const TOO_MUCH_LENGTH: u32 = 1024 * 1024 * 50; // 50 MB
type Reader = BufReader<OwnedReadHalf>;
type Writer = OwnedWriteHalf;

type PopulationMap = HashMap<Species, PopulationTarget>;

#[derive(Debug, Clone)]
struct Policy {
    id: PolicyId,
    action: Action,
    species: Species,
}

struct Site {
    id: SiteId,
    target_population: PopulationMap,

    authority: Connection,
    policies: HashMap<Species, Policy>,
}

impl Site {
    async fn new(id: SiteId) -> anyhow::Result<Self> {
        let mut conn = establish_authority_connection().await?;
        conn.send_message(&Message::DialAuthority { site: id })
            .await?;
        let msg = conn.read_msg().await?;
        let target_populations = match msg {
            Message::TargetPopulations { site, populations } => populations.to_map(),
            _ => bail!("Was expecting target populations"),
        };

        Ok(Self {
            id,
            target_population: target_populations,
            authority: conn,
            policies: HashMap::new(),
        })
    }

    async fn register_population(
        &mut self,
        curr_population: Vec<PopulationObservation>,
    ) -> anyhow::Result<()> {
        let curr_population = {
            let mut rv = HashMap::new();
            for population in curr_population {
                let old = rv.insert(population.species.clone(), population.clone());
                if let Some(x) = old {
                    if x.count != population.count {
                        bail!(
                            "Conflicting observations for population of {:?} at site: {}",
                            x,
                            self.id
                        );
                    }
                }
            }
            rv
        };
        for (species, target_population) in self.target_population.clone() {
            dbg!(
                &target_population,
                &curr_population.get(&species),
                self.id,
                &species
            );
            let count = match curr_population.get(&species) {
                None => 0,
                Some(x) => x.count,
            };

            let action = if count < target_population.min {
                Some(Action::Conserve)
            } else if count > target_population.max {
                Some(Action::Cull)
            } else {
                None
            };

            dbg!(action, &species, self.id);
            self.update_policy(species, action).await?;
        }
        Ok(())
    }

    async fn update_policy(
        &mut self,
        species: Species,
        new_action: Option<Action>,
    ) -> anyhow::Result<()> {
        let old_policy = self.policies.get(&species).cloned();

        dbg!("start", &species, self.policies.get(&species));
        if old_policy.is_some() && Some(old_policy.clone().unwrap().action) != new_action {
            println!(">> Deleting old policy {} self.id={}", &species, self.id);
            self.authority
                .send_message(&Message::DeletePolicy {
                    policy: old_policy.clone().unwrap().id,
                })
                .await?;
            match self.authority.read_msg().await? {
                Message::Ok => {}
                _ => bail!("Expected Ok while deleting policy"),
            };
            self.policies.remove(&species);
            println!(">> Deleted {} self.id={}", &species, self.id);
        }

        if new_action.is_none() {
            println!(">> Not touching new policy self.id={}", self.id);
            return Ok(());
        }
        let new_action = new_action.unwrap();

        if old_policy.is_none() || old_policy.unwrap().action != new_action {
            println!(">> Creating new policy, {} self.id={}", &species, self.id);
            self.authority
                .send_message(&Message::CreatePolicy {
                    species: species.clone(),
                    action: new_action,
                })
                .await?;
            match self.authority.read_msg().await? {
                Message::PolicyResult { policy } => self.policies.insert(
                    species.clone(),
                    Policy {
                        id: policy,
                        action: new_action,
                        species: species.clone(),
                    },
                ),
                _ => bail!("Expected policy result"),
            };
            println!(
                ">> New Policy Created, {} self.id={} policy_now={:?}",
                &species,
                self.id,
                self.policies.get(&species)
            );
        }

        dbg!("End", &species, self.policies.get(&species));
        Ok(())
    }
}

#[derive(Clone)]
struct SiteManager {
    sites: Arc<Mutex<HashMap<SiteId, Arc<Mutex<Site>>>>>,
}

impl SiteManager {
    async fn register_population(
        &mut self,
        site_id: SiteId,
        curr_population: Vec<PopulationObservation>,
    ) -> anyhow::Result<()> {
        let site = {
            let mut map_lock = self.sites.lock().await;
            if !map_lock.contains_key(&site_id) {
                map_lock.insert(site_id, Arc::new(Mutex::new(Site::new(site_id).await?)));
            }
            map_lock.get(&site_id).unwrap().clone()
        };

        println!("waiting for lock");
        let mut lock = site.lock().await;

        println!("got  lock");
        lock.register_population(curr_population).await?;
        println!("Processed new poulation");
        Ok(())
    }
}

async fn process_messages(conn: &mut Connection, mut manager: SiteManager) -> anyhow::Result<()> {
    let msg = conn.read_msg().await.ok();
    conn.send_message(&Message::create_hello()).await?;
    if msg != Some(Message::create_hello()) {
        bail!("Expected a better greeting from you.");
    }
    loop {
        let msg = conn.read_msg().await?;
        match msg {
            Message::SiteVisit { site, populations } => {
                println!("registering population");
                manager.register_population(site, populations).await?;
            }
            _ => bail!("Only site-visit was expected"),
        }
    }
}

async fn handle_client(socket: tokio::net::TcpStream, manager: SiteManager) -> anyhow::Result<()> {
    let mut conn = socket.into();
    if let Err(x) = process_messages(&mut conn, manager).await {
        println!("Closing connection with {:?}", x);
        conn.send_message(&Message::Error {
            message: format!("{:?}", x),
        })
        .await?;
        return Err(x);
    }
    Ok(())
}

async fn establish_authority_connection() -> anyhow::Result<Connection> {
    let tcp_conn = tokio::net::TcpStream::connect("pestcontrol.protohackers.com:20547").await?;
    let mut rv: Connection = tcp_conn.into();
    rv.send_message(&Message::create_hello()).await?;
    let msg = rv.read_msg().await?;
    if msg != Message::create_hello() {
        rv.send_message(&Message::Error {
            message: "Expected you to send a simple hello atleast. :(".to_string(),
        })
        .await?;
        bail!("Expected valid hello msg, got: {:?}", msg);
    }
    Ok(rv)
}

#[async_trait]
trait MessageSender {
    async fn send_message(&mut self, msg: &Message) -> anyhow::Result<()>;
}

#[async_trait]
impl MessageSender for OwnedWriteHalf {
    async fn send_message(&mut self, msg: &Message) -> anyhow::Result<()> {
        Ok(self.write_all(&msg.to_bytes()).await?)
    }
}

#[async_trait]
trait MessageReciever {
    async fn read_msg(&mut self) -> anyhow::Result<Message>;
}

#[async_trait]
impl MessageReciever for BufReader<OwnedReadHalf> {
    async fn read_msg(&mut self) -> anyhow::Result<Message> {
        let msg_type = self.read_u8().await?;
        let msg_len = self.read_u32().await?;
        if msg_len > TOO_MUCH_LENGTH {
            bail!("Message too long.")
        }
        if msg_len < 5 {
            bail!("Message too small.")
        }
        let mut buffer = vec![0u8; msg_len as usize]; // msg_len - u32 - u8
        buffer[0] = msg_type;
        buffer[1] = msg_len.to_be_bytes()[0];
        buffer[2] = msg_len.to_be_bytes()[1];
        buffer[3] = msg_len.to_be_bytes()[2];
        buffer[4] = msg_len.to_be_bytes()[3];

        self.read_exact(&mut buffer[5..]).await?;
        let rv = Message::from_bytes(&buffer)?;
        // println!("Recv msg: {:?}", rv);
        Ok(rv)
    }
}

struct Connection {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl From<tokio::net::TcpStream> for Connection {
    fn from(value: tokio::net::TcpStream) -> Self {
        let (rh, wh) = value.into_split();
        let reader = tokio::io::BufReader::new(rh);
        Self { reader, writer: wh }
    }
}
#[async_trait]
impl MessageReciever for Connection {
    async fn read_msg(&mut self) -> anyhow::Result<Message> {
        self.reader.read_msg().await
    }
}

#[async_trait]
impl MessageSender for Connection {
    async fn send_message(&mut self, msg: &Message) -> anyhow::Result<()> {
        self.writer.send_message(msg).await
    }
}

trait ToPopulationMap {
    fn to_map(&self) -> PopulationMap;
}

impl ToPopulationMap for Vec<PopulationTarget> {
    fn to_map(&self) -> PopulationMap {
        let mut result: PopulationMap = HashMap::new();
        for p in self {
            let species = p.species.clone();
            result.insert(species, p.clone());
        }
        result
    }
}
trait ToPopulationVec {
    fn to_vec(&self) -> Vec<PopulationTarget>;
}

impl ToPopulationVec for PopulationMap {
    fn to_vec(&self) -> Vec<PopulationTarget> {
        return self.values().cloned().collect();
    }
}

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    let tcp_conn = tokio::net::TcpListener::bind("0.0.0.0:3007").await?;
    let site_manager = SiteManager {
        sites: Default::default(),
    };

    loop {
        let sm = site_manager.clone();
        let (socket, _) = tcp_conn.accept().await?;
        tokio::spawn(async move {
            println!("New Client");
            dbg!(handle_client(socket, sm).await).ok();
            println!("Client closed");
        });
    }
}

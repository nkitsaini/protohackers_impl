use std::ops::Index;

use crate::prelude::*;
use tokio::net::UdpSocket;

// #[tokio::main]
// async fn main() -> io::Result<()> {
//     let sock = UdpSocket::bind("0.0.0.0:8080").await?;
//     let mut buf = [0; 1024];
//     loop {
//         let (len, addr) = sock.recv_from(&mut buf).await?;
//         println!("{:?} bytes received from {:?}", len, addr);

//         let len = sock.send_to(&buf[..len], addr).await?;
//         println!("{:?} bytes sent", len);
//     }
// }

// TODO: search why utf-8 is backwards compatible when ascii uses complete u8 space
// but utf-8 can contain more information
type Key = String;
type Value = String;
const VERSION_KEY: &'static str = "version";
const VERSION_VAL: &'static str = "nkit's server 1.0";

const PACKET_LEN: usize = 999;
type Packet = [u8; PACKET_LEN];

#[derive(Debug)]
enum Request {
	Insert(Key, Value),
	Query(Key)
}

impl Request {
	pub fn parse(value: Vec<u8>) -> Option<Self> {
		let str = String::from_utf8(value).ok()?;
		let i = str.find('=');
		Some(match str.find('=') {
			Some(x) => {
				Self::Insert(str[..x].to_string(), str[x+1..].to_string())
			},
			None => {
				Self::Query(str)
			}
		})
	}
}

async fn run() -> anyhow::Result<()> {
    let sock = UdpSocket::bind("0.0.0.0:3007").await?;
    let mut buf = [0; 999];
	let mut kv_store = HashMap::new();

    loop {
        let (len, addr) = sock.recv_from(&mut buf).await?;
		let req = match Request::parse(buf[..len].to_vec()) {
			Some(x) => x,
			None => continue
		};
		
		match dbg!(req) {
			Request::Insert(k, v) => {
				if k != VERSION_KEY {
					kv_store.insert(k, v);
				}
			},
			Request::Query(k) => {
				if let Some(v) = kv_store.get(&k) {
					let message = format!("{k}={v}");
					debug_assert!(message.len() <= PACKET_LEN);
					sock.send_to(message.as_bytes(), addr).await?;
				} else if k == VERSION_KEY {
					let message = format!("{k}={VERSION_VAL}");
					debug_assert!(message.len() <= PACKET_LEN);
					sock.send_to(message.as_bytes(), addr).await?;
				}
			}
		}
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

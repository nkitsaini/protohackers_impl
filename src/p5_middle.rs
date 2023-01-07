//! Failing write now
use std::net::{SocketAddr, ToSocketAddrs};

use once_cell::sync::Lazy;

use crate::prelude::*;

const ADDRESS_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(^| )7[a-zA-Z0-9]{25,34}($| |\n)"#).unwrap());

const TONY_ADDRESS: &'static str = "7YWHMfk9JZe0LM0g1ZauHuiSxhI";
const TONY_ADDRESS_REP: &'static str = "${1}7YWHMfk9JZe0LM0g1ZauHuiSxhI${2}";
const CHAT_SERVER_ADDR: &'static str = "chat.protohackers.com:16963";

const CHAT_SERVER_SOCK_ADDR: Lazy<SocketAddr> = Lazy::new(|| {
    CHAT_SERVER_ADDR
        .to_socket_addrs()
        .unwrap()
        .filter(|x| x.is_ipv4())
        .next()
        .unwrap()
});

fn replace_address(val: &str) -> String {
    let val = val.replace(' ', "  ");
    let val = ADDRESS_REGEX
        .replace_all(&val, TONY_ADDRESS_REP)
        .to_string();
    val.replace("  ", " ")
}

struct LinedBufReader {
    reader: tokio::io::BufReader<tokio::net::tcp::OwnedReadHalf>,
    prev_part: String,
}

impl LinedBufReader {
    pub fn new(val: tokio::io::BufReader<tokio::net::tcp::OwnedReadHalf>) -> Self {
        Self {
            reader: val,
            prev_part: Default::default(),
        }
    }
    pub async fn get_next_line(&mut self) -> Option<String> {
        loop {
            if let Some(i) = self.prev_part.find('\n') {
                let extra_part = self.prev_part[i + 1..].to_string();
                let curr_part = self.prev_part[..i + 1].to_string();
                self.prev_part = extra_part;
                return Some(curr_part);
            }
            let mut buf = [0; 1024];
            let r = self.reader.read(&mut buf).await.ok()?;
            self.prev_part
                .push_str(&String::from_utf8(buf[..r].to_vec()).ok()?);
        }
    }
}

async fn handle_user(stream: tokio::net::TcpStream) -> anyhow::Result<()> {
    let (cread_half, mut cwrite_half) = stream.into_split();
    let s_stream = tokio::net::TcpSocket::new_v4()?
        .connect(dbg!(*CHAT_SERVER_SOCK_ADDR))
        .await?;
    let (sread_half, mut swrite_half) = s_stream.into_split();

    let c_reader = tokio::io::BufReader::new(cread_half);
    let s_reader = tokio::io::BufReader::new(sread_half);

    // let server_buf =
    // let mut sr_lines = s_reader.lines();
    // let mut cr_lines = c_reader.lines();
    let mut srl = LinedBufReader::new(s_reader);
    let mut crl = LinedBufReader::new(c_reader);
    let mut name_established = false;
    loop {
        tokio::select!(
            server_res = srl.get_next_line() => {
                // let mut server_res = match server_res {
                // 	Ok(x) => x,
                // 	Err(_) => break
                // };
                let mut server_res = match server_res  {
                    Some(x) => x,
                    None => break
                };
                server_res = dbg!(replace_address(&server_res));
                // server_res.push('\n');

                if cwrite_half.write(server_res.as_bytes()).await.is_err() {
                    break
                };
            },
            client_req = crl.get_next_line() => {
                // let client_req = match client_req {
                // 	Ok(x) => x,
                // 	Err(_) => break
                // };
                let mut client_req = match client_req  {
                    Some(x) => x,
                    None => break
                };
                if name_established {
                    dbg!("Updating", &client_req);
                    client_req = dbg!(replace_address(&client_req));
                } else {
                    dbg!("Ignoring", &client_req);
                    name_established = true;
                }
                // client_req.push('\n');
                if swrite_half.write(client_req.as_bytes()).await.is_err() {
                    break
                };
            }
        );
    }
    swrite_half.shutdown().await.ok();
    cwrite_half.shutdown().await.ok();

    Ok(())
}
async fn run() -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3007").await?;

    loop {
        let (stream, _) = listener.accept().await?;
        tokio::task::spawn(async move { dbg!(handle_user(stream).await) });
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

// [Fri Jan  6 16:47:30 2023 UTC] [0simple.test] NOTE:check starts
// [Fri Jan  6 16:47:31 2023 UTC] [0simple.test] NOTE:checking whether basic chat works
// [Fri Jan  6 16:47:32 2023 UTC] [0simple.test] NOTE:BrownEdward498 connected to 207.148.123.137 port 3007
// [Fri Jan  6 16:47:33 2023 UTC] [0simple.test] NOTE:CrazyMike648 connected to 207.148.123.137 port 3007
// [Fri Jan  6 16:47:33 2023 UTC] [0simple.test] NOTE:BrownEdward498 joined the chat room
// [Fri Jan  6 16:47:34 2023 UTC] [0simple.test] NOTE:CrazyMike648 joined the chat room
// [Fri Jan  6 16:47:43 2023 UTC] [0simple.test] PASS
// [Fri Jan  6 16:47:44 2023 UTC] [1payment.test] NOTE:check starts
// [Fri Jan  6 16:47:45 2023 UTC] [1payment.test] NOTE:checking whether address rewriting works
// [Fri Jan  6 16:47:46 2023 UTC] [1payment.test] NOTE:RedCharlie8 connected to 207.148.123.137 port 3007
// [Fri Jan  6 16:47:47 2023 UTC] [1payment.test] NOTE:ProtoDave518 connected to 207.148.123.137 port 3007
// [Fri Jan  6 16:47:47 2023 UTC] [1payment.test] NOTE:RedCharlie8 joined the chat room
// [Fri Jan  6 16:47:48 2023 UTC] [1payment.test] NOTE:ProtoDave518 joined the chat room
// [Fri Jan  6 16:47:58 2023 UTC] [1payment.test] PASS
// [Fri Jan  6 16:47:59 2023 UTC] [2conference.test] NOTE:check starts
// [Fri Jan  6 16:47:59 2023 UTC] [2conference.test] NOTE:checking address rewriting in both directions
// [Fri Jan  6 16:48:00 2023 UTC] [2conference.test] NOTE:SmallAlice303 connected to 207.148.123.137 port 3007
// [Fri Jan  6 16:48:01 2023 UTC] [2conference.test] NOTE:SmallFrank398 connected to 207.148.123.137 port 3007
// [Fri Jan  6 16:48:02 2023 UTC] [2conference.test] NOTE:SlimyCoder632 connected to 207.148.123.137 port 3007
// [Fri Jan  6 16:48:03 2023 UTC] [2conference.test] NOTE:MadNewbie721 connected to 207.148.123.137 port 3007
// [Fri Jan  6 16:48:03 2023 UTC] [2conference.test] NOTE:MadDave624 connected to 207.148.123.137 port 3007
// [Fri Jan  6 16:48:04 2023 UTC] [2conference.test] NOTE:PoorMike502 connected to 207.148.123.137 port 3007
// [Fri Jan  6 16:48:05 2023 UTC] [2conference.test] NOTE:LargeCoder497 connected to 207.148.123.137 port 3007
// [Fri Jan  6 16:48:06 2023 UTC] [2conference.test] NOTE:PoorCharlie357 connected to 207.148.123.137 port 3007
// [Fri Jan  6 16:48:06 2023 UTC] [2conference.test] NOTE:SmallAlice303 joined the chat room
// [Fri Jan  6 16:48:06 2023 UTC] [2conference.test] NOTE:SmallFrank398 joined the chat room
// [Fri Jan  6 16:48:07 2023 UTC] [2conference.test] NOTE:SlimyCoder632 joined the chat room
// [Fri Jan  6 16:48:07 2023 UTC] [2conference.test] NOTE:SmallFrank764 joined the chat room
// [Fri Jan  6 16:48:07 2023 UTC] [2conference.test] NOTE:MadNewbie721 joined the chat room
// [Fri Jan  6 16:48:08 2023 UTC] [2conference.test] NOTE:MadDave624 joined the chat room
// [Fri Jan  6 16:48:08 2023 UTC] [2conference.test] NOTE:PoorMike502 joined the chat room
// [Fri Jan  6 16:48:09 2023 UTC] [2conference.test] NOTE:LargeCoder497 joined the chat room
// [Fri Jan  6 16:48:09 2023 UTC] [2conference.test] NOTE:PoorCharlie357 joined the chat room
// [Fri Jan  6 16:48:17 2023 UTC] [2conference.test] NOTE:Tony8799418 joined the chat room
// [Fri Jan  6 16:48:18 2023 UTC] [2conference.test] NOTE:Tony8799418 (fully joined) disconnected
// [Fri Jan  6 16:48:18 2023 UTC] [2conference.test] NOTE:SmallAlice303 (fully joined) disconnected
// [Fri Jan  6 16:48:28 2023 UTC] [2conference.test] FAIL:server did not send 'PoorCharlie357' the quit message for 'SmallAlice303' within 10 seconds

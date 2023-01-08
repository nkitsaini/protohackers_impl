use std::num::Wrapping;

use anyhow::{bail, Context};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

#[derive(Debug)]
enum CipherOp {
    ReverseBit,
    Xor(u8),
    XorPos,
    Add(u8),
    AddPos,
}

impl CipherOp {
    fn encode(&self, pos_end: u8, bt: u8) -> u8 {
        let wbt = Wrapping(bt);
        let wpe = Wrapping(pos_end);
        match self {
            Self::ReverseBit => bt.reverse_bits(),
            Self::Xor(v) => bt ^ v,
            Self::XorPos => bt ^ pos_end,
            Self::Add(v) => (wbt + Wrapping(*v)).0,
            Self::AddPos => (wbt + wpe).0,
        }
    }

    fn decode(&self, pos_end: u8, bt: u8) -> u8 {
        let wbt = Wrapping(bt);
        let wpe = Wrapping(pos_end);
        match self {
            Self::ReverseBit => bt.reverse_bits(),
            Self::Xor(v) => bt ^ v,
            Self::XorPos => bt ^ pos_end,
            Self::Add(v) => (wbt - Wrapping(*v)).0,
            Self::AddPos => (wbt - wpe).0,
        }
    }

    fn parse_all(buf: Vec<u8>) -> anyhow::Result<Vec<Self>> {
        let mut i = 0;
        let mut rv = vec![];
        while i < buf.len() {
            rv.push(match buf[i] {
                0x01 => Self::ReverseBit,
                0x02 => {
                    i += 1;
                    Self::Xor(*buf.get(i).context("invalid")?)
                }
                0x03 => Self::XorPos,
                0x04 => {
                    i += 1;

                    Self::Add(*buf.get(i).context("invalid")?)
                }
                0x05 => Self::AddPos,
                _ => bail!("Unknown cipher type"),
            });
            i += 1;
        }
        Ok(rv)
    }
}

#[derive(Debug)]
struct Cipher {
    operations: Vec<CipherOp>,
}
impl Cipher {
    fn is_dumb(&self) -> bool {
        for pos in 0x00..0xff {
            for bt in 0x00..0xff {
                if self.encode(pos, bt) != bt {
                    return false;
                }
            }
        }
        true
    }
    fn new(cipher: Vec<u8>) -> anyhow::Result<Self> {
        Ok(Self {
            operations: CipherOp::parse_all(cipher)?,
        })
    }
    fn decode(&self, pos: usize, val: u8) -> u8 {
        let mut rv = val;
        let pos_end = (pos & 0xff) as u8;
        for cipher in self.operations.iter().rev() {
            rv = cipher.decode(pos_end, rv);
        }
        rv
    }
    fn encode(&self, pos: usize, val: u8) -> u8 {
        let mut rv = val;
        let pos_end = (pos & 0xff) as u8;
        for cipher in self.operations.iter() {
            rv = cipher.encode(pos_end, rv);
        }
        rv
    }
}

#[derive(Debug)]
struct ToyInfo {
    text: String,
    count: u64,
}

impl ToyInfo {
    pub fn new(content: &str) -> anyhow::Result<Self> {
        Ok(Self {
            text: content.to_string(),
            count: regex::Regex::new(r#"(^\d+)x"#)
                .unwrap()
                .captures(content)
                .context("no number at start")?
                .get(1)
                .unwrap()
                .as_str()
                .parse()?,
        })
    }
}

async fn handle_client(sock: tokio::net::TcpStream) -> anyhow::Result<()> {
    let (rh, mut wh) = sock.into_split();
    let mut reader = tokio::io::BufReader::new(rh);

    let mut buf = vec![];
    reader.read_until(0x00, &mut buf).await?;

    let mut res_pos = 0;
    let mut prev_buf = String::new();
    let mut req_pos = 0;

    buf.pop();
    let cipher = Cipher::new(buf)?;
    if cipher.is_dumb() {
        bail!("Using a dumb cipher");
    }

    loop {
        while let Some(i) = prev_buf.find('\n') {
            let pb = prev_buf.clone();
            let (mut line, extra) = pb.split_at(i + 1);
            prev_buf = extra.to_string();
            line = &line[..i];
            if line.len() == 0 {
                bail!("Empty line");
            }
            let toys = line
                .split(',')
                .into_iter()
                .map(|x| ToyInfo::new(x))
                .collect::<anyhow::Result<Vec<_>>>()?;
            let best_toy = toys.iter().max_by(|a, b| a.count.cmp(&b.count)).unwrap();

            let mut resp_bytes = best_toy.text.as_bytes().to_vec();
            resp_bytes.push('\n' as u8);
            let mut encoded_resp_bytes = vec![];
            for (i, bt) in resp_bytes.into_iter().enumerate() {
                let pos = i + res_pos;
                encoded_resp_bytes.push(cipher.encode(pos, bt));
            }
            res_pos += &encoded_resp_bytes.len();
            wh.write_all(&encoded_resp_bytes).await?;
        }
        let mut line = vec![0u8; 1024];
        let read_len = reader.read(&mut line).await?;
        line = line[..read_len].to_vec();
        if line.len() == 0 {
            bail!("EOF from client");
        }
        let mut decoded_line = vec![];
        for (i, bt) in line.into_iter().enumerate() {
            let pos = i + req_pos;
            decoded_line.push(cipher.decode(pos, bt));
        }
        req_pos += decoded_line.len();
        prev_buf += &String::from_utf8(decoded_line)?;
    }
    Ok(())
}

async fn run() -> anyhow::Result<()> {
    let tcp_conn = tokio::net::TcpListener::bind("0.0.0.0:3007").await?;
    let mut client_id = 0;

    loop {
        let (socket, _) = tcp_conn.accept().await?;
        tokio::spawn(async move { dbg!(handle_client(socket).await) });
        client_id += 1;
    }
    Ok(())
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
    async fn p8_test_client() -> anyhow::Result<()> {
        tokio::task::spawn(run()); // server chala
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let (read_half1, mut write_half1) = tokio::net::TcpSocket::new_v4()
            .unwrap()
            .connect("0.0.0.0:3007".parse().unwrap())
            .await?
            .into_split();

        let mut reader = tokio::io::BufReader::new(read_half1);

        let msg1 = vec![0x02, 0x7b, 0x05, 0x01, 0x00];
        write_half1.write_all(&msg1).await?;

        let msg1 = vec![
            0xf2, 0x20, 0xba, 0x44, 0x18, 0x84, 0xba, 0xaa, 0xd0, 0x26, 0x44, 0xa4, 0xa8, 0x7e,
        ];
        write_half1.write_all(&msg1).await?;

        let mut rlines = reader.lines();
        dbg!("Going to wait");
        tokio::select! {
            l = rlines.next_line() => {
                dbg!("Got resp");
                dbg!(l.unwrap().unwrap());
                assert!(false);
            },
            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
            dbg!("Timed out");
                anyhow::bail!("Timed out read");
            }
        }
        Ok(())
    }
}

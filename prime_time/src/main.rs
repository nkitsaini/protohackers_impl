use serde::{Serialize, Deserialize};
use std::io::prelude::*;
use std::ops::Add;
use std::thread;
use std::net::{TcpListener, TcpStream};

const METHOD_NAME: &'static str = "isPrime";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Input {
    method: String,
    number: f64
}
impl Input {
    pub fn parse_bytes(data: Vec<u8>) -> Option<Self> {
        match String::from_utf8(data) {
            Ok(v) => {
                Self::parse(v)
            },
            Err(x) => {
                None
            }
        }
    }
    pub fn parse(content: String) -> Option<Self> {
        let data: Self = serde_json::from_str(&content).ok()?;
        if &data.method != METHOD_NAME {
            None
        } else {
            Some(data)
        }

    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Output {
    method: String,
    prime: Option<bool>
}
impl Output {
    pub fn new(is_prime: Option<bool>) -> Self {
        Self {
            method: METHOD_NAME.to_string(),
            prime: is_prime
        }
    }
    pub fn malformed() -> Self {
        Self::new(None)
    }
    pub fn valid(is_prime: bool) -> Self {
        Self::new(Some(is_prime))
    }
}

fn is_prime(x: f64) -> bool {
    if x < 0. {
        return false
    }
    let y = x.ceil() as u64;
    if y as f64 != x {
        return false
    }

    primes::is_prime(y)
}

fn handle_client(stream: &mut TcpStream) -> anyhow::Result<()> {
    loop {
        let mut data = vec![];
        let mut buf = [0; 1];
        stream.read_exact(&mut buf)?;
        data.append(&mut buf.clone().to_vec());
        while !dbg!(String::from_utf8(data.clone()))?.contains("\n") {
            let mut buf = [0; 1];
            stream.read_exact(&mut buf)?;
            data.append(&mut buf.clone().to_vec());
        }
        
        let val = String::from_utf8(data.clone())?;
        dbg!("INput str", val.clone());
        let output = match Input::parse_bytes(data.clone()) {
            Some(v) => {
                dbg!("Valid Input", v.clone(), val.clone());
                Output::valid(is_prime(v.number))    
            },
            None => {
                dbg!("InValid Input", val.clone());
                break
            }
        };
        dbg!(output.clone());
        let out = serde_json::to_string(&output).unwrap();
        let out = out.add("\n");
        dbg!(out.clone());

        stream.write(&out.as_bytes())?;
        stream.flush()?;
    }
    stream.shutdown(std::net::Shutdown::Both)?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let listener = TcpListener::bind("0.0.0.0:3007")?;

    for stream in listener.incoming() {
        thread::spawn(|| -> anyhow::Result<()> {
            dbg!(handle_client(&mut stream?))
        });
    }

    Ok(())

}


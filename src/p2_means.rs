
use std::time::Duration;

use anyhow::Context;

use crate::prelude::*;

type IType = i64;

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug)]
struct I32 {
    val: IType
}
impl I32 {
    fn new(val: IType) -> Self {
        Self {
            val
        }
    }

    fn to_bytes(&self) -> [u8; 4] {
        let signed_bit: bool = self.val < 0;
        let value = if signed_bit { self.val + (1 << (32 - 1))} else {self.val};
        let signed_del = if signed_bit { 1 << 7} else {0} as u8;
        let all_ones8 = ((1 << 8) -1) as u8; // 11111111
        return [
            (all_ones8 as IType & (value >> 24)) as u8 + signed_del,
            (all_ones8 as IType & (value >> 16)) as u8,
            (all_ones8 as IType & (value >> 8)) as u8,
            (all_ones8 as IType & value) as u8
        ]
    }

    fn parse(val: &[u8; 4]) -> Self {
        let signed = val[0] & (1 << 7) > 0; // 10000000 or 00000000
        let mut value: IType = 0;
        value += val[3] as IType;
        value += ((val[2] as u32) << 8) as IType;
        value += ((val[1] as u32) << 16) as IType;
        value += (((val[0] as u32) & ((1<<7) -1)) << 24) as IType;
        if signed {
            value += -(1 << (32 -1))
        }
        return Self {
            val: value
        }
        // todo!()
    }
}

#[derive(Debug)]
struct InsertMessage {
    timestamp: I32,
    price: I32
}
#[derive(Debug)]
struct QueryMessage {
    mintime: I32,
    maxtime: I32
}

#[derive(Debug)]
enum Message {
    Insert(InsertMessage),
    Query(QueryMessage)
}


impl Message {
    fn try_parse(value: &[u8; 9]) -> Option<Self> {
        let action = value[0] as char;
        let val1 = I32::parse(value[1..5].try_into().unwrap());
        let val2 = I32::parse(value[5..9].try_into().unwrap());
        return match action {
            'I' => {
                Some(Self::Insert(InsertMessage { timestamp: val1, price: val2 }))
            },
            'Q' => {
                Some(Self::Query(QueryMessage { mintime: val1, maxtime: val2 }))
            },
            _ => None
        }
    }
}

// impl 
fn handle_client(mut stream: TcpStream) -> anyhow::Result<()> {
    let mut pricings = BTreeMap::new();

    loop {
        let mut data = [0; 9];
        stream.read_exact(&mut data)?;
        let input = Message::try_parse(&data).context("Input not valid")?;
        let response = match input {
            Message::Insert(im) => {
                pricings.insert(im.timestamp, im.price);
                None
            },
            Message::Query(qm) => {
                if qm.mintime > qm.maxtime {
                    Some(I32::new(0))
                } else {
                    let r= pricings.range((Included(qm.mintime), Included(qm.maxtime)));
                    let mut count = 0;
                    let mut sum = 0;
                    for (_, val) in r {
                        count += 1;
                        sum += val.val;
                    }
                    if count == 0 {
                        Some(I32::new(0))
                    } else {
                        Some(I32::new( (sum as f64 / count as f64).round() as IType))
                    }
                }

            }
        };
        if let Some(val) = response {
            stream.write(&val.to_bytes())?;
        }
        stream.flush()?;
    }
}


pub fn main() -> anyhow::Result<()> {
    let listener = TcpListener::bind("0.0.0.0:3007")?;

    for stream in listener.incoming() {
        thread::spawn(|| -> anyhow::Result<()> {
            dbg!(handle_client(stream?))
        });
    }

    Ok(())
}
#[test]
fn test_i32_parse() {
    assert_eq!(I32::parse(&[0, 0, 0x3, 0xe8]).val, 1000);
    for val in vec![1, -1, -100, 100, 0, -237923, 2398723, 1000] {
        dbg!(val);
        assert_eq!(I32::parse(dbg!(&I32::new(val).to_bytes())).val, val);
    }
}

// 
//     Hexadecimal:                 Decoded:
// 49 00 00 30 39 00 00 00 65   I 12345 101
// 49 00 00 30 3a 00 00 00 66   I 12346 102
// 49 00 00 30 3b 00 00 00 64   I 12347 100
// 49 00 00 a0 00 00 00 00 05   I 40960 5
// 51 00 00 30 00 00 00 40 00   Q 12288 16384
// 00 00 00 65                  101
// 

#[test]
fn test_flow() {
    let t = thread::spawn(|| {
        dbg!(main()).unwrap();
    });

    thread::sleep(Duration::from_millis(200));
    let mut st = TcpStream::connect("0.0.0.0:3007").unwrap();

    let input: [u8; 45] = [
        0x49, 0x00, 0x00, 0x30, 0x39, 0x00, 0x00, 0x00, 0x65,
        0x49, 0x00, 0x00, 0x30, 0x3a, 0x00, 0x00, 0x00, 0x66,
        0x49, 0x00, 0x00, 0x30, 0x3b, 0x00, 0x00, 0x00, 0x64,
        0x49, 0x00, 0x00, 0xa0, 0x00, 0x00, 0x00, 0x00, 0x05,
        0x51, 0x00, 0x00, 0x30, 0x00, 0x00, 0x00, 0x40, 0x00,
    ];

    st.write(&input).unwrap();
    let mut read_buf = [0; 4];
    st.read_exact(&mut read_buf).unwrap();
    let k = u32::from_be_bytes(read_buf);
    assert_eq!(k, 101);

}
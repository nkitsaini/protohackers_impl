use std::{collections::HashMap, io::Bytes};

use anyhow::{bail, Context};

pub enum DataType {
    U32(u32),
    String(String),
    Array(Vec<HashMap<String, DataType>>),
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct PopulationTarget {
    pub species: Species,
    pub min: u32,
    pub max: u32,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct PopulationObservation {
    pub species: Species,
    pub count: u32,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Action {
    /// 0x90
    Cull,

    /// 0xa0
    Conserve,
}

#[derive(Debug, PartialEq, Eq)]
pub enum MessageKind {
    Hello = 0x50,
    Error = 0x51,
    Ok = 0x52,
    DialAuthority = 0x53,
    TargetPopulations = 0x54,
    CreatePolicy = 0x55,
    DeletePolicy = 0x56,
    PolicyResult = 0x57,
    SiteVisit = 0x58,
}

impl MessageKind {
    pub fn from_code(code: u8) -> Option<Self> {
        use MessageKind::*;
        Some(match code {
            0x50 => Hello,
            0x51 => Error,
            0x52 => Ok,
            0x53 => DialAuthority,
            0x54 => TargetPopulations,
            0x55 => CreatePolicy,
            0x56 => DeletePolicy,
            0x57 => PolicyResult,
            0x58 => SiteVisit,
            _ => return None,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Message {
    /// 0x50
    Hello { protocol: String, version: u32 },

    /// 0x51
    Error { message: String },

    /// 0x52
    Ok,

    /// 0x53
    DialAuthority { site: SiteId },

    /// 0x54
    TargetPopulations {
        site: SiteId,
        populations: Vec<PopulationTarget>,
    },

    /// 0x55
    CreatePolicy { species: Species, action: Action },

    /// 0x56
    DeletePolicy { policy: PolicyId },

    /// 0x57
    PolicyResult { policy: PolicyId },

    /// 0x58
    SiteVisit {
        site: SiteId,
        populations: Vec<PopulationObservation>,
    },
}

impl Message {
    pub fn to_bytes(&self) -> Vec<u8> {
        let content = self.content_bytes();

        // msg_type + msg_len + content + checksum
        let total_length = 1 + 4 + content.len() + 1;

        let mut rv = vec![];
        rv.push(self.kind() as u8);
        rv.extend(encode_u32(total_length as u32));
        rv.extend(content);

        let mut sum: u8 = 0;
        for byte in rv.iter() {
            sum = sum.wrapping_add(*byte);
        }

        // 255 - 254 + 1
        // 255 - 0 + 1
        let checksum = 0u8.wrapping_sub(sum);
        rv.push(checksum);
        rv
    }

    pub fn from_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
        let mut parser = Parser::new(bytes);
        let msg_kind =
            MessageKind::from_code(parser.parse_u8()?).context("Unknown message type")?;

        let msg_length = parser.parse_u32()?;
        if msg_length as usize != bytes.len() {
            bail!("Message length does not match content size");
        }

        let total = bytes
            .iter()
            .cloned()
            .reduce(|acc, e| acc.wrapping_add(e))
            .unwrap();

        if total != 0 {
            bail!("Invalid checksum");
        }

        let rv = Ok(match msg_kind {
            MessageKind::Hello => {
                let protocol = parser.parse_string()?;
                let version = parser.parse_u32()?;
                Self::Hello { protocol, version }
            }
            MessageKind::Ok => Self::Ok,
            MessageKind::TargetPopulations => Self::TargetPopulations {
                site: parser.parse_u32()?,
                populations: parser.parse_target_array()?,
            },
            MessageKind::PolicyResult => Self::PolicyResult {
                policy: parser.parse_u32()?,
            },
            MessageKind::SiteVisit => Self::SiteVisit {
                site: parser.parse_u32()?,
                populations: parser.parse_obs_array()?,
            },
            _ => bail!("Should never have recieved this type of message"),
        });

        parser.parse_u8()?; // checksum byte

        if parser.remaining() != 0 {
            bail!("Extra bytes in message");
        }

        rv
    }

    pub fn create_hello() -> Self {
        Self::Hello {
            protocol: "pestcontrol".to_string(),
            version: 1,
        }
    }

    fn content_bytes(&self) -> Vec<u8> {
        let mut result = vec![];
        match self {
            Self::Hello { protocol, version } => {
                result.extend(encode_string(&protocol));
                result.extend(encode_u32(*version));
            }
            Self::Error { message } => {
                result.extend(encode_string(message));
            }
            Self::DialAuthority { site } => {
                result.extend(encode_u32(*site));
            }
            Self::CreatePolicy { species, action } => {
                result.extend(encode_string(&species));
                result.extend(encode_action(*action));
            }
            Self::DeletePolicy { policy } => result.extend(encode_u32(*policy)),
            _ => unreachable!("We shouldn't be sending such msg: {:?}", &self),
        };
        result
    }

    pub fn kind(&self) -> MessageKind {
        match self {
            Self::Hello {
                protocol: _,
                version: _,
            } => MessageKind::Hello,
            Self::Error { message: _ } => MessageKind::Error,
            Self::Ok => MessageKind::Ok,
            Self::DialAuthority { site: _ } => MessageKind::DialAuthority,
            Self::TargetPopulations {
                site: _,
                populations: _,
            } => MessageKind::TargetPopulations,
            Self::CreatePolicy {
                species: _,
                action: _,
            } => MessageKind::CreatePolicy,
            Self::DeletePolicy { policy: _ } => MessageKind::DeletePolicy,
            Self::PolicyResult { policy: _ } => MessageKind::PolicyResult,
            Self::SiteVisit {
                site: _,
                populations: _,
            } => MessageKind::SiteVisit,
        }
    }
}

pub type SiteId = u32;

/// Always specific to a SiteId
/// A site does not re-use policy ID even after deletion
pub type PolicyId = u32;
pub type Species = String;

struct Parser<'a> {
    content: &'a [u8],
    offset: usize,
}

impl<'a> Parser<'a> {
    pub fn new(content: &'a [u8]) -> Self {
        Self { content, offset: 0 }
    }

    pub fn remaining(self) -> usize {
        return self.content.len() - self.offset;
    }

    pub fn parse_u8(&mut self) -> anyhow::Result<u8> {
        if self.content.len() < self.offset + 1 {
            bail!("Can't parse u8, less then 1 bytes");
        }
        self.offset += 1;
        Ok(self.content[self.offset - 1])
    }
    pub fn parse_u32(&mut self) -> anyhow::Result<u32> {
        if self.content.len() < self.offset + 4 {
            bail!("Can't parse u32, less then 4 bytes")
        }
        let offset = self.offset;
        let res = u32::from_be_bytes([
            self.content[offset],
            self.content[offset + 1],
            self.content[offset + 2],
            self.content[offset + 3],
        ]);
        self.offset += 4;
        Ok(res)
    }

    pub fn parse_string(&mut self) -> anyhow::Result<String> {
        let length = self.parse_u32()?;
        if self.content.len() < self.offset + length as usize {
            bail!("String length larger then content");
        }
        let res =
            String::from_utf8(self.content[self.offset..self.offset + length as usize].to_vec())?;
        self.offset += length as usize;
        Ok(res)
    }

    pub fn parse_obs_array(&mut self) -> anyhow::Result<Vec<PopulationObservation>> {
        let length = self.parse_u32()?;
        let mut rv = vec![];
        for i in 0..length {
            let species = self.parse_string()?;
            let count = self.parse_u32()?;
            rv.push(PopulationObservation { species, count });
        }
        Ok(rv)
    }

    pub fn parse_target_array(&mut self) -> anyhow::Result<Vec<PopulationTarget>> {
        let length = self.parse_u32()?;
        let mut rv = vec![];
        for i in 0..length {
            let species = self.parse_string()?;
            let min = self.parse_u32()?;
            let max = self.parse_u32()?;
            rv.push(PopulationTarget { species, min, max });
        }
        Ok(rv)
    }
}

// pub fn parse_u32(bytes: &[u8]) -> anyhow::Result<(u32, usize)> {
//     if bytes.len() < 4 {
//         bail!("Can't parse u32, less then 4 bytes")
//     }
//     let res = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
//     Ok((res, 4))
// }

// pub fn parse_string(bytes: &[u8]) -> anyhow::Result<(String, usize)> {
//     let (length, length_offset) = parse_u32(bytes)?;
//     if bytes.len() < length_offset + length as usize {
//         bail!("String length larger then content");
//     }
//     let res = String::from_utf8(bytes[length_offset..length_offset + length as usize].to_vec())?;
//     Ok((res, length_offset + length as usize))
// }

// pub fn parse_array(bytes: &[u8]) -> anyhow::Result<(Vec<Population>, usize)> {
//     let (length, length_offset) = parse_u32(bytes)?;

//     let mut offset = length_offset;
//     let mut rv = vec![];
//     for i in 0..length {
//         let (species, species_offset) = parse_string(&bytes[offset..])?;
//         offset += species_offset;
//         let (min, min_offset) = parse_u32(&bytes[offset..])?;
//         offset += min_offset;
//         let (max, max_offset) = parse_u32(&bytes[offset..])?;
//         offset += max_offset;
//         rv.push(Population { species, min, max });
//     }
//     Ok((rv, offset))
// }

fn encode_string(val: &str) -> Vec<u8> {
    let mut rv = vec![];
    rv.extend(encode_u32(val.len() as u32));
    rv.extend(val.as_bytes());
    return rv;
}

fn encode_action(action: Action) -> Vec<u8> {
    match action {
        Action::Cull => vec![0x90],
        Action::Conserve => vec![0xa0],
    }
}

fn encode_u32(val: u32) -> Vec<u8> {
    return val.to_be_bytes().to_vec();
}

fn encode_array(populations: &Vec<PopulationTarget>) -> Vec<u8> {
    let mut rv = vec![];
    rv.extend(encode_u32(populations.len() as u32));
    for population in populations {
        rv.extend(encode_string(&population.species));
        rv.extend(encode_u32(population.min));
        rv.extend(encode_u32(population.max));
    }
    return rv;
}

#[test]
fn test_hello_to_bytes() {
    assert_eq!(
        Message::Hello {
            protocol: "pestcontrol".to_string(),
            version: 1
        }
        .to_bytes(),
        vec![
            0x50, /* length*/ 0x00, 0x00, 0x00, 0x19, /*protocol len*/ 0x00, 0x00, 0x00,
            0x0b, /*pestcontrol*/ 0x70, 0x65, 0x73, 0x74, 0x63, 0x6f, 0x6e, 0x74, 0x72, 0x6f,
            0x6c, /*version*/ 0x00, 0x00, 0x00, 0x01, /* checksum */ 0xce
        ]
    )
}
#[test]
fn test_hello_from_bytes() {
    let bytes = vec![
        0x50, /* length*/ 0x00, 0x00, 0x00, 0x19, /*protocol len*/ 0x00, 0x00, 0x00,
        0x0b, /*pestcontrol*/ 0x70, 0x65, 0x73, 0x74, 0x63, 0x6f, 0x6e, 0x74, 0x72, 0x6f,
        0x6c, /*version*/ 0x00, 0x00, 0x00, 0x01, /* checksum */ 0xce,
    ];
    assert_eq!(
        Message::from_bytes(&bytes).unwrap(),
        Message::Hello {
            protocol: "pestcontrol".to_string(),
            version: 1
        }
    );
}

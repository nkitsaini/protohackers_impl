use clap::Arg;
use tokio::net::tcp::OwnedReadHalf;

use crate::prelude::*;
use std::{path::{PathBuf, Path}, collections::{HashMap, VecDeque}};

type Version = String;
type Name = String;
type Content = Vec<u8>;
type FPath = VecDeque<String>;

#[derive(Debug)]
struct VersionedFile {
	pub(self) versions: Vec<Content>
}

impl VersionedFile {
	fn new(initial_content: Content) -> Self {
		let versions = vec![initial_content];
		Self {versions}
	}
	fn latest_version(&self) -> Version {
		format!("r{}", self.versions.len())
	}
	fn get_latest(&self) -> Content {
		self.versions.last().unwrap().clone()
	}
	fn get_version(&self, version: Version) -> Option<Content> {
		let version = version.strip_prefix('r')?;
		let version: usize = version.parse().ok()?;
		self.versions.get(version - 1).cloned()
	}
	fn add(&mut self, content: Content) -> Version {
		if &content != self.versions.last().unwrap() {
			self.versions.push(content);
		}
		self.latest_version()
	}
}


#[derive(Default)]
struct InMemoryStorage{
	files: HashMap<Name, VersionedFile>,
	folders: HashMap<Name, InMemoryStorage>
}

impl VCS for InMemoryStorage {
	fn list_all(&self, mut prefix_path: FPath) -> Vec<DirItem> {
		// Sorted by name
		if prefix_path.is_empty() {
			let mut rv = vec![];
			for (x, y) in self.files.iter() {
				rv.push(DirItem::File { name: x.into(), version: y.latest_version() })
			}
			for (x, y) in self.folders.iter() {
				rv.push(DirItem::Dir { name: x.into() })
			}
			rv
		} else {
			match self.folders.get(&prefix_path.pop_front().unwrap()) {
				None => {
					return Vec::new()
				},
				Some(x) => {
					x.list(prefix_path)
				}
			}
		}
	}
	fn get(&self, mut file_path: FPath, revision: Option<Version>) -> Result<Content, GetError> {
		debug_assert!(file_path.len() != 0);
		if file_path.len() == 1 {
			match self.files.get(file_path.front().unwrap().into()) {
				None => {
					return Err(GetError::NoSuchFile)
				},
				Some(x) => {
					Ok(match revision {
						None => x.get_latest(),
						Some(y) => x.get_version(y).ok_or(GetError::NoSuchRevision)?
					})
				}
			}
		} else {
			match self.folders.get(&file_path.pop_front().unwrap()) {
				None => {
					return Err(GetError::NoSuchFile)
				},
				Some(x) => {
					x.get(file_path, revision)
				}
			}
		}
	}
	fn put(&mut self, mut file_path: FPath, content: Content) -> Version {
		debug_assert!(file_path.len() != 0);
		if file_path.len() == 1 {
			match self.files.get_mut(file_path.front().unwrap()) {
				None => {
					let f = VersionedFile::new(content);
					let version = f.latest_version();
					self.files.insert(file_path.front().unwrap().into(), f);
					version
				},
				Some(x) => {
					x.add(content)
				}
			}
			// let x = self.files.entry(file_path.front().unwrap().into()).or_insert_with(|| VersionedFile::new(content));
			// x.latest_version()
		} else {
			let f = self.folders.entry(file_path.pop_front().unwrap()).or_default();
			f.put(file_path, content)
		}
	}
}

enum DirItem {
	File {name: Name, version: Version},
	Dir {name: Name}
}
impl DirItem {
	fn is_file(&self) -> bool {
		match self {
			Self::File { name, version } => true,
			_ => false
		}
	}
	fn is_dir(&self) -> bool {
		!self.is_file()
	}

	fn name(&self) -> &str {
		match self {
			Self::File { name, version } => name,
			Self::Dir { name } => name
		}
	}
}

#[derive(Debug)]
enum GetError {
	NoSuchRevision,
	NoSuchFile
}

impl std::fmt::Display for GetError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let msg = match self {
			Self::NoSuchFile => "no such file",
			Self::NoSuchRevision => "no such revision"
		};
		f.write_str(msg)
	}
}
impl std::error::Error for GetError {}

#[derive(Debug)]
struct IllegalFileName;

impl std::fmt::Display for IllegalFileName {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("illegal file name")
	}
}
impl std::error::Error for IllegalFileName {}

trait VCS {
	/// Return all files and dirs inside
	fn list_all(&self, prefix_path: FPath) -> Vec<DirItem>;

	fn get(&self, file_path: FPath, revision: Option<Version>) -> Result<Content, GetError>;

	/// Return only file part if both file and dir of same name
	fn list(&self, prefix_path: FPath) -> Vec<DirItem> {
		let mut rv: HashMap<Name, DirItem> = Default::default();
		for item in self.list_all(prefix_path) {
			match rv.get(item.name()) {
				Some(DirItem::File { name, version }) => {
					// Files have higher priority then DIR in list.
					// So if we already have a file we don't need to update it.
				},
				_ => {
					rv.insert(item.name().to_string(), item);
				}
			};
		}
		let mut rv = rv.into_values().collect::<Vec<_>>();

		rv.sort_by(|a, b| a.name().cmp(b.name()));
		rv

	}

	fn put(&mut self, file_path: FPath, content: Content) -> Version;

	fn execute(&mut self, command: Command) -> Result<Vec<u8>, String> {
		use Command::*;
		match command {
			Help => Ok("usage: HELP|GET|PUT|LIST\n".into()),
			Get { file_path, revision } => {
				self.get(file_path, revision).map(|mut content| {
					let rv_str = format!("{length}\n", length=content.len());
					let mut rv_vec = rv_str.into_bytes();
					rv_vec.append(&mut content);
					rv_vec
				}).map_err(|e| format!("{e}\n"))
			}
			Put { file_path, content} => {
				let version = self.put(file_path, content);
				Ok(format!("{version}\n").into_bytes())
			},
			List { prefix } => {
				let list = self.list(prefix);
				let mut rv = String::new();
				rv.push_str(&list.len().to_string());
				rv.push('\n');
				for item in list {
					match item {
						DirItem::Dir { name } => {
							rv.push_str(&format!("{name}/ DIR\n"))
						},
						DirItem::File { name, version } => {
							rv.push_str(&format!("{name} {version}\n"))
						}
					}
				}
				Ok(rv.into_bytes())
			}

		}
	}
}

#[derive(Debug, Clone, Copy)]
enum CommandType {
	Help,
	Get,
	Put,
	List
}

impl CommandType {
	fn get_help_msg(&self) -> &'static str {
		match self {
			Self::Help => "HELP|GET|PUT|LIST",
			Self::Put => "PUT file length newline data",
			Self::Get => "GET file [revision]",
			Self::List => "LIST dir"
		}
	}

	fn from_name(name: &str) -> Option<Self> {
		match name.to_lowercase().as_str() {
			"help" => Some(CommandType::Help),
			"list" => Some(CommandType::List),
			"put" => Some(CommandType::Put),
			"get" => Some(CommandType::Get),
			_ => None
		}

	}
}

enum Command {
	Help,
	Get{file_path: FPath, revision: Option<Version>},
	Put {file_path: FPath, content: Vec<u8>},
	List {prefix: FPath}
}

enum CommandParseResult {
	UnknownCommand,

	PartialParse(CommandType),

	// specify custom parse error msg
	// For example: File name invalid
	// If none, the default msg for CommandType will be used
	ArgError(String),
	FullParse(Command)
}

impl Command {
	fn parse_path(path: &str, is_file: bool) -> Option<FPath> {
		if path.len() == 0 {
			return None
		}
		if !is_file && path == "/" {
			return Some(VecDeque::new());
		}
		let a = path.strip_prefix('/')?;
		let a = match path.strip_suffix('/') {
			Some(_) if is_file =>  {
				// Files can't contain `/`
				return None
			},
			Some(x) => x,
			_ => a,
		};
		let components = a.split('/').collect::<Vec<_>>();
		if components.iter().any(|x| x.is_empty()) {
			return None
		}
		if components.iter().any(|x| x.chars().any(|x| {
			if x.is_alphanumeric() {
				return false
			}
			if "_-.".find(x).is_some() {
				return false
			}
			return true
		})) {
			return None
		}
		Some(components.into_iter().map(|x| x.to_string()).collect())
	}
	async fn parse_next(reader: &mut tokio::io::BufReader<OwnedReadHalf>) -> anyhow::Result<CommandParseResult> {
		use CommandParseResult::*;
		let mut buf = "".into();
		reader.read_line(&mut buf).await?;
		buf.pop(); // remove \n
		let args: Vec<&str> = buf.split(' ').into_iter().collect();
		let ct = CommandType::from_name(args[0]);
		match ct {
			None => {
				return Ok(UnknownCommand)
			},
			Some(CommandType::Help) => {
				return Ok(FullParse(Command::Help))
			}
			Some(CommandType::Put) => {
				dbg!(&args);
				if args.len() != 3 {
					return Ok(PartialParse(CommandType::Put));
				}

				let file_path = match Self::parse_path(args[1], true) {
					Some(x) => x,
					None => return Ok(ArgError("illegal file name".into()))
				};
				let content_len = match args[2].parse::<usize>() {
					Ok(x) => x,
					Err(e) => {
						return Ok(PartialParse(CommandType::Put));
					}
				};
				let mut buf = vec![0u8; content_len];


				reader.read_exact(&mut buf).await?;
				if buf.iter().any(|x| {
					let x = *x as char;
					!(x.is_ascii_graphic() || x.is_ascii_whitespace())
				}) {
					return Ok(ArgError("text files only".into()));
				}

				return Ok(FullParse(Self::Put { file_path, content: buf }))
			},
			Some(CommandType::Get) => {
				if args.len() < 2 || args.len() > 3 {
					return Ok(PartialParse(CommandType::Get));
				}
				let file_path = match Self::parse_path(args[1], true) {
					Some(x) => x,
					None => return Ok(ArgError("illegal file name".into()))
				};
				let revision = args.get(2).map(|x| x.to_string());
				return Ok(FullParse(Self::Get { file_path, revision }))
			},
			Some(CommandType::List) => {
				if args.len() != 2 {
					return Ok(PartialParse(CommandType::List));
				}
				let prefix = match Self::parse_path(args[1], false) {
					Some(x) => x,
					None => return Ok(ArgError("illegal dir name".into()))
				};
				return Ok(FullParse(Self::List { prefix}))

			},
		}
	}
}

async fn handle_client(socket: tokio::net::TcpStream, vcs: Arc<Mutex<impl VCS>>) -> anyhow::Result<()> {
    let (rh, mut wh) = socket.into_split();
    let mut reader = tokio::io::BufReader::new(rh);

	loop {
		wh.write_all("READY\n".as_bytes()).await?;
		let command = Command::parse_next(&mut reader).await?;
		use CommandParseResult::*;
		match command {
			UnknownCommand => {
				// TODO: also tell command
				wh.write_all("ERR illegal method: askdfj\n".as_bytes()).await?;
				// Close connection
				return Ok(());
			},
			PartialParse(cmd) => {
				wh.write_all(format!("ERR usage: {}\n", cmd.get_help_msg()).as_bytes()).await?;
			},
			ArgError(error) => {
				wh.write_all(format!("ERR {}\n", error).as_bytes()).await?;
			},
			FullParse(cmd) => {
				// Need to put on seperate line otherwise compiler cries.
				let data = vcs.lock().execute(cmd);

				match data {
					Ok(resp) => {
						wh.write_all("OK ".as_bytes()).await?;
						wh.write_all(&resp).await?;
					},
					Err(err) => {
						wh.write_all("ERR ".as_bytes()).await?;
						wh.write_all(err.as_bytes()).await?;
					}
				}

			}
		}

	}

	Ok(())
}

async fn run() -> anyhow::Result<()> {
    let tcp_conn = tokio::net::TcpListener::bind("0.0.0.0:3007").await?;
	let vcs = InMemoryStorage::default();
	let vcs = Arc::new(Mutex::new(vcs));
    
    loop {
        let (socket, _) = tcp_conn.accept().await?;
		let vcs = vcs.clone();
        // let st_c = storage.clone();
        tokio::spawn(async move { dbg!(handle_client(socket, vcs).await) });
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
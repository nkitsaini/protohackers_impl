use std::{
    collections::{BTreeMap, HashMap, HashSet, BTreeSet},
    sync::Arc,
};

use anyhow::Context;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{io::{AsyncBufReadExt, AsyncWriteExt}, select};

type Job = serde_json::Value;
type QueueId = String; // text
type Priority = i64; // priority
type JobId = u64;

#[derive(Debug)]
enum GetResponseType {
    NoJob, // wait = false
    Ok {
        id: JobId,
        job: Job,
        pri: Priority,
        queue: QueueId,
    },
}

#[derive(Debug)]
enum Response {
    PutResponse { id: JobId },
    GetResponse( GetResponseType ),
    AbortResponse { no_job: bool },
    DeleteResponse { no_job: bool },
    Error { msg: String },
}
// --> {"status":"ok","id":12345}
// --> {"status":"ok","id":12345,"job":{"title":"example-job"},"pri":123,"queue":"queue1"}
// --> {"status":"ok"}
// --> {"status":"ok","id":12345,"job":{"title":"example-job"},"pri":123,"queue":"queue1"}
// --> {"status":"ok"}
// --> {"status":"no-job"}

impl Response {
    fn serialize(&self) -> serde_json::Value {
        match self {
            Response::AbortResponse { no_job } => {
                if *no_job {
                    json!({
                        "status": "no-job"
                    })
                } else {
                    json!({
                        "status": "ok"
                    })
                }
            }
            Response::DeleteResponse { no_job } => {
                if *no_job {
                    json!({
                        "status": "no-job"
                    })
                } else {
                    json!({
                        "status": "ok"
                    })
                }
            }
            Response::GetResponse(gt_resposne) => match gt_resposne {
                GetResponseType::NoJob => {
                    json!({
                        "status": "no-job"
                    })
                }
                GetResponseType::Ok {
                    id,
                    job,
                    pri,
                    queue,
                } => {
                    json!({
                        "status": "ok",
                        "id": id,
                        "job": job,
                        "pri": pri,
                        "queue": queue,
                    })
                }
            },
            Response::PutResponse { id } => {
                json!({
                    "status": "ok",
                    "id": id
                })
            },
            Response::Error { msg }  => {
                json!({
                    "status": "error",
                    "error": msg
                })
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct GetReq {
    queues: Vec<QueueId>,

    #[serde(default)]
    wait: bool,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "request", rename_all = "lowercase")]
enum Request {
    Put {
        queue: QueueId,
        job: Job,
        pri: Priority,
    },
    Get(GetReq), // wait
    Delete {
        id: JobId,
    },
    Abort {
        id: JobId,
    },
}

/// queue1:
///      1: job
///      2: job
/// queue2:
/// 	7: job

// get best priority job (across queue)
// get job by id

#[derive(Debug, Clone)]
struct JobDetails {
    job: Job,
    priority: Priority,
    id: JobId,
    queue: QueueId,
}

type ClientId = u64;

#[derive(Default)]
struct Queue {
    jobs: HashMap<JobId, JobDetails>,
    priorities: BTreeMap<(Priority, JobId), JobId>,
    active_jobs: HashMap<JobId, ClientId>,
}

impl Queue {
    fn add_new_job(&mut self, queue: QueueId, job: Job, pri: Priority, id: JobId) {
        let details = JobDetails {
            queue,
            priority: pri,
            job,
            id,
        };
        self.jobs.insert(id, details);
        self.priorities.insert((pri, id), id);
    }

    fn get_best_job(&self) -> Option<JobDetails> {
        let ((pri, job_id), v) = self.priorities.iter().last()?;
        Some(self.jobs.get(job_id).unwrap().clone())
    }

    fn move_job_to_active(&mut self, job: JobDetails, client: ClientId) {
        self.priorities.remove(&(job.priority, job.id));
        self.active_jobs.insert(job.id, client);
    }

    fn abort_active_job(&mut self, job_id: JobId, client: ClientId) -> anyhow::Result<bool> {
        if let Some(client_id) = self.active_jobs.get(&job_id) {
            if *client_id != client {
				// dbg!("Removing Client: Misbehaving", client);
                anyhow::bail!("Not the same client")
            }

            self.active_jobs.remove(&job_id).unwrap();
            let job_details = self.jobs.get(&job_id).unwrap(); // only in active_jobs we know the job
            self.priorities
                .insert((job_details.priority, job_details.id), job_details.id);
            Ok(false)
        } else {
            Ok(true)
        }
    }

    fn delete_active_id(&mut self, job_id: JobId) -> bool {
		if let Some(job) = self.jobs.get(&job_id) {
			let ac_rem = self.active_jobs.remove(&job_id);
			if ac_rem.is_none() {
				self.priorities.remove(&(job.priority, job.id)).is_none()
			} else {
				false
			}
		} else {
			true
		}
    }
}

type WaiterId = u64;

#[derive(Default)]
struct WaitingSystem {
    waiters: HashMap<QueueId, BTreeSet<WaiterId>>,
    waiters_to_queue: HashMap<WaiterId, (Vec<QueueId>, tokio::sync::oneshot::Sender<Response>, ClientId)>,
    last_waiter_id: u64,
}

impl WaitingSystem {
    fn wait(&mut self, queues: Vec<QueueId>, client_id: ClientId) -> tokio::sync::oneshot::Receiver<Response> {
		let (ts, rs) = tokio::sync::oneshot::channel();
		let waiter_id = self.last_waiter_id + 1;
		for queue in queues.iter() {
			self.waiters.entry(queue.clone()).or_default().insert(waiter_id);
		}
		self.waiters_to_queue.insert(waiter_id, (queues, ts, client_id));
		self.last_waiter_id = waiter_id;
		rs
    }
	fn queue_update(&mut self, queue_name: QueueId, queue: &mut Queue) {
		let waiters = self.waiters.entry(queue_name.clone()).or_default();
		let job = match queue.get_best_job() {
			Some(x) => x,
			None => return
		};
		let waiter_id = match waiters.pop_first() {
			Some(x) => x,
			None => return
		};
		let (waiter_queues, waiter_ts, client_id) = self.waiters_to_queue.remove(&waiter_id).unwrap();
		for oq in waiter_queues {
			self.waiters.entry(oq).or_default().remove(&waiter_id);
		}
		queue.move_job_to_active(job.clone(), client_id);
		waiter_ts.send(Response::GetResponse(
			GetResponseType::Ok { id: job.id, job: job.job, pri: job.priority, queue: queue_name }
		)).unwrap();
	}
}

struct Storage {
    queues: HashMap<QueueId, Queue>,
    msg_queues: Vec<QueueId>, // position is jobId and value is queueId
    waiting_system: WaitingSystem,
}

impl Storage {
    fn handle_message(
        &mut self,
        msg: Request,
        client_id: ClientId,
    ) -> anyhow::Result<tokio::sync::oneshot::Receiver<Response>> {
        let rv = match msg {
            Request::Put { queue, job, pri } => {
                let job_id = self.msg_queues.len() as JobId;
                self.msg_queues.push(queue.clone());
                let q = self.queues.entry(queue.clone()).or_default();
                q.add_new_job(queue.clone(), job, pri, job_id);
				self.waiting_system.queue_update(queue.clone(), q);
                Response::PutResponse { id: job_id }
            }

            // wait -> todo
            Request::Get(GetReq { queues, wait }) => {
                let mut best_job = None;
                    for queue_name in queues.clone() {
                        if let Some(queue) = self.queues.get(&queue_name) {
                            if let Some(job) = queue.get_best_job() {
                                let mut jb = best_job.unwrap_or(job.clone());
                                if jb.priority < job.priority {
                                    jb = job
                                }
                                best_job = Some(jb);
                            };
                        }
                    }
                if let Some(job) = best_job {
                    self.queues
                        .get_mut(&job.queue)
                        .unwrap()
                        .move_job_to_active(job.clone(), client_id);
                    Response::GetResponse(GetResponseType::Ok {
                        id: job.id,
                        job: job.job,
                        pri: job.priority,
                        queue: job.queue,
                    })
                } else {
					if wait {
						return Ok(self.waiting_system.wait(queues, client_id));
					} else {
						Response::GetResponse(GetResponseType::NoJob)
					}
                }
            }

            Request::Abort { id } => {
                if let Some(queue_name) = self.msg_queues.get(id as usize) {
					let is_no_job = self
                            .queues
                            .get_mut(queue_name)
                            .unwrap()
                            .abort_active_job(id, client_id)?;
					self.waiting_system.queue_update(queue_name.clone(), self.queues.get_mut(queue_name).unwrap());
					Response::AbortResponse { no_job: is_no_job }
                } else {
                    Response::AbortResponse { no_job: true }
                }
            }

            Request::Delete { id } => {
                if let Some(queue_name) = self.msg_queues.get(id as usize) {
                    Response::DeleteResponse {
                        no_job: self
                            .queues
                            .get_mut(queue_name)
                            .unwrap()
                            .delete_active_id(id),
                    }
                } else {
                    Response::DeleteResponse { no_job: true }
                }
            }
        };
		let (ts, rs) = tokio::sync::oneshot::channel();
		ts.send(rv).unwrap();
        Ok(rs)
    }
}

async fn handle_client(
    socket: tokio::net::TcpStream,
    client_id: ClientId,
    storage: Arc<Mutex<Storage>>,
) -> anyhow::Result<()> {
    let (rh, mut wh) = socket.into_split();
    let reader = tokio::io::BufReader::new(rh);
	let mut lines = reader.lines();

	let mut owned_messages: Vec<JobId> = vec![];
	// 1
	// 2
	// 3

	// abort 1 --> dusre

	// DISCONNECT: abort1: abort2: abort3

    loop {
		let curr_line = lines.next_line().await;
		let curr_line = || -> Option<String> {
			Some(curr_line.ok()??)
		}();
		let curr_line = match curr_line {
			Some(x) => x,
			None => {
				let mut st_lock = storage.lock();
				for job_id in owned_messages {
					st_lock.handle_message(Request::Abort { id: job_id }, client_id).ok();
				}
				anyhow::bail!("Closed the connection");
			}
		};

		// dbg!(client_id);
		let msg: Request = match serde_json::from_str(&curr_line) {
			Ok(x) => x,
			Err(e) => {
				let mut response = serde_json::to_string(&Response::Error { msg: "Wrong msg format".into() }.serialize())?;
				response.push('\n');
				wh.write_all(response.as_bytes()).await?;
				continue
			}
		};

		let res = storage.lock().handle_message(msg, client_id);
		let res = match res {
			Ok(x) => x,
			Err(e) => {
				let mut response = serde_json::to_string(&Response::Error { msg: "You can't abort that".into() }.serialize())?;
				response.push('\n');
				wh.write_all(response.as_bytes()).await?;
				continue
			}
		};

		let res = res.await.expect("Tokio One shot disconnected");

		match &res {
			Response::GetResponse(GetResponseType::Ok { id, job, pri, queue }) => {
				owned_messages.push(*id);
			},
			_ => {}
		}
		let mut response = serde_json::to_string(&res.serialize())?;
		response.push('\n');
		wh.write_all(response.as_bytes()).await?;
    }
}

async fn run() -> anyhow::Result<()> {
    let tcp_conn = tokio::net::TcpListener::bind("0.0.0.0:3007").await?;
    let mut client_id = 0;
    let storage: Arc<Mutex<Storage>> = Arc::new(Mutex::new(Storage {
        queues: Default::default(),
        msg_queues: Default::default(),
        waiting_system: Default::default(),
    }));
    loop {
        let (socket, _) = tcp_conn.accept().await?;
        let st_c = storage.clone();
        tokio::spawn(async move { dbg!(handle_client(socket, client_id, st_c).await) });
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
	async fn test_client_disconnect3() -> anyhow::Result<()> {
		tokio::task::spawn(run()); // server chala
		tokio::time::sleep(std::time::Duration::from_millis(100)).await;

		let (read_half1, mut write_half1) = tokio::net::TcpSocket::new_v4().unwrap().connect("0.0.0.0:3007".parse().unwrap()).await?.into_split();
		let (read_half2, mut write_half2) = tokio::net::TcpSocket::new_v4().unwrap().connect("0.0.0.0:3007".parse().unwrap()).await?.into_split();
		let (read_half3, mut write_half3) = tokio::net::TcpSocket::new_v4().unwrap().connect("0.0.0.0:3007".parse().unwrap()).await?.into_split();
		let (read_half4, mut write_half4) = tokio::net::TcpSocket::new_v4().unwrap().connect("0.0.0.0:3007".parse().unwrap()).await?.into_split();
		let (read_half5, mut write_half5) = tokio::net::TcpSocket::new_v4().unwrap().connect("0.0.0.0:3007".parse().unwrap()).await?.into_split();
		let (read_half6, mut write_half6) = tokio::net::TcpSocket::new_v4().unwrap().connect("0.0.0.0:3007".parse().unwrap()).await?.into_split();

		// Client 1 -> give job
		// Client 2 -> take job
		// Client 2 -> DISCONNECT
		// Client 3 -> take job (should get job)

		let mut reader2 = tokio::io::BufReader::new(read_half2).lines();
		let mut reader3 = tokio::io::BufReader::new(read_half3).lines();

		let mut msg1 = serde_json::to_string( &Request::Put { queue: "Q1".into(), job: json!("title"), pri: 123 })?;
		msg1.push('\n');
		write_half1.write(msg1.as_bytes()).await?;

		let mut msg2 = serde_json::to_string( &Request::Get(GetReq { queues: vec!["Q1".into()], wait: false }))?;
		msg2.push('\n');
		write_half2.write(msg2.as_bytes()).await?;

		let job = reader2.next_line().await.unwrap().unwrap();
		dbg!(job);
		write_half2.shutdown().await.unwrap();
		
		let mut msg3 = serde_json::to_string( &Request::Get(GetReq { queues: vec!["Q1".into()], wait: false }))?;
		msg3.push('\n');
		write_half3.write(msg3.as_bytes()).await?;

		select! {
			l = reader3.next_line() => {
				dbg!(l.unwrap().unwrap());
				assert!(false);
			},
			_ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
				anyhow::bail!("Timed out read");
			}
		}
		Ok(())


	}
}
P5: 
	- Maybe Issue with some client connect/disconnect
P6:
	- Sometimes there is unexpected ticket produced [./logs/p6-10.log]
	- Sometimes some messages are not recieved


```txt

handle_client(lrclient):
	while let Some(line) = lrclient.read_line().await {
		lrclient.send(line.reverse()).await
	}

class lrclient:
	buf = ""

	def read_line():
		while '\n' not in buf:
			buf += self.recv.get().await?

	def send(msg):
		for m in msg.split():
			self.send.put(m)

class lrconn:
	conn: UdpConn
	clients: lrclient
	def handle(msg, socket):
		# Connect/Disconnect
		# Ack/Data
		...


def main():
	conn = udp.bind()
	lrconn(conn).run().await

```
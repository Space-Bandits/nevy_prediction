//! A UDP relay that simulates network conditions such as latency, jitter and packet loss.
//!
//! Clients send to the relay's listen address and every datagram is forwarded to the target,
//! with the configured [`Conditions`] applied separately in each direction.
//! Each client address gets its own upstream socket, so the target sees one peer per client.

use std::{
    collections::HashMap,
    io,
    net::{SocketAddr, ToSocketAddrs, UdpSocket},
    sync::{Arc, Mutex, RwLock, mpsc::Sender},
    thread,
    time::Instant,
};

use conditions::{Conditions, Direction};
use scheduler::{Packet, Stats};

pub mod conditions;
pub mod config;
pub mod scheduler;

const MAX_DATAGRAM: usize = 65536;

pub struct Relay {
    listen: Arc<UdpSocket>,
    target: SocketAddr,
    up: Sender<Packet>,
    down: Sender<Packet>,
    up_stats: Arc<Mutex<Stats>>,
    down_stats: Arc<Mutex<Stats>>,
}

impl Relay {
    pub fn bind(
        listen: impl ToSocketAddrs,
        target: SocketAddr,
        conditions: Arc<RwLock<Conditions>>,
        seed: u64,
    ) -> io::Result<Self> {
        let listen = Arc::new(UdpSocket::bind(listen)?);

        let up_stats = Arc::new(Mutex::new(Stats::default()));
        let down_stats = Arc::new(Mutex::new(Stats::default()));

        let up = scheduler::spawn(Direction::Up, conditions.clone(), up_stats.clone(), seed);
        let down = scheduler::spawn(
            Direction::Down,
            conditions,
            down_stats.clone(),
            seed ^ 0x5DEECE66D,
        );

        Ok(Relay {
            listen,
            target,
            up,
            down,
            up_stats,
            down_stats,
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listen.local_addr()
    }

    /// Statistics for client -> server traffic.
    pub fn up_stats(&self) -> Arc<Mutex<Stats>> {
        self.up_stats.clone()
    }

    /// Statistics for server -> client traffic.
    pub fn down_stats(&self) -> Arc<Mutex<Stats>> {
        self.down_stats.clone()
    }

    /// Relays traffic forever on the current thread.
    pub fn run(self) -> io::Result<()> {
        let mut upstreams = HashMap::<SocketAddr, Arc<UdpSocket>>::new();
        let mut buffer = vec![0; MAX_DATAGRAM];

        loop {
            let (len, client) = match self.listen.recv_from(&mut buffer) {
                Ok(received) => received,
                Err(error) if is_transient(&error) => continue,
                Err(error) => return Err(error),
            };
            let received = Instant::now();

            let upstream = match upstreams.get(&client) {
                Some(upstream) => upstream.clone(),
                None => {
                    let upstream = self.connect_upstream(client)?;
                    upstreams.insert(client, upstream.clone());
                    upstream
                }
            };

            let _ = self.up.send(Packet {
                data: buffer[..len].to_vec(),
                received,
                socket: upstream,
                destination: self.target,
            });
        }
    }

    /// Binds a new socket used to talk to the target on behalf of `client`,
    /// and spawns a thread that forwards replies back to the client.
    fn connect_upstream(&self, client: SocketAddr) -> io::Result<Arc<UdpSocket>> {
        let bind = if self.target.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let upstream = Arc::new(UdpSocket::bind(bind)?);

        println!(
            "netsim: new client {client} (upstream {})",
            upstream.local_addr()?
        );

        let reader = upstream.clone();
        let listen = self.listen.clone();
        let down = self.down.clone();

        thread::Builder::new()
            .name(format!("netsim upstream {client}"))
            .spawn(move || {
                let mut buffer = vec![0; MAX_DATAGRAM];

                loop {
                    let len = match reader.recv_from(&mut buffer) {
                        Ok((len, _)) => len,
                        Err(error) if is_transient(&error) => continue,
                        Err(error) => {
                            eprintln!("netsim: upstream for {client} failed: {error}");
                            return;
                        }
                    };

                    let packet = Packet {
                        data: buffer[..len].to_vec(),
                        received: Instant::now(),
                        socket: listen.clone(),
                        destination: client,
                    };

                    if down.send(packet).is_err() {
                        return;
                    }
                }
            })?;

        Ok(upstream)
    }
}

/// On windows a udp socket reports `ConnectionReset` when a previous send was rejected
/// by the destination (e.g. the server isn't running yet). This shouldn't stop the relay.
fn is_transient(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionReset | io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
    )
}

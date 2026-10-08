use std::{
    cmp::Ordering,
    collections::BinaryHeap,
    net::{SocketAddr, UdpSocket},
    sync::{
        Arc, Mutex, RwLock,
        mpsc::{self, RecvTimeoutError, Sender},
    },
    thread,
    time::Instant,
};

use crate::conditions::{Conditions, Direction, LinkState};

/// A datagram waiting to be released.
pub struct Packet {
    pub data: Vec<u8>,
    pub received: Instant,
    /// Socket the packet will be sent from.
    pub socket: Arc<UdpSocket>,
    pub destination: SocketAddr,
}

/// Traffic statistics for one direction, reset whenever they are taken.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub received: u64,
    pub dropped: u64,
    pub duplicated: u64,
    pub delay_count: u64,
    /// Sum of delays in milliseconds.
    pub delay_sum: f64,
    pub delay_min: f64,
    pub delay_max: f64,
}

impl Stats {
    fn record_delay(&mut self, delay: f64) {
        if self.delay_count == 0 {
            self.delay_min = delay;
            self.delay_max = delay;
        } else {
            self.delay_min = self.delay_min.min(delay);
            self.delay_max = self.delay_max.max(delay);
        }

        self.delay_count += 1;
        self.delay_sum += delay;
    }
}

impl std::fmt::Display for Stats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} pkts, {} dropped, {} dup",
            self.received, self.dropped, self.duplicated
        )?;

        if self.delay_count > 0 {
            write!(
                f,
                ", delay avg {:.1}ms min {:.1}ms max {:.1}ms",
                self.delay_sum / self.delay_count as f64,
                self.delay_min,
                self.delay_max
            )?;
        }

        Ok(())
    }
}

struct Scheduled {
    release: Instant,
    sequence: u64,
    packet: Arc<Packet>,
}

impl PartialEq for Scheduled {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Scheduled {}

impl PartialOrd for Scheduled {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Scheduled {
    // reversed so that the binary heap pops the earliest release first
    fn cmp(&self, other: &Self) -> Ordering {
        (other.release, other.sequence).cmp(&(self.release, self.sequence))
    }
}

/// Spawns a thread that delays, drops and duplicates packets for one direction of traffic.
///
/// The thread exits once the returned sender and all its clones are dropped.
pub fn spawn(
    direction: Direction,
    conditions: Arc<RwLock<Conditions>>,
    stats: Arc<Mutex<Stats>>,
    seed: u64,
) -> Sender<Packet> {
    let (sender, receiver) = mpsc::channel::<Packet>();

    thread::Builder::new()
        .name(format!("netsim {direction:?}"))
        .spawn(move || {
            let mut link = LinkState::new(seed);
            let mut queue = BinaryHeap::<Scheduled>::new();
            let mut sequence = 0u64;

            loop {
                let next = match queue.peek() {
                    Some(next) => receiver
                        .recv_timeout(next.release.saturating_duration_since(Instant::now())),
                    None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
                };

                match next {
                    Ok(packet) => {
                        let releases = {
                            let conditions = conditions.read().unwrap();

                            if conditions.paused {
                                Vec::new()
                            } else {
                                link.plan(
                                    conditions.link(direction),
                                    packet.received,
                                    packet.data.len(),
                                )
                            }
                        };

                        let mut stats = stats.lock().unwrap();
                        stats.received += 1;
                        if releases.is_empty() {
                            stats.dropped += 1;
                        }
                        if releases.len() > 1 {
                            stats.duplicated += 1;
                        }
                        if let Some(&release) = releases.first() {
                            stats.record_delay((release - packet.received).as_secs_f64() * 1000.);
                        }
                        drop(stats);

                        let packet = Arc::new(packet);
                        for release in releases {
                            sequence += 1;
                            queue.push(Scheduled {
                                release,
                                sequence,
                                packet: packet.clone(),
                            });
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }

                let now = Instant::now();
                while queue.peek().is_some_and(|next| next.release <= now) {
                    let Scheduled { packet, .. } = queue.pop().unwrap();

                    if let Err(error) = packet.socket.send_to(&packet.data, packet.destination) {
                        eprintln!(
                            "netsim: failed to send {direction:?} packet to {}: {error}",
                            packet.destination
                        );
                    }
                }
            }
        })
        .expect("failed to spawn scheduler thread");

    sender
}

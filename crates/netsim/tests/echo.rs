use std::{
    net::UdpSocket,
    sync::{Arc, RwLock},
    thread,
    time::{Duration, Instant},
};

use netsim::{Relay, conditions::Conditions};

/// Starts an echo server behind a relay and returns the client socket connected to the relay.
fn setup(conditions: &Arc<RwLock<Conditions>>) -> UdpSocket {
    let echo = UdpSocket::bind("127.0.0.1:0").unwrap();
    let echo_addr = echo.local_addr().unwrap();
    thread::spawn(move || {
        let mut buffer = [0; 2048];
        while let Ok((len, from)) = echo.recv_from(&mut buffer) {
            let _ = echo.send_to(&buffer[..len], from);
        }
    });

    let relay = Relay::bind("127.0.0.1:0", echo_addr, conditions.clone(), 1).unwrap();
    let relay_addr = relay.local_addr().unwrap();
    thread::spawn(move || relay.run());

    let client = UdpSocket::bind("127.0.0.1:0").unwrap();
    client.connect(relay_addr).unwrap();
    client
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    client
}

fn round_trip(client: &UdpSocket, payload: &[u8]) -> Option<Duration> {
    let start = Instant::now();
    client.send(payload).unwrap();

    let mut buffer = [0; 2048];
    let len = client.recv(&mut buffer).ok()?;
    assert_eq!(&buffer[..len], payload);
    Some(start.elapsed())
}

#[test]
fn round_trip_includes_latency_in_both_directions() {
    let conditions = Arc::new(RwLock::new(Conditions::default()));
    {
        let mut conditions = conditions.write().unwrap();
        conditions.up.latency = 20.;
        conditions.down.latency = 30.;
    }

    let client = setup(&conditions);

    for i in 0..10u8 {
        let rtt = round_trip(&client, &[i; 32]).expect("packet was lost");
        assert!(rtt >= Duration::from_millis(50), "rtt {rtt:?} too short");
        assert!(rtt < Duration::from_millis(100), "rtt {rtt:?} too long");
    }
}

#[test]
fn conditions_change_live() {
    let conditions = Arc::new(RwLock::new(Conditions::default()));
    let client = setup(&conditions);

    assert!(round_trip(&client, b"hello").is_some());

    conditions.write().unwrap().up.loss = 1.;
    assert!(round_trip(&client, b"lost").is_none());

    *conditions.write().unwrap() = Conditions {
        paused: true,
        ..Default::default()
    };
    assert!(round_trip(&client, b"paused").is_none());

    conditions.write().unwrap().paused = false;
    assert!(round_trip(&client, b"back").is_some());
}

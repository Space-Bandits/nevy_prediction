use std::{
    net::{SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use netsim::{
    Relay,
    conditions::Conditions,
    config::{self, Config},
};

const DEFAULT_CONFIG: &str = "netsim.toml";
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// UDP relay that simulates latency, jitter, packet loss and more.
///
/// All settings come from a config file (`netsim.toml` by default, or the first argument).
/// If it doesn't exist a commented template is created. The file is watched and changes are applied live.
fn main() {
    let path = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| DEFAULT_CONFIG.to_string()),
    );

    if let Err(error) = run(&path) {
        eprintln!("netsim: {error}");
        std::process::exit(1);
    }
}

fn run(path: &Path) -> Result<(), String> {
    if !path.exists() {
        std::fs::write(path, config::TEMPLATE)
            .map_err(|error| format!("failed to create {}: {error}", path.display()))?;
        println!("netsim: created default config at {}", path.display());
    }

    let text = read(path)?;
    let config = Config::parse(&text).map_err(|error| format!("{}: {error}", path.display()))?;

    let target = resolve(&config.target)?;
    let seed = config.seed.unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
    });

    let conditions = Arc::new(RwLock::new(config.conditions()?));

    let relay = Relay::bind(&config.listen, target, conditions.clone(), seed)
        .map_err(|error| format!("failed to bind {}: {error}", config.listen))?;

    println!(
        "netsim: relaying {} -> {target} (seed {seed})",
        relay.local_addr().map_err(|error| error.to_string())?
    );
    println!("netsim: watching {} for changes", path.display());
    println!("{}", conditions.read().unwrap());

    let stats_millis = Arc::new(AtomicU64::new(stats_interval_millis(&config)));
    spawn_stats(&relay, stats_millis.clone());

    {
        let path = path.to_path_buf();
        thread::spawn(move || watch(&path, text, config, conditions, stats_millis));
    }

    relay
        .run()
        .map_err(|error| format!("relay failed: {error}"))
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))
}

fn resolve(address: &str) -> Result<SocketAddr, String> {
    address
        .to_socket_addrs()
        .map_err(|error| format!("invalid address '{address}': {error}"))?
        .next()
        .ok_or_else(|| format!("address '{address}' didn't resolve"))
}

fn stats_interval_millis(config: &Config) -> u64 {
    (config.stats * 1000.) as u64
}

/// Polls the config file and applies changes.
///
/// Invalid configs are reported and ignored, keeping the previous settings.
fn watch(
    path: &Path,
    mut text: String,
    mut config: Config,
    conditions: Arc<RwLock<Conditions>>,
    stats_millis: Arc<AtomicU64>,
) {
    let modified = |path: &Path| std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let mut last_modified = modified(path);

    loop {
        thread::sleep(POLL_INTERVAL);

        let current_modified = modified(path);
        if current_modified == last_modified {
            continue;
        }
        last_modified = current_modified;

        // editors may truncate before writing, a partial file will be caught by the next modification
        let Ok(new_text) = read(path) else { continue };
        if new_text == text {
            continue;
        }
        text = new_text;

        let new_config = match Config::parse(&text) {
            Ok(new_config) => new_config,
            Err(error) => {
                println!("netsim: config error, keeping previous settings: {error}");
                continue;
            }
        };

        if new_config.listen != config.listen
            || new_config.target != config.target
            || new_config.seed != config.seed
        {
            println!("netsim: changes to listen, target and seed only apply after a restart");
        }

        *conditions.write().unwrap() = new_config.conditions().unwrap();
        stats_millis.store(stats_interval_millis(&new_config), Ordering::Relaxed);

        println!("netsim: reloaded config");
        println!("{}", conditions.read().unwrap());

        config = new_config;
    }
}

fn spawn_stats(relay: &Relay, millis: Arc<AtomicU64>) {
    let up = relay.up_stats();
    let down = relay.down_stats();

    thread::spawn(move || {
        let mut last_print = Instant::now();

        loop {
            thread::sleep(POLL_INTERVAL);

            let interval = millis.load(Ordering::Relaxed);
            if interval == 0 {
                // keep the stats from accumulating while disabled
                *up.lock().unwrap() = Default::default();
                *down.lock().unwrap() = Default::default();
                last_print = Instant::now();
                continue;
            }

            if last_print.elapsed() < Duration::from_millis(interval) {
                continue;
            }
            last_print = Instant::now();

            let up = std::mem::take(&mut *up.lock().unwrap());
            let down = std::mem::take(&mut *down.lock().unwrap());

            if up.received > 0 || down.received > 0 {
                println!("[stats] up: {up} | down: {down}");
            }
        }
    });
}

mod provider;
mod router;
mod server;
mod settings;

use anyhow::Result;
use local_ip_address::{list_afinet_netifas, local_ip};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use router::Router;
use std::env;
use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::process::{self, Command};
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:18100";

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args: Vec<String> = env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("start") => start_daemon().await,
        Some("stop") => stop_daemon(),
        Some("status") => show_status(),
        Some("reload") => reload_daemon(),
        Some("config") => open_config_file(),
        Some("help") | Some("--help") | Some("-h") => {
            print_help();
            Ok(())
        }
        _ => {
            println!("Usage: cc-mapping [start|stop|status|reload|config|help]");
            println!("Run 'cc-mapping help' for more information");
            Ok(())
        }
    }
}

async fn start_daemon() -> Result<()> {
    // Check if already running
    if is_running() {
        println!("❌ cc-mapping is already running");
        println!("   Run 'cc-mapping stop' to stop it first");
        process::exit(1);
    }

    println!("🚀 Starting cc-mapping...");
    println!();

    // Write PID file
    let pid = process::id();
    write_pid_file(pid)?;

    let advertise_addr = detect_advertise_addr(DEFAULT_BIND_ADDR);

    // Configure CLI tools
    println!("⚙️  Configuring CLI tools...");
    if let Err(e) = settings::configure_all(&advertise_addr) {
        tracing::warn!("Failed to configure CLI tools: {}", e);
        println!("⚠️  Warning: Failed to configure CLI tools automatically");
        println!("   You may need to configure Claude Code and Codex manually");
    }
    println!();

    // Keep upstream responses compressed so headers stay consistent end-to-end.
    let http_client = reqwest::Client::builder()
        .no_gzip()
        .no_deflate()
        .no_brotli()
        .build()?;

    // Initialize router
    let router = Arc::new(Router::new(http_client)?);

    // Start config file watcher
    start_config_watcher(router.clone())?;

    #[cfg(unix)]
    {
        let router_for_signal = router.clone();
        tokio::spawn(async move {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sig = signal(SignalKind::user_defined1()).expect("register SIGUSR1");
            while sig.recv().await.is_some() {
                tracing::info!("Received SIGUSR1, reloading config");
                if let Err(e) = router_for_signal.reload_config().await {
                    tracing::error!("Failed to reload config: {}", e);
                }
            }
        });
    }

    // Start server
    println!("✨ cc-mapping is running!");
    println!("   Listening on:   http://{}", DEFAULT_BIND_ADDR);
    println!("   Share this URL: http://{}", advertise_addr);
    println!("   Claude Code: POST /v1/messages");
    println!("   Codex:       POST /responses");
    println!();
    println!("💡 Tip: Edit ~/.cc-mapping/provider.json to configure model_mapping");
    println!();

    // Run server (blocks until shutdown)
    server::run_server(router, DEFAULT_BIND_ADDR).await?;

    // Cleanup on shutdown
    remove_pid_file()?;

    Ok(())
}

fn start_config_watcher(router: Arc<Router>) -> Result<()> {
    // Get config file path
    let config_path = provider::get_config_path()?;

    tracing::info!("Starting config file watcher");
    tracing::debug!("Watching: {:?}", config_path);

    // Create async channel for file events
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    // Spawn watcher in a blocking thread (notify requires blocking context)
    std::thread::spawn(move || {
        let tx_clone = tx.clone();
        let mut watcher = match RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let _ = tx_clone.send(event);
                }
            },
            Config::default(),
        ) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("Failed to create file watcher: {}", e);
                return;
            }
        };

        // Watch config file
        if let Err(e) = watcher.watch(&config_path, RecursiveMode::NonRecursive) {
            tracing::warn!("Failed to watch config: {}", e);
        }

        // Keep watcher alive
        loop {
            std::thread::park();
        }
    });

    // Spawn async task to handle file events
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            // Only reload on modify/create events
            if !matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                continue;
            }

            tracing::info!("Config file changed: {:?}", event.paths);

            // Reload config with a small delay to avoid partial writes
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            if let Err(e) = router.reload_config().await {
                tracing::error!("Failed to reload model_mapping: {}", e);
            }
        }
    });

    Ok(())
}

fn stop_daemon() -> Result<()> {
    if !is_running() {
        println!("cc-mapping is not running");
        return Ok(());
    }

    let pid = read_pid_file()?;

    println!("Stopping cc-mapping (PID: {})...", pid);

    // Send SIGTERM
    #[cfg(unix)]
    {
        use std::process::Command;
        Command::new("kill")
            .arg(pid.to_string())
            .output()
            .expect("Failed to send kill signal");
    }

    remove_pid_file()?;
    println!("✓ cc-mapping stopped");

    Ok(())
}

fn reload_daemon() -> Result<()> {
    if !is_running() {
        println!("cc-mapping is not running");
        return Ok(());
    }

    let mapping = provider::load_model_mapping()?;
    let pid = read_pid_file()?;

    #[cfg(unix)]
    {
        use std::process::Command;
        let output = Command::new("kill")
            .arg("-USR1")
            .arg(pid.to_string())
            .output()?;
        if !output.status.success() {
            anyhow::bail!("Failed to send reload signal to PID {}", pid);
        }
    }

    #[cfg(not(unix))]
    {
        println!("reload is not supported on this platform");
        return Ok(());
    }

    println!(
        "✓ Configuration reload triggered (PID: {}, {} mapping(s))",
        pid,
        mapping.len()
    );
    Ok(())
}

fn show_status() -> Result<()> {
    if !is_running() {
        println!("Status: ❌ Not running");
        return Ok(());
    }

    let pid = read_pid_file()?;
    println!("Status: ✅ Running");
    println!("PID:    {}", pid);
    println!("Bind:   http://{}", DEFAULT_BIND_ADDR);
    println!(
        "Share:  http://{}",
        detect_advertise_addr(DEFAULT_BIND_ADDR)
    );

    Ok(())
}

fn open_config_file() -> Result<()> {
    let config_path = provider::get_config_path()?;
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !config_path.exists() {
        fs::write(&config_path, "{\n  \"model_mapping\": {}\n}\n")?;
    }

    println!("Configuration file: {}", config_path.display());

    let status = open_file_with_system_default(&config_path)?;
    if !status.success() {
        anyhow::bail!(
            "Failed to open config file with system default application: {}",
            config_path.display()
        );
    }
    Ok(())
}

fn open_file_with_system_default(path: &std::path::Path) -> Result<std::process::ExitStatus> {
    #[cfg(target_os = "macos")]
    {
        Ok(Command::new("open").arg(path).status()?)
    }

    #[cfg(target_os = "linux")]
    {
        Ok(Command::new("xdg-open").arg(path).status()?)
    }

    #[cfg(target_os = "windows")]
    {
        Ok(Command::new("cmd")
            .arg("/C")
            .arg("start")
            .arg("")
            .arg(path)
            .status()?)
    }
}

fn print_help() {
    println!("cc-mapping - HTTP Proxy for Claude Code & Codex");
    println!();
    println!("USAGE:");
    println!("    cc-mapping [COMMAND]");
    println!();
    println!("COMMANDS:");
    println!("    start     Start the proxy daemon");
    println!("    stop      Stop the proxy daemon");
    println!("    status    Show proxy status");
    println!("    reload    Reload provider.json configuration");
    println!("    config    Print and open provider.json");
    println!("    help      Show this help message");
    println!();
    println!("DESCRIPTION:");
    println!("    cc-mapping is an HTTP proxy that routes Claude Code and Codex");
    println!("    requests by model via model_mapping, forwarding each model to");
    println!("    its configured upstream URL and API key.");
    println!();
    println!("FEATURES:");
    println!("    • Model-aware routing via model_mapping (substring match)");
    println!("    • Optional upstream model name replacement");
    println!("    • Auto-configuration (sets up Claude Code & Codex)");
    println!();
    println!("CONFIGURATION:");
    println!("    ~/.cc-mapping/provider.json");
    println!();
    println!("EXAMPLES:");
    println!("    # Start the proxy");
    println!("    cc-mapping start");
    println!();
    println!("    # Check if running");
    println!("    cc-mapping status");
    println!();
    println!("    # Stop the proxy");
    println!("    cc-mapping stop");
    println!();
    println!("    # Reload configuration");
    println!("    cc-mapping reload");
    println!();
    println!("    # Print and open configuration file");
    println!("    cc-mapping config");
    println!();
    println!("For more information: https://github.com/yourusername/cc-mapping");
}

fn detect_advertise_addr(bind_addr: &str) -> String {
    let socket = bind_addr.parse::<SocketAddr>().ok();
    let port = socket.map(|sock| sock.port()).unwrap_or(18100);

    if let Some(socket) = socket {
        let ip = socket.ip();
        if !ip.is_loopback() && !ip.is_unspecified() {
            return format!("{}:{}", ip, port);
        }
    }

    if let Some(ip) = detect_lan_ip() {
        let addr = format!("{}:{}", ip, port);
        tracing::info!("Detected LAN address for CLI config: {}", addr);
        return addr;
    }

    tracing::warn!(
        "Falling back to 127.0.0.1:{} for CLI configuration; could not detect LAN IP",
        port
    );
    format!("127.0.0.1:{}", port)
}

fn detect_lan_ip() -> Option<IpAddr> {
    if let Ok(netifs) = list_afinet_netifas() {
        for (iface, ip) in netifs {
            if is_virtual_iface(&iface) || !is_usable_ip(&ip) {
                continue;
            }

            match ip {
                IpAddr::V4(_) => return Some(ip),
                IpAddr::V6(_) => continue,
            }
        }
    }

    if let Ok(ip) = local_ip() {
        if is_usable_ip(&ip) {
            return Some(ip);
        }
    }

    None
}

fn is_virtual_iface(iface: &str) -> bool {
    let name = iface.to_ascii_lowercase();
    matches!(name.as_str(), "lo" | "localhost" | "loopback")
        || name.starts_with("docker")
        || name.starts_with("br-")
        || name.starts_with("veth")
        || name.starts_with("virbr")
        || name.starts_with("vmnet")
        || name.starts_with("tailscale")
        || name.starts_with("wg")
        || name.starts_with("tun")
        || name.starts_with("tap")
        || name.starts_with("zt")
}

fn is_usable_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback() {
                return false;
            }
            let octets = v4.octets();
            if octets[0] == 169 && octets[1] == 254 {
                return false; // IPv4 link-local
            }
            true
        }
        IpAddr::V6(_) => false,
    }
}

// Helper functions for PID file management
fn get_pid_file_path() -> Result<PathBuf> {
    let home = env::var("HOME")?;
    let pid_dir = PathBuf::from(home).join(".cc-mapping");
    fs::create_dir_all(&pid_dir)?;
    Ok(pid_dir.join("cc-mapping.pid"))
}

fn write_pid_file(pid: u32) -> Result<()> {
    let path = get_pid_file_path()?;
    fs::write(path, pid.to_string())?;
    Ok(())
}

fn read_pid_file() -> Result<u32> {
    let path = get_pid_file_path()?;
    let content = fs::read_to_string(path)?;
    Ok(content.trim().parse()?)
}

fn remove_pid_file() -> Result<()> {
    let path = get_pid_file_path()?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn is_running() -> bool {
    let Ok(pid_path) = get_pid_file_path() else {
        return false;
    };

    if !pid_path.exists() {
        return false;
    }

    let Ok(pid) = read_pid_file() else {
        return false;
    };

    // Check if process is actually running
    #[cfg(unix)]
    {
        use std::process::Command;
        let output = Command::new("kill").arg("-0").arg(pid.to_string()).output();

        matches!(output, Ok(o) if o.status.success())
    }

    #[cfg(not(unix))]
    {
        true // Assume running on non-Unix systems
    }
}

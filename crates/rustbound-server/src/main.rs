use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rustbound_server::config::{load_config, parse_args};
use rustbound_server::server::Server;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let server_args = match parse_args(&args) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("failed to parse arguments: {e}");
            std::process::exit(1);
        }
    };

    let config = match load_config(&server_args) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("failed to load config: {e}");
            std::process::exit(1);
        }
    };

    let mut server = match Server::start(config) {
        Ok(server) => {
            eprintln!("Rustbound server listening on {}", server.bind_addr());
            server
        }
        Err(error) => {
            eprintln!("failed to start server: {error}");
            std::process::exit(1);
        }
    };

    // Set up Ctrl+C / SIGTERM handler for clean shutdown
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();
    if ctrlc::set_handler(move || {
        running_clone.store(false, Ordering::Release);
    })
    .is_err()
    {
        eprintln!("warning: failed to set Ctrl+C handler");
    }

    // Wait for the shutdown signal
    while running.load(Ordering::Acquire) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    server.shutdown();
    eprintln!("server shut down");
}

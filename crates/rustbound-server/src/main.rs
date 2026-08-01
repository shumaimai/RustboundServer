use rustbound_server::listener::{ListenerConfig, start_listener};

fn main() {
    let config = ListenerConfig::default();
    let mut handle = match start_listener(config, |stream, addr| {
        std::thread::spawn(move || {
            eprintln!("accepted connection from {addr}");
            // Connection handling will be implemented in Issue #40
            drop(stream);
        });
    }) {
        Ok(handle) => {
            eprintln!("Rustbound server listening on {}", handle.bind_addr());
            handle
        }
        Err(error) => {
            eprintln!("failed to start server: {error}");
            std::process::exit(1);
        }
    };

    // Wait for Ctrl+C
    let _ = ctrlc_handler();
    handle.shutdown();
    eprintln!("server shut down");
}

fn ctrlc_handler() -> std::io::Result<()> {
    // Simple blocking wait - in a real server this would use a signal handler
    // For now, just park the main thread; the listener handle's Drop will
    // handle shutdown when the process exits.
    std::thread::park();
    Ok(())
}

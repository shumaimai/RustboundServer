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

    // Wait for Ctrl+C (simplified - in production this would use a signal handler)
    // For now, just park the main thread
    std::thread::park();

    server.shutdown();
    eprintln!("server shut down");
}

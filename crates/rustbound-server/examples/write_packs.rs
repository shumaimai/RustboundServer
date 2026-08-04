fn main() {
    let dir = std::path::Path::new("data/hakoniwa/packs");
    match rustbound_server::container::write_bundled_packs(dir) {
        Ok(n) => println!("wrote {n} packs to {}", dir.display()),
        Err(e) => {
            eprintln!("failed to write packs: {e}");
            std::process::exit(1);
        }
    }
}

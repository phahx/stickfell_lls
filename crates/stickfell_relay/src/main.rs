//! `stickfell_relay [--bind 0.0.0.0:7777]` — run this on any machine both
//! players can reach (a small VPS is ideal). It only forwards packets, so it
//! needs almost no CPU or bandwidth.

fn main() {
    let mut bind = format!("0.0.0.0:{}", stickfell_net::DEFAULT_RELAY_PORT);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--bind" => bind = args.next().expect("--bind needs an address"),
            "-h" | "--help" => {
                println!("usage: stickfell_relay [--bind ADDR:PORT]   (default {bind})");
                return;
            }
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    let mut server = match stickfell_net::RelayServer::bind(&bind) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("could not bind {bind}: {e}");
            std::process::exit(1);
        }
    };
    server.verbose = true;
    eprintln!("[relay] listening on {}", server.local_addr().map(|a| a.to_string()).unwrap_or(bind));
    server.run();
}

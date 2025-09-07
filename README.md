# rbit

This is a Rust project initialized with Cargo.

## Getting Started

To build the project:

```sh
cargo build
```

To run the project:

```sh
cargo run
```

To test the project:

```sh
cargo test
```

---

## Usage

Build the binary:

```sh
cargo build --release
```

Run the built binary. The program accepts either a magnet link or a path to a .torrent file as the first positional argument. Optionally pass `--dest` to set the save folder and `--config` to point at a config file.

Examples:

```sh
# add a magnet to remote qBittorrent
./target/release/rbit 'magnet:?xt=urn:btih:...' --dest=/downloads/etc

# add a torrent file, using a custom config and override host
./target/release/rbit ./some.torrent --config ./rbit.toml --host http://127.0.0.1:8080
```

Config file (toml) example — place `rbit.toml` in the repo root or in your XDG config dir (`$XDG_CONFIG_HOME/com/example/rbit/rbit.toml`):

```toml
default_save_path = "/downloads"

[qbittorrent]
host = "http://http://127.0.0.1:8080"
username = "admin"
password = "secret"
```

Notes & troubleshooting
- If you see connection refused, ensure the `host` is reachable from this machine and the qBittorrent Web UI is enabled.
- You can override credentials on the command line with `--username` and `--password`.
- For headless systems, `cargo build --release` produces the optimized binary in `./target/release`.


use std::fs::File;
use std::path::PathBuf;

use clap::Parser;
use directories::BaseDirs;
use reqwest::blocking::Client;
use reqwest::blocking::multipart;
use serde::Deserialize;
use tabled::{Table, Tabled};
use config::{Config as ConfigLoader, File as ConfigFile, FileFormat};

#[derive(Parser, Debug)]
#[command(author, version, about = "simple qBittorrent client", long_about = None)]
struct Cli {
    /// Path to config file (optional)
    #[arg(short = 'c', long)]
    config: Option<PathBuf>,

    /// qBittorrent host (overrides config)
    #[arg(long)]
    host: Option<String>,

    /// qBittorrent username (overrides config)
    #[arg(long)]
    username: Option<String>,

    /// qBittorrent password (overrides config)
    #[arg(long)]
    password: Option<String>,

    /// Do not send requests; print what would be sent
    #[arg(long)]
    dry_run: bool,

    /// Print verbose HTTP requests/responses
    #[arg(long, short = 'v')]
    verbose: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Add a torrent (magnet link or .torrent file)
    Add {
        /// Path to a .torrent file or a magnet link
        input: String,

        /// Destination folder for the torrent content
        #[arg(short, long)]
        dest: Option<PathBuf>,
    },
    /// List torrents (default: active torrents). Use --all to show all.
    List {
        /// Show all torrents, not only active ones
        #[arg(long)]
        all: bool,
    },
}

#[derive(Deserialize, Debug)]
struct Config {
    default_save_path: Option<String>,
    qbittorrent: Option<QBConfig>,
}

#[derive(Deserialize, Debug)]
struct QBConfig {
    host: String,
    username: Option<String>,
    password: Option<String>,
}

fn read_config(path: Option<PathBuf>) -> Config {
    // Build a config loader that reads from (in order):
    // 1) explicit `--config` path (if provided)
    // 2) XDG config path (if present)
    // 3) local `./rbit.toml` (if present)
    // All file sources are added as optional (not required) so missing files don't error.
    let mut builder = ConfigLoader::builder();

    if let Some(p) = path {
        builder = builder.add_source(ConfigFile::from(p).format(FileFormat::Toml).required(false));
    } else {
        // Prefer ~/.config/rbit/config.toml per user preference
        if let Some(basedirs) = BaseDirs::new() {
            let xdg = basedirs.config_dir().join("rbit").join("config.toml");
            builder = builder.add_source(ConfigFile::from(xdg).format(FileFormat::Toml).required(false));
        }
        // Also allow local ./rbit.toml for repo-level config
    builder = builder.add_source(ConfigFile::from(PathBuf::from("rbit.toml")).format(FileFormat::Toml).required(false));
    }

    // Build the config loader; if building or deserialization fails, return defaults
    match builder.build() {
        Ok(loader) => loader.try_deserialize::<Config>().unwrap_or(Config {
            default_save_path: None,
            qbittorrent: None,
        }),
        Err(_) => Config {
            default_save_path: None,
            qbittorrent: None,
        },
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = read_config(cli.config.clone());

    let client = Client::builder().cookie_store(true).build()?;

    // Determine effective host and credentials (CLI overrides > config > default)
    let host = if let Some(h) = cli.host.clone() {
        h.trim_end_matches('/').to_string()
    } else if let Some(ref qb) = config.qbittorrent {
        qb.host.trim_end_matches('/').to_string()
    } else {
        "http://127.0.0.1:8080".to_string()
    };

    let username = cli.username.clone().or_else(|| config.qbittorrent.as_ref().and_then(|q| q.username.clone()));
    let password = cli.password.clone().or_else(|| config.qbittorrent.as_ref().and_then(|q| q.password.clone()));

    match cli.command {
        Command::Add { input, dest } => {
            // save path: CLI override > config.default_save_path > cwd
            let save_path = if let Some(d) = dest {
                d
            } else if let Some(ref s) = config.default_save_path {
                PathBuf::from(s)
            } else {
                std::env::current_dir()?
            };

            if input.starts_with("magnet:") {
                add_magnet(&client, &host, username.as_deref(), password.as_deref(), &input, &save_path, cli.dry_run, cli.verbose)?;
            } else {
                add_torrent_file(&client, &host, username.as_deref(), password.as_deref(), PathBuf::from(input), &save_path, cli.dry_run, cli.verbose)?;
            }
            println!("Added to qBittorrent (destination: {})", save_path.display());
        }
        Command::List { all } => {
            list_torrents(&client, &host, username.as_deref(), password.as_deref(), all, cli.verbose)?;
        }
    }

    Ok(())
}

#[derive(serde::Deserialize, Debug)]
struct TorrentInfo {
    name: String,
    hash: String,
    state: String,
    progress: Option<f64>,
    dlspeed: Option<u64>,
    upspeed: Option<u64>,
}

#[derive(Tabled)]
struct TorrentRow {
    id: String,
    name: String,
    status: String,
    progress: String,
    dl: String,
    up: String,
}

fn bytes_human(b: u64) -> String {
    let kb = 1024u64;
    if b >= kb * kb * kb {
        format!("{:.2} GB/s", b as f64 / (kb * kb * kb) as f64)
    } else if b >= kb * kb {
        format!("{:.2} MB/s", b as f64 / (kb * kb) as f64)
    } else if b >= kb {
        format!("{:.2} KB/s", b as f64 / kb as f64)
    } else {
        format!("{} B/s", b)
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        let mut t = s[..n].to_string();
        t.push_str("...");
        t
    }
}

fn list_torrents(client: &Client, host: &str, username: Option<&str>, password: Option<&str>, all: bool, verbose: bool) -> anyhow::Result<()> {
    login(client, host, username, password, verbose)?;
    let url = format!("{}/api/v2/torrents/info?filter=all", host);
    let res = client.get(&url).send()?;
    let body = res.text()?;
    let torrents: Vec<TorrentInfo> = serde_json::from_str(&body)?;

    // filter active by default: progress < 1.0 or dlspeed/upspeed > 0
    let rows: Vec<&TorrentInfo> = torrents.iter().filter(|t| {
        if all {
            return true;
        }
        let progress = t.progress.unwrap_or(0.0);
        let dls = t.dlspeed.unwrap_or(0);
        let ups = t.upspeed.unwrap_or(0);
        progress < 1.0 || dls > 0 || ups > 0
    }).collect();

    let mut table_rows: Vec<TorrentRow> = Vec::new();
    for t in rows {
        let id = if t.hash.len() >= 8 { t.hash[..8].to_string() } else { t.hash.clone() };
        let name = truncate(&t.name, 40);
        let status = t.state.clone();
        let progress = t.progress.map(|p| format!("{:.1}%", p * 100.0)).unwrap_or_else(|| "-".to_string());
        let dl = bytes_human(t.dlspeed.unwrap_or(0));
        let up = bytes_human(t.upspeed.unwrap_or(0));
        table_rows.push(TorrentRow { id, name, status, progress, dl, up });
    }

    let table = Table::new(table_rows).with(tabled::Style::psql());
    println!("{}", table);
    Ok(())
}

fn login(client: &Client, host: &str, username: Option<&str>, password: Option<&str>, verbose: bool) -> anyhow::Result<()> {
    if let (Some(user), Some(pass)) = (username, password) {
        let params = [("username", user), ("password", pass)];
        let url = format!("{}/api/v2/auth/login", host);
        let res = client.post(&url).form(&params).send()?;
        let status = res.status();
        let text = res.text()?;
        if verbose {
            println!("[verbose] POST {} -> {}", url, status);
            println!("[verbose] response: {}", text);
        }
        if text != "Ok." {
            anyhow::bail!("login failed: {}", text);
        }
    }
    Ok(())
}

fn add_magnet(client: &Client, host: &str, username: Option<&str>, password: Option<&str>, magnet: &str, save_path: &PathBuf, dry_run: bool, verbose: bool) -> anyhow::Result<()> {
    let url = format!("{}/api/v2/torrents/add", host);
    let save_path_s = save_path.to_string_lossy().to_string();
    let params = [("urls", magnet), ("savepath", save_path_s.as_str())];
    if dry_run {
        println!("[dry-run] POST {}", url);
        println!("[dry-run] form params: urls={}, savepath={}", magnet, save_path.display());
        return Ok(());
    }
    login(client, host, username, password, verbose)?;
    let res = client.post(&url).form(&params).send()?;
    let status = res.status();
    let body = res.text()?;
    if verbose {
        println!("[verbose] POST {} -> {}", url, status);
        println!("[verbose] response: {}", body);
    }
    if status.is_success() {
        Ok(())
    } else {
        anyhow::bail!("failed to add magnet: {}", body);
    }
}

fn add_torrent_file(client: &Client, host: &str, _username: Option<&str>, _password: Option<&str>, file: PathBuf, save_path: &PathBuf, dry_run: bool, verbose: bool) -> anyhow::Result<()> {
    let url = format!("{}/api/v2/torrents/add", host);

    let filename = file
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("upload.torrent")
        .to_string();

    let file_part = multipart::Part::reader(File::open(&file)?).file_name(filename);

    if dry_run {
        println!("[dry-run] POST {}", url);
        println!("[dry-run] file: {}", file.display());
        println!("[dry-run] savepath: {}", save_path.display());
        return Ok(());
    }

    // perform login first (no-op if no creds)
    login(client, host, _username, _password, verbose)?;

    let form = multipart::Form::new()
        .part("torrents", file_part)
        .text("savepath", save_path.to_string_lossy().to_string());

    let res = client.post(&url).multipart(form).send()?;
    let status = res.status();
    let body = res.text()?;
    if verbose {
        println!("[verbose] POST {} -> {}", url, status);
        println!("[verbose] response: {}", body);
    }
    if status.is_success() {
        Ok(())
    } else {
        anyhow::bail!("failed to add torrent file: {}", body);
    }
}

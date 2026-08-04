//! Server configuration for the Rustbound server.
//!
//! Parses server.properties (key=value format) and provides a typed
//! configuration struct. Also handles command-line argument parsing.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// Server configuration parsed from server.properties.
///
/// # Field usage
///
/// **Wired into live behavior:**
/// - `host`, `port`: TCP bind address
/// - `max_players`: Status response + Join Game packet
/// - `online_mode`: Login encryption (deferred, see #60)
/// - `view_distance`: Join Game, Set Render Distance, chunk streaming
/// - `simulation_distance`: Join Game, Set Simulation Distance
/// - `motd`: Status response description
/// - `default_gamemode`: Join Game gamemode for new players
/// - `connection_timeout_secs`: TCP connection timeout
/// - `network_compression_threshold`: Login Set Compression + Play framing
/// - `keep_alive_timeout_secs`: Keep Alive response timeout (idle kick)
///
/// **Wired into persistence:**
/// - `level_name`: World data directory for overrides/players
/// - `autosave_interval_secs`: Periodic flush interval
///
/// **Hakoniwa (箱庭):**
/// - `hakoniwa_size`: fixed garden size (`tiny` / `small` / `medium`)
///
/// **Intentionally unused (parsed but not enforced):**
/// - `white_list`: Whitelist enforcement is not implemented
/// - `pvp`: PvP damage rules not implemented
/// - `allow_nether`: deferred until hakoniwa H3 dimension packs
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// The host to bind to.
    pub host: String,
    /// The port to bind to.
    pub port: u16,
    /// The maximum number of players.
    pub max_players: i32,
    /// Whether the server is in online mode.
    pub online_mode: bool,
    /// The view distance (in chunks).
    pub view_distance: i32,
    /// The simulation distance (in chunks).
    pub simulation_distance: i32,
    /// The server's MOTD (message of the day).
    pub motd: String,
    /// Whether to enable the whitelist.
    pub white_list: bool,
    /// The level name (world name).
    pub level_name: String,
    /// The default gamemode (0=Survival, 1=Creative, 2=Adventure, 3=Spectator).
    pub default_gamemode: u8,
    /// Whether PvP is enabled.
    pub pvp: bool,
    /// Whether the Nether is enabled.
    pub allow_nether: bool,
    /// The connection timeout (in seconds).
    pub connection_timeout_secs: u64,
    /// The network compression threshold (-1 disables, >= 0 enables).
    pub network_compression_threshold: i32,
    /// The Keep Alive timeout (in seconds). If a client does not respond
    /// to a Keep Alive within this period, it is disconnected.
    pub keep_alive_timeout_secs: u64,
    /// The autosave interval (in seconds). 0 disables autosave (save only on shutdown).
    pub autosave_interval_secs: u64,
    /// Fixed garden size preset (`tiny` / `small` / `medium`).
    pub hakoniwa_size: crate::hakoniwa::MapSize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 25565,
            max_players: 20,
            online_mode: false,
            view_distance: 10,
            simulation_distance: 10,
            motd: "A Rust Minecraft Server".to_string(),
            white_list: false,
            level_name: "world".to_string(),
            default_gamemode: 0,
            pvp: true,
            allow_nether: true,
            connection_timeout_secs: 30,
            network_compression_threshold: 256,
            keep_alive_timeout_secs: 30,
            autosave_interval_secs: 600, // 10 minutes default
            hakoniwa_size: crate::hakoniwa::MapSize::Tiny,
        }
    }
}

/// An error encountered while parsing server configuration.
#[derive(Debug)]
pub enum ConfigError {
    /// Failed to read the configuration file.
    Read(std::io::Error),
    /// A configuration value was invalid.
    InvalidValue {
        /// The key name.
        key: String,
        /// The error message.
        message: String,
    },
    /// The configuration file contained an invalid line.
    InvalidLine {
        /// The line number (1-based).
        line: usize,
        /// The line content.
        content: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => write!(formatter, "failed to read config: {error}"),
            Self::InvalidValue { key, message } => {
                write!(formatter, "invalid value for '{key}': {message}")
            }
            Self::InvalidLine { line, content } => {
                write!(formatter, "invalid line {line}: {content}")
            }
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read(error) => Some(error),
            _ => None,
        }
    }
}

/// Parses a server.properties file into a `ServerConfig`.
///
/// The file format is `key=value` with one entry per line. Lines starting
/// with `#` are comments. Unknown keys are silently ignored.
pub fn parse_config_file(path: &Path) -> Result<ServerConfig, ConfigError> {
    let content = std::fs::read_to_string(path).map_err(ConfigError::Read)?;
    parse_config_string(&content)
}

/// Parses a server.properties string into a `ServerConfig`.
pub fn parse_config_string(content: &str) -> Result<ServerConfig, ConfigError> {
    let mut map: HashMap<String, String> = HashMap::new();

    for (i, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            return Err(ConfigError::InvalidLine {
                line: i + 1,
                content: line.to_string(),
            });
        };

        map.insert(key.trim().to_string(), value.trim().to_string());
    }

    let mut config = ServerConfig::default();
    apply_config_values(&mut config, &map)?;
    Ok(config)
}

fn apply_config_values(
    config: &mut ServerConfig,
    map: &HashMap<String, String>,
) -> Result<(), ConfigError> {
    if let Some(v) = map.get("server-ip") {
        config.host = v.clone();
    }
    if let Some(v) = map.get("server-port") {
        config.port = parse_u16(v, "server-port")?;
    }
    if let Some(v) = map.get("max-players") {
        config.max_players = parse_i32(v, "max-players")?;
    }
    if let Some(v) = map.get("online-mode") {
        config.online_mode = parse_bool(v, "online-mode")?;
    }
    if let Some(v) = map.get("view-distance") {
        config.view_distance = parse_i32(v, "view-distance")?;
    }
    if let Some(v) = map.get("simulation-distance") {
        config.simulation_distance = parse_i32(v, "simulation-distance")?;
    }
    if let Some(v) = map.get("motd") {
        config.motd = v.clone();
    }
    if let Some(v) = map.get("white-list") {
        config.white_list = parse_bool(v, "white-list")?;
    }
    if let Some(v) = map.get("level-name") {
        config.level_name = v.clone();
    }
    if let Some(v) = map.get("gamemode") {
        config.default_gamemode = parse_u8(v, "gamemode")?;
    }
    if let Some(v) = map.get("pvp") {
        config.pvp = parse_bool(v, "pvp")?;
    }
    if let Some(v) = map.get("allow-nether") {
        config.allow_nether = parse_bool(v, "allow-nether")?;
    }
    if let Some(v) = map.get("connection-timeout") {
        config.connection_timeout_secs = parse_u64(v, "connection-timeout")?;
    }
    if let Some(v) = map.get("network-compression-threshold") {
        config.network_compression_threshold = parse_i32(v, "network-compression-threshold")?;
    }
    if let Some(v) = map.get("keep-alive-timeout") {
        config.keep_alive_timeout_secs = parse_u64(v, "keep-alive-timeout")?;
    }
    if let Some(v) = map.get("autosave-interval") {
        config.autosave_interval_secs = parse_u64(v, "autosave-interval")?;
    }
    if let Some(v) = map.get("hakoniwa-size") {
        config.hakoniwa_size =
            crate::hakoniwa::MapSize::parse(v).ok_or_else(|| ConfigError::InvalidValue {
                key: "hakoniwa-size".to_string(),
                message: format!("expected tiny|small|medium, got '{v}'"),
            })?;
    }
    Ok(())
}

fn parse_bool(value: &str, key: &str) -> Result<bool, ConfigError> {
    match value.to_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ConfigError::InvalidValue {
            key: key.to_string(),
            message: format!("expected 'true' or 'false', got '{value}'"),
        }),
    }
}

fn parse_i32(value: &str, key: &str) -> Result<i32, ConfigError> {
    value.parse::<i32>().map_err(|_| ConfigError::InvalidValue {
        key: key.to_string(),
        message: format!("expected an integer, got '{value}'"),
    })
}

fn parse_u16(value: &str, key: &str) -> Result<u16, ConfigError> {
    value.parse::<u16>().map_err(|_| ConfigError::InvalidValue {
        key: key.to_string(),
        message: format!("expected a port number (0-65535), got '{value}'"),
    })
}

fn parse_u8(value: &str, key: &str) -> Result<u8, ConfigError> {
    value.parse::<u8>().map_err(|_| ConfigError::InvalidValue {
        key: key.to_string(),
        message: format!("expected a byte (0-255), got '{value}'"),
    })
}

fn parse_u64(value: &str, key: &str) -> Result<u64, ConfigError> {
    value.parse::<u64>().map_err(|_| ConfigError::InvalidValue {
        key: key.to_string(),
        message: format!("expected a non-negative integer, got '{value}'"),
    })
}

/// Command-line arguments for the server.
#[derive(Debug, Clone, Default)]
pub struct ServerArgs {
    /// The path to the configuration file.
    pub config_path: Option<PathBuf>,
    /// The host to bind to (overrides config).
    pub host: Option<String>,
    /// The port to bind to (overrides config).
    pub port: Option<u16>,
}

/// Parses command-line arguments.
///
/// Supported flags:
/// - `--config <path>`: path to server.properties
/// - `--host <addr>`: override bind host
/// - `--port <port>`: override bind port
/// - `--help`: print usage
pub fn parse_args(args: &[String]) -> Result<ServerArgs, ConfigError> {
    let mut server_args = ServerArgs::default();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--config" => {
                i += 1;
                if i >= args.len() {
                    return Err(ConfigError::InvalidValue {
                        key: "--config".to_string(),
                        message: "missing path argument".to_string(),
                    });
                }
                server_args.config_path = Some(PathBuf::from(&args[i]));
            }
            "--host" => {
                i += 1;
                if i >= args.len() {
                    return Err(ConfigError::InvalidValue {
                        key: "--host".to_string(),
                        message: "missing address argument".to_string(),
                    });
                }
                server_args.host = Some(args[i].clone());
            }
            "--port" => {
                i += 1;
                if i >= args.len() {
                    return Err(ConfigError::InvalidValue {
                        key: "--port".to_string(),
                        message: "missing port argument".to_string(),
                    });
                }
                server_args.port = Some(parse_u16(&args[i], "--port")?);
            }
            _ => {
                return Err(ConfigError::InvalidLine {
                    line: i + 1,
                    content: args[i].clone(),
                });
            }
        }
        i += 1;
    }

    Ok(server_args)
}

/// Loads the final server configuration by merging file config with CLI overrides.
pub fn load_config(args: &ServerArgs) -> Result<ServerConfig, ConfigError> {
    let mut config = match &args.config_path {
        Some(path) => parse_config_file(path)?,
        None => ServerConfig::default(),
    };

    if let Some(host) = &args.host {
        config.host = host.clone();
    }
    if let Some(port) = args.port {
        config.port = port;
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::{ServerConfig, load_config, parse_args, parse_config_string};
    use std::path::PathBuf;

    #[test]
    fn default_config_has_sane_values() {
        let config = ServerConfig::default();
        assert_eq!(config.port, 25565);
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.max_players, 20);
        assert!(!config.online_mode);
        assert_eq!(config.view_distance, 10);
        assert_eq!(config.keep_alive_timeout_secs, 30);
    }

    #[test]
    fn parse_config_string_with_valid_values() -> Result<(), Box<dyn std::error::Error>> {
        let content = r#"
# Server properties
server-port=25566
max-players=50
online-mode=true
view-distance=12
motd=My Server
level-name=myworld
hakoniwa-size=small
"#;
        let config = parse_config_string(content)?;
        assert_eq!(config.port, 25566);
        assert_eq!(config.max_players, 50);
        assert!(config.online_mode);
        assert_eq!(config.view_distance, 12);
        assert_eq!(config.motd, "My Server");
        assert_eq!(config.level_name, "myworld");
        assert_eq!(config.hakoniwa_size, crate::hakoniwa::MapSize::Small);
        Ok(())
    }

    #[test]
    fn parse_config_string_with_invalid_line() {
        let content = "this is not valid";
        let result = parse_config_string(content);
        assert!(result.is_err());
    }

    #[test]
    fn parse_config_string_with_invalid_bool() {
        let content = "online-mode=maybe";
        let result = parse_config_string(content);
        assert!(result.is_err());
    }

    #[test]
    fn parse_config_string_with_invalid_port() {
        let content = "server-port=99999";
        let result = parse_config_string(content);
        assert!(result.is_err());
    }

    #[test]
    fn parse_config_string_ignores_unknown_keys() -> Result<(), Box<dyn std::error::Error>> {
        let content = "unknown-key=value\nserver-port=25567";
        let config = parse_config_string(content)?;
        assert_eq!(config.port, 25567);
        Ok(())
    }

    #[test]
    fn parse_config_string_empty_uses_defaults() -> Result<(), Box<dyn std::error::Error>> {
        let config = parse_config_string("")?;
        assert_eq!(config.port, 25565);
        Ok(())
    }

    #[test]
    fn parse_args_with_config_path() -> Result<(), Box<dyn std::error::Error>> {
        let args = vec!["--config".to_string(), "server.properties".to_string()];
        let parsed = parse_args(&args)?;
        assert_eq!(parsed.config_path, Some(PathBuf::from("server.properties")));
        Ok(())
    }

    #[test]
    fn parse_args_with_host_and_port() -> Result<(), Box<dyn std::error::Error>> {
        let args = vec![
            "--host".to_string(),
            "127.0.0.1".to_string(),
            "--port".to_string(),
            "25568".to_string(),
        ];
        let parsed = parse_args(&args)?;
        assert_eq!(parsed.host, Some("127.0.0.1".to_string()));
        assert_eq!(parsed.port, Some(25568));
        Ok(())
    }

    #[test]
    fn parse_args_with_unknown_flag_is_error() {
        let args = vec!["--unknown".to_string()];
        let result = parse_args(&args);
        assert!(result.is_err());
    }

    #[test]
    fn parse_args_with_missing_value_is_error() {
        let args = vec!["--port".to_string()];
        let result = parse_args(&args);
        assert!(result.is_err());
    }

    #[test]
    fn load_config_with_overrides() -> Result<(), Box<dyn std::error::Error>> {
        let args = super::ServerArgs {
            config_path: None,
            host: Some("127.0.0.1".to_string()),
            port: Some(25569),
        };
        let config = load_config(&args)?;
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 25569);
        Ok(())
    }
}

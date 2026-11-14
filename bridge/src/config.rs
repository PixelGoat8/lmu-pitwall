use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "lmu-bridge", about = "Le Mans Ultimate Data Bridge")]
pub struct Config {
    /// WebSocket server port (overrides config.json when provided)
    #[arg(long)]
    pub ws_port: Option<u16>,

    /// Telemetry broadcast rate in FPS
    #[arg(long, default_value_t = 20)]
    pub telemetry_fps: u32,

    /// Scoring broadcast rate in FPS
    #[arg(long, default_value_t = 20)]
    pub scoring_fps: u32,

    /// Enable LMU REST API polling (Phase 2)
    #[arg(long, default_value_t = false)]
    pub enable_rest_api: bool,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info")]
    pub log_level: String,

    /// Do not open the browser automatically on startup
    #[arg(long, default_value_t = false)]
    pub no_browser: bool,

    /// Access token for remote clients (overrides LMU_PITWALL_TOKEN and config.json)
    #[arg(long)]
    pub token: Option<String>,

    /// Run as the engineer's hub: follow several drivers' bridges instead of
    /// reading the game on this PC (default port 9100)
    #[arg(long, default_value_t = false)]
    pub hub: bool,

    /// Record everything the bridge broadcasts to this file (.pwrec)
    #[arg(long, value_name = "FILE")]
    pub record: Option<std::path::PathBuf>,

    /// Serve a recorded session instead of reading the game (no LMU needed)
    #[arg(long, value_name = "FILE")]
    pub replay: Option<std::path::PathBuf>,

    /// Playback speed for --replay (2 = twice as fast)
    #[arg(long, default_value_t = 1.0)]
    pub replay_speed: f64,

    /// With --replay: stop at the end instead of starting over
    #[arg(long, default_value_t = false)]
    pub replay_once: bool,

    /// Never exit on idle (default: exit after 10 minutes with no client and no game)
    #[arg(long, default_value_t = false)]
    pub keep_alive: bool,
}

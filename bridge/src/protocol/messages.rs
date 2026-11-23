use serde::{Deserialize, Serialize};

fn default_true() -> bool { true }

/// WebSocket message protocol — MessagePack serialized

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TireData {
    pub temp_inner: f64,
    pub temp_mid: f64,
    pub temp_outer: f64,
    pub carcass_temp: f64,
    pub pressure: f64,
    pub wear: f64,
    pub brake_temp: f64,
    /// Punctured (`mFlat`).
    #[serde(default)]
    pub flat: bool,
    /// Wheel gone (`mDetached`).
    #[serde(default)]
    pub detached: bool,
    /// The compound's optimal temperature, °C; `None` if the game gives none.
    #[serde(default)]
    pub optimal_temp: Option<f64>,
    /// What the wheel is on: 0=dry,1=wet,2=grass,3=dirt,4=gravel,5=kerb,6=special.
    #[serde(default)]
    pub surface: u8,
    /// Lock-ups since this tyre was fitted (see `crate::tyres`): how many,
    /// total seconds locked, longest single one, and whether locked right now.
    /// The basis for the dashboard's flat-spot hint; LMU publishes no flat
    /// spots as such.
    #[serde(default)]
    pub lockups: u32,
    #[serde(default)]
    pub lockup_total_s: f64,
    #[serde(default)]
    pub lockup_longest_s: f64,
    #[serde(default)]
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeatherForecastNode {
    pub sky_type: i32,      // 0=clear … 10=heavy rain
    pub temperature: f64,   // Celsius
    pub rain_chance: f64,   // 0.0–1.0
    // Optional fields parsed from REST API — not yet displayed in the widget
    pub humidity: Option<f64>,        // relative humidity 0–100 %
    pub wind_direction: Option<i32>,  // 0=N, 1=NE, 2=E, 3=SE, 4=S, 5=SW, 6=W, 7=NW
    pub wind_speed: Option<f64>,      // wind speed m/s
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeatherData {
    pub air_temp: f64,
    pub track_temp: f64,
    pub rain_intensity: f64,
    pub dark_cloud: f64,        // cloud coverage 0.0–1.0 (mDarkCloud) — unreliable in LMU
    /// Cloud coverage 0.0–1.0 derived from REST API sky_type; more reliable than dark_cloud.
    pub cloudiness: f64,
    pub avg_path_wetness: f64,  // actual water on racing line 0.0–1.0 (mAvgPathWetness)
    pub min_path_wetness: f64,  // minimum wetness on racing line (mMinPathWetness)
    pub max_path_wetness: f64,  // maximum wetness on racing line (mMaxPathWetness)
    /// Horizontal wind speed from live mWind vector (m/s). None when < 0.1 m/s (calm).
    pub wind_speed_live: Option<f64>,
    /// Wind direction relative to the player car's nose, degrees.
    /// 0° = wind blows toward the front (headwind), ±180° = tailwind, ±90° = crosswind.
    /// Positive = wind from the right side. None when calm or player vehicle not found.
    /// NOTE: mWind is a velocity vector (air flows TO this direction). Verify empirically
    /// and flip sign / add 180° in build_session_info if the convention appears inverted.
    pub wind_rel_deg: Option<f64>,
    /// 5 forecast nodes: 0%, 25%, 50%, 75%, 100% of session length.
    /// Empty when LMU REST API is unavailable.
    pub forecast: Vec<WeatherForecastNode>,
}

/// Tire snapshot for one wheel at S/F line crossing (telemetry at lap completion).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WheelSnapshot {
    pub wear: f64,                  // 0.0 (new) – 1.0 (destroyed)
    pub surface_temp: [f64; 3],     // inner/mid/outer surface temp (Celsius)
    pub carcass_temp: f64,          // carcass temperature (Celsius)
    pub inner_layer_temp: [f64; 3], // inner layer temperatures (Celsius)
    pub pressure: f64,              // tire pressure (kPa)
    pub flat: bool,
    pub detached: bool,
    pub compound_index: u8, // per-wheel compound index (mCompoundIndex)
}

/// Snapshot of one driver's state at S/F line crossing (lap completion).
/// Also sent for all drivers on client connect as initial state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverLapSnapshot {
    pub id: i32,
    pub driver_name: String,
    pub vehicle_name: String,
    pub class_name: String,
    pub position: i32,
    pub total_laps: i32,
    pub last_lap_time: f64,
    pub best_lap_time: f64,
    pub last_sector1: f64,
    pub last_sector2: f64,
    pub last_sector3: f64,  // -1.0 if invalid
    pub gap_to_leader: f64, // seconds behind leader
    pub laps_behind_leader: i32,
    pub gap_ahead: f64, // seconds to car ahead
    pub num_pitstops: i32,
    pub in_pits: bool,
    pub finish_status: i8,   // 0=none,1=finished,2=DNF,3=DQ
    pub fuel_remaining: f64, // litres; -1.0 if no telemetry
    pub fuel_capacity: f64,
    pub tire_compound_front: u8,
    pub tire_compound_rear: u8,
    pub tire_compound_front_name: String,
    pub tire_compound_rear_name: String,
    pub wheels: [WheelSnapshot; 4], // FL, FR, RL, RR
    pub lap_start_et: f64,
    pub speed_ms: f64,          // speed at S/F crossing (m/s)
    pub has_telemetry: bool,    // false = scoring data only, no per-wheel data
    pub dent_severity: [u8; 8], // 0=none, 1=dented, 2=very dented (8 body zones)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VehicleScoring {
    pub id: i32,
    pub driver_name: String,
    pub team_name: String,
    pub vehicle_class: String,
    pub position: i32,
    pub lap_dist: f64,
    pub total_laps: i32,
    pub best_lap_time: f64,
    pub last_lap_time: f64,
    pub in_pits: bool,
    // Sector times (seconds; -1.0 = invalid/not yet completed)
    pub last_sector1: f64, // last S1 time
    pub last_sector2: f64, // last S1+S2 cumulative time
    pub cur_sector1: f64,  // current S1 time (if S1 already passed this lap)
    pub cur_sector2: f64,  // current S1+S2 cumulative time (if S2 already passed)
    pub best_sector1: f64, // personal best S1
    pub best_sector2: f64, // personal best S1+S2
    pub lap_start_et: f64, // session ET when this lap started
    // Car identification
    pub car_number: i32,  // slot/car ID (used as car number)
    pub car_name: String, // vehicle model name (e.g. "Porsche 963 LMDh")
    // Derived sector 3 times (calculated: lap_time - sector2_cumulative)
    pub last_sector3: f64, // last S3 time; -1.0 if invalid
    pub best_sector3: f64, // best S3 time; -1.0 if invalid
    // World position (from mPos — metres in rF2 world space). Read from the
    // telemetry buffer when that carries this car, from scoring otherwise —
    // see `position_of` in main.rs for why the two sources are not equivalent.
    pub pos_x: f64, // mPos.x — world X coordinate
    pub pos_y: f64, // mPos.y — world Y coordinate (up axis in rF2), i.e. elevation
    pub pos_z: f64, // mPos.z — world Z coordinate (forward axis in rF2)
    // Race gap (seconds / laps behind leader)
    pub time_behind_leader: f64, // mTimeBehindLeader (s); 0.0 for leader
    pub laps_behind_leader: i32, // mLapsBehindLeader; 0 = on lead lap
    // Virtual Energy (0.0 = no VE / not a hybrid; >0 = fraction 0.0–1.0)
    pub virtual_energy: f32,
    /// Who is at the wheel (`mControl`): -1 nobody, 0 the local player,
    /// 1 local AI (also the player's own car while parked in the garage),
    /// 2 a remote player, 3 replay. On the player's row, 0 means "this PC is
    /// driving the car right now" — what the hub uses to follow the driver.
    #[serde(default)]
    pub control: i8,
    /// Gap to the car one position ahead (`mTimeBehindNext`, seconds) and laps
    /// between them. Race sessions only; 0 for the leader.
    #[serde(default)]
    pub time_behind_next: f64,
    #[serde(default)]
    pub laps_behind_next: i32,
    /// Pit stops made (`mNumPitstops`).
    #[serde(default)]
    pub num_pitstops: i32,
    /// `mPitState`: 0 none, 1 request, 2 entering, 3 stopped, 4 exiting.
    #[serde(default)]
    pub pit_state: u8,
    /// Penalties still to serve (`mNumPenalties`).
    #[serde(default)]
    pub num_penalties: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    /// High frequency: ~30Hz
    TelemetryUpdate {
        speed_ms: f64,
        rpm: f64,
        max_rpm: f64,
        gear: i32,
        throttle: f64,
        brake: f64,
        clutch: f64,
        steering: f64,
        fuel: f64,
        fuel_capacity: f64,
        water_temp: f64,
        oil_temp: f64,
        tires: [TireData; 4],
        position: Vec3,
        velocity: Vec3,
        local_accel: Vec3,
        delta_best: f64,
        current_et: f64,
        lap_start_et: f64, // session ET when current lap started (for current lap calc)
        /// World heading in degrees, 0 = North, clockwise positive (derived from mOri[2]).
        heading_deg: f64,
        /// The car is followed from the engineer seat, not driven on this PC:
        /// its fuel, tyre and brake values are LMU's placeholders, not data.
        #[serde(default)]
        spectated: bool,
    },

    /// Medium frequency: ~5Hz
    ScoringUpdate {
        session_type: String,
        session_time: f64,
        num_vehicles: i32,
        vehicles: Vec<VehicleScoring>,
        player_vehicle_id: i32,
        /// `mInRealtime`: the local player is in the car, not at the monitor.
        #[serde(default)]
        in_realtime: bool,
        /// `mPlayerName`: who is playing on this PC.
        #[serde(default)]
        player_name: String,
        /// `mVehicleModel` of the player's car (e.g. "Peugeot 9X8"), empty
        /// without a player car. `car_name` in the rows is the team entry,
        /// which is not what results and the fuel calculator are keyed by.
        #[serde(default)]
        player_vehicle_model: String,
        /// This PC has a car of its own in the session (false in the
        /// engineer seat and as a spectator).
        #[serde(default = "default_true")]
        own_car: bool,
        /// The car name chosen to follow when there is no own car (see `crate::follow`).
        #[serde(default)]
        followed_car: Option<String>,
        /// `player_vehicle_id` is that followed car.
        #[serde(default)]
        followed: bool,
    },

    /// Low frequency: ~1Hz
    SessionInfo {
        track_name: String,
        track_length: f64,
        weather: WeatherData,
        session_laps: i32,
        session_minutes: f64,
        /// `mSessionTimeRemaining`, seconds; `None` when the game has none.
        #[serde(default)]
        time_remaining: Option<f64>,
    },

    /// Low frequency: ~5Hz — electronics / driver aids (native telemetry)
    ElectronicsUpdate {
        tc: u8,
        tc_max: u8,
        tc_cut: u8,
        tc_cut_max: u8,
        tc_slip: u8,
        tc_slip_max: u8,
        abs: u8,
        abs_max: u8,
        engine_map: u8,
        engine_map_max: u8,
        front_arb: u8,
        front_arb_max: u8,
        rear_arb: u8,
        rear_arb_max: u8,
        brake_bias: f64, // front bias percent
        regen: f32,      // kW
        brake_migration: u8,
        brake_migration_max: u8,
        battery_pct: f64,    // 0.0–1.0
        soc: f32,            // state of charge
        virtual_energy: f32, // virtual energy
        tc_active: bool,     // TC intervening right now
        abs_active: bool,    // ABS intervening right now
    },

    /// Vehicle status: damage + race flags — ~5Hz
    VehicleStatusUpdate {
        // --- Damage (player vehicle, from telemetry buffer) ---
        overheating: bool,          // engine overheating indicator
        any_detached: bool,         // any body part detached
        dent_severity: [u8; 8],     // 8 body locations: 0=none, 1=dented, 2=very dented
        last_impact_magnitude: f64, // magnitude of last collision
        last_impact_et: f64,        // session time of last collision
        tire_flat: [bool; 4],       // FL, FR, RL, RR — flat tyre
        tire_detached: [bool; 4],   // FL, FR, RL, RR — detached tyre
        // --- Wearables (from REST API /rest/garage/UIScreen/RepairAndRefuel) ---
        aero_damage: f64,            // aero body damage 0.0–1.0; -1.0 = unavailable
        brake_wear: [f64; 4],        // FL, FR, RL, RR brake wear 0.0–1.0; -1.0 = unavailable
        suspension_damage: [f64; 4], // FL, FR, RL, RR suspension damage 0.0–1.0; -1.0 = unavailable
        // --- Race flags (from scoring + rules buffers) ---
        yellow_flag_state: i32, // rF2 enum: -1=no scoring, 0=none, 1=pending, 2=pits closed, 3=pit lead lap, 4=pits open, 5=last lap, 6=resume, 7=race halt
        sector_flags: [i32; 3], // local yellow per sector (S1, S2, S3): 0=clear, >0=yellow
        start_light: u8,        // 0=off, 1-5=red lights, 6=green
        game_phase: u8,         // 0=garage, 5=green, 6=full-caution, 7=stopped, 8=over
        player_flag: u8,        // mFlag for player: SDK only uses 0=no flag (green), 6=blue
        individual_phase: u8,   // mIndividualPhase: 10=under yellow, 11=under blue (unused)
        player_under_yellow: bool, // mUnderYellow for player vehicle (FCY only)
        player_sector: i32,     // player's current sector: 1=S1, 2=S2, 0=S3, -1=unknown
        /// Field under caution. Derived from `mGamePhase == 6` (full course
        /// yellow) since the port to `LMU_Data`, which has no safety-car
        /// record; the plugin's Rules buffer used to supply it directly.
        safety_car_active: bool,
    },

    /// Event-based. Re-broadcast whenever `telemetry_valid` flips, not only on
    /// game connect/disconnect.
    ConnectionStatus {
        game_connected: bool,
        plugin_version: String,
        /// `false` when the wheel block failed its plausibility check — the
        /// shared-memory layout no longer matches what we parse, so tire and
        /// brake readings must not be trusted.
        telemetry_valid: bool,
        /// Human-readable reason, present only when `telemetry_valid` is false.
        telemetry_warning: Option<String>,
        /// `true` while a session is loaded and the game is writing to it.
        /// `false` in the menus — including after leaving a session, when the
        /// shared memory still holds that session's last frame. While `false`
        /// the bridge sends no telemetry or scoring, and clients must drop
        /// what they were showing.
        session_active: bool,
    },

    /// Sent to every client every 2 s, session or not. Lets the dashboard
    /// tell a quiet bridge from a dead link; carries nothing else.
    Heartbeat {
        /// Bridge wall clock, Unix milliseconds.
        server_time_ms: u64,
    },

    /// All-drivers lap snapshot — sent when any car crosses the S/F line,
    /// and once immediately on client connect as initial state.
    /// Contains the latest snapshot per driver (updated at each S/F crossing).
    AllDriversUpdate {
        session_type: String,
        session_time: f64,
        drivers: Vec<DriverLapSnapshot>,
    },

    // -----------------------------------------------------------------------
    // Post-race results responses
    // -----------------------------------------------------------------------
    /// Response to `PostRaceInit` — full session list + import stats.
    PostRaceSessions {
        sessions: Vec<PostRaceSessionMeta>,
        total_sessions: usize,
        new_imported: usize,
        files_found: usize,
        import_errors: usize,
    },

    /// Response to `PostRaceSessionDetail` — driver summaries (no per-lap data).
    PostRaceSessionDetail {
        session_id: i64,
        drivers: Vec<PostRaceDriverSummary>,
        /// Whether this session has any recorded events (incidents, penalties, …).
        has_events: bool,
    },

    /// Response to `PostRaceDriverLaps` — every lap for one driver.
    PostRaceDriverLaps {
        driver_id: i64,
        laps: Vec<PostRaceLapData>,
    },

    /// Response to `PostRaceEvents` — all events for a session.
    PostRaceEvents {
        session_id: i64,
        summary: PostRaceEventsSummary,
        driver_summaries: Vec<PostRaceDriverEventSummary>,
        events: Vec<PostRaceEvent>,
    },

    /// Response to `PostRaceCompare` — per-lap comparison across multiple drivers.
    PostRaceCompare {
        reference_driver_id: i64,
        laps: Vec<PostRaceComparedLap>,
    },

    /// Response to `PostRaceStintSummary` — aggregated stint data for one driver.
    PostRaceStintSummary {
        driver_id: i64,
        stints: Vec<PostRaceStintData>,
    },

    /// Sent when a post-race command fails (e.g. DB not yet initialized).
    PostRaceError { message: String },

    /// Response to `PostRaceFunFacts` — pre-formatted stat strings + detected player name.
    PostRaceFunFacts {
        facts: Vec<String>,
        player_name: Option<String>,
    },

    // -----------------------------------------------------------------------
    // Fuel Calculator responses
    // -----------------------------------------------------------------------
    /// Response to `FuelCalcInit` — available track/car combinations.
    FuelCalcOptions {
        options: crate::fuel_calculator::types::FuelCalcOptions,
    },

    /// Response to `FuelCalcCompute` — full fuel/VE calculation result.
    FuelCalcResult {
        result: crate::fuel_calculator::types::FuelCalcResult,
    },

    /// Sent when a fuel calculator command fails.
    FuelCalcError { message: String },

    // -----------------------------------------------------------------------
    // Version info
    // -----------------------------------------------------------------------
    /// Sent once on client connect (and after background check completes).
    VersionInfo {
        current_version: String,
        latest_version: String,
        download_url: String,
        update_available: bool,
    },

    // -----------------------------------------------------------------------
    // Strategy (see `crate::strategy`)
    // -----------------------------------------------------------------------
    /// The game's pit-stop data for the player's car, every 2 s in a session:
    /// fuel and VE in real units, pit work times, the game's stop estimate.
    PitInfo(crate::pit_api::PitInfo),

    /// The strategy plan, at every completed lap and every 2 s.
    StrategyUpdate(Box<crate::strategy::StrategyReport>),

    /// Race event log and recent lap times of every car (see `crate::race`).
    RaceTiming(Box<crate::race::RaceTiming>),

    /// AI engineer comments and the AI layer's status (see `crate::ai`).
    AiFeed(Box<crate::ai::AiFeed>),

    // -----------------------------------------------------------------------
    // Session history (see `crate::history`); replies to history_* commands
    // -----------------------------------------------------------------------
    HistorySessions { sessions: Vec<crate::history::store::SessionSummary> },
    HistorySession(Box<crate::history::store::SessionDetail>),
    HistoryCarLaps { session_id: i64, laps: Vec<crate::history::store::CarLapRow> },
    /// The strategy plan as it was at a lap (a `StrategyReport` as JSON text).
    HistoryStrategy { session_id: i64, lap: i64, json: Option<String> },
    HistoryError { message: String },

    /// The track outline stored on this Pitwall for a track (the dashboard's
    /// `TrackOutline` record as JSON text), `None` when there is none yet.
    TrackOutline { track: String, json: Option<String> },

    // -----------------------------------------------------------------------
    // Hub mode (see `crate::hub`)
    // -----------------------------------------------------------------------
    /// The bridges a hub is connected to and which one it is following.
    /// Sent every second and on every change. Only a hub sends it; its
    /// presence is how a dashboard knows it is talking to one.
    HubStatus {
        sources: Vec<HubSourceStatus>,
        /// Bridge whose stream is being relayed right now.
        active_id: Option<String>,
        /// Bridge pinned by the engineer; `None` = follow the driver.
        follow: Option<String>,
    },

    /// Stint history of the followed car, oldest first.
    HubStints {
        stints: Vec<HubStintInfo>,
    },

    HubError {
        message: String,
    },

    /// A command this client's role may not send (a guest changing the
    /// strategy, an engineer changing the hub's AI settings), or access that
    /// has just been taken away.
    Refused {
        message: String,
    },

    /// A hub's people and who is connected now, for the Access panel (the
    /// hub's own PC only: it carries the personal tokens).
    AccessList {
        people: Vec<PersonInfo>,
        online: Vec<OnlineInfo>,
    },

    AccessError {
        message: String,
    },

    /// A hub's teams, every second, on every team's stream. `current` is the
    /// team this client is watching.
    Teams {
        teams: Vec<TeamInfo>,
        current: String,
    },

    /// A message relayed as-is, without being decoded into one of the
    /// variants above: a frame from a recording, or (in hub mode) from another
    /// bridge. Serialized as the wrapped value itself, so on the wire it is
    /// indistinguishable from the message it carries — which may come from a
    /// bridge version that knows message types this one does not.
    #[serde(untagged)]
    Raw(rmpv::Value),
}

/// A person with a personal link to a hub.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonInfo {
    pub id: String,
    pub name: String,
    /// "engineer" or "guest".
    pub role: String,
    /// Teams a guest may watch.
    pub teams: Vec<String>,
    /// For the link the admin copies.
    pub token: String,
    /// When the person was added (Unix seconds).
    pub created: u64,
}

/// A dashboard connected right now.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnlineInfo {
    /// A person's name, "hub link" or "hub PC".
    pub name: String,
    pub role: String,
    /// The person in users.json; `None` for the hub link and the hub PC.
    pub user_id: Option<String>,
    /// The team the dashboard is watching.
    pub team: String,
    /// IP address it connects from.
    pub address: String,
    /// Seconds since it connected.
    pub connected_s: u64,
}

/// One team of a hub, as the team switcher and the sister-car marks need it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TeamInfo {
    pub id: String,
    pub name: String,
    pub color: String,
    /// Drivers' bridges in this team.
    pub sources: u32,
    /// The bridge being followed, by its label; `None` when nobody is.
    pub active: Option<String>,
    /// That bridge's PC is driving the car.
    pub driving: bool,
    /// The team's car as the game names it (e.g. "Iron Lynx #63"), and who
    /// is in it; empty until a bridge in a session is followed.
    pub car_name: String,
    pub driver: String,
    /// This client may watch the team. A guest also gets the teams it may
    /// not watch, reduced to name, colour and car, so the sister car is still
    /// marked; the switcher leaves them out.
    #[serde(default = "watchable_default")]
    pub watchable: bool,
}

fn watchable_default() -> bool {
    true
}

/// One bridge as a hub sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubSourceStatus {
    pub id: String,
    pub label: String,
    /// Address of the bridge, without the token.
    pub host: String,
    /// "connecting", "connected", "unauthorized" or "unreachable".
    pub link: String,
    pub game_connected: bool,
    pub session_active: bool,
    /// Who is playing on that PC.
    pub pc_player: String,
    /// Who the game says is driving that PC's car right now.
    pub driver_name: String,
    /// That PC's car as the game names it ("Iron Lynx #63"); empty outside a
    /// session. Drivers of one team should all show the same car.
    #[serde(default)]
    pub car_name: String,
    /// `mControl` of that car as that PC sees it; `None` if unknown.
    pub control: Option<i8>,
    /// That PC is the one driving (`control == 0`).
    pub driving: bool,
    pub in_pits: bool,
    /// Milliseconds since the bridge last sent anything; `None` if never.
    pub last_frame_age_ms: Option<u64>,
}

/// One stint in a hub's history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubStintInfo {
    pub number: u32,
    pub driver: String,
    /// Label of the bridge the data came from.
    pub source: String,
    /// Session time (s) at the start; at the end, or `None` while running.
    pub start_time: f64,
    pub end_time: Option<f64>,
    pub start_lap: i32,
    pub end_lap: i32,
    /// Laps completed in the stint.
    pub laps: u32,
    pub best_lap: Option<f64>,
    /// Average lap, out-lap excluded when the stint has three laps or more.
    pub avg_lap: Option<f64>,
    pub last_lap: Option<f64>,
    /// Litres. Only readings taken while that PC was driving.
    pub fuel_start: Option<f64>,
    pub fuel_end: Option<f64>,
    /// Virtual energy fraction 0–1; `None` for cars without it.
    pub ve_start: Option<f64>,
    pub ve_end: Option<f64>,
}

// ---------------------------------------------------------------------------
// Post-race response payload structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostRaceSessionMeta {
    pub id: i64,
    pub track_venue: Option<String>,
    pub track_course: Option<String>,
    pub track_event: Option<String>,
    pub date_time: Option<String>,
    pub session_type: String,
    pub game_version: Option<String>,
    pub race_laps: Option<u32>,
    pub driver_count: i64,
    pub total_laps: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostRaceDriverSummary {
    pub id: i64,
    pub name: String,
    pub car_type: Option<String>,
    pub car_class: Option<String>,
    pub car_number: Option<u32>,
    pub team_name: Option<String>,
    pub is_player: bool,
    pub position: Option<u32>,
    pub class_position: Option<u32>,
    pub best_lap_time: Option<f64>,
    pub total_laps: Option<u32>,
    pub pitstops: Option<u32>,
    pub finish_status: Option<String>,
    /// Gap to overall leader in seconds (None for leader or if data unavailable).
    /// Always positive. For qualifying: best_lap_time delta. For race: elapsed_time delta.
    pub gap_to_leader: Option<f64>,
    /// Number of full laps behind the leader (race only, None if on same lap or not a race).
    pub laps_behind: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostRaceLapData {
    pub lap_num: u32,
    pub position: Option<u32>,
    pub lap_time: Option<f64>,
    pub s1: Option<f64>,
    pub s2: Option<f64>,
    pub s3: Option<f64>,
    pub top_speed: Option<f64>,
    pub fuel_level: Option<f64>,
    pub fuel_used: Option<f64>,
    pub tw_fl: Option<f64>,
    pub tw_fr: Option<f64>,
    pub tw_rl: Option<f64>,
    pub tw_rr: Option<f64>,
    pub compound_fl: Option<String>,
    pub compound_fr: Option<String>,
    pub compound_rl: Option<String>,
    pub compound_rr: Option<String>,
    pub is_pit: bool,
    pub stint_number: u32,
    /// Virtual Energy level at end of lap (0.0–1.0). None for classes without VE.
    pub ve_level: Option<f64>,
    /// Virtual Energy consumed this lap. None for classes without VE.
    pub ve_used: Option<f64>,
    /// Incidents that occurred during this lap (mapped by elapsed_time range).
    pub incidents: Vec<PostRaceEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostRaceDriverLapEntry {
    pub driver_id: i64,
    pub lap_time: Option<f64>,
    /// Seconds relative to the first (reference) driver in the list.
    pub delta: Option<f64>,
    pub s1: Option<f64>,
    pub s2: Option<f64>,
    pub s3: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostRaceComparedLap {
    pub lap_num: u32,
    pub drivers: Vec<PostRaceDriverLapEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostRaceStintData {
    pub stint_number: u32,
    pub lap_count: usize,
    pub avg_pace: Option<f64>,
    pub best_lap: Option<f64>,
    pub worst_lap: Option<f64>,
    pub fuel_start: Option<f64>,
    pub fuel_end: Option<f64>,
    pub fuel_consumed: Option<f64>,
    pub tw_fl_start: Option<f64>,
    pub tw_fr_start: Option<f64>,
    pub tw_rl_start: Option<f64>,
    pub tw_rr_start: Option<f64>,
    pub tw_fl_end: Option<f64>,
    pub tw_fr_end: Option<f64>,
    pub tw_rl_end: Option<f64>,
    pub tw_rr_end: Option<f64>,
    pub compound: Option<String>,
    /// VE level at start of stint (first lap). None for classes without VE.
    pub ve_start: Option<f64>,
    /// VE level at end of stint (last lap). None for classes without VE.
    pub ve_end: Option<f64>,
    /// Total VE consumed during stint (sum of ve_used). None for classes without VE.
    pub ve_consumed: Option<f64>,
    /// Average VE consumed per lap. None for classes without VE.
    pub avg_ve_per_lap: Option<f64>,
}

/// A single incident, penalty, track-limit warning, or damage report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostRaceEvent {
    pub id: i64,
    pub event_type: String,
    pub elapsed_time: f64,
    /// Formatted as "M:SS".
    pub elapsed_time_formatted: String,
    pub driver_name: Option<String>,
    pub target_name: Option<String>,
    pub severity: Option<f64>,
    pub message: Option<String>,
}

/// Session-wide event counts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostRaceEventsSummary {
    pub total_incidents: i64,
    pub vehicle_contacts: i64,
    pub object_contacts: i64,
    pub penalties: i64,
    pub track_limit_warnings: i64,
    pub damage_reports: i64,
}

/// Per-driver aggregated event summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostRaceDriverEventSummary {
    pub driver_name: String,
    pub incidents_total: i64,
    pub incidents_vehicle: i64,
    pub incidents_object: i64,
    pub avg_severity: Option<f64>,
    pub max_severity: Option<f64>,
    pub penalties: i64,
    pub track_limit_warnings: i64,
    pub track_limit_points: Option<f64>,
}

// ---------------------------------------------------------------------------
// Client → Bridge commands (JSON text frames)
// ---------------------------------------------------------------------------

/// Commands sent from the browser dashboard to the bridge over WebSocket text frames.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ClientCommand {
    /// Trigger lazy DB init + delta import; respond with full session list.
    /// `results_path`: optional override for the LMU results folder (empty = use default).
    PostRaceInit { results_path: Option<String> },
    /// Fetch all drivers for a session (no lap detail).
    PostRaceSessionDetail { session_id: i64 },
    /// Fetch all laps for a single driver.
    PostRaceDriverLaps { driver_id: i64 },
    /// Lap-by-lap comparison for multiple drivers with deltas to the first.
    PostRaceCompare { driver_ids: Vec<i64> },
    /// Per-stint aggregated summary for a driver.
    PostRaceStintSummary { driver_id: i64 },
    /// Fetch all events (incidents, penalties, …) for a session.
    PostRaceEvents { session_id: i64 },
    /// Fetch aggregate stats for the fun-fact ticker.
    PostRaceFunFacts,

    // -----------------------------------------------------------------------
    // Strategy
    // -----------------------------------------------------------------------
    // -----------------------------------------------------------------------
    // Session history
    // -----------------------------------------------------------------------
    HistoryList,
    HistorySession { session_id: i64 },
    HistoryCarLaps { session_id: i64, car_ids: Vec<i64> },
    HistoryStrategy { session_id: i64, lap: i64 },
    /// Local only.
    HistoryDelete { session_id: i64 },

    /// The stored outline for a track, so every dashboard (tablet, hub, other
    /// PC) draws the map without recording a lap of its own.
    TrackOutlineGet { track: String },
    /// Keep an outline a dashboard has just recorded (its JSON text).
    TrackOutlineSave { track: String, json: String },
    /// Forget the stored outline, so the next lap records a new one.
    TrackOutlineDelete { track: String },

    /// Follow this car (its name as the game shows it) when the PC has no car
    /// of its own, e.g. in LMU's engineer seat. `null` stops following.
    FollowCar {
        #[serde(default)]
        car_name: Option<String>,
    },

    /// Replace the strategy settings (reserve, stint limit, tyre limit, …).
    StrategyConfig { config: crate::strategy::StrategyConfig },

    // -----------------------------------------------------------------------
    // AI comments
    // -----------------------------------------------------------------------
    /// Replace the AI settings (on/off, model, frequency, budget, …).
    AiConfig { config: crate::ai::AiConfig },
    /// Ask for a comment now, optionally with a question from the engineer.
    AiAsk {
        #[serde(default)]
        question: Option<String>,
    },

    // -----------------------------------------------------------------------
    // Hub commands (answered only by a Pitwall started with --hub)
    // -----------------------------------------------------------------------
    /// Add a driver's bridge from the link its dashboard shows.
    HubAddSource {
        link: String,
        #[serde(default)]
        label: Option<String>,
    },
    HubRemoveSource { id: String },
    /// Pin the hub to one bridge, or `id: null` to follow the driver.
    HubFollow {
        #[serde(default)]
        id: Option<String>,
    },
    /// Move a driver's bridge to another team. Local only, like adding one.
    HubMoveSource { source_id: String, team_id: String },
    /// Teams of a hub. Local only.
    TeamCreate {
        name: String,
        #[serde(default)]
        color: Option<String>,
    },
    TeamUpdate {
        id: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        color: Option<String>,
    },
    /// Only a team without drivers, and never the last one.
    TeamDelete { id: String },

    // -----------------------------------------------------------------------
    // People of a hub (its own PC only); each answered with AccessList
    // -----------------------------------------------------------------------
    AccessList,
    /// `role`: "engineer" or "guest"; `teams`: what a guest may watch.
    AccessAdd {
        name: String,
        role: String,
        #[serde(default)]
        teams: Vec<String>,
    },
    AccessUpdate {
        id: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        role: Option<String>,
        #[serde(default)]
        teams: Option<Vec<String>>,
    },
    /// A new link for the person: the old one stops working at once.
    AccessRegenerate { id: String },
    /// Take the person's access away, disconnecting them at once.
    AccessRemove { id: String },

    // -----------------------------------------------------------------------
    // Fuel Calculator commands
    // -----------------------------------------------------------------------
    /// Request available track/car combinations from the PostRace DB.
    FuelCalcInit,

    /// Compute fuel/VE requirements for a given track, car, and race distance.
    FuelCalcCompute {
        track_venue: String,
        /// Track layout/variant filter (e.g. "National Circuit"). `None` matches
        /// rows whose `track_course` is NULL. Disambiguates venues with multiple layouts.
        #[serde(default)]
        track_course: Option<String>,
        /// Matches `drivers.car_type` in the DB (e.g. "Porsche 963 LMDh").
        car_name: String,
        /// Race distance in laps. Provide this OR `race_minutes`, not both.
        race_laps: Option<u32>,
        /// Race distance in minutes. Laps are estimated from historical avg lap time.
        race_minutes: Option<f64>,
        /// If false (default), only include data from the current game version.
        #[serde(default)]
        include_all_versions: bool,
        /// If true (default), include Practice sessions in addition to Race sessions.
        #[serde(default = "default_true")]
        include_practice: bool,
        /// FuelMult filter override. `None` = auto (most recent session's FuelMult).
        #[serde(default)]
        fuel_mult: Option<f64>,
        /// Extra buffer laps added to recommended start fuel/VE. Default: 1.
        #[serde(default)]
        buffer_laps: Option<u32>,
    },
}

// TypeScript types matching the Rust bridge protocol (messages.rs)
// Enum is serialized with #[serde(tag = "type")] + rmp_serde::to_vec_named

export interface Vec3 {
  x: number
  y: number
  z: number
}

export interface TireData {
  temp_inner: number
  temp_mid: number
  temp_outer: number
  carcass_temp: number
  pressure: number
  wear: number
  brake_temp: number
  // The fields below are absent from bridges older than this fork's phase 3.
  flat?: boolean
  detached?: boolean
  optimal_temp?: number | null   // °C, the compound's optimal temperature
  surface?: number               // 0=dry,1=wet,2=grass,3=dirt,4=gravel,5=kerb,6=special
  // Lock-ups since this tyre was fitted. LMU publishes no flat spots; a long
  // lock-up is the best available hint that there is one.
  lockups?: number
  lockup_total_s?: number
  lockup_longest_s?: number
  locked?: boolean               // locked right now
}

export interface WeatherForecastNode {
  sky_type: number      // 0=clear … 10=heavy rain
  temperature: number   // Celsius
  rain_chance: number   // 0.0–1.0
  // Optional — parsed from REST API, not yet displayed in widget
  humidity?: number        // 0–100 %
  wind_direction?: number  // 0=N, 1=NE, 2=E, 3=SE, 4=S, 5=SW, 6=W, 7=NW
  wind_speed?: number      // m/s
}

export interface WeatherData {
  air_temp: number
  track_temp: number
  rain_intensity: number
  dark_cloud: number          // 0.0–1.0 cloud coverage (mDarkCloud)
  avg_path_wetness: number    // actual water on racing line 0.0–1.0 (mAvgPathWetness)
  min_path_wetness: number    // minimum wetness on racing line
  max_path_wetness: number    // maximum wetness on racing line
  cloudiness: number      // 0.0–1.0 derived from REST API sky_type (reliable)
  wind_speed_live?: number    // horizontal wind speed m/s from live mWind; absent when calm
  wind_rel_deg?: number       // wind angle relative to car nose: 0=headwind, ±180=tailwind, ±90=crosswind
  forecast: WeatherForecastNode[]
}

export interface VehicleScoring {
  id: number
  driver_name: string
  team_name: string
  vehicle_class: string
  position: number
  lap_dist: number
  total_laps: number
  best_lap_time: number
  last_lap_time: number
  in_pits: boolean
  // Sector times (-1.0 = not yet completed / invalid)
  last_sector1: number      // last S1 time
  last_sector2: number      // last S1+S2 cumulative
  cur_sector1: number       // current lap S1 (if already passed)
  cur_sector2: number       // current lap S1+S2 (if already passed)
  best_sector1: number      // personal best S1
  best_sector2: number      // personal best S1+S2
  lap_start_et: number      // session ET when this lap started
  // Car identification
  car_number: number        // slot/car ID used as car number
  car_name: string          // vehicle model name
  // Derived sector 3 times (-1.0 = invalid)
  last_sector3: number      // last S3 = last_lap - last_sector2
  best_sector3: number      // best S3 = best_lap - best_sector2
  // World position in rF2 coordinate space (metres). Sampled from the 100 Hz
  // telemetry buffer where the game publishes it there, from the 5 Hz scoring
  // block otherwise — the bridge decides per car and the two are interchangeable.
  pos_x: number             // mPos.x — world X coordinate
  pos_y: number             // mPos.y — world Y coordinate (up axis in rF2), i.e. elevation
  pos_z: number             // mPos.z — world Z coordinate (forward axis in rF2)
  // Race gap
  time_behind_leader: number  // mTimeBehindLeader (s); 0.0 for leader
  laps_behind_leader: number  // mLapsBehindLeader; 0 = on lead lap
  // Virtual Energy (0.0 = no VE / not a hybrid; >0 = fraction 0.0–1.0)
  virtual_energy: number
  // Absent from bridges older than this fork's phase 3.
  control?: number            // -1 nobody, 0 local player, 1 local AI / parked, 2 remote, 3 replay
  time_behind_next?: number   // s to the car one position ahead (race)
  laps_behind_next?: number
  num_pitstops?: number
  pit_state?: number          // 0 none, 1 request, 2 entering, 3 stopped, 4 exiting
  num_penalties?: number      // penalties still to serve
}

// ---------------------------------------------------------------------------
// All-drivers lap snapshot (bridge task: AllDriversUpdate)
// ---------------------------------------------------------------------------

export interface WheelSnapshot {
  wear: number                              // 0.0 (new) – 1.0 (destroyed)
  surface_temp: [number, number, number]    // inner/mid/outer surface temp (°C)
  carcass_temp: number                      // carcass temperature (°C)
  inner_layer_temp: [number, number, number] // inner-layer temps (°C)
  pressure: number                          // tire pressure (kPa)
  flat: boolean
  detached: boolean
  compound_index: number
}

export interface DriverLapSnapshot {
  id: number
  driver_name: string
  vehicle_name: string
  class_name: string
  position: number
  total_laps: number
  last_lap_time: number          // seconds; ≤0 = no completed lap
  best_lap_time: number          // seconds; ≤0 = none
  last_sector1: number           // S1 time (s); -1 = invalid
  last_sector2: number           // S1+S2 cumulative (s); -1 = invalid
  last_sector3: number           // S3 individual (s); -1 = invalid
  gap_to_leader: number          // seconds behind leader
  laps_behind_leader: number     // 0 = on lead lap
  gap_ahead: number              // seconds to car ahead
  num_pitstops: number
  in_pits: boolean
  finish_status: number          // 0=none, 1=finished, 2=dnf, 3=dq
  fuel_remaining: number         // litres; -1 = no telemetry
  fuel_capacity: number
  tire_compound_front: number
  tire_compound_rear: number
  tire_compound_front_name: string
  tire_compound_rear_name: string
  wheels: [WheelSnapshot, WheelSnapshot, WheelSnapshot, WheelSnapshot] // FL, FR, RL, RR
  lap_start_et: number
  speed_ms: number
  has_telemetry: boolean
  dent_severity: [number, number, number, number, number, number, number, number]  // 8 body zones: 0=none, 1=dented, 2=very dented
}

export interface AllDriversUpdate {
  type: 'AllDriversUpdate'
  session_type: string
  session_time: number
  drivers: DriverLapSnapshot[]
}

export interface TelemetryUpdate {
  type: 'TelemetryUpdate'
  speed_ms: number
  rpm: number
  max_rpm: number
  gear: number
  throttle: number
  brake: number
  clutch: number
  steering: number
  fuel: number
  fuel_capacity: number
  water_temp: number
  oil_temp: number
  tires: [TireData, TireData, TireData, TireData]  // FL, FR, RL, RR
  position: Vec3
  velocity: Vec3
  local_accel: Vec3
  delta_best: number
  current_et: number
  lap_start_et: number     // session ET when current lap started
  // Fuel strategy (computed by bridge FuelTracker)
  heading_deg: number             // world heading in degrees, 0=N, clockwise (from mOri[2])
  spectated?: boolean             // car followed from the engineer seat: fuel/tyres are placeholders
}

export interface ScoringUpdate {
  type: 'ScoringUpdate'
  session_type: string
  session_time: number
  num_vehicles: number
  vehicles: VehicleScoring[]
  player_vehicle_id: number
  in_realtime?: boolean           // the local player is in the car
  player_name?: string            // who is playing on the bridge's PC
  player_vehicle_model?: string   // model of the player's car, e.g. "Peugeot 9X8"
  own_car?: boolean               // false in the engineer seat / as a spectator
  followed_car?: string | null    // car name chosen to follow when there is no own car
  followed?: boolean              // player_vehicle_id is that followed car
}

export interface SessionInfo {
  type: 'SessionInfo'
  track_name: string
  track_length: number
  weather: WeatherData
  session_laps: number
  session_minutes: number
  time_remaining?: number | null
}

export interface ElectronicsUpdate {
  type: 'ElectronicsUpdate'
  tc: number
  tc_max: number
  tc_cut: number
  tc_cut_max: number
  tc_slip: number
  tc_slip_max: number
  abs: number
  abs_max: number
  engine_map: number
  engine_map_max: number
  front_arb: number
  front_arb_max: number
  rear_arb: number
  rear_arb_max: number
  brake_bias: number           // front bias percent, e.g. 56.0
  regen: number                // kW
  brake_migration: number
  brake_migration_max: number
  battery_pct: number          // 0.0–1.0
  soc: number                  // state of charge
  virtual_energy: number       // virtual energy
  tc_active: boolean           // TC intervening right now
  abs_active: boolean          // ABS intervening right now
}

export interface VehicleStatusUpdate {
  type: 'VehicleStatusUpdate'
  // Damage (player vehicle, from telemetry)
  overheating: boolean
  any_detached: boolean
  dent_severity: [number, number, number, number, number, number, number, number] // 8 body zones
  last_impact_magnitude: number
  last_impact_et: number
  tire_flat: [boolean, boolean, boolean, boolean]      // FL, FR, RL, RR
  tire_detached: [boolean, boolean, boolean, boolean]  // FL, FR, RL, RR
  // Wearables (from REST API) — -1 = unavailable
  aero_damage: number                                  // 0.0–1.0
  brake_wear: [number, number, number, number]         // FL, FR, RL, RR 0.0–1.0
  suspension_damage: [number, number, number, number]  // FL, FR, RL, RR 0.0–1.0
  // Race flags (from scoring + rules)
  yellow_flag_state: number  // -1=no scoring, 0=none, 1=pending, 2=pits closed, 3=pit lead lap, 4=pits open, 5=last lap, 6=resume, 7=race halt
  sector_flags: [number, number, number]  // S1, S2, S3 local yellow
  start_light: number        // 0=off, 1-5=red lights, 6=green
  game_phase: number         // 0=garage, 5=green, 6=full-caution, 7=stopped, 8=over
  player_flag: number        // mFlag: SDK only uses 0=no flag (green), 6=blue
  individual_phase: number   // mIndividualPhase: 10=under yellow — authoritative per-vehicle yellow indicator
  player_under_yellow: boolean   // mUnderYellow: FCY only (crossed S/F under FCY)
  player_sector: number          // 1=S1, 2=S2, 0=S3, -1=unknown
  safety_car_active: boolean
}

export interface ConnectionStatus {
  type: 'ConnectionStatus'
  game_connected: boolean
  plugin_version: string
  // false when the shared-memory layout no longer matches what the bridge
  // parses — tire and brake readings are unreliable in that case
  telemetry_valid: boolean
  telemetry_warning: string | null
  // false in the menus, including after leaving a session: the bridge then
  // sends no telemetry and what is on screen must be dropped.
  // Optional: bridges older than this field do not send it.
  session_active?: boolean
}

// Sent every 2 s regardless of the game. Its arrival is the point: the
// connection watchdog treats silence as a dead link.
export interface Heartbeat {
  type: 'Heartbeat'
  server_time_ms: number
}

export interface VersionInfo {
  type: 'VersionInfo'
  current_version: string
  latest_version: string
  download_url: string
  update_available: boolean
}

// --- Hub mode: sent only by a Pitwall started with --hub ---------------------

export interface HubSourceStatus {
  id: string
  label: string
  host: string
  link: 'connecting' | 'connected' | 'unauthorized' | 'unreachable'
  game_connected: boolean
  session_active: boolean
  pc_player: string
  driver_name: string
  control: number | null
  driving: boolean
  in_pits: boolean
  last_frame_age_ms: number | null
}

export interface HubStatus {
  type: 'HubStatus'
  sources: HubSourceStatus[]
  active_id: string | null   // bridge whose stream is on screen
  follow: string | null      // bridge pinned by the engineer; null = follow the driver
}

export interface HubStintInfo {
  number: number
  driver: string
  source: string
  start_time: number
  end_time: number | null    // null while running
  start_lap: number
  end_lap: number
  laps: number
  best_lap: number | null
  avg_lap: number | null
  last_lap: number | null
  fuel_start: number | null
  fuel_end: number | null
  ve_start: number | null
  ve_end: number | null
}

export interface HubStints {
  type: 'HubStints'
  stints: HubStintInfo[]
}

export interface HubError {
  type: 'HubError'
  message: string
}

import type { PitInfoMsg, StrategyUpdate } from './strategy'
import type { RaceTiming } from './race'
import type { AiFeed } from './ai'
import type { HistoryMessage } from './history'

/** The track outline stored on the bridge, as JSON text (see utils/trackOutline). */
export interface TrackOutlineMsg {
  type: 'TrackOutline'
  track: string
  json: string | null
}

export type ServerMessage =
  | TelemetryUpdate
  | ScoringUpdate
  | SessionInfo
  | ElectronicsUpdate
  | VehicleStatusUpdate
  | ConnectionStatus
  | Heartbeat
  | AllDriversUpdate
  | VersionInfo
  | HubStatus
  | HubStints
  | HubError
  | StrategyUpdate
  | PitInfoMsg
  | RaceTiming
  | AiFeed
  | HistoryMessage
  | TrackOutlineMsg

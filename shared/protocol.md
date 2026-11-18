# LMU Dashboard — WebSocket Protocol

## Transport

- **Protocol:** WebSocket (`ws://HOST:9000`)
- **Format:** MessagePack (binary, compact)
- **Debug mode:** JSON via query param `?format=json`

## Access

The bridge sorts callers by where the TCP connection comes from:

| Caller | Token | May do |
|---|---|---|
| **Local**: loopback (a browser on the racing PC) | not needed | everything |
| **Remote**: any other address (LAN, Tailscale) | required | receive data, send read-only commands |

- WebSocket: `ws://HOST:9000/?token=TOKEN`. Without a valid token the
  handshake is refused with HTTP 401.
- HTTP API: `?token=TOKEN` or `Authorization: Bearer TOKEN`.
- Local-only WebSocket commands: `history_delete`, and `post_race_init` with
  a custom `results_path`. A remote client gets `HistoryError` or
  `PostRaceError` back.
- The token is generated on first run and stored in
  `%APPDATA%\LMUPitwall\config.json`; `--token` and `LMU_PITWALL_TOKEN` override it.

### HTTP API

| Endpoint | Access | Returns |
|---|---|---|
| `GET /` and static assets | anyone | the dashboard |
| `GET /api/version` | anyone | `{version}` |
| `GET /api/auth-check` | anyone | 200 `{ok, local}` or 401. Lets a client tell a refused token from an unreachable bridge |
| `GET /api/network-info` | local or remote | `{ip, port, local}`; local callers also get `tailscale_ip` and `token` |
| `POST /api/shutdown` | local, no foreign `Origin` | exits the bridge |
| `POST /api/set-port` | local, no foreign `Origin` | saves the port and restarts |

## Message Types

All messages are tagged with a `type` field.

### `TelemetryUpdate` (~30Hz)

High-frequency per-frame telemetry data.

| Field | Type | Description |
|-------|------|-------------|
| `speed_ms` | f64 | Speed in m/s |
| `rpm` | f64 | Engine RPM |
| `max_rpm` | f64 | Rev limiter RPM |
| `gear` | i32 | -1=Reverse, 0=Neutral, 1-8 |
| `throttle` | f64 | 0.0–1.0 |
| `brake` | f64 | 0.0–1.0 |
| `clutch` | f64 | 0.0–1.0 |
| `steering` | f64 | -1.0 to +1.0 |
| `fuel` | f64 | Liters remaining |
| `fuel_capacity` | f64 | Tank capacity in liters |
| `water_temp` | f64 | °C |
| `oil_temp` | f64 | °C |
| `tires` | TireData[4] | FL, FR, RL, RR |
| `delta_best` | f64 | Delta to best lap (seconds) |

### TireData

| Field | Type | Description |
|-------|------|-------------|
| `temp_inner` | f64 | Inner temperature °C |
| `temp_mid` | f64 | Middle temperature °C |
| `temp_outer` | f64 | Outer temperature °C |
| `pressure` | f64 | kPa |
| `wear` | f64 | 0.0–1.0 (1.0 = new) |
| `brake_temp` | f64 | °C |

### `ScoringUpdate` (~20Hz)

Session and standings data.

Most of it comes from LMU's scoring block, which the game rewrites at 5 Hz — so
those fields repeat between changes no matter how often this message is sent.

`pos_x` / `pos_y` / `pos_z` are the exception. They are read from the telemetry
rows, which tick at 100 Hz, so a position is fresh on every send. A car whose
telemetry row is missing, zeroed, or more than 150 m from where scoring puts it
falls back to the scoring position for that field; consumers cannot tell which
source a given car used, and do not need to — the two are the same quantity.

### `SessionInfo` (~1Hz)

Track info, weather, session type/duration.

### `ConnectionStatus` (event-based)

Sent when LMU connects or disconnects, and whenever `telemetry_valid` changes.

| Field | Type | Notes |
|---|---|---|
| `game_connected` | bool | |
| `plugin_version` | string | LMU's build number as `1.4.0.0`; `unknown` if unreadable. Named for the plugin era, when it carried a third-party DLL's version |
| `telemetry_valid` | bool | `false` = shared-memory layout drift detected |
| `telemetry_warning` | string \| null | reason, only set when `telemetry_valid` is `false` |
| `session_active` | bool | `true` while a session is loaded and live |

`session_active` is `false` in the menus, **including after leaving a
session**: LMU keeps the last session's cars in shared memory, and the bridge
tells a finished session from a live one by the `SME_UNLOAD` event. While it is
`false` the bridge sends no `TelemetryUpdate` or `ScoringUpdate`, and clients
must clear what they were showing.

Sent on connect in every case, so a client always knows the state: before the
game starts it reads `game_connected: false`.

`telemetry_valid` goes `false` when the per-wheel block fails a plausibility
check, which means a game update moved the wheel block inside
`rF2VehicleTelemetry`. Tire and brake readings are garbage in that state.

Consumers must treat the tire block in `Telemetry` as unusable while
`telemetry_valid` is `false` — the bridge keeps sending it rather than dropping
fields, so the flag is the only signal. In this repo: `TireMonitor` greys the
cells out.

### `Heartbeat` (every 2 s)

| Field | Type | Notes |
|---|---|---|
| `server_time_ms` | u64 | bridge wall clock, Unix milliseconds |

Sent to every client whether or not the game is running, together with a
WebSocket ping. Outside a session the bridge has nothing else to say, so this
is what lets a client tell a quiet bridge from a dead link: the dashboard
drops a socket that has been silent for 7 s. In the other direction, the
bridge drops a client that has sent nothing (not even a pong) for 20 s.

## Additions in this fork's phase 3

All new fields are optional on input: a message without them still parses.

### `TelemetryUpdate.tires[]`

| Field | Type | Notes |
|---|---|---|
| `flat`, `detached` | bool | puncture, wheel gone |
| `optimal_temp` | f64 \| null | the compound's optimal temperature, °C |
| `surface` | u8 | 0 dry, 1 wet, 2 grass, 3 dirt, 4 gravel, 5 kerb, 6 special |
| `lockups` | u32 | lock-ups since this tyre was fitted |
| `lockup_total_s`, `lockup_longest_s` | f64 | seconds locked in total, and in the longest single lock-up |
| `locked` | bool | locked right now |

LMU publishes no flat spots. The lock-up tally (wheel far slower than the car
under braking, for at least 80 ms) is what the dashboard's "flat spot?" hint is
inferred from. It resets when a new tyre is fitted.

### `ScoringUpdate`

| Field | Type | Notes |
|---|---|---|
| `in_realtime` | bool | the local player is in the car |
| `player_name` | string | who is playing on the bridge's PC |
| `player_vehicle_model` | string | model of the player's car, e.g. "Peugeot 9X8" |
| `vehicles[].control` | i8 | `mControl`: -1 nobody, 0 local player, 1 local AI (also the player's car parked in the garage), 2 remote, 3 replay |
| `vehicles[].time_behind_next`, `laps_behind_next` | f64, i32 | gap to the car one position ahead |
| `vehicles[].num_pitstops`, `pit_state` | i32, u8 | stops made; 0 none, 1 request, 2 entering, 3 stopped, 4 exiting |

### Hub mode (`--hub`)

A hub is an ordinary remote client of each driver's bridge and an ordinary
server to its own dashboards. It relays the messages of the bridge it follows
unchanged, and adds:

**`HubStatus`** (every second and on change): `sources[]` with `id`, `label`,
`host`, `link` (`connecting`, `connected`, `unauthorized`, `unreachable`),
`game_connected`, `session_active`, `pc_player`, `driver_name`, `control`,
`driving`, `in_pits`, `last_frame_age_ms`; `active_id` (bridge on screen);
`follow` (pinned bridge, or null to follow the driver).

**`HubStints`** (on change and every 5 s): `stints[]` with `number`, `driver`,
`source`, `start_time`, `end_time` (null while running), `start_lap`,
`end_lap`, `laps`, `best_lap`, `avg_lap`, `last_lap`, `fuel_start`, `fuel_end`,
`ve_start`, `ve_end`.

**`HubError`**: `message`.

Commands: `hub_add_source {link, label?}`, `hub_remove_source {id}`,
`hub_follow {id | null}`. A bridge that is not a hub answers them with
`HubError`.

Which bridge is followed: a pinned one if linked; else one that is driving
(`control == 0` on its own car), keeping the current one if it still is; else
the current one while it has a session; else any with a session.

### Recordings (`.pwrec`)

One JSON object per line: `{"t": <ms since start>, "m": "<base64 MessagePack>"}`
as written by `--record`, or `{"t": …, "j": {…message as JSON…}}` for
hand-made files. `--replay` accepts both; lines starting with `#` are ignored.

## Additions in this fork's phase 4

### `SessionInfo.time_remaining`

f64 | null: `mSessionTimeRemaining` in seconds.

### `PitInfo` (every 2 s in a session)

The game's pit data for the player's car, from `/rest/garage/UIScreen/RepairAndRefuel`
and `/rest/strategy/pitstop-estimate`; relayed by a hub.

| Field | Notes |
|---|---|
| `fuel`, `fuel_capacity` | litres |
| `ve` | fraction 0–1, `null` for cars without virtual energy |
| `times` | `fuel_fill_rate` (L/s), `fuel_insert_s`, `fuel_concurrent`, `ve_fill_rate` (fraction/s), `ve_insert_s`, `ve_concurrent`, `four_tyres_s`, `two_tyres_s`, `tyres_concurrent`, `driver_change_s`, `driver_concurrent` |
| `estimate` | the game's stationary-time estimate for the menu as set: `total`, `fuel`, `ve`, `tires`, `driver_swap`, `damage`, `penalties` |
| `menu_ve`, `menu_fuel` | the pit menu's VE setting (fraction) and fuel line text |
| `recommended_fuel`, `recommended_ve` | the game's recommendation, as sent |

### `StrategyUpdate` (every completed lap and every 2 s)

Produced by the strategy engine of the bridge or hub the client is connected
to. Main fields: `horizon` (`laps_to_go`, `basis`, `time_remaining_s`), `pace`
(`ref_lap_s`, `clean_laps`, `stat`), `fuel` and `ve` (`now`, `capacity`,
`per_lap` stats, `planning_per_lap`, `reserve`, `laps_left`,
`needed_to_finish`, `to_add`, `stint_laps_max`, `stops`; VE values are
fractions), `plan` (`stops`, `limiting`, `stint_laps_max`, `range_laps`,
`window`), `what_if[]` (`in_laps`, `pit_lap`, `reachable`, `fuel_add`,
`ve_add`, `stops_after`, `stationary_s`, `stop_cost_s`), `tyres`,
`tyre_choice`, `stint`, `pit_loss` (`lane_loss_s`, `source`, `samples`,
`game_estimate`), `weather`, `notes[]`, `config`. See
`bridge/src/strategy/mod.rs` for the exact shape. Figures that cannot be
computed are `null`; `notes` says why.

### Command `strategy_config`

`{"command":"strategy_config","config":{"reserve_laps":0.5,"max_stint_minutes":null,"tyre_wear_limit":0.3,"change_tyres":false,"pit_lane_loss_s":null}}`

Replaces the settings; the engine answers with a `StrategyUpdate` to every
client. Allowed from remote clients.

## Additions in this fork's phases 5 and 6

### `VehicleScoring.num_penalties`

i32: penalties still to serve (`mNumPenalties`).

### `RaceTiming` (on change, at most 1/s, and every 5 s)

Built from the stream by the bridge or hub the client is connected to.

| Field | Notes |
|---|---|
| `events[]` | `t` (session s), `lap` (laps completed by our car), `kind` (`fcy`, `fcy_end`, `pits`, `start`, `finish`, `pit_in`, `pit_out`, `penalty`, `driver`, `position`, `impact`, `blue_flag`, `best_lap`, `class_best`), `severity` (`info`, `warn`, `alert`), `car_id`, `own`, `text`. Last 300 |
| `laps[]` | `id`, `laps[]` of `{lap, time, pit}`: the last 20 laps of every car |

### `AiFeed` (on change and every 10 s)

`status`: `enabled`, `key_present`, `busy`, `calls_today`, `cost_today_usd`,
`calls_last_hour`, `blocked` (why no comment is coming), `last_error`,
`config`. `comments[]` (last 30, newest last): `id`, `session_time`, `lap`,
`trigger`, `question`, `text` (empty when withheld or refused), `unverified`
(numbers not in the snapshot), `withheld`, `refused`, `model`, `usage`,
`cost_usd`, `latency_ms`.

### Commands

- `{"command":"ai_config","config":{"enabled":true,"model":"claude-opus-5-5","effort":"low","every_laps":2,"on_events":true,"min_interval_s":60,"max_calls_per_hour":40,"daily_budget_usd":3,"language":"Italian","show_unverified":false}}`
- `{"command":"ai_ask","question":"…"}`: a comment now; `question` may be null.

## Additions: engineer seat and session history

### Following a car

- `ScoringUpdate.own_car` (bool): this PC has a car of its own. `followed_car`
  (string | null): the car name chosen to follow. `followed` (bool):
  `player_vehicle_id` is that followed car.
- `TelemetryUpdate.spectated` (bool): the car is followed, not driven here; its
  fuel, tyre and brake values are LMU's placeholders.
- Command `{"command":"follow_car","car_name":"Team X #93"}` (`null` to stop).
  A hub answers it with `HubError`: choose the car on the Pitwall connected to
  the game.

### History commands and replies

| Command | Reply |
|---|---|
| `history_list` | `HistorySessions { sessions[] }`: `id`, `started_at`, `last_seen`, `track`, `session_type`, `car_name`, `car_class`, `followed`, `laps`, `drivers[]`, `best_lap` |
| `history_session { session_id }` | `HistorySession`: `session`, `laps[]`, `stints[]`, `events[]`, `ai[]`, `cars[]` (with best and average lap), `strategy_laps[]` |
| `history_car_laps { session_id, car_ids[] }` | `HistoryCarLaps { session_id, laps[] }` (up to 8 cars) |
| `history_strategy { session_id, lap }` | `HistoryStrategy { session_id, lap, json }` (a StrategyUpdate as JSON text) |
| `history_delete { session_id }` | `HistorySessions`; local clients only |

Errors come back as `HistoryError { message }`.

# LMU data sources: REST API (:6397) and `LMU_Data` shared memory

> **Status: partly verified.** Measured on LMU 1.4.2 (`gameVersion` 14200,
> Steam build 25661166) on 2026-11-14: REST in the main menu, shared memory
> layout against the shipped headers (`tools/layout-check`). Anything marked
> *to verify* still comes from this repo's bridge or from TinyPedal
> (`tinypedal/adapter/lmu_restapi.py`). Re-run `tools/rest-probe/rest-probe.ps1`
> after each game update.

## REST API — `http://localhost:6397`

- Local only: the game binds it on the racing PC. It is not the Motorsport
  Games online API, which is rate-limited and not public.
- Plain HTTP, JSON, no auth. Only `GET` is documented here.
- Some endpoints answer only inside a session (garage or on track).
  Expect connection errors in the main menu and empty objects between sessions.

Status codes measured in the main menu (no session loaded):

| Endpoint | Menu | What it gives | Used by |
|---|---|---|---|
| `/rest/sessions` | 200, 9 KB | All session settings (see below) | TinyPedal |
| `/rest/sessions/weather` | 200, 5 KB | Forecast per session (`PRACTICE`, `QUALIFY`, `RACE`), 5 nodes each | pitwall (5 s), TinyPedal |
| `/rest/strategy/usage` | 200, `null` | Per-lap usage history per driver. *To verify in session* | pitwall (3 s) |
| `/rest/strategy/pitstop-estimate` | 200, `null` | Estimated stationary pit time: `total`, `damage`. *To verify in session* | TinyPedal |
| `/rest/garage/UIScreen/RepairAndRefuel` | **404** | Wear and damage, pit menu, `fuelInfo.maxVirtualEnergy`. *To verify in session* | pitwall (2 s), TinyPedal |
| `/rest/garage/getPlayerGarageData` | **404** | Current car setup. *To verify in session* | TinyPedal |
| `/rest/race/car` | 200, 522 KB | Array of every car in the game (676), with fields `id`, `name`, `manufacturer`, `vehFile`, `engine`, `owned`, `fullPathTree`, `displayProperties`, `image`, `thumbnail`, `dlcappID`, `premId`, `sig` | TinyPedal |

`/rest/sessions/setting/<KEY>` returns **404**: there is no per-setting path.
The settings are keys inside `/rest/sessions`.

### `/rest/sessions` (measured)

An object with one key per setting, each shaped like:

```json
{"currentValue":1,"numStepsTotal":3,"settingID":40,"stringValue":"Realistic","uiSelectionType":"Arrow","valueType":"LONG"}
```

Keys useful for strategy: `SESSSET_Fuel_Usage`, `SESSSET_Tire_Wear`,
`SESSSET_race_timescale`, `SESSSET_race_time`, `SESSSET_Race_Laps`,
`SESSSET_Finish_Criteria`, `SESSSET_tire_warmers`,
`SESSSET_tires_available_in_garage`, `SESSSET_timescaled_weather`,
`SESSSET_weather`, `SESSSET_safetycar_thresh`, `SESSSET_blue_flags`,
`SESSSET_Mech_Failures`, `SESSSET_Damage_Multi`.

### `/rest/sessions/weather`

```text
{
  "PRACTICE": { "START": node, "NODE_25": node, "NODE_50": node, "NODE_75": node, "FINISH": node },
  "QUALIFY":  { ... },
  "RACE":     { ... }
}
node = {   // every value is { "currentValue": number, "stringValue": label }
  "WNV_SKY":           { "currentValue": 3, "stringValue": "Mostly Cloudy" },  // 0..10: 0 clear, 4 overcast, 6 cloudy + light rain, 10 storm
  "WNV_TEMPERATURE":   { "currentValue": °C },
  "WNV_RAIN_CHANCE":   { "currentValue": 0, "stringValue": "0%" },          // percent
  "WNV_HUMIDITY":      { "currentValue": ... },
  "WNV_WINDDIRECTION": { "currentValue": ... },
  "WNV_WINDSPEED":     { "currentValue": ... }
}
```

The nodes sit at 0 %, 25 %, 50 %, 75 % and 100 % of the session length.
That is the forecast resolution: for a 6-hour race, one point every 90 minutes.
Keys are uppercase.

### `/rest/strategy/usage`

```text
{ "<DriverName>": [ { "ve": 0.0..1.0, ... }, ... ] }
```

Only `ve` is read today. **To verify:** the other fields in each entry, whether
teammates and opponents appear, and what the key is when two drivers share a car.

### `/rest/garage/UIScreen/RepairAndRefuel`

```text
{
  "wearables": {
    "body":       { "aero": 0..1 },
    "brakes":     [FL, FR, RL, RR],   // 0 = new, 1 = worn out
    "suspension": [FL, FR, RL, RR]
  },
  "pitMenu":  { "pitMenu": [...] },   // planned refuel / tyres / repairs
  "fuelInfo": { "maxVirtualEnergy": ... }
}
```

### `/rest/strategy/pitstop-estimate`

```text
{ "total": seconds, "damage": seconds, ... }
```

This is the stationary time the game predicts for the current pit menu. It
does **not** include the pit lane drive-through. The total pit loss must be
measured per track (see the strategy phase).

## Shared memory — `LMU_Data`

Requires *Settings → Gameplay → Enable Plugins → ON* and a full game restart.
Layout: `bridge/src/shared_memory/lmu_data.rs`, from S397's
`Support\SharedMemoryInterface\SharedMemoryInterface.hpp`. On 1.4.2 every
size and offset matches the shipped headers (`tools/layout-check`).

### Synchronisation (new in the 1.4.2 header, not used by the bridge yet)

The shipped header now defines named kernel objects. All of them exist while
the game is running (checked by opening them):

| Name | Kind | Use |
|---|---|---|
| `LMU_Data` | file mapping | the data |
| `LMU_Data_HoldEvent`, `LMU_Data_DataEvent` | events | `WaitForNextFrame`: wait on Hold, then Data → a new frame is ready. No polling needed |
| `LMU_SharedMemoryLockData` + `LMU_SharedMemoryLockEvent` | mapping + event | `SharedMemoryLock`: spin lock (`busy`/`waiters`) held by the game while writing. A reader takes it around its copy |

The bridge still polls at 50 Hz and checks for torn copies by re-reading a few
fields (the "witness" read). Switching to the events and the lock gives
coherent copies at the game's own frame rate. This matters for coaching, which
needs every 100 Hz frame. The lock blocks the game's writer, so the copy under
it must stay short.

`generic.events[16]` is described in the header as a *queue of event types*
(`SharedMemoryEvent` values), not as counters. `CopySharedMemoryObj` scans it
for `SME_UPDATE_SCORING` / `SME_UPDATE_TELEMETRY` to decide what to copy. The
bridge names those slots like counters; the size is the same.

| Block | Rate | Content |
|---|---|---|
| `generic` | events | Event counters, `gameVersion` |
| `scoring.scoringInfo` | 5 Hz | Session, phase, yellow state, sector flags, **current weather** (`mRaining`, `mDarkCloud`, `mCloudCoverage`, ambient/track temp, wind, path wetness min/avg/max), time remaining, time of day, grip level |
| `scoring.vehScoringInfo[104]` | 5 Hz | Per car: position, lap distance, sectors, gaps, pit state, pit stops, penalties, flag, `mUnderYellow`, `mControl` (local, AI, remote), `mIsPlayer`, class, estimated lap time |
| `telemetry.telemInfo[104]` | 100 Hz | Per car: position, velocity, inputs, engine, fuel, wheels; plus `playerVehicleIdx` and `playerHasVehicle` |

### What is real for cars other than ours

Taken from this repo's code and comments, measured by the upstream author
(`bridge/src/main.rs::position_of`, `bridge/src/post_race/live_laps.rs`):

| Field | Other cars |
|---|---|
| `mPos` (world position) | real, 100 Hz |
| `mVirtualEnergy` | real for the whole grid |
| `mFuel` | **placeholder**: fixed at half the tank |
| `mWheels[].mWear` | **placeholder**: fixed at 1.0 |
| inputs, tyre temperatures and pressures, brake temperatures | **not verified** (`tools/lmu-probe` run C) |

### Where VE and weather live

| Data | Shared memory | REST |
|---|---|---|
| VE now (own car) | `mVirtualEnergy` (100 Hz) | — |
| VE now (other cars) | `mVirtualEnergy` | — |
| VE per lap, history | — (computed at the line) | `/rest/strategy/usage` |
| VE tank size | — | `fuelInfo.maxVirtualEnergy` |
| Lift & coast | `mLiftAndCoastProgress` (meaning to verify) | — |
| Weather now | `scoringInfo`: rain, cloud, temperatures, wetness | — |
| Weather forecast | — | `/rest/sessions/weather` |

### Known gaps

- No safety car flag: `mSafetyCarActive` existed only in the rF2 plugin's
  Rules buffer. Caution shows up as `mGamePhase == 6` (full course yellow) plus
  `mYellowFlagState`.
- No version block: the bridge uses a witness re-read to detect torn copies.
- `mSectorFlag`: 1 = yellow, 11 = clear (not the rF2 encoding).
- Online result XML files carry no fuel, VE or wear data (`ClientFuelVisible`),
  so live capture is the only source for online races.

## To verify with the game open

1. Run `tools/rest-probe/rest-probe.ps1` in the menu, in the garage and on track.
2. Run `tools/lmu-probe` run C (offline against AI, then online) for
   other cars' telemetry.
3. Join as a spectator or engineer and check `playerHasVehicle`,
   `playerVehicleIdx` and the team car's telemetry row.

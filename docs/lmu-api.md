# LMU data sources: REST API (:6397) and `LMU_Data` shared memory

> **Status: draft, not yet verified against a running game.**
> Everything below comes from code that already works against LMU: this
> repo's bridge (`bridge/src/garage_api.rs`, `bridge/src/shared_memory/`) and
> TinyPedal (`tinypedal/adapter/lmu_restapi.py`). No response has been captured
> here yet. Each entry says where it comes from. Run
> `tools/rest-probe/rest-probe.ps1` with the game open to confirm the
> endpoints and paste real (anonymised) samples below.

## REST API — `http://localhost:6397`

- Local only: the game binds it on the racing PC. It is not the Motorsport
  Games online API, which is rate-limited and not public.
- Plain HTTP, JSON, no auth. Only `GET` is documented here.
- Some endpoints answer only inside a session (garage or on track).
  Expect connection errors in the main menu and empty objects between sessions.

| Endpoint | Used by | What it gives | Polled |
|---|---|---|---|
| `/rest/sessions/weather` | pitwall, TinyPedal | Forecast per session (`PRACTICE`, `QUALIFY`, `RACE`), 5 nodes each | pitwall every 5 s |
| `/rest/strategy/usage` | pitwall | Per-lap usage history per driver, keyed by driver name | pitwall every 3 s |
| `/rest/garage/UIScreen/RepairAndRefuel` | pitwall, TinyPedal | Wear and damage, pit menu, `fuelInfo.maxVirtualEnergy` | pitwall every 2 s |
| `/rest/strategy/pitstop-estimate` | TinyPedal | Estimated stationary pit time: `total`, `damage` (repair time) | — |
| `/rest/sessions` | TinyPedal | Session settings, e.g. `SESSSET_race_timescale.currentValue` | — |
| `/rest/sessions/setting/SESSSET_race_timescale` | TinyPedal | Race time scale | — |
| `/rest/sessions/setting/SESSSET_private_qual` | TinyPedal | Private qualifying on/off | — |
| `/rest/garage/getPlayerGarageData` | TinyPedal | Current car setup, `VM_STEER_LOCK.stringValue` | — |
| `/rest/race/car` | TinyPedal (brand editor) | Car list with brands | — |

### `/rest/sessions/weather`

```text
{
  "PRACTICE": { "START": node, "NODE_25": node, "NODE_50": node, "NODE_75": node, "FINISH": node },
  "QUALIFY":  { ... },
  "RACE":     { ... }
}
node = {
  "WNV_SKY":           { "currentValue": 0..10 },   // 0 clear, 4 overcast, 6 cloudy + light rain, 10 storm
  "WNV_TEMPERATURE":   { "currentValue": °C },
  "WNV_RAIN_CHANCE":   { "currentValue": 0..100 },  // percent
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
`Support\SharedMemoryInterface\SharedMemoryInterface.hpp`.

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

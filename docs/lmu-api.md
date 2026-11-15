# LMU data sources: REST API (:6397) and `LMU_Data` shared memory

> **Status: measured** on LMU 1.4.2 (`gameVersion` 14200, Steam build
> 25661166), 2026-11-14, in an offline practice session at Monza: 37 cars
> (Hypercar, LMP2, GTE), player in a Hypercar, 3 laps driven.
> Tools: `tools/rest-probe`, `tools/lmu-probe`, `tools/layout-check`.
> Online: an LMP3 server, 14 cars, watched from the monitor.
> **Not measured yet:** a shared team car, driver swaps, rain, races.
> Re-run the tools after each game update.

## REST API — `http://localhost:6397`

- Local only: the game binds it on the racing PC. It is not the Motorsport
  Games online API.
- Plain HTTP, JSON, no auth. Only `GET` is documented here.
- Responses are UTF-8 **without a charset header**. Decode the bytes as UTF-8
  explicitly, or driver names with accents break.

| Endpoint | Main menu | In session | What it gives |
|---|---|---|---|
| `/rest/sessions` | 200 | 200 | Every session setting |
| `/rest/sessions/weather` | 200 | 200 | Forecast per session type, 5 nodes each |
| `/rest/strategy/usage` | `null` | 200 | Per-lap history for **every driver** |
| `/rest/strategy/pitstop-estimate` | `null` | 200 | Predicted stationary pit time, itemised |
| `/rest/garage/UIScreen/RepairAndRefuel` | 404 | 200 | Fuel and VE in real units, pit menu, pit work times, wear, team, forecast |
| `/rest/garage/getPlayerGarageData` | 404 | 200 | Car setup (`VM_*` keys, 42 KB) |
| `/rest/race/car` | 200 | 200 | Every car in the game (676 entries, 522 KB) |

"In session" means a session is loaded (garage, monitor or on track). After
leaving the session the session endpoints go back to `null` / 404.
There is no `/rest/sessions/setting/<KEY>` path (404).

### `/rest/sessions`

One key per setting, each shaped like:

```json
{"currentValue":1,"numStepsTotal":3,"settingID":40,"stringValue":"Realistic","uiSelectionType":"Arrow","valueType":"LONG"}
```

Useful for strategy: `SESSSET_Fuel_Usage`, `SESSSET_Tire_Wear`,
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

### `/rest/strategy/usage`

Keyed by driver name, one entry per lap line crossing (lap 0 = session start).

```text
{
  "<own driver>":   [ { "lap": 3, "stint": 1, "pit": true, "ve": 0.9647, "fuel": 0.7294, "tyres": [96.08, 97.25, 96.86, 97.25] }, ... ],
  "<other driver>": [ { "lap": 2, "stint": 1, "pit": false, "ve": 0.9333 }, ... ]
}
```

- `ve`, `fuel`: fraction remaining, 0–1. `tyres`: tread remaining in percent
  (FL, FR, RL, RR).
- `fuel` and `tyres` exist **only for the own car**. Other drivers get `lap`,
  `stint`, `pit`, `ve`.
- `ve` is 0 for classes without virtual energy (LMP2 and GTE in this session).
- `stint` is the game's own stint counter, for every car.
- `pit`: exact meaning *to verify* (it was `true` on laps that started from
  the pits or passed through them).
- *To verify:* the key when several drivers share one car.

### `/rest/strategy/pitstop-estimate`

```json
{"brakeDucts":0.0,"brakes":0.0,"damage":0.0,"driverSwap":0.0,"fuel":0.0,"penalties":10.0,"tires":0.0,"total":10.0,"ve":0.0}
```

Seconds of stationary time for the **current pit menu**, split by cause.
It does not include the pit lane drive-through, which must be measured per
track.

### `/rest/garage/UIScreen/RepairAndRefuel`

The richest endpoint for strategy. Top-level keys:

| Key | Content (measured values as examples) |
|---|---|
| `fuelInfo` | `currentFuel` 80.09, `maxFuel` 110.0 (litres); `currentVirtualEnergy` 867 711 104, `maxVirtualEnergy` 900 000 000; `currentBattery`, `maxBattery`. `current/max` VE = 0.964, matching `mVirtualEnergy` |
| `pitStopTimes.times` | Pit work model of the car: `FuelFillRate` 4.0, `virtualEnergyFillRate` 0.025 (fraction per second, so 40 s for a full refill), `FourTireChange` 12.0, `TwoTireChange` 4.5, `DriverChange` 25.0, `FixAeroDamage` 25.0, `BrakeChange` 120.0, plus `*TimeConcurrent` flags (what runs in parallel) and `*RandomDelay` |
| `pitStopLength` | `timeInSeconds`: same total as `pitstop-estimate` |
| `pitMenu.pitMenu[]` | Each pit menu line: `name` ("VIRTUAL ENERGY:", "DRIVER:", "STOP/GO:", tyres…), `currentSetting`, `default`, `settings[].text` (e.g. "25% 7 laps") |
| `pitRecommendations` | The game's own advice: `fuel`, `virtualEnergy`, per-tyre |
| `wearables` | `tires[4]` tread remaining 0–1; `brakes[4]` wear 0–1; `suspension[4]`; `body.aero`; `body.detachableParts[]` |
| `teamInfo` | `teamName`, `vehicleName`, **`driverNames[]`** (the car's driver roster) |
| `racePosition` | `placeInClass`, `placeOverall`, gap to first and last in class (time and laps) |
| `currentWeather` | `ambientTempKelvin`, `trackTempKelvin`, `rainIntensity`, `raining`, `cloudCoverage`, `humidity`, `airPressure`, `lightLevel` |
| `weatherForecast.nodes` | Arrays of 5: `Sky`, `RainChance`, `Temperature`, `Humidity`, `WindSpeed`, `WindDirection`, `StartTime`, `Duration` |
| `sessionTime` | `timeOfDay` (seconds) |

## Shared memory — `LMU_Data`

Requires *Settings → Gameplay → Enable Plugins → ON* and a full game restart.
Layout: `bridge/src/shared_memory/lmu_data.rs`. On 1.4.2 every size and offset
matches the headers shipped in `Support\SharedMemoryInterface`
(`tools/layout-check`), and `lmu-probe` found all 90 fields the bridge reads
plausible (71 proven live, 19 legal zeros, 0 failing).

| Block | Rate (measured) | Content |
|---|---|---|
| `generic` | events | Event queue, `gameVersion` |
| `scoring.scoringInfo` | ~5 Hz | Session, phase, yellow state, sector flags, **current weather**, time remaining, time of day, grip level |
| `scoring.vehScoringInfo[104]` | ~5 Hz | Per car: position, lap distance, sectors, gaps, pit state, pit stops, penalties, flag, `mControl`, `mIsPlayer`, class |
| `telemetry.telemInfo[104]` | ~100 Hz | Per car: position, velocity, inputs, engine, fuel, wheels; plus `playerVehicleIdx`, `playerHasVehicle` |

Units confirmed: tyre and brake temperatures are Kelvin; fuel is litres.
Torn reads with plain polling: 0 of 4207 snapshots in a stable field.

### What other cars' telemetry rows contain

Two measurements: 36 AI cars offline (Monza), and 13 human-driven cars online
(LMP3 server, watched from the monitor with our own car parked in the garage,
30 s at ~13 Hz). Online the game hides the consumables.

| Field | Offline, AI cars | Online, remote cars |
|---|---|---|
| Position, velocity, `mElapsedTime`, lap number | real | real |
| Throttle, brake, steering, gear, RPM | **real** | **real** |
| `mFuel` | real, changes per lap | **placeholder**: 50.0 on every car |
| `mWheels[].mWear` | real, changes per lap | **placeholder**: 1.0 on every car |
| `mWheels[].mBrakeTemp` | real (300–760 °C under braking) | **placeholder**: ambient, never moves |
| `mVirtualEnergy`, `mBatteryChargeFraction` | real for Hypercars; 0 for LMP2 and GTE | 0 here (LMP3 has no VE). Upstream reports VE is real online for VE classes; *to verify* |
| Tyre surface and carcass temperature, tyre pressure | not published (0) | not published (0) |
| TC, ABS, motor map and other electronics | not published (0) | not published (0) |

Online, `mControl` is 2 (remote) for every other car. Our own parked car had
`mIsPlayer` = 1 and `mControl` = 1, and `playerHasVehicle` was 1.

REST online, from the monitor: every endpoint answers, but
`RepairAndRefuel`, `pitstop-estimate` and the garage data describe **our own
car only**. `/rest/strategy/usage` lists all 14 drivers with `lap`, `stint`,
`pit`, `ve`; `fuel` and `tyres` appear only on our own entry.

**Consequence:** watching from another PC gives the pedals and pace of any
car, but not its fuel, tyre wear, tyre or brake temperatures, or electronics.
Those need a bridge on the PC of whoever is driving.

### LMU's native engineer mode (measured 2026-11-15)

Online race at Bahrain (LMGT3, 38 cars), joined as the engineer of a team car
while a teammate drove it:

| What | Value |
|---|---|
| `playerHasVehicle` | 0 |
| `mIsPlayer` | 0 on every car, the team car included |
| `mControl` | 2 (remote) on every car |
| Team car telemetry: fuel, tyre wear, brake temperature | placeholders, like any other car (fuel exactly half the tank, wear 1.0, brakes static) |
| Team car: tyre temperatures and pressures, electronics | not published (0) |
| All cars: throttle, RPM, speed, position | live |
| All cars: `mVirtualEnergy` | live (changes lap by lap) |
| `RepairAndRefuel` | describes the team car (its driver roster, its pit menu), but `fuelInfo.currentFuel` is half of `maxFuel` and `wearables.tires` is 1.0: the same placeholders |
| `/rest/strategy/usage` | every driver with `lap`, `stint`, `pit`, `ve`; nobody has `fuel` or `tyres` |

So the engineer seat shows the team car no better than any other car: VE,
pace and inputs yes; fuel, tyres, brakes and electronics no. The bridge on the
driving PC is required for those.

The own car's tyre temperatures and pressures also read 0 while the car sits
in the garage; they are live only on track.

### Events and stale data

`generic.events[16]` is a **queue of event types** for the current frame, not
a set of counters: unused slots hold `SME_MAX` (16). Upstream names the slots
like counters; the size is the same, but the meaning is not.

**After leaving a session the mapping is not cleared.** Measured: back in the
main menu, `mNumVehicles` was still 37, `playerHasVehicle` still 1, and all
37 telemetry rows still held their last values. The only signals were
`events[0] == SME_UNLOAD` (5) and clocks that no longer advance. A reader must
treat a frozen `mElapsedTime` or an `SME_UNLOAD` / `SME_END_SESSION` event as
"no session", or it will show a ghost session.

### Synchronisation (new in the 1.4.2 header, not used by the bridge yet)

| Name | Kind | Use |
|---|---|---|
| `LMU_Data` | file mapping | the data |
| `LMU_Data_HoldEvent`, `LMU_Data_DataEvent` | events | `WaitForNextFrame`: wait on Hold, then Data → a new frame is ready. No polling |
| `LMU_SharedMemoryLockData` + `LMU_SharedMemoryLockEvent` | mapping + event | `SharedMemoryLock`: spin lock held by the game while writing; a reader takes it around its copy |

All of them exist while the game runs (checked by opening them). The bridge
polls at 50 Hz and detects torn copies by re-reading a few fields. Waiting on
the events gives every 100 Hz frame, which coaching needs. The lock blocks the
game's writer, so the copy under it must stay short.

### Where VE, fuel and weather live

| Data | Shared memory | REST |
|---|---|---|
| VE now, own car | `mVirtualEnergy` (fraction) | `fuelInfo.currentVirtualEnergy` / `max` |
| VE now, other cars | `mVirtualEnergy` (VE classes only) | — |
| VE per lap, all drivers | computed at the line | `/rest/strategy/usage` |
| Fuel and tyre wear per lap, own car | computed at the line | `/rest/strategy/usage` |
| Refill rates, tyre change and driver swap times | — | `RepairAndRefuel.pitStopTimes` |
| Stationary time of the planned stop | — | `/rest/strategy/pitstop-estimate` |
| Driver roster of the car | — | `RepairAndRefuel.teamInfo.driverNames` |
| Lift & coast | `mLiftAndCoastProgress` (meaning to verify) | — |
| Weather now | `scoringInfo`: rain, cloud, temperatures, wetness | `RepairAndRefuel.currentWeather` |
| Weather forecast | — | `/rest/sessions/weather`, `RepairAndRefuel.weatherForecast` |

### Known gaps

- No safety car flag. Caution shows up as `mGamePhase == 6` plus
  `mYellowFlagState`.
- `mSectorFlag`: 1 = yellow, 11 = clear (not the rF2 encoding).
- Online result XML files carry no fuel, VE or wear data, so live capture is
  the only source for online races.

## Still to verify

1. ~~Shared team car / engineer mode~~: measured, see above.
2. **Spectator / engineer:** `playerHasVehicle`, `playerVehicleIdx`, the team
   car's telemetry row, and whether the REST garage endpoints answer for the
   team car.
3. **Driver swap:** `mControl` and `mIsPlayer` on the team car before and
   after, and the `/rest/strategy/usage` key.
4. **Rain:** wetness fields and forecast behaviour with timescaled weather.
5. `pit` in `/rest/strategy/usage`; `mLiftAndCoastProgress`.

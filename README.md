# LMU Pitwall

Real-time sim racing dashboard for [Le Mans Ultimate](https://www.lemansultimate.com/). Runs as a single `.exe` on your racing PC and serves the dashboard to any browser on the same network.

![LMU Pitwall Dashboard](docs/screenshots/Full.png)

> **Fork: remote race engineer.** This fork (`remote-engineer` branch) is
> turning Pitwall into a remote pitwall for endurance teams: a bridge on each
> driver's PC, reached over Tailscale, with a hub for the engineer that follows
> whoever is driving. Work in progress.
>
> | Phase | Status |
> |---|---|
> | 1. Analysis of the code and of LMU's data sources | done: [docs/analisi-fase1.md](docs/analisi-fase1.md) (Italian), [docs/lmu-api.md](docs/lmu-api.md) |
> | 2. Remote access: token, reconnects, no shutdown mid-race | done: [docs/accesso-remoto.md](docs/accesso-remoto.md) (Italian) |
> | 3. Multi-driver hub, stint history, record/replay; class gaps, tyre status, fuel calculator auto-select | done: [docs/hub-multi-pilota.md](docs/hub-multi-pilota.md) (Italian) |
> | 4. Live strategy engine: consumption, stops, pit window, "box in N laps", tyres, pit lane loss, weather | done: [docs/strategia.md](docs/strategia.md) (Italian) |
> | 5. Timing and race events (rivals lap by lap, multiclass traffic and blue flags, race log), touch layout and installable web app for tablets | done: [docs/tablet.md](docs/tablet.md) (Italian) |
> | 6. AI engineer comments (Anthropic API) on the computed state only, numbers checked | done: [docs/ai.md](docs/ai.md) (Italian) |
> | 7. Session history in SQLite: laps, every car's laps, stints, events, strategy, AI; History panel | done: [docs/storico.md](docs/storico.md) (Italian) |
> | Engineer-seat fallback: follow the team car from LMU's engineer seat (timing, VE strategy, events, AI; no fuel or tyres) | done |
> | Extra. Driver coaching: per-corner metrics, lift & coast, reference lap comparison | optional |

## Hub: one dashboard for a team of drivers

In a race driven in stints, each driver runs Pitwall on their own PC and the
engineer runs one more in hub mode:

```
lmu-pitwall.exe --hub        # dashboard on http://localhost:9100
```

Paste each driver's link into the **Team & Stints** panel (click the 👥 badge).
The hub relays the stream of whichever PC is driving, switches by itself at a
driver swap, and keeps the stint history across swaps: driver, duration, laps,
best and average lap, fuel and virtual energy per lap. A bridge can be pinned
when the automatic choice is not what you want. Details and current limits:
[docs/hub-multi-pilota.md](docs/hub-multi-pilota.md).

## Session history

Every session is saved as it happens to `history.sqlite` (our laps with fuel,
VE and wear, every car's laps, stints, events, the strategy plan at each lap,
AI comments). The 🗂 button opens the History panel: stint comparison, lap
times against up to three rivals, events and AI comments. Details:
[docs/storico.md](docs/storico.md).

## Engineer seat

In LMU's engineer seat the game gives your PC no car of its own. Pick the team
car in the toolbar (**👁 Follow a car…**): timing, gaps, rivals, VE strategy,
race events and AI comments work from your PC alone. Fuel and tyres are not
shared by the game with other PCs (measured), so those widgets say so; they
need Pitwall on the driver's PC.

## Tablet

Add the dashboard to the home screen (Android: Chrome menu → *Add to Home
screen*; iPad: Share → *Add to Home Screen*), pick the **Engineer** preset and
tap ☀ to keep the screen on. Touch screens start with the layout locked;
portrait layouts are derived from the landscape one. Details:
[docs/tablet.md](docs/tablet.md).

## Record and replay

```
lmu-pitwall.exe --record race.pwrec                  # write the broadcast stream while you play
lmu-pitwall.exe --replay race.pwrec --ws-port 9001   # serve it back, no game needed
```

Replay loops by default (`--replay-once`, `--replay-speed 2`). For layout work
and hub tests without any recording, `tools/mock-session/make_session.py`
generates synthetic sessions; they are not real telemetry.

## Remote access

Anyone on the racing PC itself uses the dashboard as before. From another
machine the bridge asks for an **access token**, generated on first run:

1. On the racing PC, click **🔗** in the toolbar and copy the link
   (`http://<address>:9000/?token=…`). With [Tailscale](https://tailscale.com)
   running, the panel offers a link that works from outside the home network.
2. Open the link on the other device. The browser remembers the token.

Remote clients can watch but not control the racing PC: shutting the bridge
down, changing its port and installing voices only work locally. The bridge
stays up for as long as the game is running, and exits after 10 minutes with
no client and no game (`--keep-alive` disables that). Details and
troubleshooting: [docs/accesso-remoto.md](docs/accesso-remoto.md); wire
protocol: [shared/protocol.md](shared/protocol.md).

## Install

Download `LMU-Pitwall-Setup-X.X.X.exe` from the [latest release](https://github.com/Swizzjack/lmu-pitwall/releases), run the installer, start Le Mans Ultimate, then open the dashboard in any browser:

```
http://<racing-pc-ip>:9000
```

No configuration required. A portable `lmu-pitwall.exe` (no installer) is also on the releases page.

## Shared Memory (no plugin required)

LMU Pitwall reads Le Mans Ultimate's own `LMU_Data` shared memory, which the
game publishes itself. There is nothing to download and nothing to keep in step
with game updates.

**The only setting:** Settings → Gameplay → Enable Plugins → **ON**, then
restart LMU. The switch also governs the game's built-in shared memory, and it
only takes effect after a full restart. This is required even with no plugin
installed — verified: with the switch off, LMU publishes no `LMU_Data` mapping
and Pitwall stays on *Waiting for Le Mans Ultimate*. If the dashboard never
connects while the game is running, that switch is the first thing to check.

> **Upgrading from an older LMU Pitwall?** The
> [rF2 Shared Memory Map Plugin](https://github.com/TheIronWolfModding/rF2SharedMemoryMapPlugin)
> is no longer used. You can delete
> `Le Mans Ultimate\Plugins\rFactor2SharedMemoryMapPlugin64.dll` and its entry
> in `UserData\player\CustomPluginVariables.JSON`. Leaving them in place does
> no harm — Pitwall simply ignores those buffers now.

## Usage

1. Start Le Mans Ultimate
2. Run `lmu-pitwall.exe`
3. Open a session (Practice, Qualifying, or Race)
4. Open `http://<racing-pc-ip>:9000` in any browser on your local network

The dashboard auto-connects and updates in real time. On first run, Windows Firewall may prompt you to allow port 9000.

## Features

| Widget | Description |
|--------|-------------|
| **Fuel** | Fuel remaining, median consumption per lap (rolling average, excludes first lap of each stint), laps to empty, fuel needed to finish |
| **Standings** | Live positions with car number, brand, gap, sector times, pit status, and a MiniDamageGrid per car. In multiclass fields the list can be narrowed to your own class, and the gap can be measured against your class leader instead of the overall leader |
| **VehicleStatus** | Damage overview at three detail levels: compact, medium, or full — covers aero, brakes, and suspension |
| **Electronics** | TC, ABS, Engine Map, ARB, Regen, Brake Migration — read directly from shared memory, no button-counting; works in online sessions (EAC-protected) |
| **Tires** | Temperatures (inner/middle/outer/carcass), pressures, wear %, and brake disc temps per corner. A status chip per tyre says the one thing that matters: puncture, wheel off, locked, flat-spot risk (inferred from lock-ups, since LMU reports no flat spots), worn, cold or hot relative to the compound's optimal temperature |
| **Gaps (class)** | Gap to the cars one place ahead and behind in your class, and whether each is closing or growing lap over lap |
| **Team & Stints** | Hub mode only: the drivers' PCs, who is on screen, and the stint history |
| **Traffic** | Faster classes closing from behind and slower ones ahead to lap, with seconds until they meet; blue flag banner |
| **Rivals** | Our recent laps next to the class rivals ahead and behind and the class leader, lap by lap |
| **Race events** | Full course yellow, pit lane open/closed, pit stops in our class, penalties, driver changes, position changes, impacts, best laps |
| **AI engineer** | Short comments from Claude for the engineer, from the computed state only; any comment with a number not in the data is withheld. Off by default, with a daily budget. See [docs/ai.md](docs/ai.md) |
| **Race Engineer** | Spoken callouts during practice, qualifying, and race: tire wear warnings, weather escalations, pace deltas, fuel status, flag alerts |
| **Strategy** | Live plan recomputed every lap: fuel and virtual energy use (median, trend), laps to go, energy to the finish, stops and pit window, a "box in N laps" table with amounts and stop time from the game's own pit times, tyre wear and pace trend with the new-tyres trade-off, pit lane loss measured per track, rain outlook from the forecast. Works on a hub across driver swaps. Plus the Fuel Calculator for pre-race planning, preselecting the track and car of the session in progress |
| **Track Map** | Live positions of the whole field, drawn smoothly at your display's refresh rate, on an outline recorded from one clean lap. Sector segments with start/finish and sector lines, local yellows on the sector they apply to, track and sector lengths, elevation range, climb per lap and an elevation profile |
| **Weather** | Current and forecast conditions |
| **Inputs** | Throttle, brake, and steering trace |
| **Flags** | Real-time flag display (green, blue, yellow, red, chequered) with session-phase awareness |
| **Time** | Session time, time remaining, lap counter |
| **Post Race Results** | Load any LMU session XML to view full classification, lap times, sectors, gaps, and pitstop data |
| **Toolbar** | Shows local IP and port for quick browser access from a second device |

Widget layout is drag-and-drop; positions and sizes are saved automatically.

## Architecture

```
Le Mans Ultimate (Windows)
├── LMU_Data shared memory  (100 Hz telemetry, 5 Hz scoring)
└── REST API  :6397
    ├── GET /rest/strategy/usage                        (Virtual Energy history)
    └── GET /rest/garage/UIScreen/RepairAndRefuel       (wearables: aero, brakes, suspension)

lmu-pitwall.exe (Rust, single binary)
├── Shared Memory reader ──────────────────────────────► WebSocket server :9000
├── REST API client ───────────────────────────────────► WebSocket server :9000
└── HTTP server :9000
    ├── Upgrade: websocket  → WebSocket handler (live telemetry)
    ├── GET /              → index.html  (React SPA entry)
    ├── GET /assets/*      → hashed JS/CSS bundles (immutable cache)
    ├── GET /api/network-info   (token)
    ├── GET /api/auth-check
    ├── GET /api/version
    └── POST /api/shutdown      (racing PC only)

Browser (racing PC, LAN, or Tailscale)
├── ws://<ip>:9000/?token=…   live telemetry stream (no token needed on the racing PC)
└── http://<ip>:9000           React SPA (served from embedded rust-embed assets)
```

A single port handles both WebSocket upgrades and static file serving. The React bundle is embedded in the binary via `rust-embed` — no separate file deployment needed.

## Building from Source

**Environment:** Bazzite (Fedora Atomic) or any Fedora-based system. A [distrobox](https://github.com/89luca89/distrobox) container works well for keeping the toolchain isolated.

**One-time setup:**

```bash
# Rust toolchain
curl https://sh.rustup.rs -sSf | sh
rustup target add x86_64-pc-windows-gnu

# cargo-zigbuild (cross-compilation, no MinGW needed)
cargo install cargo-zigbuild
# Zig 0.13+: https://ziglang.org/download/ or via dnf/distrobox

# Node.js — use system package manager or https://nodejs.org
cd dashboard && npm install && cd ..
```

**Build and release:**

```bash
./scripts/release.sh
```

This script bumps the patch version, builds the React dashboard, cross-compiles the Rust backend to `x86_64-pc-windows-gnu` via `cargo zigbuild`, assembles `dist/`, and starts a local HTTP server so the `.exe` can be downloaded directly from a Windows machine on the same network.

**Script options:**

| Flag | Effect |
|------|--------|
| `--no-bump` | Skip version bump |
| `--no-serve` | Skip HTTP server after build |
| `--no-installer` | Skip Inno Setup installer build |
| `--port 8080` | HTTP server port (default: 8080) |

**Installer build (one-time setup):**

To produce `LMU-Pitwall-Setup-X.X.X.exe`, install Wine and Inno Setup 6 once:

```bash
sudo dnf install wine
./scripts/install-innosetup.sh
```

After that, `release.sh` automatically builds both the standalone `.exe` and the installer.

**Building on Windows (no cross-compilation):**

Needs [rustup](https://rustup.rs) with the default MSVC toolchain, the Visual
Studio Build Tools (C++ workload) and Node.js.

```powershell
cd dashboard; npm ci; npm run build; cd ..
cd bridge; cargo test; cargo build --release
# -> bridge\target\release\lmu-pitwall.exe (dashboard embedded)
```

Build the dashboard first: the bridge embeds `dashboard/dist` at compile time.
Without Zig the icon embedding step is skipped with a warning; the binary
works, it just has no icon.

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Backend | Rust — shared memory reader, WebSocket server, REST API client |
| Frontend | React + TypeScript — widget-based layout with drag & drop |
| Cross-compilation | `cargo-zigbuild` — Windows `.exe` from Linux, no MinGW required |
| Static embedding | `rust-embed` — React bundle baked into the binary |
| Installer | Inno Setup 6 via Wine |
| LMU data sources | `LMU_Data` shared memory (100 Hz telemetry / 5 Hz scoring) + LMU REST API port 6397 |

## Design

| Token | Value |
|-------|-------|
| Background | `#0f0f0f` |
| Primary | `#facc15` |
| Accent | `#f97316` |
| Fonts | Teko · Roboto Condensed · JetBrains Mono |

## Contributing

PRs are welcome. For larger changes, open an issue first to align on scope.

**Workflow:**

1. Fork the repo and create a feature branch off `main`
2. Build and verify: `./scripts/release.sh --no-bump --no-serve --no-installer`
3. Open a PR against `main` with a clear description of what and why

## Credits

Built by [Swizzjack](https://github.com/Swizzjack) with the help of [Claude](https://claude.ai) (Anthropic) for architecture, code generation, and development workflow.

## License

[MIT](LICENSE)

#!/usr/bin/env python3
"""Generate a synthetic LMU Pitwall recording (.pwrec) for development.

    python make_session.py out.pwrec
    lmu-pitwall.exe --replay out.pwrec --ws-port 9001 --no-browser

The result is NOT real telemetry. It is a small scripted race, shaped like the
bridge's messages, good for laying out widgets and for testing the hub without
the game or a second PC. For anything about how the game behaves, record a
real session instead:  lmu-pitwall.exe --record real.pwrec

One race, several points of view
--------------------------------
A shared car is driven in turns. `--team` lists who drives when, `--me` says
whose PC this recording is from. Run the script once per driver with the same
`--seed` and `--team` and you get the same race seen from each PC:

    python make_session.py alice.pwrec --me Alice --team "Alice:0-150,Bob:150-300"
    python make_session.py bob.pwrec   --me Bob   --team "Alice:0-150,Bob:150-300"

While someone else drives, this PC's own row has control=2 (remote) and the
consumables are placeholders (fuel 50, wear 1.0), which is what LMU shows for
other people's cars online. That the *team* car looks like this from a
teammate's PC is an assumption: it has not been measured in a team event.
"""
import argparse
import json
import math
import random

CLASSES = [("Hyper", 1.00, ["Toyota GR010", "Ferrari 499P", "Porsche 963", "Peugeot 9X8"]),
           ("GT3", 1.16, ["Ferrari 296 LMGT3", "Porsche 911 LMGT3 R", "BMW M4 LMGT3"])]
NAMES = ["M. Conway", "A. Fuoco", "K. Estre", "N. Catsburg", "E. Bamber", "F. Nasr",
         "D. Vanthoor", "R. Kubica", "V. Rossi", "A. Farfus", "M. Christensen", "T. Vautier"]


def parse_team(spec, duration):
    """'Alice:0-150,Bob:150-300' -> [(0.0, 150.0, 'Alice'), ...]; default: one driver all race."""
    if not spec:
        return [(0.0, duration, "Test Driver")]
    out = []
    for part in spec.split(","):
        name, span = part.split(":")
        a, b = span.split("-")
        out.append((float(a), float(b), name.strip()))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("out")
    ap.add_argument("--duration", type=float, default=300.0, help="seconds of session (default 300)")
    ap.add_argument("--lap-time", type=float, default=30.0, help="Hypercar lap time in seconds (default 30: short laps so stints fit in minutes)")
    ap.add_argument("--cars", type=int, default=8)
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--team", default="", help='who drives the team car when, e.g. "Alice:0-150,Bob:150-300"')
    ap.add_argument("--me", default="", help="whose PC this is (default: the first driver in --team)")
    ap.add_argument("--pit-every", type=int, default=5, help="team car pits every N laps (0 = never)")
    ap.add_argument("--lockup-at", type=float, default=40.0, help="session time of a long front-left lock-up (-1 = none)")
    ap.add_argument("--session", default="Race", choices=["Race", "Practice", "Qualifying"])
    ap.add_argument("--track", default="Mock Circuit")
    args = ap.parse_args()

    rng = random.Random(args.seed)
    team = parse_team(args.team, args.duration)
    me = args.me or team[0][2]
    track_len = 5800.0
    player_id = 2  # the team car is the third Hypercar on the grid

    cars = []
    for i in range(args.cars):
        cls, factor, models = CLASSES[0] if i < (args.cars + 1) // 2 else CLASSES[1]
        cars.append(dict(id=i, cls=cls, model=rng.choice(models), name=NAMES[i % len(NAMES)],
                         lap=args.lap_time * factor * (1 + rng.uniform(-0.012, 0.012)),
                         start=-i * 0.012))  # grid stagger, in laps
    cars[player_id]["model"] = "Peugeot 9X8"
    PIT_LOSS = 12.0  # seconds stationary

    def driver_at(t):
        for a, b, name in team:
            if a <= t < b:
                return name
        return team[-1][2]

    # --- team car state that evolves over the race -------------------------
    fuel_cap, fuel = 90.0, 90.0
    fuel_per_lap = 2.9
    wear = [1.0, 1.0, 1.0, 1.0]
    stint, stint_laps, stint_start_fuel = 1, 0, fuel
    lock = dict(count=0, total=0.0, longest=0.0)
    laps_done = [0] * args.cars
    last_lap = [-1.0] * args.cars
    best_lap = [-1.0] * args.cars
    lap_start = [0.0] * args.cars
    pit_until = [-1.0] * args.cars
    pit_delay = [0.0] * args.cars  # total seconds lost in the pits so far
    pitstops = [0] * args.cars
    recent_fuel = []

    frames = []

    def emit(t, msg):
        frames.append((int(t * 1000), msg))

    emit(0, dict(type="ConnectionStatus", game_connected=True, plugin_version="1.4.2.0 (mock)",
                 telemetry_valid=True, telemetry_warning=None, session_active=True))

    dt = 0.1
    steps = int(args.duration / dt)
    for step in range(steps):
        t = step * dt
        driver = driver_at(t)
        i_drive = driver == me

        # --- advance every car ----------------------------------------------
        prog = []
        for c in cars:
            i = c["id"]
            in_pit = t < pit_until[i]
            # Time actually spent driving: frozen while the car sits in its box.
            driven = (pit_until[i] - pit_delay[i]) if in_pit else (t - pit_delay[i])
            total = c["start"] + driven / c["lap"]
            laps = int(math.floor(total)) if total >= 0 else 0
            if laps > laps_done[i]:
                lt = t - lap_start[i]
                last_lap[i] = lt
                best_lap[i] = lt if best_lap[i] < 0 else min(best_lap[i], lt)
                lap_start[i] = t
                laps_done[i] = laps
                if i == player_id:
                    used = fuel_per_lap * (1 + rng.uniform(-0.03, 0.03))
                    fuel = max(0.0, fuel - used)
                    recent_fuel.append(used)
                    stint_laps += 1
                    for w in range(4):
                        wear[w] = max(0.0, wear[w] - (0.006 if w == 0 else 0.004))
                    if args.pit_every and laps % args.pit_every == 0:
                        pit_until[i] = t + PIT_LOSS
                        pit_delay[i] += PIT_LOSS
                        pitstops[i] += 1
                        fuel, wear = fuel_cap, [1.0, 1.0, 1.0, 1.0]
                        stint, stint_laps, stint_start_fuel = stint + 1, 0, fuel
                        lock = dict(count=0, total=0.0, longest=0.0)
            prog.append((total, c))

        order = sorted(prog, key=lambda x: -x[0])
        leader_total = order[0][0]
        pos = {c["id"]: n + 1 for n, (_, c) in enumerate(order)}

        # --- lock-up on the front-left --------------------------------------
        locked_now = False
        if args.lockup_at >= 0 and args.lockup_at <= t < args.lockup_at + 0.6:
            locked_now = True
            if lock["count"] == 0:
                lock["count"] = 1
            lock["total"] = lock["longest"] = round(t - args.lockup_at + dt, 2)

        # --- ScoringUpdate at 5 Hz ------------------------------------------
        if step % 2 == 0:
            vehicles = []
            for total, c in prog:
                i = c["id"]
                frac = total - math.floor(total) if total >= 0 else 0.0
                ang = 2 * math.pi * frac
                behind_laps = leader_total - total
                is_team = i == player_id
                prev = next((o for o in order if pos[o[1]["id"]] == pos[i] - 1), None)
                lapt = c["lap"]
                vehicles.append(dict(
                    id=i, driver_name=(driver if is_team else c["name"]), team_name="",
                    vehicle_class=c["cls"], position=pos[i], lap_dist=frac * track_len,
                    total_laps=laps_done[i], best_lap_time=best_lap[i], last_lap_time=last_lap[i],
                    in_pits=t < pit_until[i],
                    last_sector1=(last_lap[i] * 0.30 if last_lap[i] > 0 else -1.0),
                    last_sector2=(last_lap[i] * 0.65 if last_lap[i] > 0 else -1.0),
                    cur_sector1=(lapt * 0.30 if frac > 0.30 else -1.0),
                    cur_sector2=(lapt * 0.65 if frac > 0.65 else -1.0),
                    best_sector1=(best_lap[i] * 0.30 if best_lap[i] > 0 else -1.0),
                    best_sector2=(best_lap[i] * 0.65 if best_lap[i] > 0 else -1.0),
                    lap_start_et=lap_start[i], car_number=i,
                    car_name=f"{c['model']} #{10 + i}",
                    last_sector3=(last_lap[i] * 0.35 if last_lap[i] > 0 else -1.0),
                    best_sector3=(best_lap[i] * 0.35 if best_lap[i] > 0 else -1.0),
                    pos_x=600 * math.cos(ang) + 150 * math.cos(3 * ang), pos_y=5 * math.sin(2 * ang),
                    pos_z=400 * math.sin(ang) + 100 * math.sin(2 * ang),
                    time_behind_leader=(behind_laps - math.floor(behind_laps)) * lapt,
                    laps_behind_leader=int(math.floor(behind_laps)),
                    virtual_energy=(max(0.0, 1 - (laps_done[i] % 12) * 0.08) if c["cls"] == "Hyper" else 0.0),
                    control=((0 if i_drive else 2) if is_team else 2),
                    time_behind_next=((prev[0] - total) * lapt if prev else 0.0),
                    laps_behind_next=0, num_pitstops=pitstops[i],
                    pit_state=(3 if t < pit_until[i] else 0),
                ))
            emit(t, dict(type="ScoringUpdate", session_type=args.session, session_time=t,
                         num_vehicles=len(vehicles), vehicles=vehicles, player_vehicle_id=player_id,
                         in_realtime=i_drive, player_name=me,
                         player_vehicle_model=cars[player_id]["model"]))

        # --- TelemetryUpdate at 10 Hz ---------------------------------------
        total = prog[player_id][0]
        frac = total - math.floor(total) if total >= 0 else 0.0
        in_pit = t < pit_until[player_id]
        speed = 0.0 if in_pit else 45 + 35 * math.sin(2 * math.pi * frac * 4)
        braking = (not in_pit) and math.cos(2 * math.pi * frac * 4) < -0.6
        avg = sum(recent_fuel[-10:]) / len(recent_fuel[-10:]) if recent_fuel else 0.0
        tires = []
        for w in range(4):
            base = 92 + 6 * math.sin(t / 7 + w) + (10 if w < 2 else 0)
            shown_wear = wear[w] if i_drive else 1.0
            tires.append(dict(
                temp_inner=base + 4, temp_mid=base, temp_outer=base - 5, carcass_temp=base - 8,
                pressure=172 + 4 * math.sin(t / 11 + w), wear=shown_wear,
                brake_temp=(420 + 180 * math.sin(t / 3 + w) if i_drive else 27.0),
                flat=False, detached=False, optimal_temp=95.0, surface=0,
                lockups=(lock["count"] if w == 0 and i_drive else 0),
                lockup_total_s=(lock["total"] if w == 0 and i_drive else 0.0),
                lockup_longest_s=(lock["longest"] if w == 0 and i_drive else 0.0),
                locked=(locked_now and w == 0 and i_drive),
            ))
        shown_fuel = fuel if i_drive else 50.0
        ang = 2 * math.pi * frac
        emit(t, dict(
            type="TelemetryUpdate", speed_ms=speed, rpm=4000 + speed * 55, max_rpm=8600,
            gear=(0 if in_pit else min(7, 1 + int(speed / 12))),
            throttle=(0.0 if braking or in_pit else 1.0), brake=(0.9 if braking else 0.0),
            clutch=0.0, steering=0.3 * math.sin(2 * math.pi * frac * 6),
            fuel=shown_fuel, fuel_capacity=fuel_cap, water_temp=84.0, oil_temp=96.0, tires=tires,
            position=dict(x=600 * math.cos(ang), y=0.0, z=400 * math.sin(ang)),
            velocity=dict(x=0.0, y=0.0, z=-speed), local_accel=dict(x=0.0, y=0.0, z=(-18.0 if braking else 3.0)),
            delta_best=0.2 * math.sin(t / 5), current_et=t, lap_start_et=lap_start[player_id],
            fuel_avg_consumption=avg, fuel_avg_sample_count=min(10, len(recent_fuel)),
            fuel_laps_remaining=(shown_fuel / avg if avg > 0 else 999.0),
            fuel_stint_number=stint, fuel_stint_laps=stint_laps,
            fuel_stint_consumption=stint_start_fuel - fuel, fuel_recommended=0.0,
            fuel_pit_detected=in_pit, fuel_avg_lap_time=(last_lap[player_id] if last_lap[player_id] > 0 else 0.0),
            ve_history=None, ve_available=True, heading_deg=(math.degrees(ang) + 90) % 360,
        ))

        # --- SessionInfo at 1 Hz --------------------------------------------
        if step % 10 == 0:
            fc = [dict(sky_type=s, temperature=24 + n, rain_chance=r, humidity=55.0, wind_direction=2, wind_speed=3.0)
                  for n, (s, r) in enumerate([(1, 0.0), (2, 0.1), (4, 0.3), (6, 0.6), (4, 0.2)])]
            emit(t, dict(type="SessionInfo", track_name=args.track, track_length=track_len,
                         weather=dict(air_temp=24.0, track_temp=36.0, rain_intensity=0.0, dark_cloud=0.2,
                                      cloudiness=1.0, avg_path_wetness=0.0, min_path_wetness=0.0,
                                      max_path_wetness=0.0, wind_speed_live=3.0, wind_rel_deg=40.0, forecast=fc),
                         session_laps=0, session_minutes=args.duration / 60.0))

    with open(args.out, "w", encoding="utf-8") as f:
        f.write("# Synthetic session from tools/mock-session/make_session.py - NOT real telemetry\n")
        f.write(f"# me={me} team={args.team or team[0][2]} seed={args.seed}\n")
        for t_ms, msg in frames:
            f.write(json.dumps({"t": t_ms, "j": msg}, separators=(",", ":")) + "\n")
    print(f"{args.out}: {len(frames)} frames, {args.duration:.0f} s, viewpoint of {me}")


if __name__ == "__main__":
    main()

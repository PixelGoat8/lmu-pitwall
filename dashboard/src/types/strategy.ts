// Mirrors bridge/src/strategy (StrategyReport) and bridge/src/pit_api.rs.

export interface Stat {
  n: number
  mean: number
  median: number
  last: number
  min: number
  max: number
  trend_per_lap: number | null
}

export interface Horizon {
  laps_to_go: number
  basis: 'time' | 'laps' | string
  time_remaining_s: number | null
}

export interface EnergyPlan {
  unit: 'L' | '%' | string   // '%' = values are fractions 0–1
  now: number
  capacity: number
  per_lap: Stat
  planning_per_lap: number
  reserve: number
  laps_left: number
  needed_to_finish: number | null
  to_add: number | null
  stint_laps_max: number
  stops: number | null
  rate: Rate
  save_stop: SaveStop | null
  target: EnergyTarget | null
}

/** The consumption the plan uses and where it comes from. */
export interface Rate {
  per_lap: number
  laps: number
  basis: 'stint' | 'lico' | 'previous stint' | string
  /** Lift-and-coast: seconds per lap coasting at speed, off both pedals. */
  coast_s: number | null
  coast_before_s: number | null
  per_lap_before: number | null
  last_lap: number | null
}

export interface SaveStop {
  stops: number
  per_lap: number
  saving: number               // fraction
  extra_coast_s: number | null
}

export interface EnergyTarget {
  /** Set as "box at the end of this lap"; null when set per lap. */
  box_lap: number | null
  laps_to_box: number | null
  per_lap: number
  off_target: number           // fraction: +0.05 = 5 % over the target
  laps_left: number
  stint_laps_max: number
  stops: number | null
  extra_coast_s: number | null
}

export interface PitWindow {
  earliest_in: number
  latest_in: number
  earliest_lap: number
  latest_lap: number
  stops: number
}

export interface PlanSummary {
  stops: number
  limiting: 'fuel' | 've' | 'stint time' | string
  stint_laps_max: number
  range_laps: number
  window: PitWindow | null
}

export interface WhatIf {
  in_laps: number
  pit_lap: number
  reachable: boolean
  fuel_add: number | null
  ve_add: number | null
  stops_after: number
  stationary_s: number | null
  stop_cost_s: number | null
}

export interface TyrePlan {
  set_laps: number
  wear_now: [number, number, number, number] | null
  wear_per_lap: [number, number, number, number] | null
  laps_to_limit: number | null
  limit: number
  pace_trend_s_per_lap: number | null
}

export interface TyreChoice {
  change_cost_s: number
  expected_gain_s: number | null
  must_change: boolean
}

export interface PitEstimate {
  total: number
  fuel: number
  ve: number
  tires: number
  driver_swap: number
  damage: number
  penalties: number
}

export interface StrategyConfig {
  reserve_laps: number
  max_stint_minutes: number | null
  tyre_wear_limit: number
  change_tyres: boolean
  pit_lane_loss_s: number | null
  ve_target_per_lap: number | null    // fraction per lap
  fuel_target_per_lap: number | null  // litres per lap
  ve_target_box_lap: number | null    // target in laps: box at the end of this lap
  fuel_target_box_lap: number | null
}

export interface StrategyUpdate {
  type: 'StrategyUpdate'
  track: string
  session_type: string
  session_time: number
  laps_completed: number
  lap_progress: number
  driver: string
  samples: number
  horizon: Horizon | null
  pace: { ref_lap_s: number | null; clean_laps: number; stat: Stat | null }
  fuel: EnergyPlan | null
  ve: EnergyPlan | null
  plan: PlanSummary | null
  /** The plan at the engineer's target consumption, when one is set. */
  plan_at_target?: PlanSummary | null
  what_if: WhatIf[]
  tyres: TyrePlan
  tyre_choice: TyreChoice | null
  stint: { started_at: number | null; elapsed_s: number | null; limit_s: number | null }
  pit_loss: { lane_loss_s: number | null; source: 'manual' | 'measured' | 'none' | string; samples: number; game_estimate: PitEstimate | null }
  weather: {
    raining_now: boolean
    wetness: number
    rain_in_s: number | null
    rain_chance: number | null
    rain_in_laps: number | null
    dry_in_s: number | null
    dry_in_laps: number | null
  }
  notes: string[]
  config: StrategyConfig
}

export interface PitInfoMsg {
  type: 'PitInfo'
}

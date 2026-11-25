// Mirrors bridge/src/history/store.rs.

export interface SessionSummary {
  id: number
  started_at: number      // unix seconds
  last_seen: number
  track: string
  session_type: string
  car_name: string
  car_class: string
  followed: boolean       // watched from the engineer seat
  laps: number
  drivers: string[]
  best_lap: number | null
}

export interface LapRow {
  lap: number
  session_time: number | null
  lap_time: number | null
  driver: string | null
  pit: boolean
  caution: boolean
  fuel: number | null
  ve: number | null
  wear: (number | null)[]
  class_position: number | null
}

export interface StintRow {
  number: number
  driver: string | null
  source: string | null
  start_time: number | null
  end_time: number | null
  start_lap: number | null
  end_lap: number | null
  laps: number
  best_lap: number | null
  avg_lap: number | null
  fuel_start: number | null
  fuel_end: number | null
  ve_start: number | null
  ve_end: number | null
}

export interface EventRow {
  t: number | null
  lap: number | null
  kind: string | null
  severity: string | null
  own: boolean
  text: string | null
}

/** A line of the change log: who changed the strategy, targets, pin or AI. */
export interface ChangeRow {
  at: number                 // Unix seconds
  session_time: number | null
  lap: number | null
  who: string
  text: string
}

export interface AiRow {
  lap: number | null
  trigger: string | null
  question: string | null
  text: string | null
  withheld: boolean
  unverified: string | null
  model: string | null
  cost_usd: number | null
}

export interface CarSummary {
  car_id: number
  car_name: string | null
  class: string | null
  drivers: string[]
  laps: number
  best_lap: number | null
  avg_lap: number | null
}

export interface SessionDetail {
  session: SessionSummary
  laps: LapRow[]
  stints: StintRow[]
  events: EventRow[]
  ai: AiRow[]
  changes?: ChangeRow[]      // absent from bridges older than the change log
  cars: CarSummary[]
  strategy_laps: number[]
}

export interface CarLapRow {
  car_id: number
  lap: number
  lap_time: number | null
  driver: string | null
  pit: boolean
}

export type HistoryMessage =
  | { type: 'HistorySessions'; sessions: SessionSummary[] }
  | ({ type: 'HistorySession' } & SessionDetail)
  | { type: 'HistoryCarLaps'; session_id: number; laps: CarLapRow[] }
  | { type: 'HistoryStrategy'; session_id: number; lap: number; json: string | null }
  | { type: 'HistoryError'; message: string }

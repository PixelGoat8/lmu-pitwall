// Mirrors bridge/src/race (RaceTiming).

export interface RaceEvent {
  t: number              // session time, s
  lap: number            // laps completed by our car
  kind: string           // fcy, fcy_end, pits, start, finish, pit_in, pit_out, penalty, driver, position, impact, blue_flag, best_lap, class_best
  severity: 'info' | 'warn' | 'alert' | string
  car_id: number | null
  own: boolean
  text: string
}

export interface LapTime {
  lap: number
  time: number
  pit: boolean
}

export interface RaceTiming {
  type: 'RaceTiming'
  events: RaceEvent[]
  laps: { id: number; laps: LapTime[] }[]
}

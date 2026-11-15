// Mirrors bridge/src/ai (AiFeed).

export interface AiConfig {
  enabled: boolean
  model: string
  effort: string
  every_laps: number
  on_events: boolean
  min_interval_s: number
  max_calls_per_hour: number
  daily_budget_usd: number
  language: string
  show_unverified: boolean
}

export interface AiComment {
  id: number
  session_time: number
  lap: number
  trigger: string
  question: string | null
  text: string
  unverified: string[]
  withheld: boolean
  refused: boolean
  model: string
  usage: { input_tokens: number; output_tokens: number; cache_read_input_tokens: number; cache_creation_input_tokens: number }
  cost_usd: number
  latency_ms: number
}

export interface AiStatus {
  enabled: boolean
  key_present: boolean
  busy: boolean
  calls_today: number
  cost_today_usd: number
  calls_last_hour: number
  blocked: string | null
  last_error: string | null
  config: AiConfig | null
}

export interface AiFeed {
  type: 'AiFeed'
  status: AiStatus
  comments: AiComment[]
}

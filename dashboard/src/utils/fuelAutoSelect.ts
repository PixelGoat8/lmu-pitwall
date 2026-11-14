// Matching the session in progress to the fuel calculator's options.
//
// The calculator's options come from past result files, keyed by the names
// the result XML uses (track venue / course, car type). The live session
// reports the track and the car model in its own words, and the two are not
// guaranteed to be spelled alike. So this matches loosely and prefers to give
// no answer over a wrong one: a wrong preselection would be worse than an
// empty field.

interface CarLike { car_class: string | null; car_name: string | null }
interface TrackLike<C extends CarLike> {
  track_venue: string
  track_course: string | null
  track_length: number | null
  cars: C[]
}

/** Lower-case letters and digits only: "Circuit de la Sarthe" → "circuitdelasarthe". */
function norm(s: string | null | undefined): string {
  return (s ?? '').toLowerCase().normalize('NFD').replace(/[^a-z0-9]/g, '')
}

/** 2 = same name, 1 = one contains the other, 0 = unrelated. */
function nameScore(a: string, b: string): number {
  if (!a || !b) return 0
  if (a === b) return 2
  // A containment only counts for names long enough not to match by accident.
  if (Math.min(a.length, b.length) >= 5 && (a.includes(b) || b.includes(a))) return 1
  return 0
}

/**
 * The track option for the live session, or `null` if none is a confident
 * match. A name match on venue or course is required. When several layouts
 * of one venue match by name, the track length picks the closest one, and
 * only if it is within 1 % and clearly closer than the runner-up.
 */
export function matchTrack<C extends CarLike, T extends TrackLike<C>>(
  tracks: T[],
  liveName: string,
  liveLengthM: number,
): T | null {
  const live = norm(liveName)
  if (!live) return null

  const scored = tracks
    .map((t) => ({ t, name: Math.max(nameScore(live, norm(t.track_venue)), nameScore(live, norm(t.track_course))) }))
    .filter((x) => x.name > 0)
  if (!scored.length) return null
  const top = Math.max(...scored.map((x) => x.name))
  const candidates = scored.filter((x) => x.name === top).map((x) => x.t)
  if (candidates.length === 1) return candidates[0]

  // Several layouts with the same name: let the length decide.
  if (liveLengthM <= 0) return null
  const byLength = candidates
    .filter((t) => !!t.track_length)
    .map((t) => ({ t, err: Math.abs((t.track_length as number) - liveLengthM) / liveLengthM }))
    .sort((a, b) => a.err - b.err)
  if (!byLength.length || byLength[0].err >= 0.01) return null
  if (byLength.length > 1 && byLength[1].err - byLength[0].err < 0.001) return null
  return byLength[0].t
}

/**
 * The car option for the live car, or `null`. Matches the model name first;
 * failing that, picks the only car of the player's class if there is exactly
 * one. Never guesses between several cars of the same class.
 */
export function matchCar<C extends CarLike>(cars: C[], liveModel: string, liveClass: string): C | null {
  const model = norm(liveModel)
  if (model) {
    const scored = cars
      .map((c) => ({ c, s: nameScore(model, norm(c.car_name)) }))
      .filter((x) => x.s > 0)
      .sort((a, b) => b.s - a.s)
    if (scored.length === 1 || (scored.length > 1 && scored[0].s > scored[1].s)) return scored[0].c
  }
  const cls = norm(liveClass)
  if (cls) {
    const sameClass = cars.filter((c) => norm(c.car_class) === cls)
    if (sameClass.length === 1) return sameClass[0]
  }
  return null
}

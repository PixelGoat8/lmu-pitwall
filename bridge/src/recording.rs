//! Record the bridge's broadcast stream to a file, and play one back.
//!
//! Two uses:
//!
//! * **Develop without the game.** `--replay session.pwrec` makes the bridge
//!   serve a recorded session exactly as it was broadcast, so the dashboard
//!   and everything downstream of the bridge can be worked on with LMU closed.
//! * **Simulate several drivers.** Each replaying bridge is a stand-in for one
//!   driver's PC, which is how the hub is tested on a single machine.
//!
//! # File format (`.pwrec`)
//!
//! Plain text, one frame per line, each a JSON object:
//!
//! ```text
//! {"t":1234,"m":"<base64 of the MessagePack message>"}
//! {"t":1250,"j":{"type":"ConnectionStatus","game_connected":true, ...}}
//! ```
//!
//! `t` is milliseconds since the start of the recording. `m` is the exact
//! MessagePack the bridge would send a client — lossless, which matters
//! because a message may hold values JSON cannot carry (an infinite or NaN
//! float). `j` is the same message
//! as plain JSON: lossy for those values, but writable by hand or by a script,
//! which is what synthetic test sessions use. The recorder writes `m`; the
//! player accepts both. Lines starting with `#` and blank lines are ignored.

use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use tokio::sync::{broadcast, watch};
use tracing::{info, warn};

use crate::protocol::messages::ServerMessage;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Pause between the end of a recording and its restart when looping.
const LOOP_GAP: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------------
// Recording
// ---------------------------------------------------------------------------

/// Write every broadcast message to `path` until the channel closes.
///
/// Runs beside the normal bridge: it is just one more subscriber, so recording
/// changes nothing about what clients receive. Per-client traffic (heartbeats,
/// command replies, engineer audio) is not on the broadcast channel and is not
/// recorded.
pub async fn record(path: std::path::PathBuf, mut rx: broadcast::Receiver<Arc<ServerMessage>>) {
    let file = match std::fs::File::create(&path) {
        Ok(f) => f,
        Err(e) => {
            warn!("Recording disabled: cannot create {:?}: {}", path, e);
            return;
        }
    };
    let mut out = std::io::BufWriter::new(file);
    let _ = writeln!(out, "# LMU Pitwall recording, bridge v{}", env!("CARGO_PKG_VERSION"));
    info!("Recording the broadcast stream to {:?}", path);

    let start = Instant::now();
    let mut last_flush = Instant::now();
    let mut frames = 0u64;

    loop {
        match rx.recv().await {
            Ok(msg) => {
                // Derived from the rest of the stream: a replay recomputes it,
                // with whatever version of the engine is running then.
                if matches!(&*msg, ServerMessage::StrategyUpdate(_) | ServerMessage::RaceTiming(_) | ServerMessage::AiFeed(_)) {
                    continue;
                }
                let Ok(bytes) = rmp_serde::to_vec_named(&*msg) else { continue };
                let line = format!(
                    "{{\"t\":{},\"m\":\"{}\"}}",
                    start.elapsed().as_millis(),
                    B64.encode(bytes)
                );
                if writeln!(out, "{line}").is_err() {
                    warn!("Recording stopped: write to {:?} failed", path);
                    return;
                }
                frames += 1;
                // Flush about once a second: a recording that is only on disk
                // after a clean exit is no use when the bridge is killed.
                if last_flush.elapsed() >= Duration::from_secs(1) {
                    let _ = out.flush();
                    last_flush = Instant::now();
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!("Recorder fell behind, {} messages not recorded", n);
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
    let _ = out.flush();
    info!("Recording finished: {} frames", frames);
}

// ---------------------------------------------------------------------------
// Playback
// ---------------------------------------------------------------------------

/// One recorded message and when it was sent.
#[derive(Debug, Clone)]
pub struct Frame {
    pub t_ms: u64,
    pub msg: rmpv::Value,
}

/// A parsed recording, frames in time order.
#[derive(Debug, Clone, Default)]
pub struct Recording {
    pub frames: Vec<Frame>,
}

impl Recording {
    pub fn load(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path).with_context(|| format!("cannot open {:?}", path))?;
        Self::parse(std::io::BufReader::new(file))
            .with_context(|| format!("cannot read recording {:?}", path))
    }

    pub fn parse(reader: impl BufRead) -> Result<Self> {
        let mut frames = Vec::new();
        for (i, line) in reader.lines().enumerate() {
            let line = line?;
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            frames.push(parse_line(line).with_context(|| format!("line {}", i + 1))?);
        }
        // Hand-written files need not be in order; playback assumes they are.
        frames.sort_by_key(|f| f.t_ms);
        Ok(Recording { frames })
    }

    pub fn duration(&self) -> Duration {
        Duration::from_millis(self.frames.last().map(|f| f.t_ms).unwrap_or(0))
    }
}

fn parse_line(line: &str) -> Result<Frame> {
    let obj: serde_json::Value = serde_json::from_str(line)?;
    let t_ms = obj.get("t").and_then(|v| v.as_u64()).ok_or_else(|| anyhow!("missing \"t\""))?;

    let msg = if let Some(b64) = obj.get("m").and_then(|v| v.as_str()) {
        let bytes = B64.decode(b64)?;
        rmpv::decode::read_value(&mut &bytes[..])?
    } else if let Some(json) = obj.get("j") {
        rmpv::ext::to_value(json)?
    } else {
        return Err(anyhow!("frame has neither \"m\" nor \"j\""));
    };

    if message_type(&msg).is_none() {
        return Err(anyhow!("message has no \"type\" field"));
    }
    Ok(Frame { t_ms, msg })
}

/// A typed message as the raw value a client would decode: a map keyed by
/// field name, exactly what goes on the wire.
///
/// The stream engines (strategy, race log, AI) read messages in this form,
/// whether they come from this bridge, a replay or a relayed bridge. Do not
/// use `rmpv::ext::to_value` for this: it encodes structs as positional
/// arrays, so every field lookup would fail and the engines would see
/// nothing from a live game.
pub fn to_raw(msg: &ServerMessage) -> Option<rmpv::Value> {
    if let ServerMessage::Raw(v) = msg {
        return Some(v.clone());
    }
    let bytes = rmp_serde::to_vec_named(msg).ok()?;
    rmpv::decode::read_value(&mut &bytes[..]).ok()
}

/// The `type` tag of a message held as a raw value.
pub fn message_type(msg: &rmpv::Value) -> Option<&str> {
    field(msg, "type")?.as_str()
}

/// Look up a top-level field of a message held as a raw value.
pub fn field<'a>(msg: &'a rmpv::Value, name: &str) -> Option<&'a rmpv::Value> {
    msg.as_map()?.iter().find(|(k, _)| k.as_str() == Some(name)).map(|(_, v)| v)
}

/// Serve `recording` as if it were live.
///
/// Frames go out on the broadcast channel at their recorded times (divided by
/// `speed`). The latest `ConnectionStatus` and `AllDriversUpdate` are also
/// kept in the watch channels the WebSocket server sends to a client on
/// connect, so a dashboard that joins mid-replay starts in the right state.
pub async fn replay(
    recording: Recording,
    speed: f64,
    looped: bool,
    tx: broadcast::Sender<Arc<ServerMessage>>,
    connection_status_tx: watch::Sender<Option<ServerMessage>>,
    all_drivers_tx: watch::Sender<Option<ServerMessage>>,
) {
    if recording.frames.is_empty() {
        warn!("Replay: the recording is empty, nothing to play");
        return;
    }
    let speed = if speed.is_finite() && speed > 0.0 { speed } else { 1.0 };
    info!(
        "Replay: {} frames, {:.0} s at {}x{}",
        recording.frames.len(),
        recording.duration().as_secs_f64(),
        speed,
        if looped { ", looping" } else { "" }
    );

    loop {
        let start = Instant::now();
        for frame in &recording.frames {
            let due = Duration::from_secs_f64(frame.t_ms as f64 / 1000.0 / speed);
            tokio::time::sleep_until((start + due).into()).await;

            let msg = ServerMessage::Raw(frame.msg.clone());
            match message_type(&frame.msg) {
                // Recomputed live by this bridge's own engine.
                Some("StrategyUpdate") | Some("RaceTiming") | Some("AiFeed") => continue,
                Some("ConnectionStatus") => {
                    let _ = connection_status_tx.send(Some(msg.clone()));
                }
                Some("AllDriversUpdate") => {
                    let _ = all_drivers_tx.send(Some(msg.clone()));
                }
                _ => {}
            }
            // No receivers just means no client is connected right now.
            let _ = tx.send(Arc::new(msg));
        }
        if !looped {
            info!("Replay finished");
            return;
        }
        tokio::time::sleep(LOOP_GAP).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(text: &str) -> Result<Recording> {
        Recording::parse(std::io::Cursor::new(text))
    }

    #[test]
    fn json_frames_are_read_and_sorted() {
        let r = rec(concat!(
            "# a comment\n",
            "\n",
            "{\"t\":500,\"j\":{\"type\":\"SessionInfo\",\"track_name\":\"Monza\"}}\n",
            "{\"t\":20,\"j\":{\"type\":\"ConnectionStatus\",\"game_connected\":true}}\n",
        ))
        .unwrap();
        assert_eq!(r.frames.len(), 2);
        assert_eq!(r.frames[0].t_ms, 20);
        assert_eq!(message_type(&r.frames[0].msg), Some("ConnectionStatus"));
        assert_eq!(field(&r.frames[1].msg, "track_name").and_then(|v| v.as_str()), Some("Monza"));
        assert_eq!(r.duration(), Duration::from_millis(500));
    }

    /// What the recorder writes must come back as the same message — including
    /// the infinity that JSON would have turned into null.
    #[test]
    fn a_recorded_message_survives_the_round_trip() {
        let original = ServerMessage::Heartbeat { server_time_ms: 42 };
        let line = format!(
            "{{\"t\":7,\"m\":\"{}\"}}",
            B64.encode(rmp_serde::to_vec_named(&original).unwrap())
        );
        let r = rec(&line).unwrap();
        assert_eq!(message_type(&r.frames[0].msg), Some("Heartbeat"));
        assert_eq!(field(&r.frames[0].msg, "server_time_ms").and_then(|v| v.as_u64()), Some(42));

        // Re-sent as a Raw message, the bytes on the wire are the original ones.
        let replayed = ServerMessage::Raw(r.frames[0].msg.clone());
        assert_eq!(
            rmp_serde::to_vec_named(&replayed).unwrap(),
            rmp_serde::to_vec_named(&original).unwrap()
        );

        let inf = rmpv::Value::Map(vec![
            (rmpv::Value::from("type"), rmpv::Value::from("TelemetryUpdate")),
            (rmpv::Value::from("laps_left"), rmpv::Value::F64(f64::INFINITY)),
        ]);
        let mut bytes = Vec::new();
        rmpv::encode::write_value(&mut bytes, &inf).unwrap();
        let r = rec(&format!("{{\"t\":0,\"m\":\"{}\"}}", B64.encode(bytes))).unwrap();
        assert_eq!(
            field(&r.frames[0].msg, "laps_left").and_then(|v| v.as_f64()),
            Some(f64::INFINITY)
        );
    }

    #[test]
    fn broken_lines_are_reported_with_their_number() {
        let err = rec("{\"t\":0,\"j\":{\"type\":\"X\"}}\n{\"t\":5}\n").unwrap_err();
        assert!(format!("{err:#}").contains("line 2"), "{err:#}");
        assert!(rec("{\"t\":0,\"j\":{\"no_type\":1}}\n").is_err());
        assert!(rec("not json\n").is_err());
    }
}

#[cfg(test)]
mod live_conversion_tests {
    use crate::protocol::messages::ServerMessage;

    /// The stream engines read the bridge's own typed messages as raw
    /// values, the way replays deliver them. This failed live once: struct
    /// fields came out as positional arrays and the engines saw nothing.
    #[test]
    fn a_typed_message_converts_to_a_tagged_value() {
        let msg = ServerMessage::Heartbeat { server_time_ms: 5 };
        let v = super::to_raw(&msg).expect("convertible");
        assert_eq!(super::message_type(&v), Some("Heartbeat"));
        assert_eq!(super::field(&v, "server_time_ms").and_then(|x| x.as_u64()), Some(5));
        // A raw message passes through unchanged.
        let raw = ServerMessage::Raw(v.clone());
        assert_eq!(super::to_raw(&raw), Some(v));
    }
}

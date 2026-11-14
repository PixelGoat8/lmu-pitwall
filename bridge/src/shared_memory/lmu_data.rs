//! Layout of LMU's built-in `LMU_Data` shared memory.
//!
//! Le Mans Ultimate publishes this mapping itself, with no plugin installed
//! and nothing to configure beyond *Settings → Gameplay → Enable Plugins*. It
//! replaces the five `$rFactor2SMMP_*$` buffers this bridge used to read, and
//! with them the dependency on a third-party DLL that has to be rebuilt for
//! every game update.
//!
//! Derived from LMU's official `SharedMemoryInterface.hpp` (S397 ships it in
//! the game's `Support\SharedMemoryInterface` folder), cross-checked against
//! <https://github.com/TinyPedal/pyLMUSharedMemory>, and verified against a
//! running LMU 1.4 by `tools/lmu-probe` — see that tool's README for what the
//! four recorded sessions did and did not prove.
//!
//! # What this file does *not* define
//!
//! `rF2ScoringInfo`, `rF2VehicleScoring` and `rF2VehicleTelemetry` are the same
//! structs the plugin used, and they stay in [`super::types`]. LMU embeds the
//! rF2 records verbatim and only wraps them differently — so the interesting
//! part of the port is the container, not the payload.
//!
//! # Two things that differ from the plugin buffers
//!
//! * **104 slots, not 128.** [`MAX_MAPPED_VEHICLES`] here is LMU's number; the
//!   one in [`super::types`] is the plugin's. Mixing them reads past the end of
//!   the vehicle arrays, so this module keeps its own.
//! * **No version block.** Every rF2 plugin buffer began with
//!   `mVersionUpdateBegin`/`mVersionUpdateEnd`, which let a reader bracket a
//!   copy and detect that a write had landed in the middle of it. `LMU_Data`
//!   has no such pair; [`super::reader`] substitutes a witness re-read.

#![allow(non_snake_case)]

use super::types::{rF2ScoringInfo, rF2VehicleScoring, rF2VehicleTelemetry, rF2Wheel};

/// The Windows named mapping LMU publishes. No `$…$` decoration, unlike the
/// plugin's names.
pub const LMU_DATA_NAME: &str = "LMU_Data";

/// Vehicle slots in `LMU_Data`. **Not** the same as
/// [`super::types::MAX_MAPPED_VEHICLES`], which is the plugin's 128.
pub const MAX_MAPPED_VEHICLES: usize = 104;

/// Compile-time `size_of` assertion.
///
/// These are the load-bearing part of this file. Every offset below is implied
/// by the sizes of what precedes it, so if S397 adds a field to any embedded
/// struct, the build breaks here rather than the dashboard quietly showing tire
/// temperatures read from the wrong bytes — which is exactly the failure a null
/// check cannot catch.
macro_rules! assert_size {
    ($t:ty, $n:expr) => {
        const _: () = assert!(std::mem::size_of::<$t>() == $n);
    };
}

assert_size!(rF2ScoringInfo, 548);
assert_size!(rF2VehicleScoring, 584);
assert_size!(rF2VehicleTelemetry, 1888);
assert_size!(rF2Wheel, 260);

// ---------------------------------------------------------------------------
// Containers (SharedMemoryInterface.hpp: SharedMemoryObjectOut)
// ---------------------------------------------------------------------------

/// The event queue: `SharedMemoryEvent events[SME_MAX]`.
///
/// **Not counters, despite the field names below.** The game's header calls it
/// a "queue of events": each slot holds the *type* of one event that fired for
/// the current frame (a `SME_*` value), and unused slots hold `SME_MAX` (16).
/// Measured on LMU 1.4.2 — on track `[15, 11, 16, 16, …]` (FFB, telemetry
/// update), and after leaving a session `[5, 16, 16, …]` (unload). The field
/// names are kept from the first port because only the size matters to the
/// layout; read the block through [`OFF_EVENTS`] and [`session_unloaded`].
#[repr(C, packed(4))]
#[derive(Clone, Copy)]
pub struct LmuEvent {
    pub SME_ENTER: u32,
    pub SME_EXIT: u32,
    pub SME_STARTUP: u32,
    pub SME_SHUTDOWN: u32,
    pub SME_LOAD: u32,
    pub SME_UNLOAD: u32,
    pub SME_START_SESSION: u32,
    pub SME_END_SESSION: u32,
    pub SME_ENTER_REALTIME: u32,
    pub SME_EXIT_REALTIME: u32,
    pub SME_UPDATE_SCORING: u32,
    pub SME_UPDATE_TELEMETRY: u32,
    pub SME_INIT_APPLICATION: u32,
    pub SME_UNINIT_APPLICATION: u32,
    pub SME_SET_ENVIRONMENT: u32,
    pub SME_FFB: u32,
}
assert_size!(LmuEvent, 64);

#[repr(C, packed(4))]
#[derive(Clone, Copy)]
pub struct LmuApplicationState {
    pub mAppWindow: u64,
    pub mWidth: u32,
    pub mHeight: u32,
    pub mRefreshRate: u32,
    pub mWindowed: u32,
    pub mOptionsLocation: u8,
    pub mOptionsPage: [u8; 31],
    pub mExpansion: [u8; 204],
}
assert_size!(LmuApplicationState, 260);

#[repr(C, packed(4))]
#[derive(Clone, Copy)]
pub struct LmuGeneric {
    pub events: LmuEvent,
    /// Build number, e.g. `14000` for LMU 1.4. Replaces the plugin version
    /// string the bridge used to fish out of the Extended buffer, and unlike
    /// that string it is written by the game itself.
    pub gameVersion: i32,
    pub FFBTorque: f32,
    pub appInfo: LmuApplicationState,
}
assert_size!(LmuGeneric, 332);

#[repr(C, packed(4))]
#[derive(Clone, Copy)]
pub struct LmuPathData {
    pub userData: [u8; 260],
    pub customVariables: [u8; 260],
    pub stewardResults: [u8; 260],
    pub playerProfile: [u8; 260],
    pub pluginsFolder: [u8; 260],
}
assert_size!(LmuPathData, 1300);

#[repr(C, packed(4))]
#[derive(Clone, Copy)]
pub struct LmuScoringData {
    pub scoringInfo: rF2ScoringInfo,
    pub scoringStreamSize: [u8; 12],
    pub vehScoringInfo: [rF2VehicleScoring; MAX_MAPPED_VEHICLES],
    pub scoringStream: [u8; 65536],
}
assert_size!(LmuScoringData, 126_832);

#[repr(C, packed(4))]
#[derive(Clone, Copy)]
pub struct LmuTelemetryData {
    pub activeVehicles: u8,
    /// Index into `telemInfo`. The plugin path had to scan for `mIsPlayer` and
    /// then match IDs across two buffers; LMU hands the slot over directly.
    pub playerVehicleIdx: u8,
    pub playerHasVehicle: u8,
    pub telemInfo: [rF2VehicleTelemetry; MAX_MAPPED_VEHICLES],
}
assert_size!(LmuTelemetryData, 196_356);

/// The complete mapping: `SharedMemoryObjectOut`.
///
/// Never read as a whole — at 324 KB a full copy would take longer than the
/// 10 ms between telemetry writes is worth risking. [`super::reader`] copies
/// the header scalars and only the occupied vehicle slots.
#[repr(C, packed(4))]
#[derive(Clone, Copy)]
pub struct LmuObjectOut {
    pub generic: LmuGeneric,
    pub paths: LmuPathData,
    pub scoring: LmuScoringData,
    pub telemetry: LmuTelemetryData,
}
assert_size!(LmuObjectOut, 324_820);

// ---------------------------------------------------------------------------
// Byte offsets used by the reader
//
// Spelled out with `offset_of!` rather than by hand: these are what the reader
// addresses instead of copying whole containers, and a hand-counted offset is
// the one part of this file the size assertions above could not protect.
// ---------------------------------------------------------------------------

use std::mem::offset_of;

/// `generic.events[0]` — the event queue, [`EVENT_SLOTS`] × `u32`.
pub const OFF_EVENTS: usize = offset_of!(LmuObjectOut, generic) + offset_of!(LmuGeneric, events);

/// Slots in the event queue (`SME_MAX`).
pub const EVENT_SLOTS: usize = 16;

/// `SME_UNLOAD`: the session's content was unloaded (back to the menus).
pub const SME_UNLOAD: u32 = 5;

/// True when the event queue says the session was unloaded.
///
/// This matters because **LMU does not clear the mapping when a session
/// ends**. Measured on 1.4.2: minutes after returning to the main menu,
/// `mNumVehicles` was still 37, `playerHasVehicle` still 1 and every telemetry
/// row still held its last value. Nothing writes to the mapping after the
/// unload, so the queue keeps showing `SME_UNLOAD` until the next session
/// starts — which makes it the one field that tells a finished session from a
/// live one. Without this check the bridge serves a ghost session, and a hub
/// following several drivers would believe one of them is still on track.
pub fn session_unloaded(events: &[u32; EVENT_SLOTS]) -> bool {
    events.contains(&SME_UNLOAD)
}

/// `generic.gameVersion`.
pub const OFF_GAME_VERSION: usize =
    offset_of!(LmuObjectOut, generic) + offset_of!(LmuGeneric, gameVersion);

/// `scoring.scoringInfo` — the session-wide record.
pub const OFF_SCORING_INFO: usize =
    offset_of!(LmuObjectOut, scoring) + offset_of!(LmuScoringData, scoringInfo);

/// `scoring.scoringInfo.mCurrentET`, the 5 Hz scoring clock. Used as the
/// witness that no scoring write landed inside our copy.
pub const OFF_SCORING_ET: usize = OFF_SCORING_INFO + offset_of!(rF2ScoringInfo, mCurrentET);

/// `scoring.scoringInfo.mNumVehicles`.
pub const OFF_SCORING_NUM_VEHICLES: usize =
    OFF_SCORING_INFO + offset_of!(rF2ScoringInfo, mNumVehicles);

/// `scoring.vehScoringInfo[0]`.
pub const OFF_VEH_SCORING: usize =
    offset_of!(LmuObjectOut, scoring) + offset_of!(LmuScoringData, vehScoringInfo);

/// `telemetry.activeVehicles`.
pub const OFF_ACTIVE_VEHICLES: usize =
    offset_of!(LmuObjectOut, telemetry) + offset_of!(LmuTelemetryData, activeVehicles);

/// `telemetry.playerVehicleIdx`.
pub const OFF_PLAYER_IDX: usize =
    offset_of!(LmuObjectOut, telemetry) + offset_of!(LmuTelemetryData, playerVehicleIdx);

/// `telemetry.playerHasVehicle`.
pub const OFF_PLAYER_HAS_VEHICLE: usize =
    offset_of!(LmuObjectOut, telemetry) + offset_of!(LmuTelemetryData, playerHasVehicle);

/// `telemetry.telemInfo[0]`.
pub const OFF_TELEM_INFO: usize =
    offset_of!(LmuObjectOut, telemetry) + offset_of!(LmuTelemetryData, telemInfo);

/// `rF2VehicleTelemetry::mElapsedTime`, the 100 Hz physics clock. The other
/// witness: telemetry ticks twenty times per scoring tick, so it is the field
/// most likely to move underneath a copy.
pub const OFF_VEH_ELAPSED: usize = offset_of!(rF2VehicleTelemetry, mElapsedTime);

#[cfg(test)]
mod tests {
    use super::*;

    /// The offsets are what the reader dereferences, so an arithmetic slip here
    /// produces exactly the plausible-looking garbage the size assertions exist
    /// to prevent. Check them against the layout independently.
    #[test]
    fn offsets_land_inside_the_mapping() {
        let size = std::mem::size_of::<LmuObjectOut>();
        for (name, off, len) in [
            ("events", OFF_EVENTS, 4 * EVENT_SLOTS),
            ("gameVersion", OFF_GAME_VERSION, 4),
            ("scoringInfo", OFF_SCORING_INFO, std::mem::size_of::<rF2ScoringInfo>()),
            ("mCurrentET", OFF_SCORING_ET, 8),
            ("mNumVehicles", OFF_SCORING_NUM_VEHICLES, 4),
            (
                "vehScoringInfo",
                OFF_VEH_SCORING,
                std::mem::size_of::<rF2VehicleScoring>() * MAX_MAPPED_VEHICLES,
            ),
            ("activeVehicles", OFF_ACTIVE_VEHICLES, 1),
            ("playerVehicleIdx", OFF_PLAYER_IDX, 1),
            ("playerHasVehicle", OFF_PLAYER_HAS_VEHICLE, 1),
            (
                "telemInfo",
                OFF_TELEM_INFO,
                std::mem::size_of::<rF2VehicleTelemetry>() * MAX_MAPPED_VEHICLES,
            ),
        ] {
            assert!(
                off + len <= size,
                "{name} at {off}+{len} runs past the {size}-byte mapping",
            );
        }
    }

    /// The scoring block precedes the telemetry block, and the vehicle arrays
    /// sit inside their own containers. Ordering mistakes would still satisfy
    /// the bounds check above.
    #[test]
    fn blocks_are_in_the_documented_order() {
        assert!(OFF_GAME_VERSION < OFF_SCORING_INFO);
        assert!(OFF_SCORING_INFO < OFF_VEH_SCORING);
        assert!(OFF_VEH_SCORING < OFF_ACTIVE_VEHICLES);
        assert!(OFF_ACTIVE_VEHICLES < OFF_TELEM_INFO);
        assert_eq!(OFF_PLAYER_IDX, OFF_ACTIVE_VEHICLES + 1);
        assert_eq!(OFF_PLAYER_HAS_VEHICLE, OFF_ACTIVE_VEHICLES + 2);
    }

    /// The queue is the first thing in the mapping and exactly fills
    /// `LmuEvent`, so reading 16 × u32 at `OFF_EVENTS` stays inside it.
    #[test]
    fn event_queue_is_the_first_block() {
        assert_eq!(OFF_EVENTS, 0);
        assert_eq!(4 * EVENT_SLOTS, std::mem::size_of::<LmuEvent>());
    }

    /// The two queues actually observed on LMU 1.4.2, plus an empty one.
    #[test]
    fn unload_is_recognised_in_the_event_queue() {
        let mut after_exit = [16u32; EVENT_SLOTS];
        after_exit[0] = SME_UNLOAD;
        assert!(session_unloaded(&after_exit));

        let mut on_track = [16u32; EVENT_SLOTS];
        on_track[0] = 15; // SME_FFB
        on_track[1] = 11; // SME_UPDATE_TELEMETRY
        assert!(!session_unloaded(&on_track));

        assert!(!session_unloaded(&[16u32; EVENT_SLOTS]));
        assert!(!session_unloaded(&[0u32; EVENT_SLOTS]));
    }

    /// LMU maps 104 slots. The plugin mapped 128, and its constant used to live
    /// in `types.rs` next to these structs — reading a vehicle array with the
    /// wrong one walks off the end of the mapping. The plugin's constant is
    /// gone with the plugin; this pins the survivor so a future edit cannot
    /// quietly reintroduce the mismatch.
    #[test]
    fn slot_count_is_lmus_own() {
        assert_eq!(MAX_MAPPED_VEHICLES, 104);
        assert_eq!(
            OFF_TELEM_INFO + std::mem::size_of::<rF2VehicleTelemetry>() * MAX_MAPPED_VEHICLES,
            std::mem::size_of::<LmuObjectOut>(),
            "telemInfo is the last block, so its end must be the end of the mapping",
        );
    }
}

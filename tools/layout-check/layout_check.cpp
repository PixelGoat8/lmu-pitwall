// Prints sizeof/offsetof of LMU's own shared-memory structs, so the Rust
// layout in bridge/src/shared_memory/lmu_data.rs can be checked against the
// headers that ship with the installed game (which we may not redistribute).
//
// Build (x64 Native Tools prompt), see README.md:
//   cl /nologo /EHsc /std:c++17 /I "<LMU>\Support\SharedMemoryInterface" layout_check.cpp
#include <windows.h>
#include <cstdint>  // InternalsPlugin.hpp uses uint8_t without including it
#include <cstddef>
#include <cstdio>
#include "SharedMemoryInterface.hpp"

#define SZ(t) std::printf("sizeof %-28s %zu\n", #t, sizeof(t))
#define OFF(t, f) std::printf("offset %-28s %zu\n", #t "." #f, offsetof(t, f))

int main() {
    SZ(ScoringInfoV01);
    SZ(VehicleScoringInfoV01);
    SZ(TelemInfoV01);
    SZ(TelemWheelV01);
    SZ(ApplicationStateV01);
    SZ(SharedMemoryGeneric);
    SZ(SharedMemoryPathData);
    SZ(SharedMemoryScoringData);
    SZ(SharedMemoryTelemetryData);
    SZ(SharedMemoryObjectOut);
    OFF(SharedMemoryGeneric, gameVersion);
    OFF(SharedMemoryScoringData, scoringStreamSize);
    OFF(SharedMemoryScoringData, vehScoringInfo);
    OFF(SharedMemoryTelemetryData, telemInfo);
    OFF(SharedMemoryObjectOut, scoring);
    OFF(SharedMemoryObjectOut, telemetry);
    OFF(TelemInfoV01, mVirtualEnergy);
    OFF(TelemInfoV01, mWheel);
    OFF(VehicleScoringInfoV01, mControl);
    OFF(ScoringInfoV01, mAvgPathWetness);
    return 0;
}

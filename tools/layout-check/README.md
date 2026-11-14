# layout-check

Prints `sizeof` and key offsets of LMU's shared-memory structs, compiled from
the headers that ship with the installed game
(`Le Mans Ultimate\Support\SharedMemoryInterface\*.hpp`). Those headers may not
be redistributed, so they are included from the game folder and not copied here.

Compare the output with the `assert_size!` lines in
`bridge/src/shared_memory/lmu_data.rs` after every game update.

## Build and run (Windows, VS Build Tools with C++)

```bat
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
cl /nologo /EHsc /std:c++17 /I "G:\SteamLibrary\steamapps\common\Le Mans Ultimate\Support\SharedMemoryInterface" layout_check.cpp
.\layout_check.exe
```

## Last result: LMU 1.4.2 (`gameVersion` 14200, Steam build 25661166), 2026-11-14

| Struct / offset | C++ | Rust | |
|---|---|---|---|
| `ScoringInfoV01` | 548 | 548 | ok |
| `VehicleScoringInfoV01` | 584 | 584 | ok |
| `TelemInfoV01` | 1888 | 1888 | ok |
| `TelemWheelV01` | 260 | 260 | ok |
| `SharedMemoryGeneric` | 332 | 332 | ok |
| `SharedMemoryScoringData` | 126832 | 126832 | ok |
| `SharedMemoryTelemetryData` | 196356 | 196356 | ok |
| `SharedMemoryObjectOut` | 324824 | 324820 | ok: 4 bytes of C++ tail padding, no field affected |
| `.scoring` offset | 1632 | 1632 | ok |
| `.telemetry` offset | 128464 | 128464 | ok |

The `SharedMemoryInterface.hpp` containers are declared after
`#pragma pack(pop)`, so they use default (8-byte) packing. That is why
`scoringStreamSize` (a `size_t`) sits at offset 552, not 548. The Rust side
models that gap as `scoringStreamSize: [u8; 12]`.

# Development environment snapshot

Observed on 2026-08-26 before implementation:

| Area | Result |
|---|---|
| Working directory | Project workspace (initially empty) |
| Host OS | Windows 11 Home, build 26200.9168 |
| Development OS | Ubuntu 26.04 LTS on WSL2, kernel 6.18.33.2 |
| CPU | AMD Ryzen 7 5700X; Windows reports 8C/16T, WSL exposes 12 logical CPUs |
| Memory | 19 GiB visible to WSL, 8 GiB swap |
| GPU | NVIDIA GeForce RTX 3070, 8 GiB; WSL `/dev/dxg`; CUDA UMD 13.3 |
| Rust | rustc/cargo 1.97.1 |
| C/C++ | Clang 21.1.8, GCC 15.2.0, CMake 4.2.3, Ninja 1.13.2 |
| Other | Python 3.14.4, uv 0.11.28, Node 24.16.0, .NET 10.0.103 |
| Windows SDKs found | 10.0.22621.0, 10.0.26100.0, 10.0.28000.0 |
| WSL audio | WSLg PulseAudio sockets exist; `/dev/snd` contains only `timer` |
| Rust targets | Linux GNU, Windows GNU, Windows MSVC, WASI p2 installed |

Consequences:

- WSL can build, unit-test, benchmark, and run the paced realtime simulator.
- Windows-native duplex and device-hotplug behavior require a Windows executable and real-device run.
- The RTX 3070 is useful for experiments, but CPU performance remains the release baseline.
- WDK/Visual Studio driver build availability was not established; no driver is built or installed in this milestone.

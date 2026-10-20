# Flower

Flower is an independent Windows Text Services Framework (TSF) Chinese Pinyin
input method. The current source provides the librime engine adapter, Windows
key translation, the initial offline Pinyin schema, and a TSF COM server
activation/key-sink foundation. Composition handling, candidate UI,
registration, and the per-user installer are still in progress.

## Dependencies

- CMake 3.24 or newer
- Visual Studio 2022 with the C++ desktop workload and Windows SDK
- [librime 1.17.0](third_party/librime) and its pinned recursive submodules
- [rime-pinyin-simp](third_party/rime-pinyin-simp), pinned to commit
  `0c6861ef7420ee780270ca6d993d18d4101049d0` (Apache-2.0)

`scripts/build-librime.ps1` builds the pinned librime sources for one Windows
architecture. It requires a matching Visual Studio C++ toolchain and a
caller-provided Boost source tree through `BOOST_ROOT`; it does not download
or install Boost.

## Configure and test

Run from a Visual Studio 2022 developer shell:

```powershell
cmake -S . -B build-x64 -G "Visual Studio 17 2022" -A x64
cmake --build build-x64 --config Release
ctest --test-dir build-x64 -C Release --output-on-failure

cmake -S . -B build-x86 -G "Visual Studio 17 2022" -A Win32
cmake --build build-x86 --config Release
ctest --test-dir build-x86 -C Release --output-on-failure
```

The current development environment has CMake at
`C:\Program Files\CMake\bin\cmake.exe`; a Visual Studio C++ compiler and Windows
SDK were not detected when this project was started.

## Third-party licenses

See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and the license files in
the pinned submodules before redistributing a built package.

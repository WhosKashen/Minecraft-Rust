# Rustcraft: a Rust + egui Minecraft Java launcher for Android

**Status: source-complete first pass, NOT yet compiled, NOT yet run on a device.** It was written in a sandbox with no Rust,
Android SDK/NDK, JDK 25 or network access, so nothing here has been built or executed. Read `docs/COMPATIBILITY.md` for the
exact verified / unverified split before relying on any claim.

What is in the box
- `crates/launcher-core`: pure-Rust launcher logic (JAR inspection, Mojang manifest/libraries/assets, launch plan, accounts, servers,
  profiles, mods, resource/shader packs, themes, logs, touch/controller input engine) with unit tests for the host.
- `crates/launcher-ui`: egui UI (Play, Profiles, Accounts, Mods, Resource Packs, Shaders, Controls, Settings, Logs) + JNI bridge.
- `android/`: Gradle project, Kotlin activities (landscape-only, cutout-aware, Keystore secrets, SAF file picker, game window + input),
  and `gamehost.c` (starts the Android-built OpenJDK in-process and runs Minecraft's main class).
- `.github/workflows/android.yml`: builds the debug APK and runs the core unit tests.

What is deliberately NOT here (needs components this repo does not contain; see `docs/NATIVE_BRIDGE.md`)
- A Java 25 runtime built for Android arm64.
- An Android-compatible LWJGL with SDL support (Minecraft 26.3 uses SDL, not GLFW).
- The OpenGL-3.3-on-GLES translation layer / Vulkan windowing glue that lets the JAR's renderer draw to the Android surface.
Until those are supplied and integrated, the launcher can prepare files and build the exact launch command, but the game will not render.

Docs: `docs/BUILD.md`, `docs/USER_GUIDE.md`, `docs/COMPATIBILITY.md`, `docs/ARCHITECTURE.md`, `docs/NATIVE_BRIDGE.md`, `docs/THIRD_PARTY.md`.
Licence of this code: LGPL-3.0-or-later (add the licence text as `LICENSE` before publishing). Minecraft is a trademark of Mojang/Microsoft;
this project is not affiliated with them. You must own Minecraft: Java Edition and supply your own client JAR.

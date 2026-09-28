# Mesh LLM Desktop

A standalone desktop app for Mesh LLM. Double-click it, choose how this
computer should take part in the mesh, and it runs a local node for you and
shows the Mesh LLM console in its own window. No terminal needed.

- **Use the mesh**: runs `mesh-llm client`. Joins a mesh and makes its models
  available at `http://localhost:9337/v1` without a GPU or a model download.
- **Share my GPU**: runs `mesh-llm serve`. Serves a model on this machine
  and contributes it to the mesh. The first start downloads a suitable model.

Leave the invite token empty to join the best public mesh (`--auto`), or paste
a private mesh invite token under **Advanced**. The token is passed to
mesh-llm through `MESH_LLM_JOIN`, not the command line, so it does not show
up in the process list. With **Start this way automatically next time** ticked,
later launches skip the launcher and start the node right away.

The app keeps running in the system tray when its window is closed, so the
local OpenAI-compatible API stays up. The tray menu can show the window, open
the console in a browser, change mode, restart the node, open the log folder,
and quit. Quitting stops the node the app started.

If a mesh-llm node is already running on the console port (for example one
you started from a terminal), the app connects to it instead of starting a
second one, and quitting the app leaves it running.

## How it works

```
desktop/
  ui/                 launcher page (plain HTML/CSS/JS, no bundler)
  src-tauri/
    src/main.rs       Tauri setup, window close → tray, exit cleanup
    src/node.rs       node lifecycle: attach or start, readiness, crash watch
    src/sidecar.rs    locate/spawn/stop the mesh-llm binary
    src/readiness.rs  GET /api/status probe on the console port
    src/settings.rs   saved launcher choices (app config dir, settings.json)
    src/logs.rs       sidecar log file and recent-line buffer
    src/tray.rs       tray icon and menu
    src/commands.rs   IPC commands used by the launcher page
```

The app ships the regular `mesh-llm` binary as a Tauri sidecar and never links
mesh-llm code itself. It starts it as
`mesh-llm --log-format json <client|serve> [--auto] --port <api> --console <console>`,
polls `GET /api/status` on the console port, and once that answers it
navigates the main window to `http://127.0.0.1:<console>/`, the same React
console `mesh-llm` serves in a browser. The console page is a remote origin
and gets no access to the app's IPC commands; only the bundled launcher page
does.

The native runtime packaged next to the host is shipped as an app resource
and passed to mesh-llm through `MESH_LLM_NATIVE_RUNTIME_BUNDLE_DIR`. The app
looks for mesh-llm in this order: `MESH_LLM_DESKTOP_SIDECAR`, the bundled
`mesh-llm-sidecar` next to the app executable, then `mesh-llm` on `PATH`. The
bundled copy has its own name so the Linux package never collides with a
separately installed `mesh-llm` CLI in `/usr/bin`. It is never started with
`--auto-update`, so it does not replace itself inside the app bundle.

On quit the app sends SIGTERM (macOS/Linux) so mesh-llm removes its runtime
directory, and force-stops it after 10 seconds. On Linux the sidecar also gets
SIGTERM if the app itself is killed. On Windows the sidecar is terminated; any
stale runtime directory is cleaned up by the next mesh-llm start.

Sidecar output is written to `mesh-llm.log` in the app log directory
(Linux `~/.local/share/cloud.meshllm.desktop/logs`, macOS
`~/Library/Logs/cloud.meshllm.desktop`, Windows
`%LOCALAPPDATA%\cloud.meshllm.desktop\logs`).

## Prerequisites

- Rust (the repository toolchain) and Node.js 24. The Tauri CLI is pinned in
  `desktop/package-lock.json`; the `just desktop-*` recipes install it with
  `npm ci`.
- Linux: `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev`
  (Debian/Ubuntu package names). Without an appindicator host the app has no
  tray icon and closing the window quits it.
- macOS: Xcode command line tools.
- Windows: WebView2 (preinstalled on Windows 10/11) and the MSVC toolchain.

The app is its own cargo workspace. It is excluded from the root workspace
because it needs the platform webview libraries above.

## Develop

```bash
just build        # debug mesh-llm product (host + native runtime)
just desktop-dev  # run the app against target/debug/mesh-llm
just desktop-check  # fmt, clippy -D warnings, unit tests
```

`desktop-check` stages a placeholder sidecar when none is staged, so it does
not need a mesh-llm build.

## Package installers

```bash
just desktop-bundle            # release host + default runtime, then installers
just desktop-bundle cuda       # same with a specific runtime backend
just desktop-package target/release/mesh-llm dist/native-runtimes  # reuse a build
```

Installers land in `desktop/src-tauri/target/release/bundle/`: `.dmg`/`.app`
on macOS, `.msi` and NSIS `.exe` on Windows, `.AppImage` and `.deb` on Linux.
Each installer carries one host and the runtime(s) you staged, so build GPU
flavours (`cuda`, `rocm`, `vulkan`) as separate installers, the same way the
CLI release archives are split. On Windows, build the host and runtime with the
usual Windows recipes and use `just desktop-package`.

`scripts/stage-desktop-sidecar.sh` (and `.ps1`) copies the host into
`src-tauri/binaries/mesh-llm-sidecar-<target-triple>` and the runtime directories into
`src-tauri/binaries/native-runtimes/`. It copies an already built product and
adds nothing beside the host, so the host dependency policy still holds.

Unsigned macOS builds must be ad-hoc signed and have quarantine removed before
they open on another machine:

```bash
codesign --force --deep -s - "Mesh LLM Desktop.app"
xattr -cr "Mesh LLM Desktop.app"
```

The app version comes from `src-tauri/Cargo.toml`. Keep it in step with the
workspace version in the root `Cargo.toml`.

## Installers from CI

The **Desktop · Installers** workflow (`.github/workflows/desktop-packages.yml`)
builds installers for an already published stable release. Run it from the
Actions tab with:

- `tag`: the release to package, for example `v0.77.0`
- `attach_to_release`: also upload the installers to that GitHub release (only
  honoured when the workflow runs from the default branch)

It downloads that release's macOS aarch64 Metal and Windows x86_64 CPU archives,
checks them against their `.sha256` files, and packages the exact mesh-llm host
and runtime inside the app. It never rebuilds mesh-llm. The installers are
`mesh-llm-desktop-<version>-aarch64-apple-darwin.dmg`,
`mesh-llm-desktop-<version>-x86_64-pc-windows-msvc.msi` and
`mesh-llm-desktop-<version>-x86_64-pc-windows-msvc-setup.exe`, each with a
`.sha256` file. They are kept as workflow artifacts for 14 days.

Current limits:

- The installers are not code-signed. macOS needs the `codesign`/`xattr` steps
  above, and Windows SmartScreen will warn on first run.
- Linux installers are not built in CI yet. Tauri's Linux build needs the
  webview packages listed above, and CI jobs must get system packages from the
  shared runner images rather than installing them per job. Build Linux
  packages locally with `just desktop-bundle` until the images carry them.

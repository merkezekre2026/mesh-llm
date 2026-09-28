# Mesh LLM desktop app (Tauri, desktop/). The Tauri CLI is pinned in
# desktop/package-lock.json and installed with `npm ci`; Linux also needs the
# webview system packages listed in desktop/README.md.

desktop_dir := "desktop/src-tauri"

# Run the desktop app against the local debug product from `just build`.
[unix]
desktop-dev:
    #!/usr/bin/env bash
    set -euo pipefail
    scripts/stage-desktop-sidecar.sh --binary target/debug/mesh-llm
    cd desktop
    npm ci
    MESH_LLM_DESKTOP_SIDECAR="{{ justfile_directory() }}/target/debug/mesh-llm" npm exec -- tauri dev

# Build the release product for this platform and package it as desktop installers.
[unix]
desktop-bundle backend="":
    #!/usr/bin/env bash
    set -euo pipefail
    just release-host-build
    rm -rf dist/native-runtimes
    just release-runtime-build "{{ backend }}"
    just desktop-package target/release/mesh-llm dist/native-runtimes

# Package an already built host and native-runtimes directory as desktop installers.
[unix]
desktop-package binary runtimes="":
    #!/usr/bin/env bash
    set -euo pipefail
    stage_args=(--binary "{{ binary }}")
    [[ -n "{{ runtimes }}" ]] && stage_args+=(--runtimes "{{ runtimes }}")
    scripts/stage-desktop-sidecar.sh "${stage_args[@]}"
    cd desktop
    npm ci
    npm exec -- tauri build
    echo "Installers: {{ desktop_dir }}/target/release/bundle/"

[windows]
desktop-package binary runtimes="":
    @powershell -NoProfile -ExecutionPolicy Bypass -File scripts/stage-desktop-sidecar.ps1 -Binary "{{ binary }}" -Runtimes "{{ runtimes }}"
    @cd desktop && npm ci && npm exec -- tauri build

# Format, lint, and unit-test the desktop app (stubs the sidecar if none is staged).
[unix]
desktop-check:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! compgen -G "{{ desktop_dir }}/binaries/mesh-llm-sidecar-*" > /dev/null; then
        scripts/stage-desktop-sidecar.sh --placeholder
    fi
    cd "{{ desktop_dir }}"
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test

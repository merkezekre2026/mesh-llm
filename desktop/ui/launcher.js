// Launcher page for the Mesh LLM desktop app. It only talks to the Rust side
// through the commands in src-tauri/src/commands.rs; once the node is ready
// the Rust side navigates this window to the mesh-llm console.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const SCREENS = ["choose", "starting", "ready", "failed"];
const $ = (id) => document.getElementById(id);

let settings = null;
let consoleUrl = null;

function showScreen(name) {
  for (const screen of SCREENS) {
    $(`screen-${screen}`).hidden = screen !== name;
  }
}

function fillForm(current) {
  const mode = document.querySelector(`input[name="mode"][value="${current.mode}"]`);
  if (mode) mode.checked = true;
  $("join-token").value = current.join_token ?? "";
  $("console-port").value = current.console_port;
  $("api-port").value = current.api_port;
  $("api-url").textContent = `http://localhost:${current.api_port}/v1`;
}

function readForm() {
  const token = $("join-token").value.trim();
  return {
    mode: document.querySelector('input[name="mode"]:checked').value,
    console_port: Number($("console-port").value),
    api_port: Number($("api-port").value),
    join_token: token === "" ? null : token,
    remember: $("remember").checked,
  };
}

function renderPhase(update) {
  consoleUrl = update.console_url ?? null;
  switch (update.phase) {
    case "starting":
      $("starting-title").textContent = update.message || "Starting Mesh LLM…";
      showScreen("starting");
      break;
    case "ready":
      $("ready-detail").textContent = update.message;
      showScreen("ready");
      break;
    case "failed": {
      $("failed-message").textContent = update.message;
      const logs = update.recent_logs ?? [];
      $("failed-logs").textContent = logs.join("\n");
      $("failed-logs").hidden = logs.length === 0;
      showScreen("failed");
      break;
    }
    default:
      showScreen("choose");
  }
}

async function run(command, args) {
  try {
    await invoke(command, args);
  } catch (error) {
    const message = String(error);
    $("form-error").textContent = message;
    $("form-error").hidden = false;
    showScreen("choose");
  }
}

async function onSubmit(event) {
  event.preventDefault();
  $("form-error").hidden = true;
  const next = readForm();
  if (next.console_port === next.api_port) {
    $("form-error").textContent = "The console port and the API port must be different.";
    $("form-error").hidden = false;
    return;
  }
  settings = next;
  $("api-url").textContent = `http://localhost:${next.api_port}/v1`;
  await run("start_mesh", { settings: next });
}

function bindActions() {
  $("launch-form").addEventListener("submit", onSubmit);
  $("open-console").addEventListener("click", () => {
    if (consoleUrl) window.location.href = consoleUrl;
  });
  for (const button of document.querySelectorAll("[data-action]")) {
    const command = {
      "change-mode": "change_mode",
      retry: "retry_mesh",
      "open-logs": "open_logs",
    }[button.dataset.action];
    button.addEventListener("click", () => run(command));
  }
}

async function init() {
  bindActions();
  await listen("mesh://phase", (event) => renderPhase(event.payload));
  await listen("mesh://log", (event) => {
    $("starting-detail").textContent = event.payload;
  });
  const state = await invoke("launch_state");
  settings = state.settings;
  fillForm(settings);
  renderPhase(state.phase);
}

init();

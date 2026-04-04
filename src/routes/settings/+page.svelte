<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { getVersion } from "@tauri-apps/api/app";
  import { check, type Update } from "@tauri-apps/plugin-updater";
  import { relaunch } from "@tauri-apps/plugin-process";
  import { afterNavigate } from "$app/navigation";
  import { onMount } from "svelte";
  import { createShortcutCaptureSession } from "$lib/keybindCapture";
  import { bindYapperShortcuts } from "$lib/shortcuts";
  import {
    applyUiTheme,
    loadUiTheme,
    persistUiTheme,
    type UiTheme,
  } from "$lib/theme";
  import {
    DEFAULT_PARAKEET_MODEL,
    PARAKEET_MODEL_OPTIONS,
    parakeetDiskMb,
  } from "$lib/parakeetModelInfo";
  import {
    formatStorageMb,
    whisperDiskMb,
    whisperRuntimeMbHint,
    WHISPER_MODEL_OPTIONS,
  } from "$lib/whisperModelInfo";

  function n(s: string, fallback: number): number {
    const v = Number.parseFloat(s);
    return Number.isFinite(v) ? v : fallback;
  }

  let uiTheme = $state<UiTheme>("system");
  type InstanceRole = "dictation" | "network_server";
  let instanceRole = $state<InstanceRole>("dictation");
  let nodeServerBind = $state<"lan" | "loopback">("lan");
  let nodeServerPort = $state("8765");
  let nodeServerToken = $state("");
  type NodeServerStatus = {
    running: boolean;
    bindMode: string;
    bindHost: string;
    port: number;
    tokenConfigured: boolean;
    suggestedClientUrls: string[];
    logTail: string[];
    scriptFound: boolean;
    scriptPath: string;
  };
  let nodeStatus = $state<NodeServerStatus | null>(null);
  let nodeActionBusy = $state(false);
  let nodeActionErr = $state<string | null>(null);

  let inferenceHost = $state("local");
  let remoteUrl = $state("ws://127.0.0.1:8765");
  let remoteToken = $state("");
  let engine = $state("whisper");
  let whisperModel = $state("base");
  let parakeetModel = $state(DEFAULT_PARAKEET_MODEL);
  let computeType = $state("int8");
  let tonePreset = $state("standard");
  let mock = $state(false);
  let cuda = $state(false);
  let whisperDevice = $state("auto");
  let inputDeviceId = $state("");
  let micDevices = $state<{ id: string; label: string }[]>([]);
  let vadEnergyThreshold = $state("0.008");
  let vadMinSilenceMs = $state("300");
  let micNormalizePeak = $state("0.88");
  let micMaxGain = $state("12");
  let lazyLoadWhisper = $state(false);
  let modelIdleUnloadMins = $state("0");

  let whisperBeamSize = $state("5");
  let whisperBestOf = $state("1");
  let whisperPatience = $state("1");
  let whisperTemperature = $state("0");
  let whisperNoSpeechThreshold = $state("0.78");
  let whisperLogProbThreshold = $state("-0.55");
  let whisperCompressionRatioThreshold = $state("1.9");
  let whisperHallucinationSilenceThreshold = $state("1.6");
  let whisperConditionOnPrevious = $state(false);
  let whisperInitialPrompt = $state("");
  let whisperLanguage = $state("");
  let whisperVadFilterPcm = $state(false);
  let whisperVadFilterFile = $state(true);

  let kPtt = $state("");
  let kMic = $state("");
  let kStop = $state("");
  type KeybindCaptureTarget = "push_to_talk" | "toggle_open_mic" | "stop_dictation";
  let captureTarget = $state<KeybindCaptureTarget | null>(null);
  let conflict = $state<string[]>([]);

  $effect(() => {
    if (typeof window === "undefined" || !captureTarget) return;
    const session = createShortcutCaptureSession();
    const onKeyDown = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.code === "Escape") {
        session.reset();
        captureTarget = null;
        return;
      }
      const s = session.consumeKeyDown(e);
      if (!s) return;
      if (captureTarget === "push_to_talk") kPtt = s;
      else if (captureTarget === "toggle_open_mic") kMic = s;
      else kStop = s;
      session.reset();
      captureTarget = null;
    };
    const onKeyUp = (e: KeyboardEvent) => {
      session.onKeyUp(e);
    };
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
    };
  });
  let nvidiaInstallBusy = $state(false);
  let nvidiaInstallLog = $state("");

  let appVersion = $state("");
  let updateCheckBusy = $state(false);
  let updateInstallBusy = $state(false);
  let updateErr = $state<string | null>(null);
  type PendingUpdate = {
    version: string;
    currentVersion: string;
    date?: string;
    body?: string;
    raw: Update;
  };
  let pendingUpdate = $state<PendingUpdate | null>(null);
  let updateProgressLabel = $state<string | null>(null);
  let updateLastCheckUpToDate = $state(false);

  type ModelCacheDiagnostic = {
    cacheDir: string;
    cacheDirExists: boolean;
    topLevelEntries: string[];
    settings: {
      whisperModel: string;
      mockTranscription: boolean;
      lazyLoadWhisper: boolean;
      whisperDevice: string;
      computeType: string;
    };
  };
  let cacheDiagnosticText = $state<string | null>(null);
  let cacheDiagnosticData = $state<ModelCacheDiagnostic | null>(null);

  async function load() {
    uiTheme = await loadUiTheme();
    applyUiTheme(uiTheme);
    inferenceHost =
      (await invoke<string | null>("get_setting_cmd", { key: "inference_host" })) ??
      "local";
    remoteUrl =
      (await invoke<string | null>("get_setting_cmd", { key: "remote_url" })) ??
      remoteUrl;
    remoteToken =
      (await invoke<string | null>("get_setting_cmd", { key: "remote_token" })) ?? "";
    engine =
      (await invoke<string | null>("get_setting_cmd", { key: "engine" })) ?? "whisper";
    whisperModel =
      (await invoke<string | null>("get_setting_cmd", { key: "whisper_model" })) ??
      "base";
    {
      const pk =
        (await invoke<string | null>("get_setting_cmd", { key: "parakeet_model" })) ??
        DEFAULT_PARAKEET_MODEL;
      parakeetModel = PARAKEET_MODEL_OPTIONS.some((o) => o.id === pk)
        ? pk
        : DEFAULT_PARAKEET_MODEL;
    }
    computeType =
      (await invoke<string | null>("get_setting_cmd", { key: "compute_type" })) ??
      "int8";
    tonePreset =
      (await invoke<string | null>("get_setting_cmd", { key: "tone_preset" })) ??
      "standard";
    const m = await invoke<string | null>("get_setting_cmd", {
      key: "mock_transcription",
    });
    mock = m === "true";
    cuda = await invoke("cuda_available");
    whisperDevice =
      (await invoke<string | null>("get_setting_cmd", { key: "whisper_device" })) ?? "auto";
    inputDeviceId =
      (await invoke<string | null>("get_setting_cmd", { key: "input_device_name" })) ?? "";
    vadEnergyThreshold =
      (await invoke<string | null>("get_setting_cmd", { key: "vad_energy_threshold" })) ??
      "0.008";
    vadMinSilenceMs =
      (await invoke<string | null>("get_setting_cmd", { key: "vad_min_silence_ms" })) ?? "300";
    micNormalizePeak =
      (await invoke<string | null>("get_setting_cmd", { key: "mic_normalize_peak" })) ?? "0.88";
    micMaxGain =
      (await invoke<string | null>("get_setting_cmd", { key: "mic_max_gain" })) ?? "12";
    try {
      micDevices = await invoke<{ id: string; label: string }[]>("list_audio_input_devices");
    } catch {
      micDevices = [{ id: "", label: "System default" }];
    }
    lazyLoadWhisper =
      (await invoke<string | null>("get_setting_cmd", { key: "lazy_load_whisper" })) === "true";
    modelIdleUnloadMins =
      (await invoke<string | null>("get_setting_cmd", { key: "model_idle_unload_mins" })) ?? "0";

    whisperBeamSize =
      (await invoke<string | null>("get_setting_cmd", { key: "whisper_beam_size" })) ?? "5";
    whisperBestOf =
      (await invoke<string | null>("get_setting_cmd", { key: "whisper_best_of" })) ?? "1";
    whisperPatience =
      (await invoke<string | null>("get_setting_cmd", { key: "whisper_patience" })) ?? "1";
    whisperTemperature =
      (await invoke<string | null>("get_setting_cmd", { key: "whisper_temperature" })) ?? "0";
    whisperNoSpeechThreshold =
      (await invoke<string | null>("get_setting_cmd", { key: "whisper_no_speech_threshold" })) ??
      "0.78";
    whisperLogProbThreshold =
      (await invoke<string | null>("get_setting_cmd", { key: "whisper_log_prob_threshold" })) ??
      "-0.55";
    whisperCompressionRatioThreshold =
      (await invoke<string | null>("get_setting_cmd", {
        key: "whisper_compression_ratio_threshold",
      })) ?? "1.9";
    whisperHallucinationSilenceThreshold =
      (await invoke<string | null>("get_setting_cmd", {
        key: "whisper_hallucination_silence_threshold",
      })) ?? "1.6";
    whisperConditionOnPrevious =
      (await invoke<string | null>("get_setting_cmd", {
        key: "whisper_condition_on_previous_text",
      })) === "true";
    whisperInitialPrompt =
      (await invoke<string | null>("get_setting_cmd", { key: "whisper_initial_prompt" })) ?? "";
    whisperLanguage =
      (await invoke<string | null>("get_setting_cmd", { key: "whisper_language" })) ?? "";
    whisperVadFilterPcm =
      (await invoke<string | null>("get_setting_cmd", { key: "whisper_vad_filter_pcm" })) ===
      "true";
    whisperVadFilterFile =
      (await invoke<string | null>("get_setting_cmd", { key: "whisper_vad_filter_file" })) !==
      "false";

    const binds = await invoke<{ action: string; shortcut: string }[]>(
      "list_keybinds_cmd",
    );
    for (const b of binds) {
      if (b.action === "push_to_talk") kPtt = b.shortcut;
      if (b.action === "toggle_open_mic") kMic = b.shortcut;
      if (b.action === "stop_dictation") kStop = b.shortcut;
    }

    const role =
      (await invoke<string | null>("get_setting_cmd", { key: "instance_role" })) ?? "dictation";
    instanceRole = role === "network_server" ? "network_server" : "dictation";
    const bind =
      (await invoke<string | null>("get_setting_cmd", { key: "node_server_bind" })) ?? "lan";
    nodeServerBind = bind === "loopback" ? "loopback" : "lan";
    nodeServerPort =
      (await invoke<string | null>("get_setting_cmd", { key: "node_server_port" })) ?? "8765";
    nodeServerToken =
      (await invoke<string | null>("get_setting_cmd", { key: "node_server_token" })) ?? "";

    if (instanceRole === "network_server") {
      await refreshNodeStatus();
    } else {
      nodeStatus = null;
    }
  }

  async function refreshNodeStatus() {
    try {
      nodeStatus = await invoke<NodeServerStatus>("yapper_node_status");
    } catch {
      nodeStatus = null;
    }
  }

  async function setInstanceRole(next: InstanceRole) {
    instanceRole = next;
    try {
      await invoke("set_setting_cmd", {
        key: "instance_role",
        value: next,
      });
    } catch (e) {
      nodeActionErr = String(e);
    }
    if (next === "network_server") {
      await refreshNodeStatus();
    } else {
      nodeStatus = null;
      nodeActionErr = null;
    }
  }

  function generateNodeToken() {
    const a = new Uint8Array(16);
    crypto.getRandomValues(a);
    nodeServerToken = [...a].map((b) => b.toString(16).padStart(2, "0")).join("");
  }

  async function saveNodeServerConfig() {
    nodeActionErr = null;
    await invoke("set_setting_cmd", {
      key: "node_server_bind",
      value: nodeServerBind,
    });
    await invoke("set_setting_cmd", { key: "node_server_port", value: nodeServerPort.trim() });
    await invoke("set_setting_cmd", {
      key: "node_server_token",
      value: nodeServerToken,
    });
    await refreshNodeStatus();
  }

  async function startProcessingServer() {
    nodeActionBusy = true;
    nodeActionErr = null;
    try {
      await saveNodeServerConfig();
      nodeStatus = await invoke<NodeServerStatus>("yapper_node_start");
    } catch (e) {
      nodeActionErr = String(e);
      await refreshNodeStatus();
    } finally {
      nodeActionBusy = false;
    }
  }

  async function stopProcessingServer() {
    nodeActionBusy = true;
    nodeActionErr = null;
    try {
      nodeStatus = await invoke<NodeServerStatus>("yapper_node_stop");
    } catch (e) {
      nodeActionErr = String(e);
    } finally {
      nodeActionBusy = false;
    }
  }

  async function copyText(t: string) {
    try {
      await navigator.clipboard.writeText(t);
    } catch {
      /* ignore */
    }
  }

  onMount(() => {
    void load();
    void (async () => {
      try {
        appVersion = await getVersion();
      } catch {
        appVersion = "";
      }
    })();
  });

  async function checkForUpdates() {
    if (import.meta.env.DEV) {
      updateErr = "Update checks run in the packaged app (after tauri build), not in dev mode.";
      pendingUpdate = null;
      return;
    }
    updateCheckBusy = true;
    updateErr = null;
    pendingUpdate = null;
    updateProgressLabel = null;
    updateLastCheckUpToDate = false;
    try {
      const u = await check({ timeout: 30_000 });
      if (!u) {
        pendingUpdate = null;
        updateErr = null;
        updateLastCheckUpToDate = true;
        return;
      }
      pendingUpdate = {
        version: u.version,
        currentVersion: u.currentVersion,
        date: u.date,
        body: u.body,
        raw: u,
      };
    } catch (e) {
      updateErr = String(e);
      pendingUpdate = null;
    } finally {
      updateCheckBusy = false;
    }
  }

  async function installPendingUpdate() {
    const u = pendingUpdate?.raw;
    if (!u) return;
    updateInstallBusy = true;
    updateErr = null;
    updateProgressLabel = null;
    try {
      let downloaded = 0;
      let total: number | undefined;
      await u.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            total = event.data.contentLength ?? undefined;
            downloaded = 0;
            updateProgressLabel = total
              ? `Downloading… 0 / ${total} bytes`
              : "Downloading…";
            break;
          case "Progress":
            downloaded += event.data.chunkLength;
            updateProgressLabel =
              total !== undefined
                ? `Downloading… ${downloaded} / ${total} bytes`
                : `Downloading… ${downloaded} bytes`;
            break;
          case "Finished":
            updateProgressLabel = "Installing…";
            break;
        }
      });
      await relaunch();
    } catch (e) {
      updateErr = String(e);
    } finally {
      updateInstallBusy = false;
    }
  }

  $effect(() => {
    if (typeof window === "undefined") return;
    if (instanceRole !== "network_server") return;
    const poll = window.setInterval(() => {
      if (document.visibilityState !== "visible") return;
      void refreshNodeStatus();
    }, 2800);
    return () => window.clearInterval(poll);
  });

  afterNavigate(({ to }) => {
    if (to?.url.pathname === "/settings") void load();
  });

  async function refreshModelCacheDiagnostic() {
    cacheDiagnosticText = "Loading…";
    cacheDiagnosticData = null;
    try {
      cacheDiagnosticData = await invoke<ModelCacheDiagnostic>("model_cache_diagnostic");
      cacheDiagnosticText = null;
    } catch (e) {
      cacheDiagnosticText = String(e);
    }
  }

  async function changeUiTheme(mode: UiTheme) {
    uiTheme = mode;
    applyUiTheme(mode);
    await persistUiTheme(mode);
  }

  async function saveCore() {
    await invoke("set_setting_cmd", {
      key: "inference_host",
      value: inferenceHost,
    });
    await invoke("set_setting_cmd", { key: "remote_url", value: remoteUrl });
    await invoke("set_setting_cmd", { key: "remote_token", value: remoteToken });
    await invoke("set_setting_cmd", { key: "engine", value: engine });
    await invoke("set_setting_cmd", { key: "whisper_model", value: whisperModel });
    await invoke("set_setting_cmd", { key: "parakeet_model", value: parakeetModel });
    await invoke("set_setting_cmd", { key: "compute_type", value: computeType });
    await invoke("set_setting_cmd", { key: "tone_preset", value: tonePreset });
    await invoke("set_setting_cmd", {
      key: "mock_transcription",
      value: mock ? "true" : "false",
    });
    await invoke("set_setting_cmd", { key: "whisper_device", value: whisperDevice });
    await invoke("set_setting_cmd", { key: "input_device_name", value: inputDeviceId });
    await invoke("set_setting_cmd", {
      key: "lazy_load_whisper",
      value: lazyLoadWhisper ? "true" : "false",
    });
    await invoke("set_setting_cmd", {
      key: "model_idle_unload_mins",
      value: modelIdleUnloadMins,
    });
    await invoke("set_setting_cmd", { key: "whisper_beam_size", value: whisperBeamSize });
    await invoke("set_setting_cmd", { key: "whisper_best_of", value: whisperBestOf });
    await invoke("set_setting_cmd", { key: "whisper_patience", value: whisperPatience });
    await invoke("set_setting_cmd", { key: "whisper_temperature", value: whisperTemperature });
    await invoke("set_setting_cmd", {
      key: "whisper_no_speech_threshold",
      value: whisperNoSpeechThreshold,
    });
    await invoke("set_setting_cmd", {
      key: "whisper_log_prob_threshold",
      value: whisperLogProbThreshold,
    });
    await invoke("set_setting_cmd", {
      key: "whisper_compression_ratio_threshold",
      value: whisperCompressionRatioThreshold,
    });
    await invoke("set_setting_cmd", {
      key: "whisper_hallucination_silence_threshold",
      value: whisperHallucinationSilenceThreshold,
    });
    await invoke("set_setting_cmd", {
      key: "whisper_condition_on_previous_text",
      value: whisperConditionOnPrevious ? "true" : "false",
    });
    await invoke("set_setting_cmd", {
      key: "whisper_initial_prompt",
      value: whisperInitialPrompt,
    });
    await invoke("set_setting_cmd", { key: "whisper_language", value: whisperLanguage });
    await invoke("set_setting_cmd", {
      key: "whisper_vad_filter_pcm",
      value: whisperVadFilterPcm ? "true" : "false",
    });
    await invoke("set_setting_cmd", {
      key: "whisper_vad_filter_file",
      value: whisperVadFilterFile ? "true" : "false",
    });
  }

  async function saveMicrophoneOnly() {
    await invoke("set_setting_cmd", { key: "input_device_name", value: inputDeviceId });
    await invoke("set_setting_cmd", {
      key: "vad_energy_threshold",
      value: vadEnergyThreshold,
    });
    await invoke("set_setting_cmd", {
      key: "vad_min_silence_ms",
      value: vadMinSilenceMs,
    });
    await invoke("set_setting_cmd", {
      key: "mic_normalize_peak",
      value: micNormalizePeak,
    });
    await invoke("set_setting_cmd", { key: "mic_max_gain", value: micMaxGain });
  }

  async function saveKeybinds() {
    conflict = [];
    let c = await invoke<string[]>("set_keybind_cmd", {
      action: "push_to_talk",
      shortcut: kPtt,
    });
    if (c.length) conflict = [...conflict, ...c];
    c = await invoke<string[]>("set_keybind_cmd", {
      action: "toggle_open_mic",
      shortcut: kMic,
    });
    if (c.length) conflict = [...conflict, ...c];
    c = await invoke<string[]>("set_keybind_cmd", {
      action: "stop_dictation",
      shortcut: kStop,
    });
    if (c.length) conflict = [...conflict, ...c];
    await bindYapperShortcuts();
  }

  async function restartEngine() {
    await saveCore();
    await invoke("engine_stop");
    await invoke("engine_start");
  }

  async function installNvidiaWhisperLibs() {
    nvidiaInstallBusy = true;
    nvidiaInstallLog = "";
    try {
      nvidiaInstallLog = await invoke<string>("install_nvidia_whisper_libs");
    } catch (e) {
      nvidiaInstallLog = String(e);
    } finally {
      nvidiaInstallBusy = false;
    }
  }
</script>

<section>
  <h1>Settings</h1>

  <div class="panel block">
    <h2>Appearance</h2>
    <p class="muted short">Choose how Yapper looks. “Match system” follows Windows light/dark.</p>
    <div class="theme-toggle" role="group" aria-label="Color theme">
      <button
        type="button"
        class="theme-seg"
        class:active={uiTheme === "light"}
        onclick={() => changeUiTheme("light")}>Light</button>
      <button
        type="button"
        class="theme-seg"
        class:active={uiTheme === "dark"}
        onclick={() => changeUiTheme("dark")}>Dark</button>
      <button
        type="button"
        class="theme-seg"
        class:active={uiTheme === "system"}
        onclick={() => changeUiTheme("system")}>Match system</button>
    </div>
  </div>

  <div class="panel block" id="app-updates">
    <h2>Updates</h2>
    <p class="muted short">
      This version: <strong>{appVersion || "—"}</strong>
    </p>
    {#if import.meta.env.DEV}
      <p class="note">Run a release build to test updates end-to-end.</p>
    {/if}
    {#if pendingUpdate}
      <p class="update-banner" role="status">
        Update available: <strong>{pendingUpdate.version}</strong>
        {#if pendingUpdate.date}
          <span class="muted">· {pendingUpdate.date}</span>
        {/if}
      </p>
      {#if pendingUpdate.body}
        <pre class="update-notes">{pendingUpdate.body}</pre>
      {/if}
    {:else if updateLastCheckUpToDate && !updateErr && !import.meta.env.DEV}
      <p class="muted short">You're up to date.</p>
    {/if}
    {#if updateProgressLabel}
      <p class="muted short" role="status">{updateProgressLabel}</p>
    {/if}
    {#if updateErr}
      <p class="warn" role="alert">{updateErr}</p>
    {/if}
    <div class="update-actions">
      <button
        type="button"
        class="btn"
        disabled={updateCheckBusy || updateInstallBusy}
        onclick={checkForUpdates}
      >
        {updateCheckBusy ? "Checking…" : "Check for updates"}
      </button>
      {#if pendingUpdate}
        <button
          type="button"
          class="btn btn-primary"
          disabled={updateInstallBusy}
          onclick={installPendingUpdate}
        >
          {updateInstallBusy ? "Installing…" : "Download & install"}
        </button>
      {/if}
    </div>
  </div>

  <div class="panel block" id="instance-role">
    <h2>This installation</h2>
    <p class="muted short">
      Say whether this PC is mainly for <strong>dictating here</strong> or for <strong>running models for other
      Yapper installs</strong> on your LAN or VPN. You can still use both; this only changes emphasis in the app.
    </p>
    <div class="theme-toggle" role="group" aria-label="Primary use of this PC">
      <button
        type="button"
        class="theme-seg"
        class:active={instanceRole === "dictation"}
        onclick={() => setInstanceRole("dictation")}>Dictation on this PC</button>
      <button
        type="button"
        class="theme-seg"
        class:active={instanceRole === "network_server"}
        onclick={() => setInstanceRole("network_server")}>Network processing server</button>
    </div>
  </div>

  {#if instanceRole === "network_server"}
  <div class="panel block" id="processing-server">
    <h2>Network processing server (Yapper Node)</h2>
    <p class="muted short">
      Turn this PC into a WebSocket server other Yapper installs can connect to (Settings → Speech engine → “Another
      computer”). Uses the same Python stack as the local sidecar: run
      <code>pip install -r yapper-node/requirements.txt</code>
      once. Prefer Tailscale or another VPN; do not expose the port to the public internet without TLS and auth you trust.
    </p>
    {#if nodeStatus && !nodeStatus.scriptFound}
      <p class="warn" role="alert">
        Yapper Node script was not found at the expected path. Use a full repo checkout, or set the
        <code>YAPPER_NODE</code> environment variable to <code>main.py</code>.
      </p>
      <p class="muted short mono-p">{nodeStatus.scriptPath}</p>
    {/if}
    <div class="field">
      <label for="ns-bind">Listen on</label>
      <select id="ns-bind" bind:value={nodeServerBind} disabled={nodeStatus?.running ?? false}>
        <option value="lan">All interfaces (LAN / VPN — other PCs can connect)</option>
        <option value="loopback">This PC only (127.0.0.1 — testing)</option>
      </select>
    </div>
    <div class="field">
      <label for="ns-port">Port</label>
      <input
        id="ns-port"
        type="text"
        bind:value={nodeServerPort}
        inputmode="numeric"
        autocomplete="off"
        disabled={nodeStatus?.running ?? false}
      />
    </div>
    <div class="field">
      <label for="ns-tok">Server password (shared secret)</label>
      <div class="token-row">
        <input id="ns-tok" type="password" bind:value={nodeServerToken} autocomplete="off" />
        <button type="button" class="btn" onclick={generateNodeToken}>Generate</button>
      </div>
      <p class="field-hint">Clients enter the same value under “Password” when connecting to this server.</p>
    </div>
    <button
      type="button"
      class="btn"
      disabled={nodeStatus?.running ?? false}
      onclick={saveNodeServerConfig}>Save server settings</button>
    <div class="node-actions">
      {#if nodeStatus?.running}
        <button
          type="button"
          class="btn btn-stop"
          disabled={nodeActionBusy}
          onclick={stopProcessingServer}>{nodeActionBusy ? "Stopping…" : "Stop processing server"}</button>
      {:else}
        <button
          type="button"
          class="btn btn-primary"
          disabled={nodeActionBusy || !nodeServerToken.trim()}
          onclick={startProcessingServer}>{nodeActionBusy ? "Starting…" : "Start processing server"}</button>
      {/if}
    </div>
    {#if nodeActionErr}
      <p class="warn" role="alert">{nodeActionErr}</p>
    {/if}
    {#if nodeStatus}
      <div class="node-status" role="status">
        <p class="node-status-line">
          <span class="dot" class:on={nodeStatus.running} aria-hidden="true"></span>
          <strong>{nodeStatus.running ? "Server running" : "Server stopped"}</strong>
          {#if nodeStatus.running}
            <span class="muted">· port {nodeStatus.port}</span>
          {/if}
        </p>
        {#if nodeStatus.running && nodeStatus.suggestedClientUrls.length}
          <p class="muted short">On other machines, set <em>Server address</em> to one of:</p>
          <ul class="url-list">
            {#each nodeStatus.suggestedClientUrls as u}
              <li>
                <code class="ws-url">{u}</code>
                <button type="button" class="btn btn-tiny" onclick={() => copyText(u)}>Copy</button>
              </li>
            {/each}
          </ul>
        {/if}
        {#if nodeStatus.logTail.length}
          <details class="node-log">
            <summary>Recent log</summary>
            <pre class="node-log-pre">{nodeStatus.logTail.join("\n")}</pre>
          </details>
        {/if}
      </div>
    {/if}
  </div>
  {/if}

  <div class="panel block" id="inference-host">
    <h2>Speech engine</h2>
    <div class="field">
      <label for="host">Where transcription runs</label>
      <select id="host" bind:value={inferenceHost}>
        <option value="local">On this computer</option>
        <option value="remote">Another computer on your network</option>
      </select>
    </div>
    {#if inferenceHost === "remote"}
      <div class="field">
        <label for="url">Server address</label>
        <input id="url" bind:value={remoteUrl} placeholder="ws://192.168.1.10:8765" />
      </div>
      <div class="field">
        <label for="tok">Password (if required)</label>
        <input id="tok" type="password" bind:value={remoteToken} autocomplete="off" />
      </div>
    {/if}
    <div class="field">
      <label for="eng">Recognition engine</label>
      <select id="eng" bind:value={engine}>
        <option value="whisper">Whisper (recommended)</option>
        <option value="parakeet" disabled={!cuda}>Parakeet (NVIDIA GPU only)</option>
      </select>
    </div>
    {#if !cuda}
      <p class="note">Parakeet needs an NVIDIA GPU. Whisper works on CPU or NVIDIA.</p>
    {/if}
    <div class="field">
      {#if engine === "whisper"}
        <label for="wm">Model size</label>
        <select id="wm" bind:value={whisperModel}>
          {#each WHISPER_MODEL_OPTIONS as m}
            <option value={m.id}>
              {m.line} — {formatStorageMb(whisperDiskMb(m.id))} on disk
            </option>
          {/each}
        </select>
        <p class="field-hint">
          One-time download into the app cache. Size is the same for int8, float16, and float32 — only speed and memory
          while running change.
        </p>
      {:else}
        <label for="wm-pk">Model</label>
        <select id="wm-pk" bind:value={parakeetModel}>
          {#each PARAKEET_MODEL_OPTIONS as m}
            <option value={m.id}>
              {m.line} — {formatStorageMb(parakeetDiskMb(m.id))} on disk
            </option>
          {/each}
        </select>
        <p class="field-hint">
          English checkpoints from Hugging Face; first load downloads weights (sizes are approximate). NeMo + CUDA
          required on the inference host.
        </p>
      {/if}
    </div>
    {#if engine === "whisper"}
      <p class="note">First use downloads the model; Wi‑Fi helps for larger sizes.</p>
    {:else}
      <p class="note">First use downloads the checkpoint; Wi‑Fi helps for the larger options.</p>
    {/if}
    <label class="check">
      <input type="checkbox" bind:checked={lazyLoadWhisper} />
      Load the model only when needed (saves memory; first use may pause briefly)
    </label>
    <div class="field">
      <label for="idle">Free memory after idle</label>
      <select id="idle" bind:value={modelIdleUnloadMins}>
        <option value="0">Never while the engine is on</option>
        <option value="5">After 5 minutes idle</option>
        <option value="10">After 10 minutes</option>
        <option value="15">After 15 minutes</option>
        <option value="30">After 30 minutes</option>
        <option value="60">After 60 minutes</option>
      </select>
    </div>
    <p class="note">After idle timeout, the model unloads from RAM. The next session loads from disk again (no new download).</p>
    {#if engine === "whisper"}
      <div class="field">
        <label for="ct">Number format (speed vs. precision)</label>
        <select id="ct" bind:value={computeType}>
          <option value="int8">int8 — smallest memory, fastest</option>
          <option value="float16">float16 — middle ground</option>
          <option value="float32">float32 — largest memory, highest precision</option>
        </select>
        <p class="field-hint">
          Rough memory while loaded (model + format): {formatStorageMb(
            whisperRuntimeMbHint(whisperModel, computeType),
          )} — ballpark only; real use depends on GPU drivers and batching.
        </p>
      </div>
    {/if}
    <div class="field">
      <label for="wd">Processor</label>
      <select id="wd" bind:value={whisperDevice}>
        <option value="auto">Automatic</option>
        <option value="cpu">CPU only</option>
        <option value="cuda">NVIDIA GPU (CUDA)</option>
      </select>
    </div>
    <p class="note">
      GPU issues? Install the libraries in <a href="#gpu-deps">NVIDIA helpers</a> below, then use Save &amp; restart.
    </p>

    {#if engine === "whisper"}
      <h3 class="settings-subh">Fine-tuning (Whisper)</h3>
      <p class="note">Applied when the engine starts — use <em>Save &amp; restart engine</em>. Higher values often mean slower runs.</p>
      <div class="whisper-grid">
        <div class="field">
          <label for="wbeam">Beam size</label>
          <div class="slider-row">
            <input
              id="wbeam"
              type="range"
              min="1"
              max="10"
              step="1"
              value={Math.round(n(whisperBeamSize, 5))}
              oninput={(e) => (whisperBeamSize = e.currentTarget.value)}
            />
            <input class="slider-value" bind:value={whisperBeamSize} inputmode="numeric" autocomplete="off" />
          </div>
        </div>
        <div class="field">
          <label for="wbest">Best of</label>
          <div class="slider-row">
            <input
              id="wbest"
              type="range"
              min="1"
              max="5"
              step="1"
              value={Math.round(n(whisperBestOf, 1))}
              oninput={(e) => (whisperBestOf = e.currentTarget.value)}
            />
            <input class="slider-value" bind:value={whisperBestOf} inputmode="numeric" autocomplete="off" />
          </div>
        </div>
        <div class="field">
          <label for="wpat">Patience</label>
          <div class="slider-row">
            <input
              id="wpat"
              type="range"
              min="0"
              max="2"
              step="0.1"
              value={n(whisperPatience, 1)}
              oninput={(e) => (whisperPatience = e.currentTarget.value)}
            />
            <input class="slider-value" bind:value={whisperPatience} inputmode="decimal" autocomplete="off" />
          </div>
        </div>
        <div class="field">
          <label for="wtemp">Temperature</label>
          <div class="slider-row">
            <input
              id="wtemp"
              type="range"
              min="0"
              max="1"
              step="0.05"
              value={n(whisperTemperature, 0)}
              oninput={(e) => (whisperTemperature = e.currentTarget.value)}
            />
            <input class="slider-value" bind:value={whisperTemperature} inputmode="decimal" autocomplete="off" />
          </div>
        </div>
        <div class="field">
          <label for="wns">No-speech cutoff</label>
          <div class="slider-row">
            <input
              id="wns"
              type="range"
              min="0"
              max="1"
              step="0.02"
              value={n(whisperNoSpeechThreshold, 0.78)}
              oninput={(e) => (whisperNoSpeechThreshold = e.currentTarget.value)}
            />
            <input class="slider-value" bind:value={whisperNoSpeechThreshold} inputmode="decimal" autocomplete="off" />
          </div>
        </div>
        <div class="field">
          <label for="wlogp">Log probability cutoff</label>
          <div class="slider-row">
            <input
              id="wlogp"
              type="range"
              min="-2"
              max="0"
              step="0.05"
              value={n(whisperLogProbThreshold, -0.55)}
              oninput={(e) => (whisperLogProbThreshold = e.currentTarget.value)}
            />
            <input class="slider-value" bind:value={whisperLogProbThreshold} inputmode="decimal" autocomplete="off" />
          </div>
        </div>
        <div class="field">
          <label for="wcr">Compression ratio cutoff</label>
          <div class="slider-row">
            <input
              id="wcr"
              type="range"
              min="1"
              max="3"
              step="0.05"
              value={n(whisperCompressionRatioThreshold, 1.9)}
              oninput={(e) => (whisperCompressionRatioThreshold = e.currentTarget.value)}
            />
            <input class="slider-value" bind:value={whisperCompressionRatioThreshold} inputmode="decimal" autocomplete="off" />
          </div>
        </div>
        <div class="field">
          <label for="whall">Hallucination silence cutoff</label>
          <div class="slider-row">
            <input
              id="whall"
              type="range"
              min="0.5"
              max="3"
              step="0.1"
              value={n(whisperHallucinationSilenceThreshold, 1.6)}
              oninput={(e) => (whisperHallucinationSilenceThreshold = e.currentTarget.value)}
            />
            <input class="slider-value" bind:value={whisperHallucinationSilenceThreshold} inputmode="decimal" autocomplete="off" />
          </div>
        </div>
      </div>
      <div class="field">
        <label for="wlang">Language</label>
        <select id="wlang" bind:value={whisperLanguage}>
          <option value="">Auto-detect</option>
          <option value="en">English</option>
          <option value="es">Spanish</option>
          <option value="fr">French</option>
          <option value="de">German</option>
          <option value="it">Italian</option>
          <option value="pt">Portuguese</option>
          <option value="ja">Japanese</option>
          <option value="zh">Chinese</option>
        </select>
        <p class="field-hint">Leave on Auto unless the wrong language keeps appearing.</p>
      </div>
      <div class="field">
        <label for="wprompt">Vocabulary hint (optional)</label>
        <textarea id="wprompt" rows="2" bind:value={whisperInitialPrompt} placeholder="e.g. medical terms, names"></textarea>
        <p class="field-hint">Short phrases nudge word choice; long text hurts quality.</p>
      </div>
      <label class="check">
        <input type="checkbox" bind:checked={whisperConditionOnPrevious} />
        Use earlier text for context (smoother paragraphs; mistakes can carry forward)
      </label>
      <label class="check">
        <input type="checkbox" bind:checked={whisperVadFilterPcm} />
        Extra voice detection on live mic (can drop normal speech — leave off unless needed)
      </label>
      <label class="check">
        <input type="checkbox" bind:checked={whisperVadFilterFile} />
        Voice detection on file uploads (recommended)
      </label>
    {/if}

    <label class="check">
      <input type="checkbox" bind:checked={mock} />
      Demo mode — fake text only, no real transcription
    </label>
    <div class="field diag-block">
      <p class="muted short">
        If downloads or the model seem stuck, check where files are stored and what the app thinks is configured.
      </p>
      <button type="button" class="btn" onclick={refreshModelCacheDiagnostic}>
        Check model folder &amp; status
      </button>
      {#if cacheDiagnosticText}
        <p class="warn">{cacheDiagnosticText}</p>
      {/if}
      {#if cacheDiagnosticData}
        <ul class="diag-list">
          <li>
            Model folder:
            {#if cacheDiagnosticData.cacheDirExists}
              found
            {:else}
              not found yet
            {/if}
          </li>
          <li class="diag-path">{cacheDiagnosticData.cacheDir}</li>
          <li>{cacheDiagnosticData.topLevelEntries.length} items in that folder</li>
          <li>Selected model size: {cacheDiagnosticData.settings.whisperModel}</li>
          <li>Demo mode: {cacheDiagnosticData.settings.mockTranscription ? "on" : "off"}</li>
          <li>Load-on-demand: {cacheDiagnosticData.settings.lazyLoadWhisper ? "on" : "off"}</li>
          <li>Processor setting: {cacheDiagnosticData.settings.whisperDevice}</li>
          <li>Number format: {cacheDiagnosticData.settings.computeType}</li>
        </ul>
        <details class="diag-raw">
          <summary>Technical details</summary>
          <pre class="cache-diag">{JSON.stringify(cacheDiagnosticData, null, 2)}</pre>
        </details>
      {/if}
    </div>
    <button type="button" class="btn btn-primary" onclick={restartEngine}>
      Save &amp; restart engine
    </button>
  </div>

  <div class="panel block" id="gpu-deps">
    <h2>NVIDIA GPU helpers</h2>
    <p class="muted short">
      Needed for GPU-accelerated Whisper on Windows (large one-time download, ~800&nbsp;MB). Model files are separate.
      Linux uses your Python environment instead.
    </p>
    {#if !cuda}
      <p class="warn">No NVIDIA GPU was detected. Install the latest GPU driver from NVIDIA first.</p>
    {/if}
    <button
      type="button"
      class="btn btn-primary"
      disabled={nvidiaInstallBusy}
      onclick={installNvidiaWhisperLibs}
    >
      {nvidiaInstallBusy ? "Installing…" : "Install GPU libraries for Whisper"}
    </button>
    {#if nvidiaInstallLog}
      <pre class="install-log">{nvidiaInstallLog}</pre>
    {/if}
  </div>

  <div class="panel block">
    <h2>Microphone</h2>
    <p class="muted short">Used for push-to-talk and dictation in the app. Sliders adjust sensitivity and volume shaping.</p>
    <div class="field">
      <label for="mic">Microphone</label>
      <select id="mic" bind:value={inputDeviceId}>
        {#each micDevices as d}
          <option value={d.id}>{d.label}</option>
        {/each}
      </select>
    </div>
    <div class="field">
      <label for="vad">Background noise gate</label>
      <div class="slider-row">
        <input
          id="vad"
          type="range"
          min="0.004"
          max="0.06"
          step="0.001"
          value={n(vadEnergyThreshold, 0.008)}
          oninput={(e) => (vadEnergyThreshold = e.currentTarget.value)}
        />
        <input class="slider-value" type="text" bind:value={vadEnergyThreshold} inputmode="decimal" autocomplete="off" />
      </div>
      <p class="field-hint">Higher = ignore more room noise; lower if soft speech is cut off.</p>
    </div>
    <div class="field">
      <label for="vadms">Pause length before a new phrase (ms)</label>
      <div class="slider-row">
        <input
          id="vadms"
          type="range"
          min="100"
          max="1200"
          step="10"
          value={Math.round(n(vadMinSilenceMs, 300))}
          oninput={(e) => (vadMinSilenceMs = e.currentTarget.value)}
        />
        <input class="slider-value" type="text" bind:value={vadMinSilenceMs} inputmode="numeric" autocomplete="off" />
      </div>
      <p class="field-hint">Shorter breaks speech into smaller pieces; longer keeps sentences together.</p>
    </div>
    <div class="field">
      <label for="peak">Recording loudness target</label>
      <div class="slider-row">
        <input
          id="peak"
          type="range"
          min="0.5"
          max="0.95"
          step="0.01"
          value={n(micNormalizePeak, 0.88)}
          oninput={(e) => (micNormalizePeak = e.currentTarget.value)}
        />
        <input class="slider-value" type="text" bind:value={micNormalizePeak} inputmode="decimal" autocomplete="off" />
      </div>
      <p class="field-hint">Lower if a hot mic clips; higher if you speak quietly.</p>
    </div>
    <div class="field">
      <label for="mgain">Maximum mic boost</label>
      <div class="slider-row">
        <input
          id="mgain"
          type="range"
          min="4"
          max="24"
          step="0.5"
          value={n(micMaxGain, 12)}
          oninput={(e) => (micMaxGain = e.currentTarget.value)}
        />
        <input class="slider-value" type="text" bind:value={micMaxGain} inputmode="decimal" autocomplete="off" />
      </div>
      <p class="field-hint">Raise if transcripts are empty; lower if sound distorts.</p>
    </div>
    <button type="button" class="btn" onclick={saveMicrophoneOnly}>Save microphone settings</button>
  </div>

  <div class="panel block">
    <h2>Written output style</h2>
    <p class="muted short">How punctuation and cleanup are applied after transcription.</p>
    <div class="field">
      <label for="tone">Style</label>
      <select id="tone" bind:value={tonePreset}>
        <option value="minimal">Minimal</option>
        <option value="standard">Standard</option>
        <option value="expressive">Expressive</option>
      </select>
    </div>
    <button type="button" class="btn" onclick={saveCore}>Save style</button>
  </div>

  <div class="panel block">
    <h2>Keyboard shortcuts</h2>
    <p class="muted short">
      Use <strong>Record shortcut</strong> and press the real keys (modifiers + one key). Esc cancels. You can still edit the
      text field manually. Global shortcuts need the inference engine running for dictation.
    </p>
    {#if captureTarget}
      <p class="capture-hint" role="status">
        Listening for <strong>{captureTarget.replaceAll("_", " ")}</strong> — press a combination, or
        <button type="button" class="linkish" onclick={() => (captureTarget = null)}>cancel</button>.
      </p>
    {/if}
    <div class="field">
      <label for="k1">Push to talk</label>
      <div class="keybind-row">
        <input id="k1" class="mono keybind-input" bind:value={kPtt} autocomplete="off" spellcheck="false" />
        <button
          type="button"
          class="btn keybind-record"
          class:active={captureTarget === "push_to_talk"}
          onclick={() =>
            (captureTarget = captureTarget === "push_to_talk" ? null : "push_to_talk")}
        >
          {captureTarget === "push_to_talk" ? "Listening…" : "Record shortcut"}
        </button>
      </div>
    </div>
    <div class="field">
      <label for="k2">Toggle open mic</label>
      <div class="keybind-row">
        <input id="k2" class="mono keybind-input" bind:value={kMic} autocomplete="off" spellcheck="false" />
        <button
          type="button"
          class="btn keybind-record"
          class:active={captureTarget === "toggle_open_mic"}
          onclick={() =>
            (captureTarget =
              captureTarget === "toggle_open_mic" ? null : "toggle_open_mic")}
        >
          {captureTarget === "toggle_open_mic" ? "Listening…" : "Record shortcut"}
        </button>
      </div>
    </div>
    <div class="field">
      <label for="k3">Stop dictation</label>
      <div class="keybind-row">
        <input id="k3" class="mono keybind-input" bind:value={kStop} autocomplete="off" spellcheck="false" />
        <button
          type="button"
          class="btn keybind-record"
          class:active={captureTarget === "stop_dictation"}
          onclick={() =>
            (captureTarget = captureTarget === "stop_dictation" ? null : "stop_dictation")}
        >
          {captureTarget === "stop_dictation" ? "Listening…" : "Record shortcut"}
        </button>
      </div>
    </div>
    {#if conflict.length}
      <p class="warn">Shortcut already used by: {conflict.join(", ")}</p>
    {/if}
    <button type="button" class="btn btn-primary" onclick={saveKeybinds}>
      Save keybinds
    </button>
  </div>
</section>

<style>
  h1 {
    margin-top: 0;
  }
  h2 {
    margin: 0 0 1rem;
    font-size: 1.1rem;
  }
  .short {
    max-width: 42rem;
  }
  .theme-toggle {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    padding: 0.25rem;
    border-radius: 10px;
    border: 1px solid var(--border);
    background: var(--bg);
    width: fit-content;
    max-width: 100%;
  }
  .theme-seg {
    border: none;
    background: transparent;
    color: var(--text-muted);
    font-weight: 600;
    font-size: 0.85rem;
    padding: 0.45rem 0.85rem;
    border-radius: 8px;
    cursor: pointer;
    transition: background 0.12s, color 0.12s;
  }
  .theme-seg:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .theme-seg.active {
    color: var(--text);
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent) 35%, transparent);
  }
  kbd {
    font-family: ui-monospace, monospace;
    font-size: 0.82em;
    padding: 0.12rem 0.35rem;
    border-radius: 4px;
    border: 1px solid var(--border);
    background: var(--bg);
  }
  .diag-list {
    margin: 0.75rem 0 0;
    padding-left: 1.2rem;
    color: var(--text-muted);
    font-size: 0.88rem;
    line-height: 1.5;
  }
  .diag-list li {
    margin-bottom: 0.25rem;
  }
  .diag-path {
    word-break: break-all;
    font-size: 0.8rem;
  }
  .diag-raw {
    margin-top: 0.75rem;
  }
  .diag-raw summary {
    cursor: pointer;
    color: var(--accent);
    font-size: 0.88rem;
    font-weight: 600;
  }
  .settings-subh {
    margin: 1.25rem 0 0.5rem;
    font-size: 1rem;
    font-weight: 700;
  }
  .whisper-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0.5rem 1rem;
    margin-bottom: 0.5rem;
  }
  @media (max-width: 560px) {
    .whisper-grid {
      grid-template-columns: 1fr;
    }
  }
  .block {
    margin-bottom: 1.5rem;
  }
  .field-hint {
    margin: 0.35rem 0 0;
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .muted {
    color: var(--text-muted);
    font-size: 0.88rem;
    margin-top: 0;
  }
  .note {
    font-size: 0.85rem;
    color: var(--text-muted);
  }
  .check {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 1rem;
    font-size: 0.92rem;
  }
  .keybind-row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: center;
  }
  .keybind-input {
    flex: 1;
    min-width: 10rem;
    font-size: 0.9rem;
  }
  .keybind-record {
    flex-shrink: 0;
    font-size: 0.85rem;
  }
  .keybind-record.active {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 22%, var(--bg-elevated));
  }
  .capture-hint {
    margin: 0 0 1rem;
    padding: 0.55rem 0.75rem;
    border-radius: 8px;
    border: 1px solid color-mix(in srgb, var(--accent) 40%, var(--border));
    background: color-mix(in srgb, var(--accent) 10%, var(--bg-elevated));
    font-size: 0.88rem;
  }
  .linkish {
    border: none;
    background: none;
    padding: 0;
    color: var(--accent);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
    text-decoration: underline;
  }
  .mono {
    font-family: ui-monospace, monospace;
  }
  .warn {
    color: var(--danger);
    font-size: 0.88rem;
  }
  .install-log {
    margin-top: 0.75rem;
    padding: 0.65rem 0.75rem;
    font-size: 0.8rem;
    white-space: pre-wrap;
    word-break: break-word;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    max-height: 14rem;
    overflow: auto;
  }
  .diag-block {
    margin: 1rem 0;
  }
  .cache-diag {
    margin-top: 0.75rem;
    padding: 0.65rem 0.75rem;
    font-size: 0.78rem;
    white-space: pre-wrap;
    word-break: break-word;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    max-height: 18rem;
    overflow: auto;
  }
  .panel a {
    color: var(--accent);
  }
  .token-row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: center;
  }
  .token-row input {
    flex: 1;
    min-width: 10rem;
  }
  .node-actions {
    margin: 1rem 0 0.5rem;
  }
  .node-status-line {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.35rem 0.6rem;
    margin: 0.75rem 0 0.25rem;
  }
  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--text-muted);
    flex-shrink: 0;
  }
  .dot.on {
    background: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 35%, transparent);
  }
  .url-list {
    margin: 0.35rem 0 0;
    padding-left: 1.1rem;
    font-size: 0.88rem;
    color: var(--text-muted);
  }
  .url-list li {
    margin-bottom: 0.35rem;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
  }
  .ws-url {
    font-size: 0.82rem;
    word-break: break-all;
  }
  .btn-tiny {
    font-size: 0.78rem;
    padding: 0.25rem 0.5rem;
  }
  .node-log {
    margin-top: 0.75rem;
  }
  .node-log summary {
    cursor: pointer;
    color: var(--accent);
    font-size: 0.88rem;
    font-weight: 600;
  }
  .node-log-pre {
    margin-top: 0.5rem;
    padding: 0.55rem 0.65rem;
    font-size: 0.75rem;
    white-space: pre-wrap;
    word-break: break-word;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    max-height: 12rem;
    overflow: auto;
  }
  .mono-p {
    font-family: ui-monospace, monospace;
    font-size: 0.78rem;
    word-break: break-all;
  }
  .btn-stop {
    background: color-mix(in srgb, var(--danger) 18%, var(--bg-elevated));
    border-color: color-mix(in srgb, var(--danger) 55%, var(--border));
    color: var(--text);
  }
  .btn-stop:hover:not(:disabled) {
    border-color: var(--danger);
    background: color-mix(in srgb, var(--danger) 28%, var(--bg-elevated));
  }
  .update-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin-top: 0.75rem;
  }
  .update-banner {
    margin: 0.75rem 0 0.35rem;
    font-size: 0.92rem;
  }
  .update-notes {
    margin: 0.5rem 0 0;
    padding: 0.55rem 0.65rem;
    font-size: 0.82rem;
    white-space: pre-wrap;
    word-break: break-word;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    max-height: 10rem;
    overflow: auto;
  }
</style>

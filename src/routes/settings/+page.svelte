<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { getVersion } from "@tauri-apps/api/app";
  import { check, type Update } from "@tauri-apps/plugin-updater";
  import { relaunch } from "@tauri-apps/plugin-process";
  import { afterNavigate } from "$app/navigation";
  import { onMount } from "svelte";
  import { createShortcutCaptureSession } from "$lib/keybindCapture";
  import {
    applyUiTheme,
    loadHighContrast,
    loadUiTheme,
    persistHighContrast,
    persistUiTheme,
    type UiTheme,
  } from "$lib/theme";
  import {
    DEFAULT_PARAKEET_MODEL,
    migrateParakeetModelId,
    PARAKEET_MODEL_OPTIONS,
    parakeetDiskMb,
  } from "$lib/parakeetModelInfo";
  import {
    DEFAULT_LIVE_STREAMING_ENGINE,
    DEFAULT_LIVE_STREAMING_MODEL,
    LIVE_STREAMING_ENGINES,
    liveStreamingModelsForEngine,
  } from "$lib/liveStreamingModelInfo";
  import {
    formatStorageMb,
    isMlxWhisperModelId,
    whisperDiskMb,
    whisperRuntimeMbHint,
    WHISPER_MODEL_OPTIONS,
    WHISPER_MODEL_OPTIONS_MLX,
  } from "$lib/whisperModelInfo";
  import { formatShortcutDisplay } from "$lib/formatShortcutDisplay";

  function n(s: string, fallback: number): number {
    const v = Number.parseFloat(s);
    return Number.isFinite(v) ? v : fallback;
  }

  let uiTheme = $state<UiTheme>("system");
  let uiHighContrast = $state(false);
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
  let grammarRestore = $state("auto");
  let mock = $state(false);
  let cuda = $state<boolean | null>(null);
  let gpuError = $state("");
  let whisperDevice = $state("auto");
  let inputDeviceId = $state("");
  let micDevices = $state<{ id: string; label: string }[]>([]);
  let dictationSoundCues = $state(false);
  let microphoneFallback = $state(true);
  let microphoneMessage = $state("");
  let microphoneError = $state("");
  let microphoneSaving = $state(false);
  let microphoneRefreshing = $state(false);
  let microphoneListLoaded = $state(false);
  let engineProgress = $state("");
  const microphoneMissing = $derived(microphoneListLoaded && Boolean(inputDeviceId) && !micDevices.some(d => d.id === inputDeviceId));

  async function refreshMicrophones() {
    if (microphoneRefreshing) return;
    microphoneRefreshing = true;
    try {
      micDevices = await invoke<{ id: string; label: string }[]>("list_audio_input_devices");
      microphoneListLoaded = true;
      microphoneError = "";
    } catch (e) { microphoneError = String(e); }
    finally { microphoneRefreshing = false; }
  }

  onMount(() => {
    void initializeSettings();
    void refreshMicrophones();
    void invoke<boolean>("cuda_available").then(value => cuda = value).catch(e => gpuError = String(e));
    const timer = setInterval(() => {
      if (restartingEngine && document.visibilityState === "visible") void invoke<{ message: string }>("engine_progress").then(p => engineProgress = p.message).catch(() => {});
    }, 2000);
    return () => clearInterval(timer);
  });

  let adaptiveMicrophone = $state(true);
  let backgroundDictation = $state(true);
  let dictionaryHints = $state(true);
  let presetMessage = $state("");
  let settingsError = $state("");
  let settingsLoading = $state(true);
  let settingsLoaded = $state(false);
  let settingsSaving = $state(false);
  let settingsMessage = $state("");
  let loadError = $state("");
  let savedKeybinds: Record<string, string> = {};

  async function initializeSettings() {
    settingsLoading = true;
    settingsLoaded = false;
    loadError = "";
    try {
      await load();
      settingsLoaded = true;
    } catch (e) {
      loadError = `Could not load saved settings: ${String(e)}`;
    } finally { settingsLoading = false; }
  }

  function requireLoadedSettings() {
    if (!settingsLoaded) throw new Error("Wait for saved settings to load before saving.");
  }


  let restartingEngine = $state(false);
  const selectedSpeed = $derived.by(() => {
    if (engine !== "whisper" || whisperBestOf !== "1" || whisperPatience !== "1" || whisperTemperature !== "0" || computeType !== "int8") return "custom";
    for (const [preset, size, beam] of [["fast", "base", "1"], ["balanced", "small", "3"], ["accurate", "medium", "5"]]) {
      if (whisperModel === (appleSilicon ? `mlx-community/whisper-${size}-mlx` : size) && whisperBeamSize === beam) return preset;
    }
    return "custom";
  });

  function choosePreset(preset: "fast" | "balanced" | "accurate") {
    engine = "whisper";
    const size = preset === "fast" ? "base" : preset === "balanced" ? "small" : "medium";
    whisperModel = appleSilicon ? `mlx-community/whisper-${size}-mlx` : size;
    whisperBeamSize = preset === "fast" ? "1" : preset === "balanced" ? "3" : "5";
    whisperBestOf = "1";
    whisperPatience = "1";
    whisperTemperature = "0";
    computeType = "int8";
    presetMessage = `${preset[0].toUpperCase() + preset.slice(1)} selected. Save and restart to apply; the model may need a download.`;
  }

  function chooseReadiness(ready: boolean) {
    lazyLoadWhisper = !ready;
    modelIdleUnloadMins = ready ? "0" : "5";
  }

  let vadEnergyThreshold = $state("0.008");
  let vadMinSilenceMs = $state("500");
  let micNormalizePeak = $state("0.88");
  let micMaxGain = $state("12");
  let lazyLoadWhisper = $state(false);
  let hudWidgetEnabled = $state(true);
  let hudWidgetStyle = $state("classic");
  let modelIdleUnloadMins = $state("0");
  const selectedReadiness = $derived(!lazyLoadWhisper && modelIdleUnloadMins === "0" ? "ready" : lazyLoadWhisper && modelIdleUnloadMins === "5" ? "memory" : "custom");

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

  let liveDictationExperimental = $state(false);
  let liveStreamingEngine = $state(DEFAULT_LIVE_STREAMING_ENGINE);
  let liveStreamingModel = $state(DEFAULT_LIVE_STREAMING_MODEL);
  let liveFeedIntervalMs = $state("200");
  let liveMinAudioMs = $state("800");

  const liveStreamingModelChoices = $derived(
    liveStreamingModelsForEngine(liveStreamingEngine),
  );

  let kPtt = $state("");
  let kMic = $state("");
  let kStop = $state("");
  /** From Rust `cfg!(target_os = "macos")` — drives ⌘⌃⌥⇧ vs Ctrl/Win labels. */
  let shortcutUiMac = $state(false);
  /** macOS app build — hide NVIDIA-only UI (Parakeet helpers, CUDA). */
  let appIsMac = $state(false);
  /** Apple Silicon only — MLX Whisper model list and Metal-backed inference. */
  let appleSilicon = $state(false);

  const whisperModelChoices = $derived(
    appleSilicon ? WHISPER_MODEL_OPTIONS_MLX : WHISPER_MODEL_OPTIONS,
  );
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
    uiHighContrast = await loadHighContrast();
    applyUiTheme(uiTheme, uiHighContrast);
    try {
      const chrome = await invoke<{ macos: boolean; appleSilicon?: boolean }>(
        "hud_chrome_info",
      );
      shortcutUiMac = chrome.macos;
      appIsMac = chrome.macos;
      appleSilicon = Boolean(chrome.appleSilicon);
    } catch {
      shortcutUiMac = false;
      appIsMac = false;
      appleSilicon = false;
    }
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
    if (appleSilicon) {
      if (whisperModel === "mlx-community/whisper-large-v3-turbo-mlx") {
        whisperModel = "mlx-community/whisper-large-v3-turbo";
      }
      const legacyToMlx: Record<string, string> = {
        tiny: "mlx-community/whisper-tiny-mlx",
        base: "mlx-community/whisper-base-mlx",
        small: "mlx-community/whisper-small-mlx",
        medium: "mlx-community/whisper-medium-mlx",
        "large-v3": "mlx-community/whisper-large-v3-mlx",
      };
      if (legacyToMlx[whisperModel]) {
        whisperModel = legacyToMlx[whisperModel]!;
      }
      if (!WHISPER_MODEL_OPTIONS_MLX.some((o) => o.id === whisperModel)) {
        whisperModel = "mlx-community/whisper-base-mlx";
      }
    } else if (isMlxWhisperModelId(whisperModel)) {
      whisperModel = "base";
    }
    {
      const pk = migrateParakeetModelId(
        (await invoke<string | null>("get_setting_cmd", { key: "parakeet_model" })) ??
          DEFAULT_PARAKEET_MODEL,
      );
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
    grammarRestore =
      (await invoke<string | null>("get_setting_cmd", { key: "grammar_restore" })) ?? "auto";
    if (!["off", "auto", "always"].includes(grammarRestore)) {
      grammarRestore = "auto";
    }
    const m = await invoke<string | null>("get_setting_cmd", {
      key: "mock_transcription",
    });
    mock = m === "true";

    whisperDevice =
      (await invoke<string | null>("get_setting_cmd", { key: "whisper_device" })) ?? "auto";
    if (appIsMac && whisperDevice === "cuda") {
      whisperDevice = "auto";
    }
    inputDeviceId =
      (await invoke<string | null>("get_setting_cmd", { key: "input_device_name" })) ?? "";
    dictationSoundCues = (await invoke<string | null>("get_setting_cmd", { key: "dictation_sound_cues" })) === "true";
    microphoneFallback = (await invoke<string | null>("get_setting_cmd", { key: "microphone_auto_fallback" })) !== "false";
    adaptiveMicrophone = (await invoke<string | null>("get_setting_cmd", { key: "adaptive_microphone" })) !== "false";
    backgroundDictation = (await invoke<string | null>("get_setting_cmd", { key: "background_dictation" })) !== "false";
    dictionaryHints = (await invoke<string | null>("get_setting_cmd", { key: "dictionary_recognition_hints" })) !== "false";
    vadEnergyThreshold =
      (await invoke<string | null>("get_setting_cmd", { key: "vad_energy_threshold" })) ??
      "0.008";
    vadMinSilenceMs =
      (await invoke<string | null>("get_setting_cmd", { key: "vad_min_silence_ms" })) ?? "500";
    micNormalizePeak =
      (await invoke<string | null>("get_setting_cmd", { key: "mic_normalize_peak" })) ?? "0.88";
    micMaxGain =
      (await invoke<string | null>("get_setting_cmd", { key: "mic_max_gain" })) ?? "12";

    lazyLoadWhisper =
      (await invoke<string | null>("get_setting_cmd", { key: "lazy_load_whisper" })) === "true";
    hudWidgetStyle = (await invoke<string | null>("get_setting_cmd", { key: "hud_widget_style" })) === "controls" ? "controls" : "classic";
    hudWidgetEnabled =
      (await invoke<string | null>("get_setting_cmd", { key: "hud_widget_enabled" })) !== "false";
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

    liveDictationExperimental =
      (await invoke<string | null>("get_setting_cmd", {
        key: "live_dictation_experimental",
      })) === "true";
    liveStreamingEngine =
      (await invoke<string | null>("get_setting_cmd", { key: "live_streaming_engine" })) ??
      DEFAULT_LIVE_STREAMING_ENGINE;
    liveStreamingModel =
      (await invoke<string | null>("get_setting_cmd", { key: "live_streaming_model" })) ??
      DEFAULT_LIVE_STREAMING_MODEL;
    if (!liveStreamingModelsForEngine(liveStreamingEngine).some((o) => o.id === liveStreamingModel)) {
      liveStreamingModel = liveStreamingModelsForEngine(liveStreamingEngine)[0]?.id ?? DEFAULT_LIVE_STREAMING_MODEL;
    }
    liveFeedIntervalMs =
      (await invoke<string | null>("get_setting_cmd", { key: "live_feed_interval_ms" })) ??
      (await invoke<string | null>("get_setting_cmd", { key: "live_chunk_interval_ms" })) ??
      "200";
    liveMinAudioMs =
      (await invoke<string | null>("get_setting_cmd", { key: "live_min_audio_ms" })) ?? "800";

    const binds = await invoke<{ action: string; shortcut: string }[]>(
      "list_keybinds_cmd",
    );
    savedKeybinds = Object.fromEntries(binds.map(b => [b.action, b.shortcut]));
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
    // Settings and device discovery load once on mount, never on hash navigation.
    if (to?.url.hash) {
      const target = document.getElementById(to.url.hash.slice(1));
      let details = target?.closest("details");
      while (details) {
        details.open = true;
        details = details.parentElement?.closest("details") ?? null;
      }
      target?.scrollIntoView({ block: "start" });
    }
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
    applyUiTheme(mode, uiHighContrast);
    await persistUiTheme(mode);
  }

  async function changeHighContrast(on: boolean) {
    uiHighContrast = on;
    applyUiTheme(uiTheme, on);
    await persistHighContrast(on);
  }

  async function saveCore() {
    requireLoadedSettings();
    await invoke("set_setting_cmd", { key: "background_dictation", value: String(backgroundDictation) });
    await invoke("set_setting_cmd", { key: "dictionary_recognition_hints", value: String(dictionaryHints) });
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
    await invoke("set_setting_cmd", { key: "grammar_restore", value: grammarRestore });
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
    await invoke("set_setting_cmd", {
      key: "live_dictation_experimental",
      value: liveDictationExperimental ? "true" : "false",
    });
    await invoke("set_setting_cmd", {
      key: "live_streaming_engine",
      value: liveStreamingEngine,
    });
    await invoke("set_setting_cmd", {
      key: "live_streaming_model",
      value: liveStreamingModel,
    });
    await invoke("set_setting_cmd", {
      key: "live_feed_interval_ms",
      value: String(Math.round(n(liveFeedIntervalMs, 200))),
    });
    await invoke("set_setting_cmd", {
      key: "live_min_audio_ms",
      value: String(Math.round(n(liveMinAudioMs, 800))),
    });
    await invoke("set_setting_cmd", {
      key: "hud_widget_enabled",
      value: hudWidgetEnabled ? "true" : "false",
    });
    try {
      await invoke("hud_sync_visibility_cmd");
    } catch {
      /* ignore if not running under Tauri */
    }
  }

  async function saveOutputStyle() {
    if (!settingsLoaded || settingsSaving) return;
    settingsSaving = true;
    settingsError = "";
    settingsMessage = "";
    try {
      await invoke("set_setting_cmd", { key: "tone_preset", value: tonePreset });
      await invoke("set_setting_cmd", { key: "grammar_restore", value: grammarRestore });
      settingsMessage = "Output style saved.";
    } catch (e) { settingsError = String(e); }
    finally { settingsSaving = false; }
  }

  async function persistHudStyle() {
    try {
      await invoke("set_setting_cmd", { key: "hud_widget_style", value: hudWidgetStyle });
      await invoke("hud_sync_visibility_cmd");
    } catch (e) { settingsError = String(e); }
  }

  async function persistHudWidget() {
    try {
      await invoke("set_setting_cmd", {
        key: "hud_widget_enabled",
        value: hudWidgetEnabled ? "true" : "false",
      });
      await invoke("hud_sync_visibility_cmd");
    } catch (e) { settingsError = String(e); }
  }

  async function saveMicrophoneOnly() {
    if (!settingsLoaded || microphoneSaving) return;
    microphoneSaving = true;
    microphoneMessage = "";
    microphoneError = "";
    try {
    await invoke("set_setting_cmd", { key: "dictation_sound_cues", value: String(dictationSoundCues) });
    await invoke("set_setting_cmd", { key: "microphone_auto_fallback", value: String(microphoneFallback) });
    await invoke("set_setting_cmd", { key: "adaptive_microphone", value: String(adaptiveMicrophone) });
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
    microphoneMessage = "Microphone and pause preferences saved for the next recording.";
    } catch (e) { microphoneError = String(e); }
    finally { microphoneSaving = false; }
  }

  async function saveKeybinds() {
    if (!settingsLoaded || settingsSaving) return;
    settingsSaving = true;
    settingsError = "";
    settingsMessage = "";
    conflict = [];
    try {
      for (const [action, shortcut] of Object.entries({ push_to_talk: kPtt, toggle_open_mic: kMic, stop_dictation: kStop })) {
        if (shortcut === savedKeybinds[action]) continue;
        const conflicts = await invoke<string[]>("set_keybind_cmd", { action, shortcut });
        if (conflicts.length) conflict = [...conflict, ...conflicts];
        else savedKeybinds[action] = shortcut;
      }
      const status = await invoke<string>("refresh_global_shortcuts");
      settingsMessage = conflict.length ? "Resolve shortcut conflicts, then save again." : `Shortcuts saved. ${status}`;
    } catch (e) { settingsError = String(e); }
    finally { settingsSaving = false; }
  }

  async function restartEngine() {
    if (restartingEngine) return;
    restartingEngine = true;
    engineProgress = "";
    presetMessage = "Saving settings and restarting the engine…";
    settingsError = "";
    try {
      await saveCore();
      await invoke("engine_stop");
      await invoke("engine_start");
      presetMessage = "Settings applied. Engine ready.";
    } catch (e) { presetMessage = ""; settingsError = String(e); }
    finally { restartingEngine = false; }
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
  {#if settingsLoading}<p role="status">Loading saved settings…</p>{/if}
  {#if loadError}
    <p class="warn" role="alert">{loadError}</p>
    <button class="btn" onclick={initializeSettings}>Retry loading settings</button>
  {/if}
  {#if settingsError}<p class="warn" role="alert">{settingsError}</p>{/if}
  {#if settingsMessage}<p role="status">{settingsMessage}</p>{/if}
  <fieldset class="settings-fields" disabled={!settingsLoaded || settingsSaving}>

  <div class="panel block">
    <h2>Appearance</h2>
    <p class="muted short">Choose how Yapper looks. “Match system” follows Windows light/dark.</p>
    <div class="theme-toggle" role="group" aria-label="Color theme">
      <button
        type="button"
        class="theme-seg"
        class:active={uiTheme === "light"}
        aria-pressed={uiTheme === "light"}
        onclick={() => changeUiTheme("light")}>Light</button>
      <button
        type="button"
        class="theme-seg"
        class:active={uiTheme === "dark"}
        aria-pressed={uiTheme === "dark"}
        onclick={() => changeUiTheme("dark")}>Dark</button>
      <button
        type="button"
        class="theme-seg"
        class:active={uiTheme === "system"}
        aria-pressed={uiTheme === "system"}
        onclick={() => changeUiTheme("system")}>Match system</button>
    </div>
    <label class="check contrast-check">
      <input
        type="checkbox"
        checked={uiHighContrast}
        onchange={(e) => changeHighContrast(e.currentTarget.checked)}
      />
      High contrast (stronger borders and text)
    </label>
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

  <details class="panel block advanced" id="instance-role">
    <summary>Advanced installation options</summary>
    <p class="muted short">Use this computer for dictation, or share its speech engine with your other devices.</p>
    <div class="theme-toggle" role="group" aria-label="Primary use of this PC">
      <button
        type="button"
        class="theme-seg"
        class:active={instanceRole === "dictation"}
        aria-pressed={instanceRole === "dictation"}
        onclick={() => setInstanceRole("dictation")}>Dictation on this PC</button>
      <button
        type="button"
        class="theme-seg"
        class:active={instanceRole === "network_server"}
        aria-pressed={instanceRole === "network_server"}
        onclick={() => setInstanceRole("network_server")}>Network processing server</button>
    </div>
  </details>

  {#if instanceRole === "network_server"}
  <div class="panel block" id="processing-server">
    <h2>Network processing server (Yapper Node)</h2>
    <p class="muted short">Share transcription with your other devices over a local network or VPN. Install server requirements with <code>pip install -r yapper-node/requirements.txt</code>. Don’t expose the server directly to the internet.</p>
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
    <div class="panel">
      <h3>Dictation speed</h3>
      <p>Larger models may improve accuracy but use more memory.</p>
      <div class="row" role="group" aria-label="Dictation speed">
        <button class="btn" aria-pressed={selectedSpeed === "fast"} onclick={() => choosePreset("fast")}>Fast · base</button>
        <button class="btn" aria-pressed={selectedSpeed === "balanced"} onclick={() => choosePreset("balanced")}>Balanced · small</button>
        <button class="btn" aria-pressed={selectedSpeed === "accurate"} onclick={() => choosePreset("accurate")}>Accurate · medium</button>
      </div>
      <p class="field-hint">Selected: {selectedSpeed === "custom" ? "Custom settings" : selectedSpeed === "fast" ? "Fast" : selectedSpeed === "balanced" ? "Balanced" : "Accurate"}. Save &amp; restart to apply changes.</p>
      {#if presetMessage}<p role="status">{presetMessage}</p>{/if}
      {#if settingsError}<p class="warn" role="alert">{settingsError}</p>{/if}
      <p>Use a recent recording on Home to benchmark this setup before choosing a larger model.</p>
      <h3>When to load the model</h3>
      <div class="row" role="group" aria-label="Engine readiness">
        <button class="btn" aria-pressed={selectedReadiness === "ready"} onclick={() => chooseReadiness(true)}>Ready instantly</button>
        <button class="btn" aria-pressed={selectedReadiness === "memory"} onclick={() => chooseReadiness(false)}>Save memory</button>
      </div>
      <p class="field-hint">Selected: {selectedReadiness === "custom" ? "Custom settings" : selectedReadiness === "ready" ? "Ready instantly" : "Save memory"}. Save &amp; restart to apply changes.</p>
      <p class="field-hint">Keep the model ready, or unload it after five idle minutes to save memory.</p>
    </div>
    <label class="check">
      <input type="checkbox" bind:checked={liveDictationExperimental} />
      Show live preview while recording (experimental)
    </label>
    <p class="field-hint">See words in the widget as you speak. Final text appears when you stop. First use may download a model.</p>
    {#if liveDictationExperimental}<p class="field-hint">Live preview is selected; background phrase processing resumes when preview is off.</p>{/if}
    <details class="advanced">
      <summary>Advanced speech engine settings</summary>
      <p class="field-hint">Models, processor, memory, recognition tuning and troubleshooting.</p>
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
      <label class="check"><input type="checkbox" bind:checked={backgroundDictation} disabled={liveDictationExperimental} />Process completed phrases while I speak</label>
      <p class="field-hint">Process phrases during pauses to reduce the wait after recording. Live preview takes priority when enabled.</p>
      <label class="check"><input type="checkbox" bind:checked={dictionaryHints} />Help Whisper recognize my dictionary words</label>
      <p class="field-hint">Helps recognize words in your dictionary. Restart the engine after adding words.</p>
    <div class="field">
      <label for="eng">Speech engine</label>
      <select id="eng" bind:value={engine}>
        <option value="whisper">Whisper (recommended)</option>
        {#if !appIsMac}
          <option value="parakeet">Parakeet (ONNX — CPU or GPU)</option>
        {:else}
          <option value="parakeet">Parakeet (ONNX — CPU)</option>
        {/if}
      </select>
    </div>
    {#if engine === "parakeet"}
      <p class="note">Includes punctuation and works on CPU or NVIDIA GPU.</p>
    {/if}
    <div class="field">
      {#if engine === "whisper"}
        <label for="wm">Model size</label>
        <select id="wm" bind:value={whisperModel}>
          {#each whisperModelChoices as m}
            <option value={m.id}>
              {m.line} — {formatStorageMb(whisperDiskMb(m.id))} on disk
            </option>
          {/each}
        </select>
        <p class="field-hint">First use downloads the model. Its size affects download time and memory use.</p>
      {:else}
        <label for="wm-pk">Model</label>
        <select id="wm-pk" bind:value={parakeetModel}>
          {#each PARAKEET_MODEL_OPTIONS as m}
            <option value={m.id}>
              {m.line} — {formatStorageMb(parakeetDiskMb(m.id))} on disk
            </option>
          {/each}
        </select>
        <p class="field-hint">First use downloads about 670 MB. Works on CPU or GPU.</p>
      {/if}
    </div>
    {#if engine === "whisper"}
      <p class="note">With load-on-demand, the first recording downloads and loads the model.</p>
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
    <p class="note">Frees memory while idle. The next recording reloads the model without downloading it again.</p>
    {#if engine === "whisper" && !appleSilicon}
      <div class="field">
        <label for="ct">Number format (speed vs. precision)</label>
        <select id="ct" bind:value={computeType}>
          <option value="int8">int8 — smallest memory, fastest</option>
          <option value="float16">float16 — middle ground</option>
          <option value="float32">float32 — largest memory, highest precision</option>
        </select>
        <p class="field-hint">Estimated memory: {formatStorageMb(whisperRuntimeMbHint(whisperModel, computeType))}. Actual use varies.</p>
      </div>
    {:else if engine === "whisper" && appleSilicon}
      <p class="field-hint">Estimated memory: {formatStorageMb(whisperRuntimeMbHint(whisperModel, computeType))}. Apple Silicon uses Metal acceleration.</p>
    {/if}
    <div class="field">
      <label for="wd">Processor</label>
      <select id="wd" bind:value={whisperDevice}>
        <option value="auto">Automatic</option>
        <option value="cpu">CPU only</option>
        {#if !appIsMac}
          <option value="cuda">NVIDIA GPU (CUDA)</option>
        {/if}
      </select>
    </div>
    {#if !appIsMac}
      <p class="note">
        GPU issues? Install the libraries in <a href="#gpu-deps">NVIDIA helpers</a> below, then use Save &amp; restart.
      </p>
    {:else if appleSilicon}
      <p class="note">
        Apple Silicon builds use <strong>MLX Whisper</strong>. Install Python deps with
        <code>pip install -r sidecar/requirements-macos.txt</code> when developing outside a bundled
        runtime.
      </p>
    {/if}

    {#if engine === "whisper"}
      <h3 class="settings-subh">Recognition tuning</h3>
      <p class="note">Use Save &amp; restart to apply changes. Higher values can slow transcription.</p>
      <div class="whisper-grid">
        {#if !appleSilicon}
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
        {/if}
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
        <p class="field-hint">Choose your spoken language for more consistent results on short recordings.</p>
      </div>
      <div class="field">
        <label for="wprompt">Vocabulary hint (optional)</label>
        <textarea
          id="wprompt"
          rows="2"
          bind:value={whisperInitialPrompt}
          placeholder={'e.g. Names: Kane, Vivian. Terms: "myocardial infarction".'}
        ></textarea>
        <p class="field-hint">Add names or specialist terms. Use normal capitalization and punctuation.</p>
      </div>
      <label class="check">
        <input type="checkbox" bind:checked={whisperConditionOnPrevious} />
        Use earlier text for context (may repeat mistakes)
      </label>
      <label class="check">
        <input type="checkbox" bind:checked={whisperVadFilterPcm} />
        Extra silence filtering (may miss quiet speech)
      </label>
      <label class="check">
        <input type="checkbox" bind:checked={whisperVadFilterFile} />
        Voice detection on file uploads (recommended)
      </label>

    {/if}

      <h3 class="settings-subh">Live preview tuning</h3>
      <div class="field">
        <label for="liveEng">Live streaming engine</label>
        <select
          id="liveEng"
          bind:value={liveStreamingEngine}
          disabled={!liveDictationExperimental}
          onchange={() => {
            const choices = liveStreamingModelsForEngine(liveStreamingEngine);
            if (!choices.some((o) => o.id === liveStreamingModel)) {
              liveStreamingModel = choices[0]?.id ?? DEFAULT_LIVE_STREAMING_MODEL;
            }
          }}
        >
          {#each LIVE_STREAMING_ENGINES as e}
            <option value={e.id}>{e.line}</option>
          {/each}
        </select>
      </div>
      <div class="field">
        <label for="liveModel">Live streaming model</label>
        <select
          id="liveModel"
          bind:value={liveStreamingModel}
          disabled={!liveDictationExperimental}
        >
          {#each liveStreamingModelChoices as m}
            <option value={m.id}>{m.line}</option>
          {/each}
        </select>
      </div>
      <div class="field">
        <label for="liveIv">Audio feed interval (ms)</label>
        <input
          id="liveIv"
          type="number"
          min="100"
          max="30000"
          step="50"
          bind:value={liveFeedIntervalMs}
          disabled={!liveDictationExperimental}
        />
        <p class="field-hint">How often to send new audio to the streaming engine (100–30000). Default 200.</p>
      </div>
      <div class="field">
        <label for="liveMin">Minimum audio (ms)</label>
        <input
          id="liveMin"
          type="number"
          min="200"
          max="10000"
          step="50"
          bind:value={liveMinAudioMs}
          disabled={!liveDictationExperimental}
        />
        <p class="field-hint">Skip early ticks if the buffer is shorter than this (200–10000). Default 800.</p>
      </div>

    <label class="check">
      <input type="checkbox" bind:checked={mock} />
      Demo mode — fake text only, no real transcription
    </label>
    <div class="field diag-block">
      <p class="muted short">Check model files and configuration when downloads or loading fail.</p>
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
          <li>
            Number format:
            {#if appleSilicon}
              MLX fp16 on Metal (saved <code>compute_type</code> is
              <code>{cacheDiagnosticData.settings.computeType}</code> — unused for MLX)
            {:else}
              {cacheDiagnosticData.settings.computeType}
            {/if}
          </li>
        </ul>
        <details class="diag-raw">
          <summary>Technical details</summary>
          <pre class="cache-diag">{JSON.stringify(cacheDiagnosticData, null, 2)}</pre>
        </details>
      {/if}
    </div>
    </details>
    <button type="button" class="btn btn-primary" disabled={restartingEngine} onclick={restartEngine}>
      {restartingEngine ? "Applying settings…" : "Save & restart engine"}
    </button>
    {#if presetMessage}<p role="status">{restartingEngine && engineProgress ? engineProgress : presetMessage}</p>{/if}
    {#if settingsError}<p class="warn" role="alert">{settingsError}</p>{/if}
  </div>

  {#if !appIsMac}
    <details class="panel block advanced" id="gpu-deps">
      <summary>Advanced NVIDIA GPU setup</summary>
      <p class="muted short">Required for NVIDIA acceleration on Windows. Downloads about 800 MB, separate from speech models.</p>
      {#if gpuError}
        <p class="warn" role="alert">Could not check NVIDIA GPU: {gpuError}</p>
      {:else if cuda === null}
        <p class="muted">Checking NVIDIA GPU…</p>
      {:else if !cuda}
        <p class="warn">NVIDIA driver check did not succeed. Check that your NVIDIA driver is installed.</p>
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
    </details>
  {/if}

  <div class="panel block">
    <h2 id="microphone">Microphone</h2>
    <div class="field">
      <label for="mic">Microphone</label>
      <select id="mic" bind:value={inputDeviceId}>
        {#if microphoneMissing}<option value={inputDeviceId}>{inputDeviceId} (not found)</option>{/if}
        {#each micDevices as d}
          <option value={d.id}>{d.label}</option>
        {/each}
      </select>
    </div>
    {#if microphoneMissing}<p role="status">Your preferred microphone was not found on the last check. {microphoneFallback ? "The system default will be used until it returns." : "Reconnect it or choose a different microphone."}</p>{/if}
    <button class="btn" disabled={microphoneRefreshing} onclick={refreshMicrophones}>{microphoneRefreshing ? "Checking microphones…" : "Refresh microphones"}</button>
    <p class="field-hint">Changed microphones? Select Refresh. Your choice applies to the next recording.</p>
    {#if !appIsMac}<label class="check"><input type="checkbox" bind:checked={dictationSoundCues} />Play brief sounds for recording, processing, insertion and errors (Windows)</label>{/if}
    <h3>Speaking pace</h3>
    <div class="row" role="group" aria-label="Pause tolerance">
      <button class="btn" aria-pressed={vadMinSilenceMs === "500"} onclick={() => vadMinSilenceMs = "500"}>Natural pauses</button>
      <button class="btn" aria-pressed={vadMinSilenceMs === "1500"} onclick={() => vadMinSilenceMs = "1500"}>Longer pauses</button>
      <button class="btn" aria-pressed={vadMinSilenceMs === "3000"} onclick={() => vadMinSilenceMs = "3000"}>Extra time</button>
    </div>
    <p class="field-hint">Allow longer pauses before processing a phrase. Recording continues until you stop it.</p>
    <details class="advanced">
      <summary>Advanced microphone settings</summary>
    <label class="check"><input type="checkbox" bind:checked={microphoneFallback} />Use the system default if my preferred microphone is unavailable</label>
    <p class="field-hint">Your preferred microphone stays saved and is checked at the next recording.</p>
    <label class="check"><input type="checkbox" bind:checked={adaptiveMicrophone} />Automatically adapt speech detection to my microphone</label>
    <p class="field-hint">Adjusts to background noise automatically. Turn off to set sensitivity manually.</p>
    <p class="muted short">Adjust microphone sensitivity and volume.</p>
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
          max="3000"
          step="10"
          value={Math.round(n(vadMinSilenceMs, 500))}
          oninput={(e) => (vadMinSilenceMs = e.currentTarget.value)}
        />
        <input class="slider-value" type="text" bind:value={vadMinSilenceMs} inputmode="numeric" autocomplete="off" />
      </div>
      <p class="field-hint">Wait this long before splitting speech into phrases. This does not stop recording.</p>
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
    </details>
    <button type="button" class="btn" disabled={microphoneSaving} onclick={saveMicrophoneOnly}>{microphoneSaving ? "Saving…" : "Save microphone settings"}</button>
    {#if microphoneMessage}<p role="status">{microphoneMessage}</p>{/if}
    {#if microphoneError}<p class="warn" role="alert">{microphoneError}</p>{/if}
  </div>

  <div class="panel block">
    <h2>Writing style</h2>
    <p class="muted short">How punctuation and cleanup are applied after transcription.</p>
    <div class="field">
      <label for="tone">Style</label>
      <select id="tone" bind:value={tonePreset}>
        <option value="minimal">Minimal</option>
        <option value="standard">Standard</option>
        <option value="expressive">Expressive</option>
      </select>
      <p class="field-hint">
        {#if tonePreset === "minimal"}
          Replaces exclamation marks with periods and softens dashes.
        {:else if tonePreset === "expressive"}
          Keeps expressive punctuation.
        {:else}
          Light cleanup; keeps most original punctuation.
        {/if}
      </p>
    </div>
    <details class="advanced">
      <summary>Advanced writing settings</summary>
    <div class="field">
      <label for="grammar-restore">Punctuation and capitals</label>
      <select id="grammar-restore" bind:value={grammarRestore}>
        <option value="off">Off</option>
        <option value="auto">Auto (when punctuation is missing)</option>
        <option value="always">Always</option>
      </select>
      <p class="field-hint">Adds punctuation and capitals. Auto runs only when punctuation is missing. First use downloads an English model.</p>
    </div>
    </details>
    <button type="button" class="btn" onclick={saveOutputStyle}>Save style</button>
    {#if settingsMessage}<p role="status">{settingsMessage}</p>{/if}
    {#if settingsError}<p class="warn" role="alert">{settingsError}</p>{/if}
  </div>

  <div class="panel block">
    <h2>Desktop widget</h2>
    <p class="muted short">Show recording status above other apps. Hiding the widget leaves dictation shortcuts active.</p>
    <label class="check">
      <input
        type="checkbox"
        bind:checked={hudWidgetEnabled}
        onchange={() => void persistHudWidget()}
      />
      Show desktop dictation widget
    </label>
    <label class="field">
      Widget style
      <select bind:value={hudWidgetStyle} onchange={() => void persistHudStyle()}>
        <option value="classic">Classic pill</option>
        <option value="controls">Speak/Stop controls</option>
      </select>
    </label>
    <p class="muted short">Classic shows audio levels. Controls adds Speak and Stop buttons.</p>
    {#if settingsError}<p class="warn" role="alert">{settingsError}</p>{/if}
  </div>

  <div class="panel block">
    <h2>Keyboard shortcuts</h2>
    <p class="muted short">Select <strong>Record shortcut</strong>, then press your key combination. Press Esc to cancel. Start the engine to use shortcuts.</p>
    {#if captureTarget}
      <p class="capture-hint" role="status">
        Listening for <strong>{captureTarget.replaceAll("_", " ")}</strong> — press a combination, or
        <button type="button" class="linkish" onclick={() => (captureTarget = null)}>cancel</button>.
      </p>
    {/if}
    <div class="field">
      <span class="keybind-label">Push to talk</span>
      <div class="keybind-row">

        <button
          type="button"
          class="btn keybind-record"
          class:active={captureTarget === "push_to_talk"}
        aria-pressed={captureTarget === "push_to_talk"}
          onclick={() =>
            (captureTarget = captureTarget === "push_to_talk" ? null : "push_to_talk")}
        >
          {captureTarget === "push_to_talk" ? "Listening…" : "Record shortcut"}
        </button>
      </div>
      {#if kPtt.trim()}
        <p class="field-hint keybind-as">
          Shown as <span class="mono">{formatShortcutDisplay(kPtt, { mac: shortcutUiMac })}</span>
        </p>
      {/if}
    </div>
    <div class="field">
      <span class="keybind-label">Toggle open mic</span>
      <div class="keybind-row">

        <button
          type="button"
          class="btn keybind-record"
          class:active={captureTarget === "toggle_open_mic"}
        aria-pressed={captureTarget === "toggle_open_mic"}
          onclick={() =>
            (captureTarget =
              captureTarget === "toggle_open_mic" ? null : "toggle_open_mic")}
        >
          {captureTarget === "toggle_open_mic" ? "Listening…" : "Record shortcut"}
        </button>
      </div>
      {#if kMic.trim()}
        <p class="field-hint keybind-as">
          Shown as <span class="mono">{formatShortcutDisplay(kMic, { mac: shortcutUiMac })}</span>
        </p>
      {/if}
    </div>
    <div class="field">
      <span class="keybind-label">Stop dictation</span>
      <div class="keybind-row">

        <button
          type="button"
          class="btn keybind-record"
          class:active={captureTarget === "stop_dictation"}
        aria-pressed={captureTarget === "stop_dictation"}
          onclick={() =>
            (captureTarget = captureTarget === "stop_dictation" ? null : "stop_dictation")}
        >
          {captureTarget === "stop_dictation" ? "Listening…" : "Record shortcut"}
        </button>
      </div>
      {#if kStop.trim()}
        <p class="field-hint keybind-as">
          Shown as <span class="mono">{formatShortcutDisplay(kStop, { mac: shortcutUiMac })}</span>
        </p>
      {/if}
    </div>
    {#if conflict.length}
      <p class="warn">Shortcut already used by: {conflict.join(", ")}</p>
    {/if}
    <details class="advanced">
      <summary>Advanced shortcut settings</summary>
      <p class="field-hint">Edit shortcut strings manually. Use Record shortcut above for the simplest setup.</p>
      <div class="field"><label for="k1">Push to talk</label>
        <input id="k1" class="mono keybind-input" bind:value={kPtt} autocomplete="off" spellcheck="false" />
      </div>
      <div class="field"><label for="k2">Toggle open mic</label>
        <input id="k2" class="mono keybind-input" bind:value={kMic} autocomplete="off" spellcheck="false" />
      </div>
      <div class="field"><label for="k3">Stop dictation</label>
        <input id="k3" class="mono keybind-input" bind:value={kStop} autocomplete="off" spellcheck="false" />
      </div>
    </details>
    <button type="button" class="btn btn-primary" onclick={saveKeybinds}>
      Save keybinds
    </button>
    {#if settingsMessage}<p role="status">{settingsMessage}</p>{/if}
    {#if settingsError}<p class="warn" role="alert">{settingsError}</p>{/if}
  </div>
  </fieldset>
</section>

<style>
  .settings-fields { border: 0; margin: 0; padding: 0; min-width: 0; }
  .advanced { margin: 1rem 0; border: 1px solid var(--border); border-radius: 10px; padding: 0.85rem 1rem; }
  .advanced > summary { cursor: pointer; font-weight: 600; color: var(--text); }
  .advanced[open] > summary { margin-bottom: 1rem; }
  .advanced > summary::marker { color: var(--accent); }
  .keybind-label { font-weight: 600; }

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
  .contrast-check {
    margin-top: 1rem;
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
  .keybind-as {
    margin-top: 0.35rem;
    margin-bottom: 0;
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

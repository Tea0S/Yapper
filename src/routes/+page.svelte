<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { beforeNavigate } from "$app/navigation";

  type EngineState = {
    ready: boolean;
    mode: string;
    message?: string | null;
    inference_detail?: string | null;
  };

  let engine = $state<EngineState | null>(null);
  let starting = $state(false);
  let stopping = $state(false);
  let lastError = $state<string | null>(null);

  let testTranscript = $state("");
  let testRecording = $state(false);
  let testTranscribing = $state(false);
  let testNotice = $state("");
  let testStarting = $state(false);
  let testError = $state<string | null>(null);
  /** Owns the test session while microphone startup is pending. */
  let testPttArmed = $state(false);
  let testPttStartPromise: Promise<void> | null = null;
  let engineProgress = $state("");
  let microphoneNotice = $state("");
  let microphoneName = $state("");
  let microphoneError = $state("");
  let dictationOutcome = $state("");
  type DictationJob = { id: number; text: string; status: string; error: string | null; audio_seconds: number; elapsed_ms: number; app_key: string | null };
  let jobs = $state<DictationJob[]>([]);
  let pasteModes = $state<Record<string, boolean>>({});
  let pasteModeBusy = $state(false);
  let jobError = $state("");
  let jobNotice = $state("");
  let jobAction = $state<number | null>(null);
  let correctionFrom = $state("");
  let correctionTo = $state("");
  let correctionJob = $state<number | null>(null);

  beforeNavigate(({ cancel }) => {
    if (testStarting || testRecording) {
      cancel();
      testError = "Stop the microphone test before leaving this page.";
    }
  });

  async function refreshJobs() {
    try {
      jobs = await invoke<DictationJob[]>("dictation_jobs");
      for (const key of new Set(jobs.map(j => j.app_key).filter((key): key is string => Boolean(key)))) {
        if (!(key in pasteModes)) {
          pasteModes[key] = (await invoke<string | null>("get_setting_cmd", { key: `paste_multiline_app_${key}` })) === "true";
        }
      }
    } catch { /* A later refresh retries unavailable settings. */ }
  }

  async function jobCommand(id: number, command: "retry_dictation" | "discard_dictation" | "benchmark_dictation") {
    jobAction = id;
    jobError = "";
    jobNotice = "";
    try {
      if (command === "benchmark_dictation") {
        const result = await invoke<{ audio_seconds: number; processing_seconds: number; realtime_factor: number; recommendation: string }>(command, { id });
        jobNotice = `${result.audio_seconds.toFixed(1)}s of audio processed in ${result.processing_seconds.toFixed(1)}s (${result.realtime_factor.toFixed(2)}× real time). ${result.recommendation} This measures speed, not accuracy; compare the transcript yourself.`;
      } else { await invoke(command, { id }); }
      await refreshJobs();
    } catch (e) { jobError = String(e); }
    finally { jobAction = null; }
  }

  async function copyJob(text: string) {
    try { await navigator.clipboard.writeText(text); jobNotice = "Copied."; }
    catch (e) { jobError = String(e); }
  }

  async function rememberCorrection() {
    jobError = "";
    const from = correctionFrom.trim();
    const to = correctionTo.trim();
    const original = jobs.find(j => j.id === correctionJob)?.text ?? "";
    if (!from || !to || from === to || !original.toLowerCase().includes(from.toLowerCase())) {
      jobError = "Enter a phrase from this transcript and a different replacement.";
      return;
    }
    try {
      const existing = await invoke<{ id: number; mishear: string; intended: string; priority: number }[]>("list_corrections_cmd");
      const match = existing.find(c => c.mishear.toLowerCase() === from.toLowerCase());
      await invoke("upsert_correction_cmd", { entry: { id: match?.id ?? null, mishear: from, intended: to, priority: match?.priority ?? 0 } });
      jobNotice = "Correction saved for future dictations.";
      correctionJob = null;
    } catch (e) { jobError = String(e); }
  }

  async function setPasteMode(appKey: string, multiline: boolean) {
    if (pasteModeBusy) return;
    pasteModeBusy = true;
    jobError = "";
    try {
      await invoke("set_setting_cmd", { key: `paste_multiline_app_${appKey}`, value: String(multiline) });
      pasteModes[appKey] = multiline;
      jobNotice = multiline ? "Single-paste paragraphs enabled for this application." : "Chat-safe line breaks enabled for this application.";
    } catch (e) { jobError = String(e); }
    finally { pasteModeBusy = false; }
  }

  let showWhatsNew = $state(false);
  const WHATS_NEW_VERSION = "1.3.8";

  type MicLevel = { rms: number; peak: number };
  let micLevel = $state<MicLevel>({ rms: 0, peak: 0 });

  type InstanceRole = "dictation" | "network_server";
  let instanceRole = $state<InstanceRole>("dictation");
  type NodeServerStatus = {
    running: boolean;
    suggestedClientUrls: string[];
    port: number;
    scriptFound: boolean;
  };
  let nodeQuick = $state<NodeServerStatus | null>(null);

  async function refreshNodeQuick() {
    try {
      nodeQuick = await invoke<NodeServerStatus>("yapper_node_status");
    } catch {
      nodeQuick = null;
    }
  }

  /** Must exceed Rust `wait_ptt_chunk_transcript` + model load (first run can be several minutes). */
  const PTT_STOP_TIMEOUT_MS = 660_000;

  function withTimeout<T>(p: Promise<T>, ms: number, errMsg: string): Promise<T> {
    return new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error(errMsg)), ms);
      p.then(
        (v) => {
          clearTimeout(t);
          resolve(v);
        },
        (e) => {
          clearTimeout(t);
          reject(e);
        },
      );
    });
  }

  async function refreshStatus() {
    lastError = null;
    try {
      engine = await invoke<EngineState>("engine_status");
    } catch {
      engine = { ready: false, mode: "none", message: "Could not read engine status." };
    }
  }

  onMount(() => {
    refreshStatus();
    void refreshJobs();
    void (async () => {
      const r =
        (await invoke<string | null>("get_setting_cmd", { key: "instance_role" })) ?? "dictation";
      instanceRole = r === "network_server" ? "network_server" : "dictation";
      await refreshNodeQuick();
      try {
        const seen =
          (await invoke<string | null>("get_setting_cmd", { key: "whats_new_seen_version" })) ?? "";
        showWhatsNew = seen !== WHATS_NEW_VERSION;
      } catch {
        showWhatsNew = false;
      }
    })();
    const unsubs: Array<() => void> = [];
    void listen<{ message?: string; recovering?: boolean }>("engine-crashed", async (ev) => {
      starting = Boolean(ev.payload?.recovering);
      try {
        engine = await invoke<EngineState>("engine_status");
      } catch {
        engine = { ready: false, mode: "none", message: "Could not read engine status." };
      }
      const msg = ev.payload?.message ?? "Inference engine exited.";
      lastError = ev.payload?.recovering
        ? `${msg} Restarting automatically…`
        : msg;
    }).then((u) => unsubs.push(u));
    void listen<{ attempt?: number }>("engine-auto-restart", () => {
      // Rust owns recovery, including when Home is not mounted. Never launch a
      // second engine from this notification while the watchdog is loading one.
      starting = true;
      lastError = null;
      engine = { ready: false, mode: "local", message: "Restarting inference engine…" };
    }).then((u) => unsubs.push(u));
    void listen<{ message?: string }>("engine-recovered", async (ev) => {
      starting = false;
      lastError = null;
      try {
        engine = await invoke<EngineState>("engine_status");
      } catch {
        engine = {
          ready: true,
          mode: "local",
          message: ev.payload?.message ?? "Inference engine restarted.",
        };
      }
    }).then((u) => unsubs.push(u));
    const id = setInterval(() => {
      if (typeof document !== "undefined" && document.visibilityState !== "visible") return;
      void refreshJobs();
      void invoke<{ message: string }>("engine_progress").then(p => engineProgress = p.message).catch(() => {});
      void invoke<{ device: string; notice: string; error: string; recording: boolean }>("microphone_status").then(m => {
        microphoneName = m.device; microphoneNotice = m.notice; microphoneError = m.error;
        if (testRecording && !m.recording && !testStarting) {
          testRecording = false; testPttArmed = false; testPttStartPromise = null;
          testNotice = "Recording stopped. Your transcript will appear in Recent dictations when processing finishes.";
        }
      }).catch(() => {});
      invoke<MicLevel>("get_mic_input_level")
        .then((l) => {
          micLevel = l;
        })
        .catch(() => {});
      invoke<string>("last_dictation_outcome_cmd")
        .then((msg) => {
          dictationOutcome = msg ?? "";
        })
        .catch(() => {});
    }, 400);
    invoke<MicLevel>("get_mic_input_level")
      .then((l) => {
        micLevel = l;
      })
      .catch(() => {});
    const nodePoll = setInterval(() => {
      if (typeof document !== "undefined" && document.visibilityState !== "visible") return;
      void refreshNodeQuick();
    }, 3200);
    return () => {
      clearInterval(id);
      clearInterval(nodePoll);
      for (const u of unsubs) u();
    };
  });

  async function dismissWhatsNew() {
    showWhatsNew = false;
    try {
      await invoke("set_setting_cmd", {
        key: "whats_new_seen_version",
        value: WHATS_NEW_VERSION,
      });
    } catch {
      /* ignore */
    }
  }
  async function startEngine() {
    lastError = null;
    starting = true;
    engineProgress = "";
    try {
      const next = await invoke<EngineState>("engine_start");
      engine = next;
      // Double-check from Rust state (helps if anything ever desyncs)
      engine = await invoke<EngineState>("engine_status");
    } catch (e) {
      lastError = String(e);
      engine = {
        ready: false,
        mode: "none",
        message: lastError,
      };
    } finally {
      starting = false;
    }
  }

  async function stopEngine() {
    lastError = null;
    stopping = true;
    try {
      await invoke("engine_stop");
      engine = await invoke<EngineState>("engine_status");
    } catch (e) {
      lastError = String(e);
      try {
        engine = await invoke<EngineState>("engine_status");
      } catch {
        engine = { ready: false, mode: "none", message: lastError };
      }
    } finally {
      stopping = false;
    }
  }

  function testPttDown() {
    if (!engine?.ready || testTranscribing) {
      testError = "Start the engine first.";
      return;
    }
    if (testPttArmed || testRecording) return;
    testError = null;
    testNotice = "";
    testStarting = true;
    testPttArmed = true;
    testPttStartPromise = invoke("ptt_start")
      .then(() => {
        testRecording = true;
      })
      .catch((e) => {
        testError = String(e);
        testPttArmed = false;
        testRecording = false;
        testPttStartPromise = null;
      })
      .finally(() => { testStarting = false; });
  }

  async function testPttUp() {
    if (!testPttArmed && !testPttStartPromise) return;
    const startP = testPttStartPromise;
    testPttStartPromise = null;
    testPttArmed = false;
    testRecording = false;
    testTranscribing = true;
    testError = null;
    try {
      if (startP) await startP.catch(() => {});
      const text = await withTimeout(
        invoke<string>("ptt_stop"),
        PTT_STOP_TIMEOUT_MS,
        "Transcription took too long. Check the engine status and try again.",
      );
      testTranscript = text;
      testNotice = text.trim() ? "Transcript ready. You can edit it here before copying." : "No speech detected. Check your microphone and try a full sentence.";
    } catch (e) {
      testError = String(e);
    } finally {
      testTranscribing = false;
    }
  }

  async function copyTestTranscript() {
    if (!testTranscript.trim()) return;
    testError = null;
    try {
      await navigator.clipboard.writeText(testTranscript);
      testNotice = "Transcript copied.";
    } catch {
      testError = "Could not copy. Select the transcript and copy it with your keyboard.";
    }
  }

</script>

<section class="hero">
  {#if showWhatsNew}
    <div class="panel whats-new" role="region" aria-label="What's new in 1.3.8">
      <div class="whats-new-head">
        <h2 class="whats-new-title">What’s new in 1.3.8</h2>
        <button type="button" class="btn mini" onclick={dismissWhatsNew}>Dismiss</button>
      </div>
      <ul class="whats-new-list">
        <li>Restored native transparent rendering for the pill and removed custom window clipping</li>
        <li>Classic pill restored by default, with Speak/Stop controls as an optional widget style</li>
        <li>Saved settings and shortcuts load before editing; save failures are shown clearly</li>
        <li>Microphone discovery and GPU checks run independently of settings loading</li>
        <li>Automatic paste supports fields without Windows accessibility element IDs</li>
      </ul>
    </div>
  {/if}

  {#if instanceRole === "network_server"}
    <div class="panel server-spotlight" role="region" aria-label="Processing server">
      <h2 class="server-spotlight-title">This PC is your processing server</h2>
      <p class="muted server-spotlight-lede">
        Start the WebSocket bridge here so other Yapper installs can send audio for transcription. Full controls live in
        Settings.
      </p>
      {#if nodeQuick}
        <p class="server-spotlight-status">
          <span class="dot" class:on={nodeQuick.running} aria-hidden="true"></span>
          <span>{nodeQuick.running ? `Listening on port ${nodeQuick.port}` : "Server not running"}</span>
        </p>
        {#if nodeQuick.running && nodeQuick.suggestedClientUrls.length}
          <p class="muted short">Clients connect to:</p>
          <ul class="server-urls">
            {#each nodeQuick.suggestedClientUrls as u}
              <li><code>{u}</code></li>
            {/each}
          </ul>
        {/if}
        {#if !nodeQuick.scriptFound}
          <p class="warn">Yapper Node script not found — use a full repo install or set <code>YAPPER_NODE</code>.</p>
        {/if}
      {/if}
      <a class="btn btn-primary" href="/settings#processing-server">Open server setup</a>
    </div>
  {/if}

  <h1>Speak locally. Stay in control.</h1>
  <p class="lede">
    Yapper runs Whisper-class models on your machine or a <strong>self-hosted</strong> node on
    your LAN/VPN. Dictionary, corrections, and tone presets apply on this device after
    transcription.
  </p>
  <div class="actions">
    {#if engine?.ready}
      <button
        type="button"
        class="btn btn-stop"
        disabled={stopping || starting || testRecording || testTranscribing}
        onclick={stopEngine}
      >
        {stopping ? "Stopping engine…" : "Stop inference engine"}
      </button>
    {:else}
      <button
        type="button"
        class="btn btn-primary"
        disabled={starting || stopping}
        onclick={startEngine}
      >
        {starting ? "Starting engine…" : "Start inference engine"}
      </button>
    {/if}
    <a class="btn" href="/settings">Open settings</a>
  </div>

  {#if lastError}
    <p class="warn engine-action-err" role="alert">{lastError}</p>
  {/if}

  <!-- Screen reader + live updates when status changes -->
  <div class="sr-only" aria-live="polite" aria-atomic="true">
    {#if starting}
      Starting inference engine, please wait.
    {:else if stopping}
      Stopping inference engine, please wait.
    {:else if engine?.ready}
      Engine ready, {engine.mode} mode.
    {:else if lastError}
      Engine failed: {lastError}
    {:else if engine}
      Engine not running.
    {/if}
  </div>

  {#if starting}
    <div class="panel status starting" role="status">
      <span class="pulse" aria-hidden="true"></span>
      <div>
        <strong>Starting engine</strong>
        <p class="detail">{engineProgress || "Starting speech engine or connecting to your node…"}</p>
      </div>
    </div>
  {:else if engine}
    <div
      class="panel status"
      class:ready={engine.ready}
      class:offline={!engine.ready}
      role="status"
    >
      <div class="status-head">
        <span class="dot" class:on={engine.ready} aria-hidden="true"></span>
        <strong>{engine.ready ? "Engine running" : "Engine off"}</strong>
      </div>
      {#if engine.ready}
        <p class="mode-line">
          <span class="badge">{engine.mode}</span>
          <span class="hint">Ready for dictation</span>
        </p>
        {#if engine.message}
          <p class="detail success">{engine.message}</p>
        {/if}
        {#if engine.inference_detail}
          <p class="detail mono">{engine.inference_detail}</p>
        {/if}
        {#if dictationOutcome}
          <p class="detail outcome-line" role="status">{dictationOutcome}</p>
        {/if}
      {:else}
        <p class="detail">
          {engine.message ?? "Start the engine to use push-to-talk and file transcription."}
        </p>
      {/if}
    </div>
  {/if}

  <div class="panel dictation-test">
    <h2>Recent dictations</h2>
    <p class="muted">Completed recordings expire after ten minutes; the latest five are kept in memory, with active jobs retained until they finish. Audio is not saved to disk. Retry uses the currently running engine; to try another model, change it in Settings and restart first. Retried text appears here for you to copy.</p>
    {#if jobError}<p class="warn" role="alert">{jobError}</p>{/if}
    {#if jobNotice}<p role="status">{jobNotice}</p>{/if}
    {#if jobs.length === 0}<p>No recent dictations.</p>{/if}
    {#each jobs as job (job.id)}
      <article class="panel">
        <p><strong>{job.status === "queued" ? "Waiting to process" : job.status === "processing" ? "Processing…" : job.status === "failed" ? "Needs a retry" : "Transcript ready"}</strong> · {job.audio_seconds.toFixed(1)}s recording{#if job.elapsed_ms > 0} · ready in {(job.elapsed_ms / 1000).toFixed(1)}s{/if}</p>
        {#if job.error}<p class="warn">{job.error}</p>{/if}
        {#if job.text}<p style="white-space: pre-wrap; overflow-wrap: anywhere">{job.text}</p>{/if}
        <div class="actions">
          <button class="btn" disabled={!job.text} onclick={() => copyJob(job.text)}>Copy</button>
          <button class="btn" disabled={jobAction !== null || job.status === "queued" || job.status === "processing" || !engine?.ready} onclick={() => jobCommand(job.id, "retry_dictation")}>Retry recording</button>
          <button class="btn" disabled={jobAction !== null || job.status === "queued" || job.status === "processing" || !engine?.ready} onclick={() => jobCommand(job.id, "benchmark_dictation")}>Benchmark this recording</button>
          <button class="btn" disabled={!job.text} onclick={() => { correctionJob = job.id; correctionFrom = ""; correctionTo = ""; }}>Teach a correction</button>
          <button class="btn" disabled={jobAction !== null || job.status === "queued" || job.status === "processing"} onclick={() => jobCommand(job.id, "discard_dictation")}>Discard</button>
        </div>
        {#if job.app_key}
          <details><summary>Line breaks for this destination</summary>
            <p>Applies to this application ({job.app_key.split(/[\\/]/).pop()}). Choose single paste for document editors; chat-safe mode uses Shift+Enter between lines.</p>
            <div role="group" aria-label="Line breaks for this application">
              <button class="btn" disabled={pasteModeBusy || !(job.app_key in pasteModes)} aria-pressed={pasteModes[job.app_key] === true} onclick={() => setPasteMode(job.app_key!, true)}>Single paste</button>
              <button class="btn" disabled={pasteModeBusy || !(job.app_key in pasteModes)} aria-pressed={pasteModes[job.app_key] === false} onclick={() => setPasteMode(job.app_key!, false)}>Chat-safe line breaks</button>
            </div>
          </details>
        {/if}
        {#if correctionJob === job.id}
          <label for="correction-from">Phrase Yapper heard</label>
          <input id="correction-from" bind:value={correctionFrom} />
          <label for="correction-to">Use this instead</label>
          <input id="correction-to" bind:value={correctionTo} />
          <button class="btn" onclick={rememberCorrection}>Remember correction</button>
          <button class="btn" onclick={() => correctionJob = null}>Cancel</button>
        {/if}
      </article>
    {/each}
  </div>

  <div class="panel dictation-test">
    <h2 class="test-title">Try your microphone</h2>
    {#if microphoneName}<p>Last used microphone: <strong>{microphoneName}</strong></p>{/if}
    {#if microphoneNotice}<p role="status">{microphoneNotice}</p>{/if}
    {#if microphoneError}<p class="warn" role="alert">{microphoneError} <a href="/settings#microphone">Microphone settings</a></p>{/if}
    {#if (testRecording || testTranscribing) && engineProgress}<p role="status">{engineProgress}</p>{/if}
    <ol class="setup-steps">
      <li>Choose your microphone in <a href="/settings#microphone">Microphone settings</a>.</li>
      <li>Start the engine, then select <strong>Start recording</strong>. Say a full sentence at your own pace.</li>
      <li>Select <strong>Stop and transcribe</strong>, then check the words below. Nothing is pasted into another app.</li>
    </ol>
    {#if !engine?.ready}<p class="muted">Start the engine above to try dictation.</p>{/if}
    <div
      class="input-meter"
      role="group"
      aria-label="Microphone level while recording"
    >
      <div class="input-meter-head">
        <span class="input-meter-title">Input level</span>
        <span class="input-meter-hint muted">
          {#if testRecording}
            Live
          {:else}
            Select Start recording to check your microphone
          {/if}
        </span>
      </div>
      <div class="input-meter-track" aria-hidden="true">
        <div
          class="input-meter-rms"
          style="width: {Math.min(100, Math.round(Math.pow(Math.min(1, micLevel.rms), 0.42) * 100))}%"
        ></div>
        <div
          class="input-meter-peak"
          style="width: {Math.min(100, Math.round(Math.pow(Math.min(1, micLevel.peak), 0.42) * 100))}%"
        ></div>
      </div>
    </div>
    <button
      type="button"
      class="btn btn-primary test-ptt"
      disabled={testStarting || testTranscribing || (!testRecording && (starting || stopping || !engine?.ready))}
      onclick={() => { if (testRecording) void testPttUp(); else testPttDown(); }}
    >
      {testStarting ? "Opening microphone…" : testTranscribing ? "Transcribing…" : testRecording ? "Stop and transcribe" : "Start recording"}
    </button>
    <p role="status">{testRecording ? "Recording. Take your time; select Stop and transcribe when finished." : testTranscribing ? "Preparing your transcript. The first recording can take longer while the model loads." : testNotice}</p>
    {#if testError}
      <p class="warn" role="alert">{testError}</p>
    {/if}
    <label class="out-label" for="test-out">Transcript</label>
    <textarea id="test-out" class="test-out" readonly={testStarting || testRecording || testTranscribing} rows="4" bind:value={testTranscript}></textarea>
    <button type="button" class="btn" onclick={copyTestTranscript} disabled={!testTranscript.trim() || testRecording || testTranscribing}>Copy transcript</button>
  </div>

  <ul class="tips">
    <li>Hold <kbd>Push-to-talk</kbd> (see Settings) to dictate; text is pasted on release.</li>
    <li>Prefer not to hold a key? Set a toggle recording shortcut in <a href="/settings">Settings</a>.</li>
    <li>
      Choosing a Whisper size for the first time can <strong>download</strong> model weights (see Settings). Stopping
      the engine exits the sidecar and frees GPU memory; optional idle unload is in Settings too.
    </li>

  </ul>
</section>

<style>
  .hero {
    max-width: 40rem;
  }
  .whats-new {
    margin-bottom: 1.35rem;
    border-color: color-mix(in srgb, var(--accent) 35%, var(--border));
  }
  .whats-new-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    margin-bottom: 0.5rem;
  }
  .whats-new-title {
    margin: 0;
    font-size: 1.05rem;
  }
  .whats-new-list {
    margin: 0;
    padding-left: 1.15rem;
    font-size: 0.92rem;
    color: var(--text-muted);
  }
  .whats-new-list li {
    margin-bottom: 0.25rem;
  }
  .btn.mini {
    padding: 0.3rem 0.65rem;
    font-size: 0.8rem;
  }
  .outcome-line {
    color: var(--accent-dim);
    font-weight: 600;
  }
  .server-spotlight {
    margin-bottom: 1.5rem;
    border-color: color-mix(in srgb, var(--accent) 40%, var(--border));
    background: color-mix(in srgb, var(--accent) 8%, var(--bg-elevated));
  }
  .server-spotlight-title {
    margin: 0 0 0.5rem;
    font-size: 1.1rem;
  }
  .server-spotlight-lede {
    margin: 0 0 0.75rem;
    font-size: 0.92rem;
  }
  .server-spotlight-status {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    margin: 0 0 0.5rem;
    font-size: 0.92rem;
    font-weight: 600;
  }
  .server-urls {
    margin: 0 0 1rem;
    padding-left: 1.1rem;
    font-size: 0.85rem;
    color: var(--text-muted);
  }
  .server-urls code {
    font-size: 0.82rem;
    word-break: break-all;
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
  h1 {
    font-size: clamp(1.85rem, 4vw, 2.35rem);
    margin: 0 0 1rem;
    line-height: 1.15;
  }
  .lede {
    color: var(--text-muted);
    margin: 0 0 1.5rem;
    font-size: 1.05rem;
  }
  .muted {
    color: var(--text-muted);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
    margin-bottom: 1rem;
  }
  .engine-action-err {
    margin: -0.35rem 0 1rem;
  }
  .btn-primary:disabled,
  .btn-stop:disabled {
    opacity: 0.75;
    cursor: wait;
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
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }
  .status {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin-bottom: 1.5rem;
    border-width: 1px;
    border-style: solid;
    transition:
      border-color 0.2s,
      background 0.2s;
  }
  .status.starting {
    flex-direction: row;
    align-items: flex-start;
    gap: 0.85rem;
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
    background: color-mix(in srgb, var(--accent) 8%, var(--bg-elevated));
  }
  .status.ready {
    border-color: color-mix(in srgb, var(--accent) 55%, var(--border));
    background: color-mix(in srgb, var(--accent) 10%, var(--bg-elevated));
  }
  .status.offline {
    border-color: var(--border);
  }
  .status-head {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--text-muted);
    flex-shrink: 0;
  }
  .dot.on {
    background: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 35%, transparent);
  }
  .mode-line {
    margin: 0;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem 0.75rem;
  }
  .badge {
    font-size: 0.72rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    padding: 0.2rem 0.5rem;
    border-radius: 6px;
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    color: var(--text);
  }
  .hint {
    font-size: 0.88rem;
    color: var(--text-muted);
  }
  .detail {
    margin: 0;
    font-size: 0.9rem;
    color: var(--text-muted);
    line-height: 1.45;
  }
  .detail.success {
    color: var(--text);
  }
  .detail.mono {
    font-family: ui-monospace, monospace;
    font-size: 0.82rem;
  }
  .dictation-test {
    margin-bottom: 1.5rem;
  }
  .test-title {
    margin: 0 0 0.5rem;
    font-size: 1.05rem;
  }
  .setup-steps {
    margin: 0 0 1rem;
    font-size: 0.9rem;
  }
  .input-meter {
    margin-bottom: 1rem;
  }
  .input-meter-head {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    align-items: baseline;
    gap: 0.35rem 0.75rem;
    margin-bottom: 0.4rem;
  }
  .input-meter-title {
    font-size: 0.8rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
  }
  .input-meter-hint {
    font-size: 0.78rem;
  }
  .input-meter-track {
    position: relative;
    height: 10px;
    border-radius: 6px;
    background: color-mix(in srgb, var(--border) 70%, var(--bg));
    overflow: hidden;
    border: 1px solid var(--border);
  }
  .input-meter-rms {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    border-radius: 5px 0 0 5px;
    background: color-mix(in srgb, var(--accent) 45%, var(--text-muted));
    transition: width 0.045s linear;
    pointer-events: none;
  }
  .input-meter-peak {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    border-radius: 5px;
    background: linear-gradient(
      90deg,
      transparent 0%,
      color-mix(in srgb, var(--accent) 85%, transparent) 100%
    );
    opacity: 0.95;
    transition: width 0.045s linear;
    pointer-events: none;
  }
  .test-ptt {
    margin-bottom: 0.75rem;
    touch-action: none;
    user-select: none;
  }
  .out-label {
    display: block;
    font-size: 0.85rem;
    margin-bottom: 0.35rem;
    color: var(--text-muted);
  }
  .test-out {
    width: 100%;
    box-sizing: border-box;
    margin-bottom: 0.6rem;
    font-size: 0.92rem;
    line-height: 1.45;
    resize: vertical;
    min-height: 5rem;
    padding: 0.55rem 0.75rem;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--text);
  }
  .warn {
    color: var(--danger);
    font-size: 0.88rem;
    margin: 0 0 0.5rem;
  }
  .pulse {
    width: 12px;
    height: 12px;
    margin-top: 0.2rem;
    border-radius: 50%;
    background: var(--accent);
    animation: pulse 1.1s ease-in-out infinite;
    flex-shrink: 0;
  }
  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
      transform: scale(1);
    }
    50% {
      opacity: 0.45;
      transform: scale(0.92);
    }
  }
  .tips {
    margin: 0;
    padding-left: 1.2rem;
    color: var(--text-muted);
    font-size: 0.92rem;
    line-height: 1.55;
  }
  kbd,
  code {
    font-size: 0.85em;
    background: var(--bg);
    padding: 0.12rem 0.35rem;
    border-radius: 4px;
    border: 1px solid var(--border);
  }
</style>

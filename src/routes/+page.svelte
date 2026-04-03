<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

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
  let testError = $state<string | null>(null);
  /** True as soon as pointer goes down — before `ptt_start` IPC returns (fixes quick-click / event order bugs). */
  let testPttArmed = $state(false);
  let testPttStartPromise: Promise<void> | null = null;

  type MicLevel = { rms: number; peak: number };
  let micLevel = $state<MicLevel>({ rms: 0, peak: 0 });

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
    const id = setInterval(() => {
      if (typeof document !== "undefined" && document.visibilityState !== "visible") return;
      invoke<MicLevel>("get_mic_input_level")
        .then((l) => {
          micLevel = l;
        })
        .catch(() => {});
    }, 55);
    invoke<MicLevel>("get_mic_input_level")
      .then((l) => {
        micLevel = l;
      })
      .catch(() => {});
    return () => clearInterval(id);
  });

  async function startEngine() {
    lastError = null;
    starting = true;
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
      });
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
        "Transcription timed out after ~11 min — check the dev terminal for [yapper-sidecar] lines or a sidecar error above. Use Python 3.10–3.12 and: py -3.12 -m pip install -r sidecar/requirements.txt",
      );
      testTranscript = text.trim() ? text : "(no speech detected)";
    } catch (e) {
      testError = String(e);
    } finally {
      testTranscribing = false;
    }
  }

  async function copyTestTranscript() {
    if (!testTranscript || testTranscript === "(no speech detected)") return;
    try {
      await navigator.clipboard.writeText(testTranscript);
    } catch {
      /* ignore */
    }
  }
</script>

<section class="hero">
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

  {#if lastError && engine?.ready}
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
        <p class="detail">Loading the sidecar or connecting to your node — this can take a few seconds the first time.</p>
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
      {:else}
        <p class="detail">
          {engine.message ?? "Start the engine to use push-to-talk and file transcription."}
        </p>
      {/if}
    </div>
  {/if}

  <div class="panel dictation-test">
    <h2 class="test-title">Try dictation</h2>
    <p class="muted test-lede">
      Hold the button while you speak (same pipeline as push-to-talk). Hold at least <strong>about one second</strong>
      so enough audio reaches Whisper; very short taps often come back empty. Transcript shows here only — nothing is
      pasted elsewhere.
    </p>
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
            Hold to speak or use global push-to-talk
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
      disabled={starting || stopping || !engine?.ready || testTranscribing}
      onpointerdown={(e) => {
        try {
          e.currentTarget?.setPointerCapture?.(e.pointerId);
        } catch {
          /* capture unsupported or wrong phase */
        }
        testPttDown();
      }}
      onpointerup={(e) => {
        try {
          e.currentTarget?.releasePointerCapture?.(e.pointerId);
        } catch {
          /* ignore */
        }
        void testPttUp();
      }}
      onpointerleave={() => {
        if (testPttArmed || testRecording) void testPttUp();
      }}
    >
      {testTranscribing
        ? "Transcribing… (first run can take a while)"
        : testRecording
          ? "Recording… release to transcribe"
          : "Hold to speak"}
    </button>
    {#if testError}
      <p class="warn" role="alert">{testError}</p>
    {/if}
    <label class="out-label" for="test-out">Transcript</label>
    <textarea id="test-out" class="test-out" readonly rows="4" bind:value={testTranscript}></textarea>
    <button type="button" class="btn" onclick={copyTestTranscript} disabled={!testTranscript}>Copy</button>
  </div>

  <ul class="tips">
    <li>Hold <kbd>Push-to-talk</kbd> (see Settings) to dictate; text is pasted on release.</li>
    <li>Install Python deps: <code>pip install -r sidecar/requirements.txt</code></li>
    <li>
      Choosing a Whisper size for the first time can <strong>download</strong> model weights (see Settings). Stopping
      the engine exits the sidecar and frees GPU memory; optional idle unload is in Settings too.
    </li>
    <li>Optional GPU node: <code>python yapper-node/main.py --token your-secret</code></li>
  </ul>
</section>

<style>
  .hero {
    max-width: 40rem;
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
  .test-lede {
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

<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";

  let path = $state("");
  let busy = $state(false);
  let progress = $state<number | null>(null);
  let result = $state("");
  let err = $state("");

  const transcribeCtx = { busy: false, path: "" };
  $effect(() => {
    transcribeCtx.busy = busy;
    transcribeCtx.path = path;
  });

  onMount(() => {
    let unlisten: UnlistenFn | undefined;
    void listen<{ path: string; percent: number }>("transcribe_file_progress", (ev) => {
      if (!transcribeCtx.busy || ev.payload.path !== transcribeCtx.path) return;
      progress = Math.max(0, Math.min(100, ev.payload.percent));
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  });

  async function pickFile() {
    err = "";
    const sel = await open({
      multiple: false,
      filters: [
        {
          name: "Audio",
          extensions: ["wav", "mp3", "m4a", "flac", "ogg", "webm"],
        },
      ],
    });
    if (sel && typeof sel === "string") path = sel;
  }

  async function run() {
    if (!path) {
      err = "Choose an audio file.";
      return;
    }
    busy = true;
    progress = 0;
    err = "";
    result = "";
    try {
      result = await invoke<string>("transcribe_file", { path });
      progress = 100;
    } catch (e) {
      err = String(e);
      progress = null;
    } finally {
      busy = false;
    }
  }

  async function pasteResult() {
    if (!result) return;
    await invoke("paste_text", { text: result });
  }
</script>

<section>
  <h1>File transcription</h1>
  <p class="muted">
    The file path must be readable by the inference host (this PC for local engine).
  </p>

  <div class="panel">
    <div class="row">
      <input type="text" readonly placeholder="No file selected" bind:value={path} />
      <button type="button" class="btn" onclick={pickFile}>Browse…</button>
      <button type="button" class="btn btn-primary" disabled={busy} onclick={run}>
        {busy ? "Transcribing…" : "Transcribe"}
      </button>
    </div>

    {#if busy && progress !== null}
      <div class="progress-wrap" aria-live="polite">
        <div class="progress-head">
          <span class="progress-label">Transcription progress</span>
          <span class="progress-pct">{Math.round(progress)}%</span>
        </div>
        <div
          class="progress-track"
          role="progressbar"
          aria-valuemin="0"
          aria-valuemax="100"
          aria-valuenow={Math.round(progress)}
        >
          <div class="progress-fill" style:width="{progress}%"></div>
        </div>
      </div>
    {/if}

    {#if err}
      <p class="err">{err}</p>
    {/if}
    {#if result}
      <div class="out">
        <pre>{result}</pre>
        <button type="button" class="btn" onclick={pasteResult}>Paste</button>
      </div>
    {/if}
  </div>
</section>

<style>
  h1 {
    margin-top: 0;
  }
  .muted {
    color: var(--text-muted);
    max-width: 42rem;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: center;
  }
  .row input {
    flex: 1;
    min-width: 200px;
    padding: 0.55rem 0.75rem;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--bg);
  }
  .progress-wrap {
    margin-top: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
  }
  .progress-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 0.75rem;
  }
  .progress-label {
    font-size: 0.88rem;
    color: var(--text-muted);
  }
  .progress-pct {
    font-size: 0.88rem;
    font-variant-numeric: tabular-nums;
    color: var(--accent);
    font-weight: 600;
  }
  .progress-track {
    height: 8px;
    border-radius: 999px;
    background: var(--bg);
    border: 1px solid var(--border);
    overflow: hidden;
  }
  .progress-fill {
    height: 100%;
    border-radius: inherit;
    background: linear-gradient(
      90deg,
      color-mix(in srgb, var(--accent) 85%, transparent),
      var(--accent)
    );
    transition: width 0.35s ease;
  }
  .err {
    color: var(--danger);
    margin-top: 1rem;
  }
  .out {
    margin-top: 1.25rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  pre {
    margin: 0;
    padding: 1rem;
    border-radius: 8px;
    background: var(--bg);
    border: 1px solid var(--border);
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 320px;
    overflow: auto;
  }
</style>

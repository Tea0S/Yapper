<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";

  let path = $state("");
  let busy = $state(false);
  let result = $state("");
  let err = $state("");

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
    err = "";
    result = "";
    try {
      result = await invoke<string>("transcribe_file", { path });
    } catch (e) {
      err = String(e);
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
        {busy ? "Working…" : "Transcribe"}
      </button>
    </div>
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

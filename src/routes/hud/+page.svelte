<script lang="ts">
  import ClassicPill from "$lib/ClassicPill.svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";

  type Snapshot = {
    style: "classic" | "controls";
    phase: "hidden" | "idle" | "listening" | "transcribing";
    preview: string; outcome: string; pending: number;
    microphone: { device: string; notice: string; error: string };
    preview_status: string;
    engine_progress: { stage: string; message: string };
  };
  let snap = $state<Snapshot>({ style: "classic", phase: "idle", preview: "", outcome: "", pending: 0,
    microphone: { device: "", notice: "", error: "" }, preview_status: "", engine_progress: { stage: "", message: "" } });
  let peak = $state(0);
  let busy = $state(false);
  let error = $state("");
  const recording = $derived(snap.phase === "listening");
  const preparing = $derived(["checking", "downloading", "loading", "warming"].includes(snap.engine_progress.stage));
  const label = $derived(recording ? "Listening" : snap.phase === "transcribing" ? "Processing" : preparing ? "Preparing" : snap.outcome === "Dictation pasted" ? "Inserted" : "Ready");
  const previewTail = $derived(snap.preview.length > 150 ? "…" + snap.preview.slice(-149) : snap.preview);
  const detail = $derived(error || snap.microphone.error || (preparing ? snap.engine_progress.message : "") ||
    (recording ? previewTail || snap.preview_status || snap.microphone.notice : snap.outcome));

  async function toggle() {
    if (busy) return;
    error = "";
    busy = true;
    try { await invoke("hud_toggle_recording"); }
    catch (e) { error = String(e); }
    finally { busy = false; }
  }

  onMount(() => {
    let dead = false;
    let timer: ReturnType<typeof setTimeout>;
    async function tick() {
      try {
        const next = await invoke<Snapshot>("hud_snapshot");
        if (dead) return;
        snap = next;
        if (next.phase === "listening") {
          const level = await invoke<{ peak: number }>("get_mic_input_level");
          if (!dead) peak = level.peak;
        }
      } catch { /* Keep the last state during a transient IPC failure. */ }
      if (!dead) timer = setTimeout(tick, 120);
    }
    void tick();
    return () => { dead = true; clearTimeout(timer); };
  });
</script>

{#if snap.style === "classic"}
  <ClassicPill {recording} processing={snap.phase === "transcribing" || preparing} {peak} {detail} {label} />
{:else}
<div class="widget" class:recording>
  <div class="controls">
    <button class="grip" title="Drag to move" aria-label="Move dictation widget"
      onpointerdown={(e) => { if (e.button === 0) void getCurrentWindow().startDragging().catch(() => {}); }}>⠿</button>
    <div class="state">
      <span class="status-dot" class:pulse={recording || preparing || snap.phase === "transcribing"}></span>
      <span role="status">{label}</span>
    </div>
    <button class="record" disabled={busy} onclick={toggle}
      title={recording ? "Stop recording and insert text" : "Dictate into the selected text field"}
      aria-label={recording ? "Stop recording and insert text" : "Start dictation"}>
      {recording ? "Stop" : busy ? "Wait…" : "Speak"}
    </button>
    <button class="open" title="Open Yapper" aria-label="Open Yapper" onclick={() => invoke("focus_main_window")}>↗</button>
  </div>
  {#if recording}
    <div class="meter" aria-hidden="true"><span style:width={`${Math.min(100, Math.sqrt(Math.max(0, peak)) * 150)}%`}></span></div>
  {/if}
  {#if detail}<p class="detail" class:preview={recording && Boolean(snap.preview) && detail === previewTail} role="status" title={detail}>{detail}</p>{/if}
</div>

{/if}

<style>
  :global(html), :global(body) { margin: 0; width: 100%; height: 100%; overflow: hidden; background: transparent !important; }
  .widget { box-sizing: border-box; width: 100vw; height: 100vh; padding: 4px 8px; border-radius: 16px;
    background: #151920; color: #f5f7fa; border: 1px solid #727c8b; font: 12px/1.35 system-ui, sans-serif; overflow: hidden; }
  .controls { display: flex; align-items: center; gap: 5px; height: 30px; }
  button { padding: 3px 5px; border: 0; border-radius: 6px; color: inherit; background: transparent; font: inherit; cursor: pointer; }
  button:hover { background: #343b46; }
  button:focus-visible { outline: 2px solid #e8b4d4; outline-offset: -2px; }
  button:disabled { opacity: .6; }
  .grip { cursor: grab; font-size: 16px; }
  .state { display: flex; align-items: center; gap: 5px; flex: 1; min-width: 0; font-weight: 600; }
  .status-dot { width: 6px; height: 6px; border-radius: 50%; background: #a6d9b0; flex-shrink: 0; }
  .record { background: #e8b4d4; color: #19131a; font-weight: 700; }
  .record:hover { background: #f1cce3; }
  .recording .status-dot { background: #f7a7b4; }
  .meter { height: 3px; margin: 2px 5px 4px; background: #343b46; border-radius: 3px; overflow: hidden; }
  .meter span { display: block; height: 100%; background: #e8b4d4; transition: width .1s; }
  .detail { margin: 4px 5px 0; font-size: 12px; line-height: 1.4; max-height: 50px; overflow: auto; overflow-wrap: anywhere; }
  .detail.preview { overflow: auto; }
  .pulse { animation: pulse 1s ease-in-out infinite alternate; }
  @keyframes pulse { to { opacity: .45; } }
  @media (prefers-reduced-motion: reduce) { .pulse { animation: none; } .meter span { transition: none; } }
</style>

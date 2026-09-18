<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  let { recording, processing, peak, detail, label }:
    { recording: boolean; processing: boolean; peak: number; detail: string; label: string } = $props();
  let macos = $state(false);
  onMount(() => {
    void invoke<{ macos: boolean }>("hud_chrome_info").then(info => macos = info.macos).catch(() => {});
  });
  const DRAG_THRESHOLD_PX = 6;
  let pointerDown = false;
  let pointerStartX = 0;
  let pointerStartY = 0;
  let dragStarted = false;
  async function openYapper() {
    try {
      await invoke("focus_main_window");
    } catch { /* Window may have closed. */ }
  }

  function onPillPointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    pointerDown = true;
    dragStarted = false;
    pointerStartX = e.clientX;
    pointerStartY = e.clientY;
    (e.currentTarget as HTMLButtonElement).setPointerCapture(e.pointerId);
  }

  function onPillPointerMove(e: PointerEvent) {
    if (!pointerDown || (e.buttons & 1) === 0) return;
    const dx = e.clientX - pointerStartX;
    const dy = e.clientY - pointerStartY;
    if (!dragStarted && dx * dx + dy * dy >= DRAG_THRESHOLD_PX * DRAG_THRESHOLD_PX) {
      dragStarted = true;
      void getCurrentWindow()
        .startDragging()
        .catch(() => {});
    }
  }

  function onPillPointerUp(e: PointerEvent) {
    if (e.button !== 0) return;
    pointerDown = false;
    try {
      (e.currentTarget as HTMLButtonElement).releasePointerCapture(e.pointerId);
    } catch { /* Window may have closed. */ }
    if (!dragStarted) {
      void openYapper();
    }
  }

  function onPillPointerCancel() {
    pointerDown = false;
    dragStarted = false;
  }

  function onPillKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      void openYapper();
    }
  }

</script>
<button type="button" class="pill" class:macos class:expanded={recording || processing || Boolean(detail)}
  aria-label={`${label}. Open Yapper or drag to move`} title={`${label} · Click to open Yapper · Drag to move`}
  onpointerdown={onPillPointerDown} onpointermove={onPillPointerMove} onpointerup={onPillPointerUp}
  onpointercancel={onPillPointerCancel} onkeydown={onPillKeydown}>
  {#if recording || processing || detail}
    <div class="live-wrap">
      {#if recording || processing}
        <div class="dots" aria-hidden="true">
          {#each Array.from({ length: 9 }, (_, i) => i) as i}
            <span class="dot" class:busy={processing} style={`--lvl: ${Math.min(1, .12 + (1 - Math.abs(i - 4) / 4) * .28 + peak * 3.2)}; --i: ${i}`}></span>
          {/each}
        </div>
      {/if}
      {#if detail || processing}<p class="live-preview" role="status" title={detail}>{detail.length > 42 ? "…" + detail.slice(-41) : detail || label}</p>{/if}
    </div>
  {:else}<span class="idle-cap" aria-hidden="true"></span>{/if}
</button>
<style>
  .pill {
    margin: 0;
    padding: 0;
    appearance: none;
    -webkit-appearance: none;
    cursor: grab;
    border-radius: 999px;
    border: 1px solid rgba(255, 255, 255, 0.38);
    background: rgba(6, 8, 10, 0.45);
    backdrop-filter: blur(10px);
    display: flex;
    align-items: center;
    justify-content: center;
    transition:
      min-width 0.18s ease,
      min-height 0.18s ease,
      padding 0.18s ease,
      border-color 0.15s ease;
    min-width: 72px;
    min-height: 22px;
    padding: 5px 14px;
    outline: none;
  }

  .pill:focus-visible {
    outline: 2px solid rgba(232, 180, 212, 0.65);
    outline-offset: 2px;
  }

  .pill:hover {
    border-color: rgba(255, 255, 255, 0.52);
    background: rgba(10, 12, 16, 0.55);
  }

  .pill:active {
    cursor: grabbing;
  }

  .pill.expanded {
    width: auto;
    max-width: 100%;
    min-width: 88px;
    min-height: 36px;
    padding: 8px 16px;
  }




  .idle-cap {
    display: block;
    width: 44px;
    height: 3px;
    border-radius: 2px;
    background: rgba(255, 255, 255, 0.2);
  }

  .dots {
    display: flex;
    align-items: flex-end;
    justify-content: center;
    gap: 3px;
    height: 22px;
    width: 100%;
    max-width: 100%;
    padding: 0 2px;
  }

  .live-wrap {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 5px;
    width: 100%;
    min-width: 0;
  }

  .live-preview {
    margin: 0;
    padding: 0 2px;
    font-size: 11px;
    line-height: 1.25;
    font-weight: 500;
    color: rgba(248, 250, 252, 0.92);
    text-align: center;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: clip;
    max-width: 100%;
  }


  .dot {
    width: 4px;
    flex-shrink: 0;
    height: 18px;
    border-radius: 2px;
    background: rgba(255, 255, 255, 0.88);
    transform: scaleY(var(--lvl));
    transform-origin: center bottom;
    transition: transform 0.06s ease-out, opacity 0.2s ease;
    opacity: 0.92;
  }


  .dot.busy {
    animation: breathe 0.9s ease-in-out infinite;
    animation-delay: calc(var(--i, 0) * 0.06s);
    opacity: 0.55;
  }

  .dot.busy:nth-child(1) {
    --i: 0;
  }
  .dot.busy:nth-child(2) {
    --i: 1;
  }
  .dot.busy:nth-child(3) {
    --i: 2;
  }
  .dot.busy:nth-child(4) {
    --i: 3;
  }
  .dot.busy:nth-child(5) {
    --i: 4;
  }
  .dot.busy:nth-child(6) {
    --i: 5;
  }
  .dot.busy:nth-child(7) {
    --i: 6;
  }
  .dot.busy:nth-child(8) {
    --i: 7;
  }
  .dot.busy:nth-child(9) {
    --i: 8;
  }

  @keyframes breathe {
    0%,
    100% {
      transform: scaleY(0.25);
      opacity: 0.45;
    }
    50% {
      transform: scaleY(0.85);
      opacity: 0.85;
    }
  }

  .pill, .pill.expanded { box-sizing: border-box; width: 100vw; height: 100vh; min-width: 0; min-height: 0; padding: 3px 12px; overflow: hidden; }
  .pill:focus-visible { outline-offset: -2px; }
  @media (prefers-reduced-motion: reduce) { .dot.busy { animation: none; } .dot { transition: none; } }
  .pill.macos {
    background: rgba(255, 255, 255, 0.14);
    border-color: rgba(255, 255, 255, 0.45);
    backdrop-filter: saturate(180%) blur(20px);
    -webkit-backdrop-filter: saturate(180%) blur(20px);
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.25),
      0 4px 20px rgba(0, 0, 0, 0.2);
  }
  .pill.macos:hover {
    background: rgba(255, 255, 255, 0.2);
    border-color: rgba(255, 255, 255, 0.52);
  }
  .pill.macos .dots {
    height: 20px;
  }
  .pill.macos .dot {
    height: 16px;
  }
</style>

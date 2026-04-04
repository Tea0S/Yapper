<script lang="ts">
  import { formatShortcutDisplay } from "$lib/formatShortcutDisplay";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";

  type HudPhase = "hidden" | "idle" | "listening" | "transcribing";
  type MicLevel = { rms: number; peak: number };

  let phase = $state<HudPhase>("idle");
  let mic = $state<MicLevel>({ rms: 0, peak: 0 });
  /** Layout/styling only — from Rust `cfg!(target_os = "macos")`, never UA sniffing. */
  let isMacChrome = $state(false);
  let pttHint = $state("Push-to-talk");
  let toggleMicHint = $state("");

  const dotCount = 9;
  const DRAG_THRESHOLD_PX = 6;

  let pointerDown = false;
  let pointerStartX = 0;
  let pointerStartY = 0;
  let dragStarted = false;

  const expanded = $derived(phase === "listening" || phase === "transcribing");

  function dotLevel(i: number): number {
    const center = (dotCount - 1) / 2;
    const dist = Math.abs(i - center) / Math.max(center, 1);
    const rest = 0.12 + (1 - dist) * 0.28;
    const e = Math.min(1, mic.peak * 3.2 + mic.rms * 5.5);
    return Math.min(1, rest + e * (0.55 + (1 - dist) * 0.45));
  }

  async function openYapper() {
    try {
      await invoke("focus_main_window");
    } catch {
      /* ignore */
    }
  }

  /** Windows + macOS: small movement = click; past threshold = native window drag. */
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
    } catch {
      /* already released */
    }
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

  onMount(() => {
    let dead = false;
    void (async () => {
      let macos = false;
      try {
        const info = await invoke<{ macos: boolean }>("hud_chrome_info");
        macos = info.macos;
      } catch {
        /* browser / old build */
      }
      if (dead) return;
      isMacChrome = macos;
      try {
        const rows = await invoke<{ action: string; shortcut: string }[]>("list_keybinds_cmd");
        if (dead) return;
        const ptt = rows.find((r) => r.action === "push_to_talk" && r.shortcut.trim());
        if (ptt) pttHint = formatShortcutDisplay(ptt.shortcut, { mac: macos });
        const tom = rows.find((r) => r.action === "toggle_open_mic" && r.shortcut.trim());
        if (tom) toggleMicHint = formatShortcutDisplay(tom.shortcut, { mac: macos });
      } catch {
        /* ignore */
      }
    })();

    const tick = async () => {
      if (dead) return;
      try {
        const snap = await invoke<{ phase: HudPhase }>("hud_snapshot");
        phase = snap.phase;
      } catch {
        phase = "hidden";
      }
      if (phase === "listening") {
        try {
          mic = await invoke<MicLevel>("get_mic_input_level");
        } catch {
          mic = { rms: 0, peak: 0 };
        }
      }
    };
    const id = setInterval(tick, 72);
    void tick();
    return () => {
      dead = true;
      clearInterval(id);
    };
  });
</script>

<div class="hud-root">
  <div class="hud-shell">
    <div class="stack">
      <div class="tooltip" role="tooltip">
        <span class="tip-line"
          >Hold <strong class="accent">{pttHint}</strong> to dictate · release to transcribe</span
        >
        {#if toggleMicHint}
          <span class="tip-line tip-gap"
            >Press <strong class="accent">{toggleMicHint}</strong> to toggle open mic</span
          >
        {/if}
        <span class="tip-sub">Click to open Yapper · drag to move the widget</span>
      </div>
      <button
        type="button"
        class="pill"
        class:expanded
        class:macos={isMacChrome}
        aria-label="Open Yapper — or drag to move"
        onpointerdown={onPillPointerDown}
        onpointermove={onPillPointerMove}
        onpointerup={onPillPointerUp}
        onpointercancel={onPillPointerCancel}
        onkeydown={onPillKeydown}
      >
        {#if expanded}
          <div class="dots" aria-hidden="true">
            {#each Array.from({ length: dotCount }, (_, i) => i) as i (i)}
              <span
                class="dot"
                class:busy={phase === "transcribing"}
                style="--lvl: {phase === 'listening' ? dotLevel(i) : 0.22}"
              ></span>
            {/each}
          </div>
        {:else}
          <span class="idle-cap" aria-hidden="true"></span>
        {/if}
      </button>
    </div>
  </div>
</div>

<style>
  :global(html),
  :global(body) {
    background: transparent !important;
    margin: 0;
    min-height: 100%;
    overflow: visible;
  }

  .hud-root {
    box-sizing: border-box;
    min-height: 100%;
    width: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-end;
    padding: 8px 6px 10px;
    font-family: "DM Sans", system-ui, sans-serif;
    -webkit-font-smoothing: antialiased;
    overflow: visible;
  }

  .hud-root * {
    box-sizing: border-box;
  }

  .hud-shell {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    width: 100%;
    max-width: 100%;
  }

  .stack {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    width: 100%;
    max-width: 100%;
    overflow: visible;
  }

  .stack:hover .tooltip {
    opacity: 1;
  }

  .tooltip {
    position: absolute;
    bottom: calc(100% + 10px);
    left: 50%;
    transform: translateX(-50%);
    width: max-content;
    max-width: min(300px, 100%);
    padding: 9px 14px;
    border-radius: 10px;
    font-size: 12px;
    font-weight: 500;
    line-height: 1.4;
    text-align: center;
    color: rgba(248, 250, 252, 0.95);
    background: rgba(12, 14, 18, 0.92);
    border: 1px solid rgba(255, 255, 255, 0.2);
    box-shadow: 0 8px 28px rgba(0, 0, 0, 0.45);
    backdrop-filter: blur(12px);
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.16s ease;
    z-index: 10;
    white-space: normal;
    word-wrap: break-word;
  }

  .tip-line {
    display: block;
  }

  .tip-gap {
    margin-top: 6px;
  }

  .tip-sub {
    display: block;
    margin-top: 4px;
    font-size: 11px;
    font-weight: 400;
    color: rgba(248, 250, 252, 0.65);
  }

  .accent {
    color: #e8b4d4;
    font-weight: 600;
  }

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

  /* Width tracks Rust logical window (collapsed + expanded). */
  .pill.expanded {
    align-self: stretch;
    width: 100%;
    max-width: 100%;
    min-width: 0;
    min-height: 36px;
    padding: 8px 10px;
  }

  /* macOS: same footprint as other platforms; local glass via backdrop-filter (no full-window vibrancy). */
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
    gap: 4px;
    height: 22px;
    width: 100%;
    max-width: 100%;
    padding: 0 2px;
  }

  .pill.macos .dots {
    height: 20px;
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

  .pill.macos .dot {
    height: 16px;
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
</style>

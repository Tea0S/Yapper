<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";

  type HudPhase = "hidden" | "idle" | "listening" | "transcribing";
  type MicLevel = { rms: number; peak: number };

  let phase = $state<HudPhase>("idle");
  let mic = $state<MicLevel>({ rms: 0, peak: 0 });
  let isMacHud = $state(false);
  let pointerDown = false;
  let pointerStartX = 0;
  let pointerStartY = 0;
  let dragStarted = false;

  const DRAG_THRESHOLD_PX = 6;

  const dotCount = 9;

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

  async function startHudDrag() {
    if (!isMacHud) return;
    try {
      await getCurrentWindow().startDragging();
    } catch {
      /* ignore */
    }
  }

  function handleMacPointerDown(event: PointerEvent) {
    if (!isMacHud || event.button !== 0) return;
    pointerDown = true;
    dragStarted = false;
    pointerStartX = event.clientX;
    pointerStartY = event.clientY;
  }

  async function handleMacPointerMove(event: PointerEvent) {
    if (!isMacHud || !pointerDown || dragStarted) return;
    const dx = event.clientX - pointerStartX;
    const dy = event.clientY - pointerStartY;
    if (Math.hypot(dx, dy) < DRAG_THRESHOLD_PX) return;
    dragStarted = true;
    await startHudDrag();
  }

  async function handleMacPointerUp() {
    if (!isMacHud || !pointerDown) return;
    const shouldOpen = !dragStarted;
    pointerDown = false;
    dragStarted = false;
    if (shouldOpen) {
      await openYapper();
    }
  }

  function handleMacPointerCancel() {
    pointerDown = false;
    dragStarted = false;
  }

  onMount(() => {
    isMacHud = /Mac|iPhone|iPad|iPod/.test(navigator.userAgent);
    let dead = false;

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

<div class="hud-root" class:macos={isMacHud}>
  <div class="hud-shell" class:macos={isMacHud}>
    <div
      class="stack"
      class:macos={isMacHud}
      onpointerdown={isMacHud ? handleMacPointerDown : undefined}
      onpointermove={isMacHud ? handleMacPointerMove : undefined}
      onpointerup={isMacHud ? handleMacPointerUp : undefined}
      onpointercancel={isMacHud ? handleMacPointerCancel : undefined}
      role={isMacHud ? "presentation" : undefined}
    >
      <button
        type="button"
        class="pill"
        class:expanded
        class:macos={isMacHud}
        onclick={isMacHud ? undefined : openYapper}
        aria-label="Open Yapper"
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
    overflow: hidden;
  }

  .hud-root {
    box-sizing: border-box;
    min-height: 100vh;
    width: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-end;
    padding: 4px 6px 10px;
    font-family: "DM Sans", system-ui, sans-serif;
    -webkit-font-smoothing: antialiased;
  }

  .hud-root.macos {
    min-height: 100%;
    height: 100%;
    justify-content: center;
    padding: 0;
    background: transparent;
  }

  .hud-shell {
    display: flex;
    flex-direction: column;
    align-items: stretch;
  }

  .hud-shell.macos {
    width: 100%;
    height: 100%;
    padding: 0;
    background: transparent;
  }

  .stack {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
  }

  .hud-root.macos .stack {
    width: 100%;
    height: 100%;
    cursor: grab;
    align-items: stretch;
  }

  .hud-root.macos .stack:active {
    cursor: grabbing;
  }

  .pill {
    box-sizing: border-box;
    margin: 0;
    padding: 0;
    border: none;
    background: transparent;
    cursor: pointer;
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
  }

  .pill.macos {
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 28px;
    padding: 0 16px;
    border-radius: 999px;
    background: rgba(18, 22, 27, 0.84);
    border-color: rgba(255, 255, 255, 0.34);
    box-shadow:
      inset 0 0 0 1px rgba(255, 255, 255, 0.05),
      0 1px 8px rgba(0, 0, 0, 0.28);
  }

  .pill:hover {
    border-color: rgba(255, 255, 255, 0.52);
    background: rgba(10, 12, 16, 0.55);
  }

  .pill.expanded {
    min-width: 236px;
    min-height: 36px;
    padding: 10px 20px;
  }

  .pill.expanded.macos {
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 32px;
    padding: 0 14px;
  }

  .dots {
    display: flex;
    align-items: flex-end;
    justify-content: center;
    gap: 5px;
    height: 22px;
  }

  .pill.macos .dots {
    gap: 4px;
    height: 20px;
  }

  .dot {
    width: 4px;
    height: 18px;
    border-radius: 2px;
    background: rgba(255, 255, 255, 0.88);
    transform: scaleY(var(--lvl));
    transform-origin: center bottom;
    transition: transform 0.06s ease-out, opacity 0.2s ease;
    opacity: 0.92;
  }

  .pill.macos .dot {
    width: 4px;
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

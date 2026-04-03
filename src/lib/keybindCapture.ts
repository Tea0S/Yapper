/**
 * Build global-shortcut strings from key events. Uses `event.code` (physical key).
 *
 * WebView2 / some browsers omit `ctrlKey`/`shiftKey` on the main key's `keydown` while modifiers
 * are held; we track modifier keydown/keyup as a latch and combine with getModifierState.
 */

export type ModifierLatch = {
  shift: boolean;
  control: boolean;
  alt: boolean;
  meta: boolean;
};

function isModifierPhysicalKey(code: string): boolean {
  return /^(Control|Shift|Alt|Meta|OS)(Left|Right)?$/i.test(code);
}

function applyModifierKeyDown(latch: ModifierLatch, code: string): void {
  if (/^Shift/i.test(code)) latch.shift = true;
  else if (/^Control/i.test(code)) latch.control = true;
  else if (/^Alt/i.test(code)) latch.alt = true;
  else if (/^Meta/i.test(code) || /^OS/i.test(code)) latch.meta = true;
}

function applyModifierKeyUp(latch: ModifierLatch, code: string): void {
  if (/^Shift/i.test(code)) latch.shift = false;
  else if (/^Control/i.test(code)) latch.control = false;
  else if (/^Alt/i.test(code)) latch.alt = false;
  else if (/^Meta/i.test(code) || /^OS/i.test(code)) latch.meta = false;
}

function effectiveModifiers(e: KeyboardEvent, latch: ModifierLatch): ModifierLatch {
  return {
    shift: latch.shift || e.shiftKey || e.getModifierState("Shift"),
    control: latch.control || e.ctrlKey || e.getModifierState("Control"),
    alt: latch.alt || e.altKey || e.getModifierState("Alt"),
    meta:
      latch.meta ||
      e.metaKey ||
      e.getModifierState("Meta") ||
      e.getModifierState("OS"),
  };
}

export function createShortcutCaptureSession() {
  const latch: ModifierLatch = {
    shift: false,
    control: false,
    alt: false,
    meta: false,
  };

  return {
    latch,
    reset() {
      latch.shift = false;
      latch.control = false;
      latch.alt = false;
      latch.meta = false;
    },
    /** Call on keyup so releasing a modifier updates latch. */
    onKeyUp(e: KeyboardEvent) {
      if (isModifierPhysicalKey(e.code)) {
        applyModifierKeyUp(latch, e.code);
      }
    },
    /**
     * Call on keydown. Returns `null` until a non-modifier key is pressed with a usable combo.
     * Modifier-only keydowns update latch and return null.
     */
    consumeKeyDown(e: KeyboardEvent): string | null {
      if (e.code === "Escape") return null;

      const code = e.code;
      if (!code || code === "Unidentified") return null;

      if (isModifierPhysicalKey(code)) {
        if (!e.repeat) applyModifierKeyDown(latch, code);
        return null;
      }

      if (e.repeat) return null;

      const m = effectiveModifiers(e, latch);
      const parts: string[] = [];
      if (m.shift) parts.push("Shift");
      if (m.control) parts.push("Control");
      if (m.alt) parts.push("Alt");
      if (m.meta) parts.push("Super");

      const hasModifier = parts.length > 0;
      const isLetterOrDigit = /^Key[A-Z]$/.test(code) || /^Digit[0-9]$/.test(code);
      if (!hasModifier && isLetterOrDigit) {
        return null;
      }

      parts.push(code);
      return parts.join("+");
    },
  };
}

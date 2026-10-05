const setValue = Object.getOwnPropertyDescriptor(
  HTMLInputElement.prototype,
  "value",
)!.set!;

function decimals(value: number) {
  const text = String(value);
  const exponent = /e-(\d+)$/.exec(text);
  if (exponent) return Number(exponent[1]);
  return text.split(".")[1]?.length ?? 0;
}

/** The value one drag step changes, following `step` when it is usable. */
function stepOf(input: HTMLInputElement, value: number) {
  const declared = Number(input.step);
  if (input.step && input.step !== "any" && declared > 0) return declared;
  // A missing step means 1 to the browser, but fractional fields such as
  // weights carry no step; follow the precision the value already shows.
  if (input.step === "any" || !Number.isInteger(value))
    return 10 ** -Math.max(2, Math.min(4, decimals(value)));
  return 1;
}

/**
 * Editor-style value scrubbing for every `input[type=number]`: dragging the
 * unfocused field sideways changes it, a plain click still edits text.
 * Shift slows down, Ctrl speeds up. `data-no-scrub` on the input or an
 * ancestor opts out. Values are written through the native setter and an
 * `input` event so controlled React inputs receive normal onChange calls.
 */
export function installNumberScrub(document: Document, threshold = 3) {
  let drag: {
    input: HTMLInputElement;
    pointer: number;
    origin: number;
    last: number;
    value: number;
    step: number;
    moved: boolean;
  } | null = null;
  function finish(commit: boolean) {
    if (!drag) return;
    const { input, moved, pointer } = drag;
    drag = null;
    if (input.hasPointerCapture(pointer)) input.releasePointerCapture(pointer);
    input.classList.remove("wb-scrub-active");
    document.documentElement.classList.remove("wb-scrubbing");
    if (moved) {
      if (commit) input.dispatchEvent(new Event("change", { bubbles: true }));
    } else if (commit) {
      input.focus();
      input.select();
    }
  }
  const down = (event: PointerEvent) => {
    const input = event.target;
    if (
      event.button !== 0 ||
      event.pointerType === "touch" ||
      !(input instanceof HTMLInputElement) ||
      input.type !== "number" ||
      input.disabled ||
      input.readOnly ||
      document.activeElement === input ||
      input.closest("[data-no-scrub]")
    )
      return;
    // Decide between click-to-edit and scrub on release.
    event.preventDefault();
    const current = input.valueAsNumber;
    const value = Number.isFinite(current)
      ? current
      : Number.isFinite(Number(input.min)) && input.min !== ""
        ? Number(input.min)
        : 0;
    drag = {
      input,
      pointer: event.pointerId,
      origin: event.clientX,
      last: event.clientX,
      value,
      step: stepOf(input, value),
      moved: false,
    };
    input.setPointerCapture(event.pointerId);
  };
  const move = (event: PointerEvent) => {
    if (!drag || event.pointerId !== drag.pointer) return;
    if (!drag.moved) {
      if (Math.abs(event.clientX - drag.origin) < threshold) return;
      drag.moved = true;
      drag.last = event.clientX;
      drag.input.classList.add("wb-scrub-active");
      document.documentElement.classList.add("wb-scrubbing");
      return;
    }
    const { input, step } = drag;
    const pixelsPerStep = event.shiftKey ? 30 : event.ctrlKey ? 0.3 : 3;
    drag.value += ((event.clientX - drag.last) / pixelsPerStep) * step;
    drag.last = event.clientX;
    const min = input.min === "" ? -Infinity : Number(input.min);
    const max = input.max === "" ? Infinity : Number(input.max);
    drag.value = Math.max(min, Math.min(max, drag.value));
    const snapped = Math.max(
      min,
      Math.min(max, Math.round(drag.value / step) * step),
    );
    const text = String(Number(snapped.toFixed(decimals(step))));
    if (text !== input.value) {
      setValue.call(input, text);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    }
  };
  const up = (event: PointerEvent) => {
    if (drag && event.pointerId === drag.pointer) finish(true);
  };
  const cancel = (event: PointerEvent) => {
    if (drag && event.pointerId === drag.pointer) finish(false);
  };
  document.addEventListener("pointerdown", down);
  document.addEventListener("pointermove", move);
  document.addEventListener("pointerup", up);
  document.addEventListener("pointercancel", cancel);
  return () => {
    finish(false);
    document.removeEventListener("pointerdown", down);
    document.removeEventListener("pointermove", move);
    document.removeEventListener("pointerup", up);
    document.removeEventListener("pointercancel", cancel);
  };
}

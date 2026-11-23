// The backend moves keyboard focus into the UI whenever it hides a page for
// an overlay, which makes the window blur and refocus within a few ms. Menus
// and hover cards that close on `blur` would close themselves as they open.
// This waits for the blur to settle and only reports a real loss of focus
// (switching apps, clicking into a page).

/** Calls `onLost` when the window loses focus and has not regained it after `delayMs`. */
export function addSettledBlurListener(onLost: () => void, delayMs = 120): () => void {
  let timer: number | null = null;
  const onBlur = () => {
    if (timer !== null) window.clearTimeout(timer);
    timer = window.setTimeout(() => {
      timer = null;
      if (!document.hasFocus()) onLost();
    }, delayMs);
  };
  window.addEventListener('blur', onBlur);
  return () => {
    if (timer !== null) window.clearTimeout(timer);
    window.removeEventListener('blur', onBlur);
  };
}

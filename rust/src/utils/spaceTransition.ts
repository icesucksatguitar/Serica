// Space switch transition. App starts it; BrowserViewport, which owns the
// page area, runs it: the page being left is frozen to a capture and slides
// out while the new space's page (its last capture) slides in, then the live
// page takes over in place. Pages are native views that cannot move with the
// UI, so the slide runs on captures.

export type SpaceDirection = 1 | -1;

export interface SpaceTransitionRequest {
  dir: SpaceDirection;
  /** Switches the space; resolves once the backend has switched. */
  doSwitch: () => Promise<unknown>;
  /** Called (once) when there is no page on either side to slide. */
  onNothingToSlide: () => void;
}

type Runner = (req: SpaceTransitionRequest) => Promise<void>;

let runner: Runner | null = null;

export function setSpaceTransitionRunner(r: Runner | null): void {
  runner = r;
}

export function runSpaceTransition(req: SpaceTransitionRequest): Promise<void> {
  if (runner) return runner(req);
  req.onNothingToSlide();
  return Promise.resolve(req.doSwitch()).then(() => undefined);
}

/** Duration and curve shared by the slide and the UI's own space animation. */
export const SPACE_SLIDE_MS = 420;

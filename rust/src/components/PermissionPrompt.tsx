import React, { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { AppWindow, Bell, Camera, ClipboardPaste, Download, FileText, MapPin, Mic, Music, Radar, Type } from 'lucide-react';
import type { PermissionRequest } from '../adapter/tauriBridge';
import { useElementHole } from '../hooks/useViewportOverlay';
import { usePresence } from '../hooks/usePresence';

/** What each permission lets a site do, finishing "<site> wants to …". */
export const PERMISSION_COPY: Record<string, { verb: string; label: string; Icon: typeof Camera }> = {
  camera: { verb: 'use your camera', label: 'Camera', Icon: Camera },
  microphone: { verb: 'use your microphone', label: 'Microphone', Icon: Mic },
  location: { verb: 'know your location', label: 'Location', Icon: MapPin },
  notifications: { verb: 'show notifications', label: 'Notifications', Icon: Bell },
  clipboard: { verb: 'see text and images you copy', label: 'Clipboard', Icon: ClipboardPaste },
  downloads: { verb: 'download several files at once', label: 'Automatic downloads', Icon: Download },
  files: { verb: 'edit files on this computer', label: 'Files', Icon: FileText },
  sensors: { verb: 'use motion and light sensors', label: 'Sensors', Icon: Radar },
  fonts: { verb: 'use the fonts installed on this computer', label: 'Local fonts', Icon: Type },
  midi: { verb: 'control your MIDI devices', label: 'MIDI devices', Icon: Music },
  windows: { verb: 'open and place windows on your displays', label: 'Window management', Icon: AppWindow },
};

function joinVerbs(kinds: string[]): string {
  // Camera + microphone read as one thing, as in every other browser.
  if (kinds.includes('camera') && kinds.includes('microphone')) {
    const rest = kinds.filter((k) => k !== 'camera' && k !== 'microphone');
    return joinVerbs(['__cammic', ...rest]);
  }
  const verbs = kinds.map((k) => (k === '__cammic' ? 'use your camera and microphone' : PERMISSION_COPY[k]?.verb ?? 'use a feature of this computer'));
  if (verbs.length <= 1) return verbs[0] ?? '';
  return `${verbs.slice(0, -1).join(', ')} and ${verbs[verbs.length - 1]}`;
}

type Decision = 'allow' | 'allow-once' | 'block' | 'deny';

const answer = (r: PermissionRequest, d: Decision) => void window.serica?.answerPermission?.(r.id, d);

/**
 * Serica's permission prompt. Sites asking for the camera, microphone,
 * location and the rest get a small card under the address bar instead of
 * WebView2's own dialog. The page stays live around it (the card is cut out
 * of the page), so a call preview can be seen while deciding. Requests from
 * a background tab wait until that tab is shown; a closed tab's are denied.
 */
export const PermissionPrompt: React.FC<{ activeTabId: string | null | undefined; tabIds: string[] }> = ({ activeTabId, tabIds }) => {
  const [queue, setQueue] = useState<PermissionRequest[]>([]);
  const [el, setEl] = useState<HTMLDivElement | null>(null);
  const allowRef = useRef<HTMLButtonElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number }>({ left: 0, top: 44 });

  useEffect(() => window.serica?.onPermissionRequest?.((r: PermissionRequest) => setQueue((q) => [...q, r])), []);

  const tabKey = tabIds.join('|');
  useEffect(() => {
    const open = new Set(tabIds);
    setQueue((q) => {
      const gone = q.filter((r) => !open.has(r.tabId) && r.tabId !== 'assistant');
      if (!gone.length) return q;
      gone.forEach((r) => answer(r, 'deny'));
      return q.filter((r) => !gone.includes(r));
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tabKey]);

  // The first waiting request of the visible tab, with every other request
  // of that tab and site (camera + microphone arrive as two).
  const group = useMemo(() => {
    const first = queue.find((r) => r.tabId === activeTabId || r.tabId === 'assistant');
    if (!first) return [];
    return queue.filter((r) => r.tabId === first.tabId && r.origin === first.origin);
  }, [queue, activeTabId]);
  const lead = group[0] ?? null;
  const { mounted, state } = usePresence(!!lead, 150);
  const [shownGroup, setShownGroup] = useState<PermissionRequest[]>([]);
  useEffect(() => {
    if (group.length) setShownGroup(group);
  }, [group]);

  const ready = useElementHole('permission-prompt', mounted ? el : null, 0);

  useLayoutEffect(() => {
    if (!mounted) return;
    const place = () => {
      const pill = document.querySelector('.top-floating-url-pill') as HTMLElement | null;
      const bar = document.querySelector('.floating-top-chrome-capsule') as HTMLElement | null;
      const top = (bar?.getBoundingClientRect().bottom ?? 36) + 8;
      const width = 360;
      const left = pill ? Math.round(pill.getBoundingClientRect().left) : Math.round((window.innerWidth - width) / 2);
      setPos({ left: Math.max(12, Math.min(left, window.innerWidth - width - 12)), top: Math.round(top) });
    };
    place();
    window.addEventListener('resize', place);
    return () => window.removeEventListener('resize', place);
  }, [mounted]);

  useEffect(() => {
    if (!lead) return;
    const t = window.setTimeout(() => allowRef.current?.focus({ preventScroll: true }), 60);
    return () => window.clearTimeout(t);
  }, [lead?.id]);

  if (!mounted) return null;
  const shown = lead ? group : shownGroup;
  const head = shown[0];
  if (!head) return null;
  const kinds = Array.from(new Set(shown.map((r) => r.kind)));
  const Icon = (kinds.includes('camera') ? PERMISSION_COPY.camera : PERMISSION_COPY[kinds[0]] ?? PERMISSION_COPY.camera).Icon;

  const decide = (d: Decision) => {
    if (!lead) return;
    group.forEach((r) => answer(r, d));
    const ids = new Set(group.map((r) => r.id));
    setQueue((q) => q.filter((r) => !ids.has(r.id)));
  };

  return (
    <div className="presence" data-state={ready ? state : 'closed'}>
      <div
        ref={setEl}
        className="perm-card"
        role="dialog"
        aria-modal="false"
        aria-labelledby="perm-title"
        style={{ left: pos.left, top: pos.top }}
        onKeyDown={(e) => {
          if (e.key === 'Escape') {
            e.preventDefault();
            e.stopPropagation();
            decide('deny');
          }
        }}
      >
        <div className="perm-head">
          <span className="perm-icon" aria-hidden="true">
            <Icon size={16} strokeWidth={1.8} />
          </span>
          <div className="perm-text">
            <div className="perm-host" id="perm-title" title={head.origin}>
              {head.host}
            </div>
            <div className="perm-ask">wants to {joinVerbs(kinds)}</div>
          </div>
        </div>
        <div className="perm-actions">
          <button type="button" className="btn btn-quiet" onClick={() => decide('block')}>
            Block
          </button>
          <span className="perm-spacer" />
          <button type="button" className="btn" onClick={() => decide('allow-once')}>
            This time
          </button>
          <button ref={allowRef} type="button" className="btn btn-primary" onClick={() => decide('allow')}>
            Allow
          </button>
        </div>
        <div className="perm-foot">Change this any time in Settings › Privacy</div>
      </div>
    </div>
  );
};

export default PermissionPrompt;

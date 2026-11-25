// Browser tools. All of them live in Settings > Appearance > Top bar tools;
// the ones the user pins also show in the top bar's "⋯" menu, next to the
// light/dark switch and Settings, which are always there.
import type { LucideIcon } from 'lucide-react';
import { Download, FileText, History, Lock, Palette, Sparkles } from 'lucide-react';

export type ToolKey = 'history' | 'downloads' | 'open-file' | 'ai' | 'vpn' | 'theme';

export interface ToolDef {
  key: ToolKey;
  name: string;
  /** Tooltip in the top bar (with the shortcut). */
  label: string;
  /** One line in Settings. */
  hint: string;
  icon: LucideIcon;
}

export const TOOLS: readonly ToolDef[] = Object.freeze([
  { key: 'history', name: 'History', label: 'History (Ctrl+H)', hint: 'Pages you visited', icon: History },
  { key: 'downloads', name: 'Downloads', label: 'Downloads (Ctrl+J)', hint: 'Files you downloaded', icon: Download },
  { key: 'open-file', name: 'Open file', label: 'Open file, e.g. a PDF (Ctrl+O)', hint: 'Open a PDF or other file from this PC', icon: FileText },
  { key: 'ai', name: 'AI assistants', label: 'AI assistants & tools', hint: 'Assistants and the MCP connection', icon: Sparkles },
  { key: 'vpn', name: 'VPN', label: 'VPN & privacy tunnel', hint: 'Route pages through the privacy tunnel', icon: Lock },
  { key: 'theme', name: 'Theme Studio', label: 'Theme Studio', hint: 'Colors and backgrounds of your spaces', icon: Palette },
]);

const KEYS = new Set<string>(TOOLS.map((t) => t.key));

/** Pinned tool keys from a saved value, in the order of `TOOLS`. */
export function normalizePinnedTools(value: unknown): ToolKey[] {
  if (!Array.isArray(value)) return [];
  const set = new Set(value.filter((v): v is string => typeof v === 'string' && KEYS.has(v)));
  return TOOLS.filter((t) => set.has(t.key)).map((t) => t.key);
}

export type ToolActions = Record<ToolKey, () => void>;

import React, { useCallback, useEffect, useState } from 'react';
import { Check, Copy, Eye, EyeOff, PlugZap, RefreshCw } from 'lucide-react';

interface McpServerStatus {
  enabled: boolean;
  running: boolean;
  port: number;
  url: string;
  token: string;
  error?: string | null;
}

/** Ready-to-paste client configs for the local MCP endpoint. */
export function claudeCodeCommand(s: Pick<McpServerStatus, 'url' | 'token'>): string {
  return `claude mcp add --transport http neutrino ${s.url} --header "Authorization: Bearer ${s.token}"`;
}

export function claudeDesktopConfig(s: Pick<McpServerStatus, 'url' | 'token'>): string {
  return JSON.stringify(
    {
      mcpServers: {
        neutrino: {
          command: 'npx',
          args: ['-y', 'mcp-remote', s.url, '--header', 'Authorization:${NEUTRINO_AUTH}'],
          env: { NEUTRINO_AUTH: `Bearer ${s.token}` },
        },
      },
    },
    null,
    2,
  );
}

const CopyBlock: React.FC<{ label: string; text: string; display?: string }> = ({ label, text, display }) => {
  const [copied, setCopied] = useState(false);
  return (
    <div className="mcp-connect-block">
      <div className="mcp-connect-block-head">
        <span>{label}</span>
        <button
          type="button"
          className="mcp-connect-copy"
          onClick={() => {
            void navigator.clipboard?.writeText(text).then(() => {
              setCopied(true);
              setTimeout(() => setCopied(false), 1400);
            });
          }}
          aria-label={`Copy ${label}`}
        >
          {copied ? <Check size={12} /> : <Copy size={12} />}
          {copied ? 'Copied' : 'Copy'}
        </button>
      </div>
      <pre className="mcp-connect-pre">{display ?? text}</pre>
    </div>
  );
};

/** Turns the local MCP endpoint on/off and shows how to connect Claude to it. */
export const McpConnectCard: React.FC = () => {
  const [status, setStatus] = useState<McpServerStatus | null>(null);
  const [showToken, setShowToken] = useState(false);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    const s = await window.serica?.mcpServerStatus?.().catch(() => null);
    if (s) setStatus(s);
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const toggle = async (enabled: boolean) => {
    setBusy(true);
    await window.serica?.updateSettings?.({ mcpServerEnabled: enabled }).catch(() => undefined);
    // The backend (re)starts the listener when settings change.
    await new Promise((r) => setTimeout(r, 250));
    await refresh();
    setBusy(false);
  };

  if (!status) return null;
  const masked = showToken ? status.token : `${status.token.slice(0, 4)}${'•'.repeat(20)}${status.token.slice(-4)}`;
  const hidden = { ...status, token: masked };

  return (
    <div className="mcp-connect-card">
      <div className="mcp-connect-row">
        <PlugZap size={18} className={status.running ? 'mcp-connect-icon on' : 'mcp-connect-icon'} />
        <div className="mcp-connect-text">
          <div className="mcp-connect-title">Connect an AI agent</div>
          <div className="mcp-connect-sub">
            {status.running
              ? `Listening on ${status.url} (this computer only)`
              : status.error
                ? status.error
                : 'Let Claude and other MCP clients control this browser.'}
          </div>
        </div>
        <label className="extension-switch" title={status.enabled ? 'Turn off' : 'Turn on'}>
          <input
            type="checkbox"
            checked={status.enabled}
            disabled={busy}
            onChange={(e) => void toggle(e.target.checked)}
            aria-label="Allow AI agents to control Serica"
          />
          <span className="extension-switch-track" />
        </label>
      </div>

      {status.enabled && (
        <>
          <div className="mcp-connect-token">
            <span className="mcp-connect-token-label">Access token</span>
            <code className="mcp-connect-token-value">{masked}</code>
            <button type="button" className="mcp-connect-copy" onClick={() => setShowToken((v) => !v)} aria-label={showToken ? 'Hide token' : 'Show token'}>
              {showToken ? <EyeOff size={12} /> : <Eye size={12} />}
            </button>
            <button
              type="button"
              className="mcp-connect-copy"
              title="Issue a new token (connected clients must be updated)"
              onClick={async () => {
                const s = await window.serica?.mcpRegenerateToken?.().catch(() => null);
                if (s) setStatus(s);
              }}
            >
              <RefreshCw size={12} /> New
            </button>
          </div>
          <CopyBlock label="Claude Code" text={claudeCodeCommand(status)} display={claudeCodeCommand(hidden)} />
          <CopyBlock
            label="Claude Desktop (claude_desktop_config.json)"
            text={claudeDesktopConfig(status)}
            display={claudeDesktopConfig(hidden)}
          />
          <div className="mcp-connect-note">Anyone with this token can control your tabs. Keep it private, and turn this off when you don't need it.</div>
        </>
      )}
    </div>
  );
};

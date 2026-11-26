import React, { useCallback, useEffect, useRef, useState } from 'react';
import { ArrowUp, Bot, Cpu, ExternalLink, FileText, Play, RefreshCw, Sparkles, X } from 'lucide-react';
import { appIcon } from './home/apps';
import { McpConnectCard } from './McpConnectCard';
import { TabState } from '../types';

type McpTab = 'assistants' | 'local' | 'tools';

interface McpModalProps {
  isOpen: boolean;
  onClose: () => void;
  activeTab?: TabState;
  onNavigate?: (url: string) => void;
  initialTab?: McpTab;
  /** Unused: assistant pages share the active profile's storage in the Rust tab host. */
  partition?: string;
}

interface Assistant {
  id: string;
  name: string;
  url: string;
}

const ASSISTANTS: Assistant[] = [
  { id: 'claude', name: 'Claude', url: 'https://claude.ai' },
  { id: 'chatgpt', name: 'ChatGPT', url: 'https://chatgpt.com' },
  { id: 'gemini', name: 'Gemini', url: 'https://gemini.google.com' },
  { id: 'deepseek', name: 'DeepSeek', url: 'https://chat.deepseek.com' },
  { id: 'perplexity', name: 'Perplexity', url: 'https://www.perplexity.ai' },
];

const TOOLS: { name: string; desc: string }[] = [
  { name: 'browser_get_status', desc: 'Active space and tab, tab count, Shields' },
  { name: 'browser_list_tabs', desc: 'Every open tab, with its space and state' },
  { name: 'browser_navigate', desc: 'Load an address in a tab' },
  { name: 'browser_create_tab', desc: 'Open a new tab in a space' },
  { name: 'browser_close_tab', desc: 'Close a tab' },
  { name: 'browser_get_adblock_stats', desc: 'What Shields has blocked' },
  { name: 'browser_zen_optimize', desc: 'Put idle background tabs to sleep' },
  { name: 'browser_media_control', desc: 'Play, pause and picture-in-picture' },
];

const WIDTH_KEY = 'serica_ai_panel_width';
const MIN_W = 360;
const MAX_W = 760;

interface ChatMessage {
  role: 'user' | 'assistant';
  content: string;
}

/**
 * The assistant's page: a native WebView2 view owned by the Rust tab host,
 * kept on this element's rectangle. It opens once the panel has finished
 * sliding in, and closes as soon as the panel starts to leave, so a native
 * view never sits over the panel's motion.
 */
const NativeAssistantFrame: React.FC<{ url: string; live: boolean }> = ({ url, live }) => {
  const ref = useRef<HTMLDivElement>(null);
  const opened = useRef(false);
  useEffect(() => {
    if (!live) return;
    let alive = true;
    const t = window.setTimeout(() => {
      const el = ref.current;
      if (!el || !alive) return;
      const r = el.getBoundingClientRect();
      void window.serica?.assistantOpen?.(url, r.left, r.top, r.width, r.height).then(() => {
        if (alive) opened.current = true;
      });
    }, 260);
    return () => {
      alive = false;
      window.clearTimeout(t);
    };
  }, [url, live]);
  useEffect(() => {
    if (live) return;
    if (opened.current) {
      opened.current = false;
      void window.serica?.assistantClose?.();
    }
  }, [live]);
  useEffect(() => {
    // Follows the element (the panel resizes) once a frame; only real changes reach the backend.
    let last = '';
    let raf = 0;
    const track = () => {
      const el = ref.current;
      if (el && opened.current) {
        const r = el.getBoundingClientRect();
        const key = `${Math.round(r.left)},${Math.round(r.top)},${Math.round(r.width)},${Math.round(r.height)}`;
        if (key !== last) {
          last = key;
          void window.serica?.assistantBounds?.(r.left, r.top, r.width, r.height);
        }
      }
      raf = requestAnimationFrame(track);
    };
    raf = requestAnimationFrame(track);
    return () => {
      cancelAnimationFrame(raf);
      opened.current = false;
      void window.serica?.assistantClose?.();
    };
  }, []);
  return (
    <div ref={ref} className="ai-frame-slot">
      <span className="ai-frame-loading">
        <img src={appIcon(new URL(url).hostname)} alt="" width={28} height={28} />
      </span>
    </div>
  );
};

/**
 * The AI panel, docked on the right: assistants (signed in as you, in a
 * real page), local models (Ollama, LM Studio), and the MCP endpoint that
 * lets agents use Serica. The width can be dragged from the left edge.
 */
export const McpModal: React.FC<McpModalProps> = ({ isOpen, onClose, activeTab, onNavigate, initialTab }) => {
  const [tab, setTab] = useState<McpTab>(initialTab || 'assistants');
  useEffect(() => {
    if (initialTab && isOpen) setTab(initialTab);
  }, [initialTab, isOpen]);

  const [assistantId, setAssistantId] = useState(() => {
    try {
      return localStorage.getItem('serica_ai_assistant') || 'claude';
    } catch {
      return 'claude';
    }
  });
  const assistant = ASSISTANTS.find((a) => a.id === assistantId) ?? ASSISTANTS[0];
  const pickAssistant = (id: string) => {
    setAssistantId(id);
    try {
      localStorage.setItem('serica_ai_assistant', id);
    } catch {
      // A private window: it only lasts this session.
    }
  };

  // ---- Width: dragged from the left edge, remembered.
  const [width, setWidth] = useState(() => {
    try {
      const v = Number(localStorage.getItem(WIDTH_KEY));
      return Number.isFinite(v) && v >= MIN_W ? Math.min(MAX_W, v) : 440;
    } catch {
      return 440;
    }
  });
  const [resizing, setResizing] = useState(false);
  const drag = useRef<{ x: number; w: number } | null>(null);
  const onEdgeDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { x: e.clientX, w: width };
    setResizing(true);
  };
  const onEdgeMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d) return;
    setWidth(Math.max(MIN_W, Math.min(Math.min(MAX_W, window.innerWidth - 80), d.w + (d.x - e.clientX))));
  };
  const onEdgeUp = () => {
    if (!drag.current) return;
    drag.current = null;
    setResizing(false);
    try {
      localStorage.setItem(WIDTH_KEY, String(Math.round(width)));
    } catch {
      // Not kept in a private window.
    }
  };

  // ---- Local models
  const [scanning, setScanning] = useState(false);
  const [ollama, setOllama] = useState(false);
  const [lmStudio, setLmStudio] = useState(false);
  const [models, setModels] = useState<{ name: string; runtime: 'ollama' | 'lmstudio' }[]>([]);
  const [model, setModel] = useState('');
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState('');
  const [generating, setGenerating] = useState(false);
  const chatEnd = useRef<HTMLDivElement>(null);

  const scan = useCallback(async () => {
    setScanning(true);
    const found: { name: string; runtime: 'ollama' | 'lmstudio' }[] = [];
    try {
      const res = await fetch('http://localhost:11434/api/tags', { signal: AbortSignal.timeout(1800) });
      setOllama(res.ok);
      if (res.ok) for (const m of (await res.json()).models ?? []) found.push({ name: m.name || m.model, runtime: 'ollama' });
    } catch {
      setOllama(false);
    }
    try {
      const res = await fetch('http://localhost:1234/v1/models', { signal: AbortSignal.timeout(1800) });
      setLmStudio(res.ok);
      if (res.ok) for (const m of (await res.json()).data ?? []) found.push({ name: m.id, runtime: 'lmstudio' });
    } catch {
      setLmStudio(false);
    }
    setModels(found);
    setModel((cur) => cur || found[0]?.name || '');
    setScanning(false);
  }, []);
  useEffect(() => {
    if (isOpen && tab === 'local') void scan();
  }, [isOpen, tab, scan]);

  const send = async () => {
    const text = input.trim();
    if (!text || generating || !model) return;
    const next: ChatMessage[] = [...messages, { role: 'user', content: text }];
    setMessages([...next, { role: 'assistant', content: '' }]);
    setInput('');
    setGenerating(true);
    const runtime = models.find((m) => m.name === model)?.runtime ?? 'ollama';
    let full = '';
    const show = () => setMessages([...next, { role: 'assistant', content: full }]);
    try {
      if (runtime === 'ollama') {
        const res = await fetch('http://localhost:11434/api/chat', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ model, messages: next, stream: true }),
        });
        const reader = res.body!.getReader();
        const dec = new TextDecoder();
        let buf = '';
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          buf += dec.decode(value, { stream: true });
          const lines = buf.split('\n');
          buf = lines.pop() ?? '';
          for (const line of lines) {
            try {
              const p = JSON.parse(line);
              if (p.message?.content) {
                full += p.message.content;
                show();
              }
            } catch {
              // A partial line: the rest comes with the next chunk.
            }
          }
        }
      } else {
        const res = await fetch('http://localhost:1234/v1/chat/completions', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ model, messages: next, stream: true }),
        });
        const reader = res.body!.getReader();
        const dec = new TextDecoder();
        let buf = '';
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          buf += dec.decode(value, { stream: true });
          const lines = buf.split('\n');
          buf = lines.pop() ?? '';
          for (const line of lines) {
            const data = line.replace(/^data:\s*/, '').trim();
            if (!data || data === '[DONE]') continue;
            try {
              const delta = JSON.parse(data).choices?.[0]?.delta?.content;
              if (delta) {
                full += delta;
                show();
              }
            } catch {
              // A partial line.
            }
          }
        }
      }
    } catch (err: any) {
      full = full || `${runtime === 'ollama' ? 'Ollama' : 'LM Studio'} didn’t answer: ${err?.message ?? 'no connection'}`;
      show();
    } finally {
      setGenerating(false);
      requestAnimationFrame(() => chatEnd.current?.scrollIntoView({ block: 'end', behavior: 'smooth' }));
    }
  };

  // ---- Agents (MCP): calls made through the endpoint
  const [logs, setLogs] = useState<any[]>([]);
  const [result, setResult] = useState<{ tool: string; text: string } | null>(null);
  useEffect(() => {
    if (!isOpen || tab !== 'tools') return;
    const load = () => void window.serica?.getMcpLogs?.().then((l: any[]) => setLogs(l || []));
    load();
    const t = window.setInterval(load, 1500);
    return () => window.clearInterval(t);
  }, [isOpen, tab]);
  const test = async (tool: string) => {
    try {
      const res = await window.serica?.callMcp?.('tools/call', { name: tool, arguments: {} });
      setResult({ tool, text: JSON.stringify(res, null, 2) });
    } catch (err: any) {
      setResult({ tool, text: `Error: ${err?.message ?? err}` });
    }
  };

  const sharePage = () => {
    if (!activeTab?.url) return;
    void window.serica?.assistantInsertText?.(`${activeTab.title} — ${activeTab.url}\n\n`);
  };

  return (
    <>
      <div className="mcp-drawer-overlay ai-backdrop" onMouseDown={onClose} />
      <aside className={`mcp-drawer ai-panel${resizing ? ' is-resizing' : ''}`} style={{ width }} role="dialog" aria-label="AI">
        <div className="ai-edge" onPointerDown={onEdgeDown} onPointerMove={onEdgeMove} onPointerUp={onEdgeUp} onPointerCancel={onEdgeUp} title="Drag to resize" />
        <header className="ai-head">
          <div className="seg ai-tabs" role="tablist" aria-label="AI">
            {(
              [
                ['assistants', 'Assistants', Sparkles],
                ['local', 'Local', Cpu],
                ['tools', 'Agents', Bot],
              ] as const
            ).map(([id, label, Icon]) => (
              <button key={id} type="button" role="tab" aria-selected={tab === id} className={tab === id ? 'on' : ''} onClick={() => setTab(id)}>
                <Icon size={13} aria-hidden="true" />
                {label}
              </button>
            ))}
          </div>
          <button type="button" className="btn btn-quiet btn-icon" aria-label="Close" title="Close (Esc)" onClick={onClose}>
            <X size={16} />
          </button>
        </header>

        {tab === 'assistants' && (
          <div className="ai-body ai-assistants">
            <div className="ai-toolbar">
              <div className="ai-logos" role="radiogroup" aria-label="Assistant">
                {ASSISTANTS.map((a) => (
                  <button
                    key={a.id}
                    type="button"
                    role="radio"
                    aria-checked={a.id === assistant.id}
                    aria-label={a.name}
                    title={a.name}
                    className={`ai-logo${a.id === assistant.id ? ' on' : ''}`}
                    onClick={() => pickAssistant(a.id)}
                  >
                    <img src={appIcon(new URL(a.url).hostname)} alt="" width={18} height={18} draggable={false} />
                  </button>
                ))}
              </div>
              <span className="ai-spacer" />
              {activeTab?.url && /^https?:/.test(activeTab.url) && (
                <button type="button" className="btn btn-quiet btn-icon" title="Share this page with the assistant" aria-label="Share this page with the assistant" onClick={sharePage}>
                  <FileText size={15} />
                </button>
              )}
              <button type="button" className="btn btn-quiet btn-icon" title={`Open ${assistant.name} in a tab`} aria-label={`Open ${assistant.name} in a tab`} onClick={() => onNavigate?.(assistant.url)}>
                <ExternalLink size={15} />
              </button>
            </div>
            <div className="ai-frame">
              <NativeAssistantFrame key={assistant.url} url={assistant.url} live={isOpen && !resizing} />
            </div>
          </div>
        )}

        {tab === 'local' && (
          <div className="ai-body ai-local">
            <div className="ai-toolbar">
              <span className={`ai-runtime${ollama ? ' on' : ''}`} title="Ollama, on port 11434">
                <span className="ai-dot" aria-hidden="true" />
                Ollama
              </span>
              <span className={`ai-runtime${lmStudio ? ' on' : ''}`} title="LM Studio, on port 1234">
                <span className="ai-dot" aria-hidden="true" />
                LM Studio
              </span>
              <span className="ai-spacer" />
              {models.length > 0 && (
                <select className="input select ai-model" value={model} onChange={(e) => setModel(e.target.value)} aria-label="Model">
                  {models.map((m) => (
                    <option key={`${m.runtime}:${m.name}`} value={m.name}>
                      {m.name}
                    </option>
                  ))}
                </select>
              )}
              <button type="button" className="btn btn-quiet btn-icon" disabled={scanning} title="Look again" aria-label="Look for local models again" onClick={() => void scan()}>
                <RefreshCw size={14} className={scanning ? 'spin' : ''} />
              </button>
            </div>
            {models.length === 0 ? (
              <div className="ai-empty">
                <Cpu size={22} aria-hidden="true" />
                <h3>No local models running</h3>
                <p>Start Ollama, or LM Studio with its local server on, and Serica finds the models. Everything stays on this PC.</p>
              </div>
            ) : (
              <>
                <div className="ai-chat">
                  {messages.length === 0 && <p className="ai-chat-hint">Ask {model} anything. It runs on this PC; nothing is sent anywhere.</p>}
                  {messages.map((m, i) => (
                    <div key={i} className={`ai-msg ${m.role}`}>
                      {m.content || (generating && i === messages.length - 1 ? <span className="ai-typing" aria-label="Writing" /> : null)}
                    </div>
                  ))}
                  <div ref={chatEnd} />
                </div>
                <form
                  className="ai-composer"
                  onSubmit={(e) => {
                    e.preventDefault();
                    void send();
                  }}
                >
                  <textarea
                    className="ai-input"
                    value={input}
                    rows={1}
                    placeholder={`Message ${model}`}
                    disabled={generating}
                    onChange={(e) => setInput(e.target.value)}
                    onKeyDown={(e) => {
                      e.stopPropagation();
                      if (e.key === 'Enter' && !e.shiftKey) {
                        e.preventDefault();
                        void send();
                      }
                    }}
                  />
                  <button type="submit" className="ai-send" disabled={generating || !input.trim()} aria-label="Send">
                    <ArrowUp size={15} strokeWidth={2.4} />
                  </button>
                </form>
              </>
            )}
          </div>
        )}

        {tab === 'tools' && (
          <div className="ai-body ai-tools">
            <McpConnectCard />
            <section className="s2-group">
              <h3 className="s2-group-title">What agents can do</h3>
              <div className="s2-card">
                {TOOLS.map((t) => (
                  <div key={t.name} className="s2-row ai-tool">
                    <div className="s2-row-text">
                      <span className="s2-row-title ai-mono">{t.name}</span>
                      <span className="s2-row-detail">{t.desc}</span>
                    </div>
                    <button type="button" className="btn btn-quiet" onClick={() => void test(t.name)} title="Run it now and see the answer">
                      <Play size={11} aria-hidden="true" />
                      Try
                    </button>
                  </div>
                ))}
              </div>
            </section>
            {result && (
              <section className="s2-group">
                <h3 className="s2-group-title">
                  Answer from <span className="ai-mono">{result.tool}</span>
                </h3>
                <pre className="ai-pre">{result.text}</pre>
              </section>
            )}
            <section className="s2-group">
              <h3 className="s2-group-title">Recent calls</h3>
              <div className="ai-log">
                {logs.length === 0 ? (
                  <p className="ai-chat-hint">Nothing yet. Calls from agents show up here as they happen.</p>
                ) : (
                  logs
                    .slice(-60)
                    .reverse()
                    .map((log, i) => (
                      <div key={i} className="ai-log-row">
                        <span className={`ai-dir ${log.direction === 'in' ? 'in' : 'out'}`}>{log.direction === 'in' ? 'in' : 'out'}</span>
                        <span className="ai-mono">{log.method || 'result'}</span>
                        <span className="ai-log-data">{JSON.stringify(log.data)}</span>
                      </div>
                    ))
                )}
              </div>
            </section>
          </div>
        )}
      </aside>
    </>
  );
};

export default McpModal;

import React, { useEffect, useState } from 'react';
import { ChevronRight, RefreshCw, X } from 'lucide-react';
import { DOH_PROVIDERS } from '../shared/vpnPresets';
import type { DohProviderId, ProxyProtocol, VpnConfig, VpnMode, VpnStatus } from '../types';
import { Switch } from './Switch';

interface VpnModalProps {
  isOpen: boolean;
  onClose: () => void;
  status: VpnStatus | null;
  onConnect: (patch?: Partial<VpnConfig>) => Promise<void>;
  onDisconnect: () => Promise<void>;
  onUpdateConfig: (patch: Partial<VpnConfig>) => Promise<void>;
  onCheckIp: () => Promise<void>;
  isCheckingIp: boolean;
  ipCheckError: string | null;
}

const ROUTES: Array<{ id: VpnMode; label: string; hint: string }> = [
  { id: 'doh-only', label: 'Encrypted DNS', hint: 'Hides the sites you look up from your network. Built in, nothing to set up' },
  { id: 'system-vpn', label: 'System VPN', hint: 'Uses the VPN app already connected on this PC' },
  { id: 'tor-local', label: 'Tor', hint: 'Through the Tor service running on this PC (port 9050 or 9150)' },
  { id: 'custom-proxy', label: 'Proxy server', hint: 'Through a SOCKS5, HTTPS or HTTP server you choose' },
];

/**
 * The VPN in one glance: whether it is on, what it routes through, and one
 * button to turn it on or off. Route details and leak protection are a
 * click away, not in the way.
 */
export const VpnModal: React.FC<VpnModalProps> = ({
  isOpen,
  onClose,
  status,
  onConnect,
  onDisconnect,
  onUpdateConfig,
  onCheckIp,
  isCheckingIp,
  ipCheckError,
}) => {
  const config = status?.config;
  const [proxyProtocol, setProxyProtocol] = useState<ProxyProtocol>('socks5');
  const [proxyHost, setProxyHost] = useState('127.0.0.1');
  const [proxyPort, setProxyPort] = useState('1080');
  const [proxyUser, setProxyUser] = useState('');
  const [proxyPass, setProxyPass] = useState('');
  const [bypassRules, setBypassRules] = useState('localhost,127.0.0.1,<local>');
  const [showAdvanced, setShowAdvanced] = useState(false);

  useEffect(() => {
    if (config) {
      setProxyProtocol(config.proxy.protocol);
      setProxyHost(config.proxy.host);
      setProxyPort(String(config.proxy.port));
      setProxyUser(config.proxy.username || '');
      setProxyPass(config.proxy.password || '');
      setBypassRules(config.proxy.bypassRules || 'localhost,127.0.0.1,<local>');
    }
  }, [
    isOpen,
    config?.proxy.protocol,
    config?.proxy.host,
    config?.proxy.port,
    config?.proxy.username,
    config?.proxy.password,
    config?.proxy.bypassRules,
  ]);

  useEffect(() => {
    if (!isOpen) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [isOpen, onClose]);

  if (!isOpen || !status || !config) return null;

  const state = status.state;
  const isConnected = state === 'connected';
  const isConnecting = state === 'connecting';
  const isError = state === 'error';
  const route = ROUTES.find((r) => r.id === config.mode) ?? ROUTES[0];
  const doh = DOH_PROVIDERS[config.dohProvider];

  const headline = isConnected ? 'Connected' : isConnecting ? 'Connecting…' : isError ? 'Could not connect' : 'Off';
  const detail = isError && status.lastError
    ? status.lastError
    : isConnected
      ? config.mode === 'system-vpn' && status.detectedOsVpn
        ? `Through ${status.detectedOsVpn.name}`
        : config.mode === 'custom-proxy' || config.mode === 'tor-local'
          ? `Through ${status.activeProxyRule || route.label}`
          : `Encrypted DNS with ${doh?.label ?? 'your resolver'}`
      : `Turn on to use ${route.id === "doh-only" ? "encrypted DNS" : route.label}`;

  const applyProxy = (e: React.FormEvent) => {
    e.preventDefault();
    const portNum = Math.max(1, Math.min(65535, parseInt(proxyPort, 10) || 1080));
    void onUpdateConfig({
      enabled: true,
      mode: 'custom-proxy',
      proxy: {
        protocol: proxyProtocol,
        host: proxyHost.trim() || '127.0.0.1',
        port: portNum,
        username: proxyUser,
        password: proxyPass,
        bypassRules: bypassRules.trim() || 'localhost,127.0.0.1,<local>',
      },
    });
  };

  return (
    <div
      className="modal-overlay vpn-overlay"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="panel vpn-panel" role="dialog" aria-modal="true" aria-label="VPN">
        <header className="panel-head">
          <h2 className="panel-title">VPN</h2>
          <button type="button" className="btn btn-quiet btn-icon" onClick={onClose} aria-label="Close" title="Close (Esc)">
            <X size={16} />
          </button>
        </header>

        <div className="panel-body">
          <div className={`vpn-hero state-${state}`}>
            <span className="vpn-state-dot" aria-hidden="true" />
            <div className="vpn-hero-text">
              <span className="vpn-headline" role="status">
                {headline}
              </span>
              <span className={`vpn-detail${isError ? ' error' : ''}`}>{detail}</span>
            </div>
            <button
              type="button"
              className={`btn ${config.enabled ? '' : 'btn-primary'} vpn-power`}
              disabled={isConnecting}
              onClick={() => void (config.enabled ? onDisconnect() : onConnect())}
            >
              {config.enabled ? 'Turn off' : 'Turn on'}
            </button>
          </div>

          <section className="panel-section">
            <h3 className="panel-label">Route</h3>
            <div className="radio-list" role="radiogroup" aria-label="Route">
              {ROUTES.map((r) => {
                const selected = config.mode === r.id;
                return (
                  <button
                    key={r.id}
                    type="button"
                    role="radio"
                    aria-checked={selected}
                    className={`radio-row${selected ? ' selected' : ''}`}
                    onClick={() => {
                      if (!selected) void onUpdateConfig({ mode: r.id });
                    }}
                  >
                    <span className="radio-mark" aria-hidden="true" />
                    <span className="radio-text">
                      <span className="radio-label">{r.label}</span>
                      {selected && <span className="radio-hint">{r.hint}</span>}
                    </span>
                  </button>
                );
              })}
            </div>

            {config.mode === 'custom-proxy' && (
              <form className="vpn-proxy" onSubmit={applyProxy}>
                <div className="field-row">
                  <select
                    value={proxyProtocol}
                    onChange={(e) => setProxyProtocol(e.target.value as ProxyProtocol)}
                    className="field field-proto"
                    aria-label="Protocol"
                  >
                    <option value="socks5">SOCKS5</option>
                    <option value="https">HTTPS</option>
                    <option value="http">HTTP</option>
                  </select>
                  <input
                    type="text"
                    value={proxyHost}
                    onChange={(e) => setProxyHost(e.target.value)}
                    placeholder="Host"
                    className="field"
                    aria-label="Host"
                  />
                  <input
                    type="number"
                    min={1}
                    max={65535}
                    value={proxyPort}
                    onChange={(e) => setProxyPort(e.target.value)}
                    placeholder="Port"
                    className="field field-port"
                    aria-label="Port"
                  />
                </div>
                <div className="field-row">
                  <input
                    type="text"
                    value={proxyUser}
                    onChange={(e) => setProxyUser(e.target.value)}
                    placeholder="Username (optional)"
                    className="field"
                    aria-label="Username"
                  />
                  <input
                    type="password"
                    value={proxyPass}
                    onChange={(e) => setProxyPass(e.target.value)}
                    placeholder="Password (optional)"
                    className="field"
                    aria-label="Password"
                  />
                </div>
                <div className="field-row">
                  <input
                    type="text"
                    value={bypassRules}
                    onChange={(e) => setBypassRules(e.target.value)}
                    placeholder="Skip for: localhost,127.0.0.1,<local>"
                    className="field"
                    aria-label="Addresses that skip the proxy"
                  />
                  <button type="submit" className="btn btn-primary">
                    Save and connect
                  </button>
                </div>
              </form>
            )}
          </section>

          <section className="panel-section">
            <div className="kv-row">
              <span className="kv-key">Your IP address</span>
              <span className="kv-val">
                {status.ipCheck
                  ? `${status.ipCheck.ip}${status.ipCheck.location ? ` · ${status.ipCheck.location}` : ''}`
                  : 'Not checked'}
              </span>
              <button type="button" className="btn btn-quiet" onClick={() => void onCheckIp()} disabled={isCheckingIp}>
                <RefreshCw size={12} className={isCheckingIp ? 'spin-icon' : ''} aria-hidden="true" />
                {isCheckingIp ? 'Checking' : 'Check'}
              </button>
            </div>
            {ipCheckError && <p className="panel-error">{ipCheckError}</p>}
          </section>

          <section className="panel-section">
            <button
              type="button"
              className="disclosure"
              aria-expanded={showAdvanced}
              onClick={() => setShowAdvanced((v) => !v)}
            >
              <ChevronRight size={13} className="disclosure-chevron" aria-hidden="true" />
              Advanced
            </button>
            {showAdvanced && (
              <div className="settings-group-rows">
                <div className="settings-row">
                  <div className="settings-row-text">
                    <span className="settings-row-label">DNS resolver</span>
                    <span className="settings-hint">{doh?.description}</span>
                  </div>
                  <select
                    className="field"
                    value={config.dohProvider}
                    aria-label="DNS resolver"
                    onChange={(e) => void onUpdateConfig({ dohProvider: e.target.value as DohProviderId })}
                  >
                    {(Object.keys(DOH_PROVIDERS) as DohProviderId[]).map((id) => (
                      <option key={id} value={id}>
                        {DOH_PROVIDERS[id].label}
                      </option>
                    ))}
                  </select>
                </div>
                <div className="settings-row">
                  <div className="settings-row-text">
                    <span className="settings-row-label">WebRTC leak protection</span>
                    <span className="settings-hint">Stops calls and pages from revealing your real IP address</span>
                  </div>
                  <Switch
                    checked={config.webrtcGuard}
                    label="WebRTC leak protection"
                    onChange={(webrtcGuard) => void onUpdateConfig({ webrtcGuard })}
                  />
                </div>
                <div className="settings-row">
                  <div className="settings-row-text">
                    <span className="settings-row-label">Kill switch</span>
                    <span className="settings-hint">
                      Blocks pages if the route drops, so nothing goes out unprotected
                      {status.killSwitchBlockedCount > 0 ? ` (${status.killSwitchBlockedCount} blocked)` : ''}
                    </span>
                  </div>
                  <Switch
                    checked={config.killSwitch}
                    label="Kill switch"
                    onChange={(killSwitch) => void onUpdateConfig({ killSwitch })}
                  />
                </div>
                <div className="kv-row">
                  <span className="kv-key">System VPN</span>
                  <span className="kv-val">
                    {status.detectedOsVpn
                      ? `${status.detectedOsVpn.name} (${status.detectedOsVpn.addresses[0] ?? ''})`
                      : 'None found'}
                  </span>
                </div>
                <div className="kv-row">
                  <span className="kv-key">DNS</span>
                  <span className="kv-val">{status.activeDnsUrl ?? 'Your network’s DNS'}</span>
                </div>
              </div>
            )}
          </section>
        </div>
      </div>
    </div>
  );
};

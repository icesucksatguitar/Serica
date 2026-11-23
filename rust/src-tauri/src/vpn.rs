//! Built-in privacy tunnel ("VPN").
//!
//! Every tab webview is created with `--proxy-server` pointing at a small
//! local proxy implemented here. The proxy routes each connection according to
//! the live VPN configuration, so switching modes never requires recreating
//! webviews (WebView2 fixes browser arguments per environment):
//!
//! * off / `system-vpn` - direct connection (the OS VPN adapter carries it)
//! * `doh-only`         - direct connection, hostnames resolved over DNS-over-HTTPS
//! * `tor-local`        - SOCKS5 to 127.0.0.1:9050 with remote DNS
//! * `custom-proxy`     - user SOCKS5 or HTTP proxy (optional auth)
//!
//! With the kill switch on, nothing falls back to a direct connection while
//! the tunnel is down. HTTPS is tunnelled with CONNECT, so TLS is end-to-end
//! between the page and the site; the proxy never sees plaintext.

use crate::model::now_ms;
use base64::Engine as _;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProxyConfig {
    #[serde(default = "default_protocol")]
    pub protocol: String,
    #[serde(default = "default_proxy_host")]
    pub host: String,
    #[serde(default = "default_proxy_port")]
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default = "default_bypass")]
    pub bypass_rules: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VpnConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_doh")]
    pub doh_provider: String,
    #[serde(default = "yes")]
    pub webrtc_guard: bool,
    #[serde(default)]
    pub kill_switch: bool,
    #[serde(default = "default_proxy")]
    pub proxy: ProxyConfig,
}

fn yes() -> bool {
    true
}
fn default_mode() -> String {
    "doh-only".into()
}
fn default_doh() -> String {
    "cloudflare".into()
}
fn default_protocol() -> String {
    "socks5".into()
}
fn default_proxy_host() -> String {
    "127.0.0.1".into()
}
fn default_proxy_port() -> u16 {
    1080
}
fn default_bypass() -> String {
    "localhost,127.0.0.1,<local>".into()
}
fn default_proxy() -> ProxyConfig {
    ProxyConfig {
        protocol: default_protocol(),
        host: default_proxy_host(),
        port: default_proxy_port(),
        username: String::new(),
        password: String::new(),
        bypass_rules: default_bypass(),
    }
}

impl Default for VpnConfig {
    fn default() -> Self {
        serde_json::from_value(serde_json::json!({})).expect("default vpn config")
    }
}

impl VpnConfig {
    fn normalize(&mut self) {
        if !matches!(self.mode.as_str(), "doh-only" | "system-vpn" | "tor-local" | "custom-proxy") {
            self.mode = default_mode();
        }
        if doh_url(&self.doh_provider).is_none() {
            self.doh_provider = default_doh();
        }
        if !matches!(self.proxy.protocol.as_str(), "socks5" | "http" | "https") {
            self.proxy.protocol = default_protocol();
        }
        let host = self.proxy.host.trim();
        let host = host.split("://").last().unwrap_or(host).split('/').next().unwrap_or("").trim();
        self.proxy.host = if host.is_empty() { default_proxy_host() } else { host.to_string() };
        if self.proxy.port == 0 {
            self.proxy.port = default_proxy_port();
        }
        if self.proxy.bypass_rules.trim().is_empty() {
            self.proxy.bypass_rules = default_bypass();
        }
    }

    /// Merges a partial UI patch (`Partial<VpnConfig>`, nested `proxy` allowed).
    pub fn patched(&self, patch: &serde_json::Value) -> Self {
        let mut base = serde_json::to_value(self).unwrap_or_default();
        if let (Some(b), Some(p)) = (base.as_object_mut(), patch.as_object()) {
            for (k, v) in p {
                if k == "proxy" {
                    if let (Some(bp), Some(pp)) = (b.get_mut("proxy").and_then(|x| x.as_object_mut()), v.as_object()) {
                        for (pk, pv) in pp {
                            bp.insert(pk.clone(), pv.clone());
                        }
                    }
                } else {
                    b.insert(k.clone(), v.clone());
                }
            }
        }
        let mut c: VpnConfig = serde_json::from_value(base).unwrap_or_else(|_| self.clone());
        c.normalize();
        c
    }
}

pub fn doh_url(provider: &str) -> Option<&'static str> {
    match provider {
        "cloudflare" => Some("https://cloudflare-dns.com/dns-query"),
        "quad9" => Some("https://dns.quad9.net/dns-query"),
        "mullvad" => Some("https://dns.mullvad.net/dns-query"),
        "adguard" => Some("https://dns.adguard-dns.com/dns-query"),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OsVpnAdapter {
    pub name: String,
    pub addresses: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IpCheck {
    pub ip: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colo: Option<String>,
    pub latency_ms: i64,
    pub checked_at: i64,
    pub via: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VpnStatus {
    pub state: String,
    pub config: VpnConfig,
    pub active_proxy_rule: Option<String>,
    pub active_dns_url: Option<String>,
    pub webrtc_protected: bool,
    pub kill_switch_blocking: bool,
    pub kill_switch_blocked_count: u64,
    pub detected_os_vpn: Option<OsVpnAdapter>,
    pub last_error: Option<String>,
    pub connected_since: Option<i64>,
    pub ip_check: Option<IpCheck>,
}

#[derive(Default)]
struct Runtime {
    state: String,
    last_error: Option<String>,
    connected_since: Option<i64>,
    ip_check: Option<IpCheck>,
    os_vpn: Option<OsVpnAdapter>,
}

pub struct Vpn {
    config: RwLock<VpnConfig>,
    rt: Mutex<Runtime>,
    blocked: AtomicU64,
    port: Mutex<u16>,
    path: PathBuf,
    dns_cache: Mutex<HashMap<String, (Vec<IpAddr>, Instant)>>,
    http: reqwest::Client,
}

enum Route {
    Direct { doh: Option<&'static str> },
    Socks5 { addr: String, user: String, pass: String },
    Http { addr: String, auth: Option<String> },
    Block(String),
}

impl Vpn {
    pub fn new(path: PathBuf) -> Arc<Self> {
        let mut config = crate::store::read_json(&path)
            .and_then(|v| serde_json::from_value::<VpnConfig>(v).ok())
            .unwrap_or_default();
        config.normalize();
        let enabled = config.enabled;
        let vpn = Arc::new(Self {
            config: RwLock::new(config),
            rt: Mutex::new(Runtime { state: "disconnected".into(), ..Default::default() }),
            blocked: AtomicU64::new(0),
            port: Mutex::new(0),
            path,
            dns_cache: Mutex::new(HashMap::new()),
            http: reqwest::Client::builder().timeout(Duration::from_secs(10)).build().expect("http client"),
        });
        if enabled {
            let v = vpn.clone();
            tauri::async_runtime::spawn(async move {
                v.connect(None).await;
            });
        }
        vpn
    }

    pub fn port(&self) -> u16 {
        *self.port.lock()
    }

    /// Binds the local proxy. Returns its port, or `None` if binding failed
    /// (tabs then connect directly and the VPN reports an error when enabled).
    pub fn start_proxy(self: &Arc<Self>) -> Option<u16> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").ok()?;
        listener.set_nonblocking(true).ok()?;
        let port = listener.local_addr().ok()?.port();
        *self.port.lock() = port;
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let Ok(listener) = tokio::net::TcpListener::from_std(listener) else { return };
            loop {
                let Ok((sock, _)) = listener.accept().await else { continue };
                let this = this.clone();
                tokio::spawn(async move {
                    let _ = sock.set_nodelay(true);
                    if let Err(e) = this.handle_client(sock).await {
                        tracing::debug!("proxy connection ended: {e}");
                    }
                });
            }
        });
        tracing::info!("local tunnel proxy on 127.0.0.1:{port}");
        Some(port)
    }

    pub fn status(&self) -> VpnStatus {
        let c = self.config.read().clone();
        let rt = self.rt.lock();
        let connected = c.enabled && rt.state == "connected";
        let blocking = c.enabled && c.kill_switch && rt.state != "connected";
        VpnStatus {
            state: if c.enabled { rt.state.clone() } else { "disconnected".into() },
            active_proxy_rule: if c.enabled { self.rule_label(&c) } else { None },
            active_dns_url: if c.enabled && c.mode == "doh-only" { doh_url(&c.doh_provider).map(str::to_string) } else { None },
            webrtc_protected: connected && c.webrtc_guard,
            kill_switch_blocking: blocking,
            kill_switch_blocked_count: self.blocked.load(Ordering::Relaxed),
            detected_os_vpn: rt.os_vpn.clone(),
            last_error: if c.enabled { rt.last_error.clone() } else { None },
            connected_since: if connected { rt.connected_since } else { None },
            ip_check: rt.ip_check.clone(),
            config: c,
        }
    }

    /// Whether pages must go through the local tunnel proxy. Off, or an OS VPN
    /// without the kill switch, means a direct connection, which is faster.
    pub fn needs_proxy(&self) -> bool {
        let c = self.config.read();
        c.enabled && (c.mode != "system-vpn" || c.kill_switch)
    }

    pub fn webrtc_guard_active(&self) -> bool {
        let c = self.config.read();
        c.enabled && c.webrtc_guard
    }

    fn rule_label(&self, c: &VpnConfig) -> Option<String> {
        match c.mode.as_str() {
            "tor-local" => Some("socks5://127.0.0.1:9050".into()),
            "custom-proxy" => Some(format!("{}://{}:{}", c.proxy.protocol, c.proxy.host, c.proxy.port)),
            "doh-only" => Some(format!("DNS over HTTPS ({})", c.doh_provider)),
            "system-vpn" => Some("system".into()),
            _ => None,
        }
    }

    fn save(&self) {
        let c = self.config.read().clone();
        if let Ok(b) = serde_json::to_vec_pretty(&c) {
            let _ = crate::store::write_atomic(&self.path, &b);
        }
    }

    pub async fn update_config(self: &Arc<Self>, patch: &serde_json::Value) -> VpnStatus {
        let next = self.config.read().patched(patch);
        let reconnect = next.enabled;
        *self.config.write() = next;
        self.dns_cache.lock().clear();
        self.save();
        if reconnect {
            self.connect(None).await
        } else {
            self.status()
        }
    }

    /// Enables the tunnel and verifies it is usable.
    pub async fn connect(self: &Arc<Self>, patch: Option<&serde_json::Value>) -> VpnStatus {
        {
            let mut c = self.config.write();
            if let Some(p) = patch {
                *c = c.patched(p);
            }
            c.enabled = true;
        }
        self.save();
        self.dns_cache.lock().clear();
        {
            let mut rt = self.rt.lock();
            rt.state = "connecting".into();
            rt.last_error = None;
            rt.ip_check = None;
        }
        let result = self.probe().await;
        let mut rt = self.rt.lock();
        match result {
            Ok(adapter) => {
                rt.state = "connected".into();
                rt.connected_since = Some(now_ms());
                rt.os_vpn = adapter;
            }
            Err(e) => {
                rt.state = "error".into();
                rt.last_error = Some(e);
                rt.connected_since = None;
            }
        }
        drop(rt);
        self.status()
    }

    pub fn disconnect(&self) -> VpnStatus {
        self.config.write().enabled = false;
        self.save();
        self.dns_cache.lock().clear();
        let mut rt = self.rt.lock();
        rt.state = "disconnected".into();
        rt.last_error = None;
        rt.connected_since = None;
        rt.ip_check = None;
        drop(rt);
        self.status()
    }

    /// Checks that the configured tunnel actually works.
    async fn probe(&self) -> Result<Option<OsVpnAdapter>, String> {
        let c = self.config.read().clone();
        match c.mode.as_str() {
            "doh-only" => {
                let url = doh_url(&c.doh_provider).ok_or("unknown DNS provider")?;
                let ips = doh_resolve(&self.http, url, "example.com").await?;
                if ips.is_empty() {
                    return Err("DNS-over-HTTPS server returned no answer".into());
                }
                Ok(None)
            }
            "system-vpn" => match detect_os_vpn() {
                Some(a) => Ok(Some(a)),
                None => Err("No active system VPN adapter was found. Connect your VPN app first.".into()),
            },
            "tor-local" => {
                let mut s = timeout_connect("127.0.0.1:9050").await.map_err(|_| "Tor is not running on 127.0.0.1:9050".to_string())?;
                socks5_handshake(&mut s, "example.com", 80, "", "").await.map_err(|e| format!("Tor SOCKS handshake failed: {e}"))?;
                Ok(None)
            }
            _ => {
                if c.proxy.protocol == "https" {
                    return Err("HTTPS (TLS) proxies are not supported; use an HTTP or SOCKS5 proxy".into());
                }
                let addr = format!("{}:{}", c.proxy.host, c.proxy.port);
                let mut s = timeout_connect(&addr).await.map_err(|e| format!("Proxy {addr} is unreachable: {e}"))?;
                if c.proxy.protocol == "socks5" {
                    socks5_handshake(&mut s, "example.com", 80, &c.proxy.username, &c.proxy.password)
                        .await
                        .map_err(|e| format!("SOCKS5 handshake failed: {e}"))?;
                }
                Ok(None)
            }
        }
    }

    /// Fetches the public IP as seen through the tunnel.
    pub async fn check_ip(&self) -> Result<IpCheck, String> {
        let port = self.port();
        let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(12));
        if port != 0 {
            builder = builder.proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}")).map_err(|e| e.to_string())?);
        }
        let client = builder.build().map_err(|e| e.to_string())?;
        let started = Instant::now();
        let text = client
            .get("https://www.cloudflare.com/cdn-cgi/trace")
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("IP check failed: {e}"))?
            .text()
            .await
            .map_err(|e| e.to_string())?;
        let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(&format!("{k}=")).map(str::to_string));
        let c = self.config.read().clone();
        let check = IpCheck {
            ip: field("ip").ok_or("unexpected IP check response")?,
            location: field("loc"),
            colo: field("colo"),
            latency_ms: started.elapsed().as_millis() as i64,
            checked_at: now_ms(),
            via: if c.enabled { self.rule_label(&c).unwrap_or_else(|| "direct".into()) } else { "direct".into() },
        };
        self.rt.lock().ip_check = Some(check.clone());
        Ok(check)
    }

    fn route_for(&self, host: &str) -> Route {
        let c = self.config.read().clone();
        if !c.enabled || bypassed(host, &c.proxy.bypass_rules) {
            return Route::Direct { doh: None };
        }
        let state = self.rt.lock().state.clone();
        if state != "connected" {
            if c.kill_switch {
                return Route::Block("VPN kill switch is active: the tunnel is down".into());
            }
            return Route::Direct { doh: None };
        }
        match c.mode.as_str() {
            "doh-only" => Route::Direct { doh: doh_url(&c.doh_provider) },
            "tor-local" => Route::Socks5 { addr: "127.0.0.1:9050".into(), user: String::new(), pass: String::new() },
            "custom-proxy" if c.proxy.protocol == "socks5" => Route::Socks5 {
                addr: format!("{}:{}", c.proxy.host, c.proxy.port),
                user: c.proxy.username.clone(),
                pass: c.proxy.password.clone(),
            },
            "custom-proxy" if c.proxy.protocol == "http" => Route::Http {
                addr: format!("{}:{}", c.proxy.host, c.proxy.port),
                auth: (!c.proxy.username.is_empty()).then(|| {
                    base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", c.proxy.username, c.proxy.password))
                }),
            },
            "custom-proxy" => Route::Block("unsupported proxy protocol".into()),
            _ => Route::Direct { doh: None },
        }
    }

    async fn resolve(&self, host: &str, port: u16, doh: Option<&'static str>) -> Result<Vec<SocketAddr>, String> {
        if let Ok(ip) = host.trim_matches(|c| c == '[' || c == ']').parse::<IpAddr>() {
            return Ok(vec![SocketAddr::new(ip, port)]);
        }
        let Some(doh) = doh else {
            return tokio::net::lookup_host((host, port)).await.map(|i| i.collect()).map_err(|e| e.to_string());
        };
        if let Some((ips, at)) = self.dns_cache.lock().get(host) {
            if at.elapsed() < Duration::from_secs(300) {
                return Ok(ips.iter().map(|ip| SocketAddr::new(*ip, port)).collect());
            }
        }
        let ips = doh_resolve(&self.http, doh, host).await?;
        if ips.is_empty() {
            return Err(format!("{host}: no DNS answer"));
        }
        self.dns_cache.lock().insert(host.to_string(), (ips.clone(), Instant::now()));
        Ok(ips.into_iter().map(|ip| SocketAddr::new(ip, port)).collect())
    }

    /// Opens a stream to host:port through the active route.
    async fn open(&self, host: &str, port: u16) -> Result<(TcpStream, bool), String> {
        match self.route_for(host) {
            Route::Block(reason) => {
                self.blocked.fetch_add(1, Ordering::Relaxed);
                Err(reason)
            }
            Route::Direct { doh } => {
                let addrs = self.resolve(host, port, doh).await?;
                let mut last = String::from("no address");
                for a in addrs {
                    match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(a)).await {
                        Ok(Ok(s)) => return Ok((s, false)),
                        Ok(Err(e)) => last = e.to_string(),
                        Err(_) => last = "timed out".into(),
                    }
                }
                Err(last)
            }
            Route::Socks5 { addr, user, pass } => {
                let mut s = timeout_connect(&addr).await?;
                socks5_handshake(&mut s, host, port, &user, &pass).await?;
                Ok((s, false))
            }
            Route::Http { addr, auth } => {
                let mut s = timeout_connect(&addr).await?;
                let mut req = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n");
                if let Some(a) = auth {
                    req.push_str(&format!("Proxy-Authorization: Basic {a}\r\n"));
                }
                req.push_str("\r\n");
                s.write_all(req.as_bytes()).await.map_err(|e| e.to_string())?;
                let (head, _) = read_head(&mut s).await?;
                let status = head.split_whitespace().nth(1).unwrap_or("");
                if status != "200" {
                    return Err(format!("upstream proxy refused CONNECT ({status})"));
                }
                Ok((s, false))
            }
        }
    }

    async fn handle_client(&self, mut client: TcpStream) -> Result<(), String> {
        let (head, rest) = read_head(&mut client).await?;
        let mut lines = head.split("\r\n");
        let request_line = lines.next().unwrap_or_default().to_string();
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let target = parts.next().unwrap_or("").to_string();
        let version = parts.next().unwrap_or("HTTP/1.1").to_string();

        if method.eq_ignore_ascii_case("CONNECT") {
            let (host, port) = split_host_port(&target, 443);
            match self.open(&host, port).await {
                Ok((mut upstream, _)) => {
                    client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.map_err(|e| e.to_string())?;
                    if !rest.is_empty() {
                        upstream.write_all(&rest).await.map_err(|e| e.to_string())?;
                    }
                    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
                    Ok(())
                }
                Err(e) => {
                    let _ = client.write_all(format!("HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nX-Serica-Error: {}\r\n\r\n", e.replace(['\r', '\n'], " ")).as_bytes()).await;
                    Err(e)
                }
            }
        } else {
            // Plain HTTP in absolute form: http://host[:port]/path
            let Ok(url) = url::Url::parse(&target) else {
                let _ = client.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n").await;
                return Err("bad request target".into());
            };
            let host = url.host_str().unwrap_or("").to_string();
            let port = url.port_or_known_default().unwrap_or(80);
            let (mut upstream, _) = match self.open(&host, port).await {
                Ok(s) => s,
                Err(e) => {
                    let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n").await;
                    return Err(e);
                }
            };
            let path = match url.query() {
                Some(q) => format!("{}?{}", url.path(), q),
                None => url.path().to_string(),
            };
            let mut out = format!("{method} {path} {version}\r\n");
            for l in lines.filter(|l| !l.is_empty()) {
                let lower = l.to_ascii_lowercase();
                if lower.starts_with("proxy-connection:") || lower.starts_with("proxy-authorization:") || lower.starts_with("connection:") {
                    continue;
                }
                out.push_str(l);
                out.push_str("\r\n");
            }
            // One request per connection: the browser may reuse a proxy
            // connection for different hosts, which a single upstream can't serve.
            out.push_str("Connection: close\r\n\r\n");
            upstream.write_all(out.as_bytes()).await.map_err(|e| e.to_string())?;
            if !rest.is_empty() {
                upstream.write_all(&rest).await.map_err(|e| e.to_string())?;
            }
            let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
            Ok(())
        }
    }
}

fn bypassed(host: &str, rules: &str) -> bool {
    let h = host.trim_matches(|c| c == '[' || c == ']').to_lowercase();
    if h == "localhost" || h == "127.0.0.1" || h == "::1" {
        return true;
    }
    rules.split([',', ';']).map(|r| r.trim().to_lowercase()).filter(|r| !r.is_empty()).any(|r| {
        if r == "<local>" {
            !h.contains('.') && !h.contains(':')
        } else if let Some(suffix) = r.strip_prefix("*.") {
            h.ends_with(&format!(".{suffix}"))
        } else {
            h == r || h.ends_with(&format!(".{}", r.trim_start_matches('.')))
        }
    })
}

fn split_host_port(target: &str, default_port: u16) -> (String, u16) {
    if let Some(stripped) = target.strip_prefix('[') {
        if let Some((h, rest)) = stripped.split_once(']') {
            let port = rest.trim_start_matches(':').parse().unwrap_or(default_port);
            return (h.to_string(), port);
        }
    }
    match target.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => (h.to_string(), p.parse().unwrap_or(default_port)),
        _ => (target.to_string(), default_port),
    }
}

async fn timeout_connect(addr: &str) -> Result<TcpStream, String> {
    match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
        Ok(Ok(s)) => Ok(s),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("connection timed out".into()),
    }
}

/// Reads an HTTP head (up to the blank line). Returns it plus any bytes read past it.
async fn read_head(s: &mut TcpStream) -> Result<(String, Vec<u8>), String> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 4096];
    loop {
        let n = s.read(&mut chunk).await.map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("connection closed before request head".into());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..pos]).to_string();
            return Ok((head, buf[pos + 4..].to_vec()));
        }
        if buf.len() > 64 * 1024 {
            return Err("request head too large".into());
        }
    }
}

/// SOCKS5 CONNECT with domain-name addressing (remote DNS, no local leak).
async fn socks5_handshake(s: &mut TcpStream, host: &str, port: u16, user: &str, pass: &str) -> Result<(), String> {
    let e = |x: std::io::Error| x.to_string();
    let with_auth = !user.is_empty();
    if with_auth {
        s.write_all(&[5, 2, 0, 2]).await.map_err(e)?;
    } else {
        s.write_all(&[5, 1, 0]).await.map_err(e)?;
    }
    let mut resp = [0u8; 2];
    s.read_exact(&mut resp).await.map_err(e)?;
    if resp[0] != 5 {
        return Err("not a SOCKS5 server".into());
    }
    match resp[1] {
        0 => {}
        2 if with_auth => {
            let mut msg = vec![1, user.len().min(255) as u8];
            msg.extend_from_slice(&user.as_bytes()[..user.len().min(255)]);
            msg.push(pass.len().min(255) as u8);
            msg.extend_from_slice(&pass.as_bytes()[..pass.len().min(255)]);
            s.write_all(&msg).await.map_err(e)?;
            let mut r = [0u8; 2];
            s.read_exact(&mut r).await.map_err(e)?;
            if r[1] != 0 {
                return Err("SOCKS5 authentication failed".into());
            }
        }
        _ => return Err("SOCKS5 server rejected the authentication method".into()),
    }
    let host = host.trim_matches(|c| c == '[' || c == ']');
    let mut req = vec![5, 1, 0];
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            req.push(1);
            req.extend_from_slice(&ip.octets());
        }
        Ok(IpAddr::V6(ip)) => {
            req.push(4);
            req.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            req.push(3);
            req.push(host.len().min(255) as u8);
            req.extend_from_slice(&host.as_bytes()[..host.len().min(255)]);
        }
    }
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req).await.map_err(e)?;
    let mut head = [0u8; 4];
    s.read_exact(&mut head).await.map_err(e)?;
    if head[1] != 0 {
        return Err(format!("SOCKS5 connect failed (code {})", head[1]));
    }
    let skip = match head[3] {
        1 => 4,
        4 => 16,
        3 => {
            let mut l = [0u8; 1];
            s.read_exact(&mut l).await.map_err(e)?;
            l[0] as usize
        }
        _ => return Err("bad SOCKS5 reply".into()),
    };
    let mut rest = vec![0u8; skip + 2];
    s.read_exact(&mut rest).await.map_err(e)?;
    Ok(())
}

/// RFC 8484 DNS-over-HTTPS lookup (wire format, works with every provider).
pub async fn doh_resolve(client: &reqwest::Client, server: &str, host: &str) -> Result<Vec<IpAddr>, String> {
    let mut out = Vec::new();
    for qtype in [1u16, 28u16] {
        let q = build_dns_query(host, qtype)?;
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&q);
        let resp = client
            .get(format!("{server}?dns={encoded}"))
            .header("accept", "application/dns-message")
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("DNS-over-HTTPS request failed: {e}"))?;
        let body = resp.bytes().await.map_err(|e| e.to_string())?;
        out.extend(parse_dns_answers(&body, qtype));
        if !out.is_empty() {
            break;
        }
    }
    Ok(out)
}

fn build_dns_query(host: &str, qtype: u16) -> Result<Vec<u8>, String> {
    let mut q = vec![0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in host.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(format!("invalid hostname {host}"));
        }
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0);
    q.extend_from_slice(&qtype.to_be_bytes());
    q.extend_from_slice(&[0, 1]);
    Ok(q)
}

fn skip_name(b: &[u8], mut i: usize) -> Option<usize> {
    loop {
        let len = *b.get(i)? as usize;
        if len == 0 {
            return Some(i + 1);
        }
        if len & 0xC0 == 0xC0 {
            return Some(i + 2);
        }
        i += len + 1;
    }
}

fn parse_dns_answers(b: &[u8], qtype: u16) -> Vec<IpAddr> {
    let mut out = Vec::new();
    if b.len() < 12 {
        return out;
    }
    let qd = u16::from_be_bytes([b[4], b[5]]) as usize;
    let an = u16::from_be_bytes([b[6], b[7]]) as usize;
    let mut i = 12;
    for _ in 0..qd {
        match skip_name(b, i) {
            Some(n) => i = n + 4,
            None => return out,
        }
    }
    for _ in 0..an {
        let Some(n) = skip_name(b, i) else { break };
        i = n;
        if i + 10 > b.len() {
            break;
        }
        let t = u16::from_be_bytes([b[i], b[i + 1]]);
        let len = u16::from_be_bytes([b[i + 8], b[i + 9]]) as usize;
        i += 10;
        if i + len > b.len() {
            break;
        }
        if t == qtype && t == 1 && len == 4 {
            out.push(IpAddr::from([b[i], b[i + 1], b[i + 2], b[i + 3]]));
        } else if t == qtype && t == 28 && len == 16 {
            let mut a = [0u8; 16];
            a.copy_from_slice(&b[i..i + 16]);
            out.push(IpAddr::from(a));
        }
        i += len;
    }
    out
}

/// Finds an up, non-loopback adapter that looks like a VPN tunnel.
#[cfg(windows)]
pub fn detect_os_vpn() -> Option<OsVpnAdapter> {
    use windows::Win32::NetworkManagement::IpHelper::*;
    use windows::Win32::Networking::WinSock::AF_UNSPEC;
    const HINTS: &[&str] = &["vpn", "wireguard", "wintun", "tap-", "tap ", "tun", "openvpn", "nordlynx", "proton", "mullvad", "tailscale", "zerotier", "cisco anyconnect", "fortinet", "globalprotect", "expressvpn", "surfshark", "pangp"];
    unsafe {
        let mut size: u32 = 32 * 1024;
        let mut buf: Vec<u8> = vec![0; size as usize];
        let mut rc = GetAdaptersAddresses(AF_UNSPEC.0 as u32, GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST, None, Some(buf.as_mut_ptr() as *mut _), &mut size);
        if rc == 111 {
            buf = vec![0; size as usize];
            rc = GetAdaptersAddresses(AF_UNSPEC.0 as u32, GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST, None, Some(buf.as_mut_ptr() as *mut _), &mut size);
        }
        if rc != 0 {
            return None;
        }
        let mut p = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !p.is_null() {
            let a = &*p;
            let up = a.OperStatus == windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
            let desc = a.Description.to_string().unwrap_or_default();
            let name = a.FriendlyName.to_string().unwrap_or_default();
            let hay = format!("{} {}", desc, name).to_lowercase();
            if up && a.IfType != 24 && HINTS.iter().any(|h| hay.contains(h)) {
                let mut addresses = Vec::new();
                let mut u = a.FirstUnicastAddress;
                while !u.is_null() {
                    let sa = (*u).Address.lpSockaddr;
                    if !sa.is_null() {
                        let fam = (*sa).sa_family.0;
                        if fam == 2 {
                            let v4 = &*(sa as *const windows::Win32::Networking::WinSock::SOCKADDR_IN);
                            addresses.push(std::net::Ipv4Addr::from(u32::from_be(v4.sin_addr.S_un.S_addr)).to_string());
                        }
                    }
                    u = (*u).Next;
                }
                return Some(OsVpnAdapter { name: if name.is_empty() { desc } else { name }, addresses });
            }
            p = a.Next;
        }
    }
    None
}

#[cfg(not(windows))]
pub fn detect_os_vpn() -> Option<OsVpnAdapter> {
    None
}

/// Page script that keeps WebRTC from exposing the real IP while the tunnel is on.
pub const WEBRTC_GUARD_JS: &str = r#"(() => {
  try {
    const Orig = window.RTCPeerConnection;
    if (!Orig || Orig.__sericaGuard) return;
    const Guarded = function (config, ...rest) {
      const c = Object.assign({}, config || {}, { iceTransportPolicy: 'relay' });
      return new Orig(c, ...rest);
    };
    Guarded.prototype = Orig.prototype;
    Guarded.__sericaGuard = true;
    Object.setPrototypeOf(Guarded, Orig);
    window.RTCPeerConnection = Guarded;
    if (window.webkitRTCPeerConnection) window.webkitRTCPeerConnection = Guarded;
  } catch (e) {}
})();"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dns_wire_round_trip() {
        let q = build_dns_query("example.com", 1).unwrap();
        // Fake response: question + one A answer using a compression pointer.
        let mut r = q.clone();
        r[2] = 0x81;
        r[3] = 0x80;
        r[7] = 1;
        r.extend_from_slice(&[0xC0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 93, 184, 216, 34]);
        assert_eq!(parse_dns_answers(&r, 1), vec![IpAddr::from([93, 184, 216, 34])]);
    }

    #[test]
    fn bypass_rules() {
        let r = "localhost,127.0.0.1,<local>,*.corp.example,intranet.net";
        assert!(bypassed("printer", r));
        assert!(bypassed("a.corp.example", r));
        assert!(bypassed("x.intranet.net", r));
        assert!(!bypassed("example.com", r));
    }

    #[test]
    fn host_port_parsing() {
        assert_eq!(split_host_port("example.com:8443", 443), ("example.com".into(), 8443));
        assert_eq!(split_host_port("[::1]:443", 1), ("::1".into(), 443));
        assert_eq!(split_host_port("example.com", 443), ("example.com".into(), 443));
    }

    #[test]
    fn config_patch_merges_nested_proxy() {
        let c = VpnConfig::default();
        let n = c.patched(&serde_json::json!({ "mode": "custom-proxy", "proxy": { "port": 9999, "host": "socks5://10.0.0.2/" } }));
        assert_eq!(n.mode, "custom-proxy");
        assert_eq!(n.proxy.port, 9999);
        assert_eq!(n.proxy.host, "10.0.0.2");
        assert_eq!(n.proxy.protocol, "socks5");
        let bad = c.patched(&serde_json::json!({ "mode": "nonsense" }));
        assert_eq!(bad.mode, "doh-only");
    }

    #[tokio::test]
    async fn proxy_tunnels_connect_to_local_server() {
        // Echo-ish origin server.
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_port = origin.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = origin.accept().await.unwrap();
            let mut b = [0u8; 5];
            s.read_exact(&mut b).await.unwrap();
            s.write_all(b"world").await.unwrap();
        });
        let vpn = Arc::new(Vpn {
            config: RwLock::new(VpnConfig::default()),
            rt: Mutex::new(Runtime { state: "disconnected".into(), ..Default::default() }),
            blocked: AtomicU64::new(0),
            port: Mutex::new(0),
            path: std::env::temp_dir().join("vpn-test.json"),
            dns_cache: Mutex::new(HashMap::new()),
            http: reqwest::Client::new(),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let v = vpn.clone();
        tokio::spawn(async move {
            let (s, _) = listener.accept().await.unwrap();
            let _ = v.handle_client(s).await;
        });
        let mut c = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        c.write_all(format!("CONNECT 127.0.0.1:{origin_port} HTTP/1.1\r\nHost: x\r\n\r\nhello").as_bytes()).await.unwrap();
        let mut resp = vec![0u8; 39];
        c.read_exact(&mut resp).await.unwrap();
        assert!(String::from_utf8_lossy(&resp).starts_with("HTTP/1.1 200"));
        let mut w = [0u8; 5];
        c.read_exact(&mut w).await.unwrap();
        assert_eq!(&w, b"world");
    }

    #[test]
    fn kill_switch_blocks_when_tunnel_down() {
        let vpn = Vpn {
            config: RwLock::new(VpnConfig { enabled: true, kill_switch: true, mode: "tor-local".into(), ..Default::default() }),
            rt: Mutex::new(Runtime { state: "error".into(), ..Default::default() }),
            blocked: AtomicU64::new(0),
            port: Mutex::new(0),
            path: std::env::temp_dir().join("vpn-test2.json"),
            dns_cache: Mutex::new(HashMap::new()),
            http: reqwest::Client::new(),
        };
        assert!(matches!(vpn.route_for("example.com"), Route::Block(_)));
        assert!(matches!(vpn.route_for("localhost"), Route::Direct { .. }));
    }
}

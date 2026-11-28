//! Scripts Serica runs in tab pages on macOS (WebKit).
//!
//! WebView2's `window.chrome.webview.postMessage` becomes
//! `window.webkit.messageHandlers.serica.postMessage`. Serica's own scripts
//! run in an isolated content world ("serica"): pages can't see them, and the
//! `serica` message handler exists only in that world, so a web page has no
//! way to post Serica messages. Scripts that must patch the page's own
//! JavaScript (adblock scriptlets, the WebRTC guard) run in the page world.

/// Shared by every Serica-world script: posts a message to the backend.
/// The handler replies (a Promise); a failed reply is ignored.
const POST: &str = r#"const post = (m) => { try { const p = window.webkit.messageHandlers.serica.postMessage(Object.assign({ __serica: 1 }, m)); if (p && p.catch) p.catch(() => {}); return p; } catch (e) { return Promise.resolve(null); } };"#;

/// Top frame of every tab (Serica world). Scrolling settled (the UI refreshes
/// its capture of the page), and the CSS classes/ids the page uses (Brave's
/// generic cosmetic filtering).
pub fn page_script() -> String {
    format!(
        r#"(() => {{
  if (window.top !== window || window.__sericaPage) return;
  Object.defineProperty(window, '__sericaPage', {{ value: true }});
  {POST}
  let settle = 0;
  addEventListener('scroll', () => {{
    clearTimeout(settle);
    settle = setTimeout(() => {{ if (document.visibilityState === 'visible') post({{ t: 'settled' }}); }}, 300);
  }}, {{ passive: true, capture: true }});
  // Keyboard focus in the page (a click into one pane of a split view makes
  // its tab the active one). Blur only once focus has left the document:
  // focus moving into one of its frames is still the page.
  addEventListener('focus', (e) => {{ if (e.target === window) post({{ t: 'focus' }}); }});
  addEventListener('blur', (e) => {{
    if (e.target !== window) return;
    setTimeout(() => {{ if (!document.hasFocus()) post({{ t: 'blur' }}); }}, 0);
  }});
  const seenC = new Set(), seenI = new Set();
  let pendC = [], pendI = [], timer = 0;
  const flush = () => {{ timer = 0; if (pendC.length || pendI.length) {{ post({{ t: 'cosmetic', classes: pendC, ids: pendI }}); pendC = []; pendI = []; }} }};
  const visit = (el) => {{
    if (seenC.size > 6000) return;
    const cl = el.classList;
    if (cl) for (const c of cl) if (!seenC.has(c)) {{ seenC.add(c); pendC.push(c); }}
    const id = el.id;
    if (id && typeof id === 'string' && !seenI.has(id)) {{ seenI.add(id); pendI.push(id); }}
  }};
  const scan = (root) => {{
    if (root.nodeType !== 1) return;
    visit(root);
    const els = root.querySelectorAll('[class],[id]');
    for (let i = 0; i < els.length && i < 4000; i++) visit(els[i]);
    if (!timer && (pendC.length || pendI.length)) timer = setTimeout(flush, 120);
  }};
  const start = () => {{
    scan(document.documentElement);
    new MutationObserver((muts) => {{ for (const m of muts) for (const n of m.addedNodes) scan(n); }})
      .observe(document.documentElement, {{ childList: true, subtree: true }});
  }};
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start, {{ once: true }});
  else start();
  // The page's icon (WebView2 reported it; WebKit leaves it to the browser):
  // the declared icon closest to 32px, SVG first, else /favicon.ico.
  let lastIcon = '';
  const pickIcon = () => {{
    if (!/^https?:$/.test(location.protocol)) return;
    let best = null, bestScore = -1;
    for (const l of document.querySelectorAll('link[rel][href]')) {{
      const rel = (l.getAttribute('rel') || '').toLowerCase().split(/\s+/);
      const touch = rel.includes('apple-touch-icon') || rel.includes('apple-touch-icon-precomposed');
      if (!rel.includes('icon') && !touch) continue;
      const href = l.href;
      if (!/^(https?:|data:image\/)/.test(href) || href.length > 200000) continue;
      let score = touch ? 10 : 50;
      if ((l.type || '').includes('svg') || /\.svg(\?|$)/i.test(href)) score = touch ? 15 : 100;
      else {{
        const m = /(\d+)x\d+/.exec(l.getAttribute('sizes') || '');
        if (m) score += Math.max(0, 40 - Math.abs(parseInt(m[1], 10) - 32) / 4);
      }}
      if (score > bestScore) {{ best = href; bestScore = score; }}
    }}
    const icon = best || (location.origin + '/favicon.ico');
    if (icon !== lastIcon) {{ lastIcon = icon; post({{ t: 'favicon', href: icon }}); }}
  }};
  const watchIcon = () => {{
    pickIcon();
    if (document.head) new MutationObserver(pickIcon).observe(document.head, {{ childList: true, subtree: true, attributes: true, attributeFilter: ['href', 'rel', 'sizes'] }});
  }};
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', watchIcon, {{ once: true }});
  else watchIcon();
}})();"#
    )
}

/// Every frame of every tab (Serica world):
/// - media: whether this frame plays something, so the mini player can reach
///   players embedded from another site (WebView2 used DevTools sessions);
/// - right-click: what is under the pointer (link, image, selection,
///   editable), read just before WebKit builds its context menu.
pub fn frame_script() -> String {
    format!(
        r#"(() => {{
  if (window.__sericaFrame) return;
  Object.defineProperty(window, '__sericaFrame', {{ value: true }});
  {POST}
  const top = window.top === window;
  let playing = false, timer = 0, announced = false;
  const media = () => {{
    const quick = document.querySelectorAll('video, audio');
    return quick.length ? Array.from(quick) : [];
  }};
  const check = () => {{
    timer = 0;
    const now = media().some((m) => !m.paused && !m.ended && m.readyState > 1);
    if (now && !announced && !top) {{ announced = true; post({{ t: 'media-frame' }}); }}
    if (now !== playing) {{ playing = now; post({{ t: 'media', playing: now, top }}); }}
  }};
  const soon = () => {{ if (!timer) timer = setTimeout(check, 150); }};
  for (const ev of ['play', 'playing', 'pause', 'ended', 'emptied', 'abort', 'volumechange']) document.addEventListener(ev, soon, true);
  addEventListener('pagehide', () => {{ if (playing) {{ playing = false; post({{ t: 'media', playing: false, top }}); }} }});
  addEventListener('contextmenu', (e) => {{
    try {{
      const path = e.composedPath ? e.composedPath() : [e.target];
      let el = path[0];
      if (el && el.nodeType !== 1) el = el.parentElement;
      const up = (sel) => {{ for (const n of path) {{ if (n && n.nodeType === 1 && n.matches && n.matches(sel)) return n; }} return null; }};
      const a = up('a[href], area[href]');
      const img = up('img');
      const av = up('video, audio');
      const field = up('input, textarea, [contenteditable]');
      const editable = !!(field && (field.isContentEditable || field.tagName === 'TEXTAREA' || (field.tagName === 'INPUT' && !/^(button|submit|reset|checkbox|radio|file|image|range|color|hidden)$/i.test(field.type || ''))));
      let selection = '';
      try {{ selection = String(window.getSelection ? window.getSelection() : ''); }} catch (x) {{}}
      const href = a ? (typeof a.href === 'string' ? a.href : (a.href && a.href.baseVal) || null) : null;
      post({{ t: 'ctx', link: href, image: img ? (img.currentSrc || img.src || null) : null, media: av ? (av.currentSrc || av.src || null) : null,
        selection: selection.replace(/\s+/g, ' ').trim().slice(0, 2000) || null, editable, page: location.href, top, x: e.clientX, y: e.clientY }});
    }} catch (x) {{}}
  }}, true);
}})();"#
    )
}

/// Find in page with every match highlighted (Serica world, top frame).
/// WebKit's `findString` shows one match and no count, so this walks the
/// page's text (shadow roots and same-origin frames included), highlights all
/// matches with the CSS Custom Highlight API (Safari 17.2+; the active match
/// alone through the selection before that) and returns `{ active, total }`.
pub const FIND_SCRIPT: &str = r#"(() => {
  if (window.top !== window || window.__sericaFind) return;
  const MAX = 2000;
  const HL = typeof CSS !== 'undefined' && CSS.highlights && typeof Highlight === 'function';
  let ranges = [], active = -1, query = '';
  const SKIP = /^(SCRIPT|STYLE|NOSCRIPT|TEMPLATE|TEXTAREA|INPUT|SELECT|OPTION|SVG|HEAD|TITLE)$/;
  const BLOCK = /^(block|flex|grid|list-item|table|table-row|table-cell|flow-root|table-caption)$/;
  const styled = new Set();
  const ensureStyle = (doc) => {
    if (!HL || styled.has(doc)) return;
    styled.add(doc);
    try {
      const s = doc.createElement('style');
      s.setAttribute('data-serica-find', '');
      s.textContent = '::highlight(serica-find){background-color:rgba(255,212,59,.75);color:inherit}::highlight(serica-find-active){background-color:#ff9632;color:#000}';
      (doc.head || doc.documentElement).appendChild(s);
    } catch (e) {}
  };
  const roots = () => {
    const out = [document];
    const walk = (doc, depth) => {
      if (depth > 3) return;
      let frames = [];
      try { frames = doc.querySelectorAll('iframe, frame'); } catch (e) {}
      for (const f of frames) { try { const d = f.contentDocument; if (d && d.documentElement) { out.push(d); walk(d, depth + 1); } } catch (e) {} }
    };
    walk(document, 0);
    return out;
  };
  const collect = (doc) => {
    // Text nodes in document order (into open shadow roots), with a break
    // between blocks so a match never spans two paragraphs.
    const nodes = [];
    const blockOf = new Map();
    const block = (el) => {
      for (let n = el; n; n = n.parentElement || (n.getRootNode && n.getRootNode().host) || null) {
        if (blockOf.has(n)) return blockOf.get(n);
        let b = false;
        try { b = BLOCK.test(getComputedStyle(n).display); } catch (e) {}
        if (b) { blockOf.set(n, n); return n; }
      }
      return null;
    };
    const visible = (el) => {
      try { return el.checkVisibility ? el.checkVisibility({ visibilityProperty: true }) : el.getClientRects().length > 0; } catch (e) { return true; }
    };
    const walk = (root) => {
      const tw = doc.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
        acceptNode: (n) => n.nodeType === 1 ? (SKIP.test(n.tagName.toUpperCase()) ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_SKIP) : NodeFilter.FILTER_ACCEPT,
      });
      // Shadow roots are not reached by the walker; visit hosts' roots too.
      const hosts = [];
      try { for (const el of root.querySelectorAll('*')) if (el.shadowRoot) hosts.push(el); } catch (e) {}
      for (let n = tw.nextNode(); n; n = tw.nextNode()) if (n.nodeValue && n.nodeValue.trim() && visible(n.parentElement)) nodes.push(n);
      for (const h of hosts) walk(h.shadowRoot);
    };
    if (doc.body) walk(doc.body);
    let text = '';
    const map = [];
    let last = null;
    for (const n of nodes) {
      const b = block(n.parentElement);
      if (last !== null && b !== last) text += '\n';
      last = b;
      const raw = n.nodeValue;
      const low = raw.toLowerCase();
      map.push({ n, at: text.length, len: raw.length });
      text += low.length === raw.length ? low : raw;
    }
    return { text, map };
  };
  const locate = (map, pos) => {
    let lo = 0, hi = map.length - 1;
    while (lo < hi) { const mid = (lo + hi + 1) >> 1; if (map[mid].at <= pos) lo = mid; else hi = mid - 1; }
    const m = map[lo];
    return m ? [m.n, Math.min(pos - m.at, m.len)] : null;
  };
  const paint = () => {
    if (HL) {
      CSS.highlights.delete('serica-find');
      CSS.highlights.delete('serica-find-active');
      if (ranges.length) {
        CSS.highlights.set('serica-find', new Highlight(...ranges.filter((_, i) => i !== active)));
        if (active >= 0) CSS.highlights.set('serica-find-active', new Highlight(ranges[active]));
      }
      // Highlights live per document: same-origin frames get their own.
      for (const d of roots()) {
        if (d === document) continue;
        try {
          const own = ranges.filter((r) => r.startContainer.ownerDocument === d);
          const H = d.defaultView && d.defaultView.Highlight;
          const reg = d.defaultView && d.defaultView.CSS && d.defaultView.CSS.highlights;
          if (!H || !reg) continue;
          reg.delete('serica-find'); reg.delete('serica-find-active');
          const act = active >= 0 && ranges[active].startContainer.ownerDocument === d ? ranges[active] : null;
          const rest = own.filter((r) => r !== act);
          if (rest.length) reg.set('serica-find', new H(...rest));
          if (act) reg.set('serica-find-active', new H(act));
        } catch (e) {}
      }
    }
    if (active >= 0) {
      const r = ranges[active];
      if (!HL) { try { const s = r.startContainer.ownerDocument.getSelection(); s.removeAllRanges(); s.addRange(r); } catch (e) {} }
      try {
        const rect = r.getBoundingClientRect();
        const win = r.startContainer.ownerDocument.defaultView || window;
        const vh = win.innerHeight, vw = win.innerWidth;
        if (rect.top < 0 || rect.bottom > vh || rect.left < 0 || rect.right > vw || win !== window) {
          const el = r.startContainer.parentElement;
          if (el) el.scrollIntoView({ block: 'center', inline: 'nearest' });
        }
      } catch (e) {}
    }
  };
  const result = () => ({ active: ranges.length ? active + 1 : 0, total: ranges.length });
  const clear = () => {
    if (HL) { CSS.highlights.delete('serica-find'); CSS.highlights.delete('serica-find-active'); }
    for (const d of roots()) { try { const reg = d.defaultView.CSS.highlights; reg.delete('serica-find'); reg.delete('serica-find-active'); } catch (e) {} }
    if (!HL && ranges.length) { try { getSelection().removeAllRanges(); } catch (e) {} }
    ranges = []; active = -1; query = '';
  };
  const find = (q) => {
    const keep = active >= 0 && ranges[active] ? ranges[active].cloneRange() : null;
    clear();
    query = String(q || '');
    const needle = query.toLowerCase();
    if (!needle) return result();
    for (const d of roots()) {
      ensureStyle(d);
      const { text, map } = collect(d);
      let from = 0;
      while (ranges.length < MAX) {
        const i = text.indexOf(needle, from);
        if (i < 0) break;
        const s = locate(map, i), e = locate(map, i + needle.length - 1);
        if (s && e) {
          try { const r = d.createRange(); r.setStart(s[0], s[1]); r.setEnd(e[0], Math.min(e[1] + 1, e[0].nodeValue.length)); ranges.push(r); } catch (x) {}
        }
        from = i + Math.max(1, needle.length);
      }
    }
    // Stay near the match shown before (typing more letters), else the first
    // one below the top of the screen.
    active = ranges.length ? 0 : -1;
    if (ranges.length) {
      if (keep) {
        const i = ranges.findIndex((r) => { try { return r.compareBoundaryPoints(Range.START_TO_START, keep) >= 0; } catch (e) { return false; } });
        active = i >= 0 ? i : 0;
      } else {
        const i = ranges.findIndex((r) => { try { return r.getBoundingClientRect().bottom >= 0; } catch (e) { return false; } });
        active = i >= 0 ? i : 0;
      }
    }
    paint();
    return result();
  };
  const next = (forward) => {
    if (!ranges.length) return result();
    active = forward ? (active + 1) % ranges.length : (active - 1 + ranges.length) % ranges.length;
    paint();
    return result();
  };
  Object.defineProperty(window, '__sericaFind', { value: Object.freeze({ find, next, clear, query: () => query }) });
})();"#;

/// Dark PDFs (Serica world, top frame). WebKit shows PDFs with its own
/// viewer in a plugin document; inverting the document's lightness (and
/// turning hues back) gives dark pages with readable text, and figures keep
/// their colors' hue. `pdf_dark_js(on)` switches it.
pub const PDF_DARK_SCRIPT: &str = r#"(() => {
  if (window.top !== window || window.__sericaPdfDark) return;
  let on = false;
  const apply = () => {
    if (document.contentType !== 'application/pdf') return;
    const root = document.documentElement;
    if (!root) { document.addEventListener('DOMContentLoaded', apply, { once: true }); return; }
    let st = document.getElementById('serica-pdf-dark');
    if (!on) { if (st) st.remove(); return; }
    if (st) return;
    st = document.createElement('style');
    st.id = 'serica-pdf-dark';
    st.textContent = 'html{filter:invert(0.9) hue-rotate(180deg) contrast(0.92) brightness(1.04)!important;background:#ececec!important}embed,object{background:transparent!important}';
    (document.head || root).appendChild(st);
  };
  Object.defineProperty(window, '__sericaPdfDark', { value: (v) => { on = !!v; apply(); } });
  Object.defineProperty(window, '__sericaPdfDarkOn', { get: () => on });
})();"#;

pub fn pdf_dark_js(on: bool) -> String {
    format!("window.__sericaPdfDark && window.__sericaPdfDark({on})")
}

/// Reads the page's `theme-color` (WebKit also reports it natively; this is
/// the fallback for pages that set it after load).
pub const THEME_COLOR_JS: &str = r#"(() => { try { const ms = Array.from(document.querySelectorAll('meta[name="theme-color" i]')); const m = ms.find((el) => !el.media || matchMedia(el.media).matches); const v = m && m.getAttribute('content'); return typeof v === 'string' ? v.trim() : null; } catch (e) { return null; } })()"#;

/// Serica's source viewer for `view-source:` (WebView2 has its own; WebKit
/// has none). Loaded in place as the page's own URL, so it reads the source
/// the way the page was loaded (same origin, with its cookies), then shows it
/// as numbered lines with tags, attributes, strings and comments tinted.
pub fn view_source_html(url: &str) -> String {
    let data = serde_json::json!({ "url": url });
    VIEW_SOURCE_HTML.replace("__DATA__", &data.to_string().replace("</", "<\\/"))
}

const VIEW_SOURCE_HTML: &str = r#"<!doctype html><html><head><meta charset="utf-8"><meta name="color-scheme" content="light dark"><title></title><style>
  :root { --bg: #ffffff; --fg: #1d1d1f; --muted: #8e8e93; --bar: #f5f5f7; --line: #e5e5ea; --tag: #8f2bb8; --attr: #b35f00; --str: #1a6fd1; --cmt: #6e7781; }
  @media (prefers-color-scheme: dark) { :root { --bg: #1c1c1e; --fg: #e5e5ea; --muted: #8e8e93; --bar: #232326; --line: #333338; --tag: #d17cf0; --attr: #f0a35b; --str: #7cb7ff; --cmt: #8b949e; } }
  html, body { margin: 0; background: var(--bg); color: var(--fg); }
  .bar { position: sticky; top: 0; display: flex; align-items: center; gap: 14px; padding: 7px 14px; background: var(--bar); border-bottom: 1px solid var(--line); font: 12px -apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif; color: var(--muted); }
  .bar label { display: flex; align-items: center; gap: 6px; color: var(--fg); }
  table { border-collapse: collapse; font: 12px/1.6 "SF Mono", ui-monospace, Menlo, monospace; }
  td.n { position: sticky; left: 0; background: var(--bg); padding: 0 12px 0 14px; text-align: right; color: var(--muted); user-select: none; -webkit-user-select: none; vertical-align: top; }
  td.l { white-space: pre; padding-right: 24px; }
  body.wrap td.l { white-space: pre-wrap; word-break: break-all; }
  .t { color: var(--tag); } .a { color: var(--attr); } .s { color: var(--str); } .c { color: var(--cmt); font-style: italic; }
</style></head><body>
<div class="bar"><label><input type="checkbox" id="wrap"> Wrap long lines</label><span id="status">Loading…</span></div>
<table id="src"></table>
<script>
(() => {
  const DATA = __DATA__;
  document.title = 'view-source:' + DATA.url;
  const status = document.getElementById('status');
  const table = document.getElementById('src');
  document.getElementById('wrap').addEventListener('change', (e) => document.body.classList.toggle('wrap', e.target.checked));
  const esc = (t) => t.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  // Tints one line of escaped source. Comments are tracked across lines.
  let inComment = false;
  const tint = (line) => {
    let out = '';
    let rest = line;
    while (rest.length) {
      if (inComment) {
        const end = rest.indexOf('--&gt;');
        if (end < 0) { out += '<span class="c">' + rest + '</span>'; return out; }
        out += '<span class="c">' + rest.slice(0, end + 6) + '</span>';
        rest = rest.slice(end + 6);
        inComment = false;
        continue;
      }
      const c = rest.indexOf('&lt;!--');
      const t = rest.search(/&lt;\/?[A-Za-z!]/);
      if (c >= 0 && (t < 0 || c <= t)) { out += rest.slice(0, c); rest = rest.slice(c); inComment = true; continue; }
      if (t < 0) { out += rest; return out; }
      out += rest.slice(0, t);
      rest = rest.slice(t);
      const close = rest.indexOf('&gt;');
      const tag = close < 0 ? rest : rest.slice(0, close + 4);
      rest = close < 0 ? '' : rest.slice(close + 4);
      // One pass over the source text (never over the spans it adds).
      out += '<span class="t">' + tag.replace(/("[^"]*"|'[^']*')|(\s)([^\s=&"']+)(?==)/g, (m, str, sp, name) =>
        str ? '</span><span class="s">' + str + '</span><span class="t">' : sp + '</span><span class="a">' + name + '</span><span class="t">') + '</span>';
    }
    return out;
  };
  fetch(location.href, { credentials: 'include', cache: 'reload' })
    .then((r) => r.text())
    .then((text) => {
      const lines = text.split(/\r\n|\r|\n/);
      const html = lines.length <= 50000;
      const frag = document.createDocumentFragment();
      lines.forEach((line, i) => {
        const tr = document.createElement('tr');
        const n = document.createElement('td');
        n.className = 'n';
        n.textContent = String(i + 1);
        const l = document.createElement('td');
        l.className = 'l';
        if (html) l.innerHTML = tint(esc(line));
        else l.textContent = line;
        tr.append(n, l);
        frag.append(tr);
      });
      table.append(frag);
      status.textContent = lines.length.toLocaleString() + (lines.length === 1 ? ' line' : ' lines');
    })
    .catch(() => { status.textContent = 'The source couldn’t be loaded.'; });
})();
</script></body></html>"#;

/// Serica's own page for a site that can't be reached. Loaded in place of the
/// failed page (with its URL), so the address bar, back/forward and reload
/// keep working; it retries by itself when the connection returns.
pub fn error_page_html(kind: &str, title: &str, body: &str, url: &str, host: &str, code: &str, nonce: &str) -> String {
    let data = serde_json::json!({ "kind": kind, "title": title, "body": body, "url": url, "host": host, "code": code, "nonce": nonce });
    ERROR_PAGE_HTML.replace("__DATA__", &data.to_string().replace("</", "<\\/"))
}

const ERROR_PAGE_HTML: &str = r#"<!doctype html><html><head><meta charset="utf-8"><meta name="color-scheme" content="light dark"><meta name="viewport" content="width=device-width,initial-scale=1"><title></title><style>
  :root { --bg: #f6f6f7; --card: #ffffff; --fg: #17171a; --muted: #6b6b73; --line: #e6e6ea; --accent: #17171a; --on-accent: #ffffff; }
  @media (prefers-color-scheme: dark) { :root { --bg: #111113; --card: #1b1b1e; --fg: #ececef; --muted: #9b9ba3; --line: #2a2a2f; --accent: #ececef; --on-accent: #111113; } }
  * { box-sizing: border-box; margin: 0; }
  html, body { height: 100%; }
  body { background: var(--bg); color: var(--fg); font: 14px/1.55 -apple-system, BlinkMacSystemFont, "SF Pro Text", "Helvetica Neue", system-ui, sans-serif;
         display: grid; place-items: center; padding: 24px; -webkit-font-smoothing: antialiased; }
  main { width: min(440px, 100%); text-align: left; animation: in .35s cubic-bezier(.2,.8,.2,1); }
  @keyframes in { from { opacity: 0; transform: translateY(6px); } }
  .icon { width: 52px; height: 52px; border-radius: 14px; background: var(--card); border: 1px solid var(--line);
          display: grid; place-items: center; margin-bottom: 22px; }
  .icon svg { width: 24px; height: 24px; stroke: var(--fg); fill: none; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
  h1 { font: 600 22px/1.25 -apple-system, BlinkMacSystemFont, "SF Pro Display", "Helvetica Neue", system-ui, sans-serif; letter-spacing: -.01em; margin-bottom: 8px; }
  p { color: var(--muted); }
  .host { display: inline-block; margin: 14px 0 26px; padding: 3px 10px; border-radius: 999px; background: var(--card);
          border: 1px solid var(--line); color: var(--muted); font-size: 12.5px; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .row { display: flex; align-items: center; gap: 14px; flex-wrap: wrap; }
  button { font: 600 13px -apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif; border: 0; border-radius: 10px; padding: 9px 18px; cursor: pointer;
           background: var(--accent); color: var(--on-accent); transition: opacity .15s, transform .1s; }
  button:hover { opacity: .88; } button:active { transform: scale(.98); }
  button.ghost { background: transparent; color: var(--muted); border: 1px solid var(--line); }
  .status { font-size: 12.5px; color: var(--muted); display: flex; align-items: center; gap: 7px; }
  .dot { width: 7px; height: 7px; border-radius: 50%; background: #d9534f; }
  .dot.on { background: #3fb950; }
  .code { margin-top: 34px; font: 11.5px "SF Mono", ui-monospace, Menlo, monospace; color: var(--muted); opacity: .7; }
</style></head><body><main>
  <div class="icon"><svg viewBox="0 0 24 24" id="ic"></svg></div>
  <h1></h1><p class="body"></p><div class="host"></div>
  <div class="row"><button id="retry">Try again</button><button id="proceed" class="ghost" hidden>Continue to this site anyway</button><span class="status"><span class="dot"></span><span class="st"></span></span></div>
  <div class="code"></div>
</main><script>(() => {
  const d = __DATA__;
  const icons = {
    offline: '<path d="M2 8.8a15 15 0 0 1 4.2-2.6"/><path d="M10.7 5.1A15 15 0 0 1 22 8.8"/><path d="M5 12.6a10 10 0 0 1 5.2-2.7"/><path d="M16.8 11.2A10 10 0 0 1 19 12.6"/><path d="M8.5 16.4a5 5 0 0 1 7 0"/><path d="M12 20h.01"/><path d="m2 2 20 20"/>',
    dns: '<circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/><path d="M8.5 8.5l5 5M13.5 8.5l-5 5"/>',
    timeout: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
    unreachable: '<circle cx="12" cy="12" r="9"/><path d="M12 8v5"/><path d="M12 16h.01"/>',
    cert: '<rect x="4" y="10" width="16" height="11" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3"/><path d="M12 14v3"/>',
  };
  document.getElementById('ic').innerHTML = icons[d.kind] || icons.unreachable;
  document.title = d.host || d.title;
  document.querySelector('h1').textContent = d.title;
  document.querySelector('.body').textContent = d.body;
  document.querySelector('.host').textContent = d.host || d.url;
  document.querySelector('.code').textContent = d.code;
  const retry = () => location.replace(d.url);
  document.getElementById('retry').onclick = retry;
  if (d.kind === 'cert') {
    const b = document.getElementById('proceed');
    b.hidden = false;
    b.onclick = () => document.dispatchEvent(new CustomEvent('serica-cert-proceed', { detail: d.nonce }));
  }
  const status = () => {
    const on = navigator.onLine;
    document.querySelector('.dot').classList.toggle('on', on);
    document.querySelector('.st').textContent = on ? 'Connected' : 'No internet connection';
  };
  status();
  addEventListener('offline', status);
  if (d.kind !== 'cert') addEventListener('online', () => { status(); setTimeout(retry, 600); });
})();</script></body></html>"#;

/// Serica world, top frame: forwards the error page's "Continue anyway"
/// (a DOM event, with the page's one-time nonce) to the backend.
pub fn cert_bridge_script() -> String {
    format!(
        r#"(() => {{
  if (window.top !== window || window.__sericaCert) return;
  Object.defineProperty(window, '__sericaCert', {{ value: true }});
  {POST}
  document.addEventListener('serica-cert-proceed', (e) => {{ post({{ t: 'cert-proceed', nonce: String(e.detail || '') }}); }});
}})();"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_use_the_webkit_bridge() {
        for s in [page_script(), frame_script(), cert_bridge_script()] {
            assert!(s.contains("window.webkit.messageHandlers.serica.postMessage"));
            assert!(!s.contains("chrome.webview"));
        }
        assert!(!FIND_SCRIPT.contains("chrome.webview"));
    }

    #[test]
    fn error_page_escapes_its_data() {
        let html = error_page_html("dns", "t", "b", "https://x.test/</script><script>alert(1)</script>", "x.test", "ERR", "n");
        assert!(!html.contains("</script><script>alert(1)"));
        assert!(html.contains("<\\/script>"));
    }
}

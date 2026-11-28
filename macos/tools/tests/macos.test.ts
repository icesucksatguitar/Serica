// The macOS build's interface speaks macOS: ⌘ shortcut hints, Mac wording and
// fonts, and no Windows window buttons. Reads the sources (not the bundle).
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

// The runner bundles tests into a temporary folder: the sources are found
// from where `npm test` runs (the macos/ folder).
const root = join(process.cwd(), 'src');

const files: string[] = [];
const walk = (dir: string) => {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p);
    else if (/\.(tsx?|css)$/.test(name)) files.push(p);
  }
};
walk(root);
if (files.length < 50) throw new Error(`expected the UI sources under ${root}, found ${files.length} files`);

// What must not appear anywhere in the macOS interface.
const forbidden: [RegExp, string][] = [
  [/\bCtrl[+ ]/, 'a Ctrl shortcut hint (macOS uses ⌘)'],
  [/\bAlt[+ ](Left|Right|Home|←|→)/, 'an Alt shortcut hint (macOS uses ⌘[ / ⌘])'],
  [/this PC\b/, '"this PC" (say "this Mac")'],
  [/Match Windows/, '"Match Windows" (say "Match macOS")'],
  [/Windows Downloads folder/, '"Windows Downloads folder"'],
  [/Windows asks in Settings/, '"Windows asks in Settings"'],
  [/Windows VPN/, '"Windows VPN" (say "System VPN")'],
  [/\bExplorer\b/, '"Explorer" (say "Finder")'],
  [/Segoe UI|Cascadia|Consolas/, 'a Windows font (use the system font, SF Mono)'],
  [/WebView2/, 'WebView2 (the macOS build runs on WebKit)'],
];

const problems: string[] = [];
for (const f of files) {
  const lines = readFileSync(f, 'utf8').split('\n');
  lines.forEach((line, i) => {
    for (const [re, what] of forbidden) {
      if (re.test(line)) problems.push(`${relative(root, f)}:${i + 1}: ${what}\n    ${line.trim()}`);
    }
  });
}

// The Windows minimize / maximize / close buttons are not drawn.
const app = readFileSync(join(root, 'App.tsx'), 'utf8');
if (/wctl-zone|window-controls-right/.test(app)) problems.push('App.tsx: still renders the Windows window buttons');
// The traffic lights get room in the top bar, and the drag regions work.
if (!/--traffic-lights-inset/.test(app)) problems.push('App.tsx: no traffic-light inset for the top bar');
if (!/installDragRegions\(\)/.test(app)) problems.push('App.tsx: drag regions are not installed');
const css = readFileSync(join(root, 'styles', 'macos.css'), 'utf8');
if (!/var\(--traffic-lights-inset/.test(css)) problems.push('styles/macos.css: the top bar does not use --traffic-lights-inset');

// Shortcut hints use the Mac glyphs.
const settings = readFileSync(join(root, 'components', 'settings', 'Settings2.tsx'), 'utf8');
for (const k of ['⌘T', '⇧⌘T', '⌘,', '⌥⌘I', '⌃Tab']) {
  if (!settings.includes(`'${k}`)) problems.push(`Settings2.tsx: the shortcut list lacks ${k}`);
}

if (problems.length) {
  throw new Error(`${problems.length} Windows leftovers in the macOS UI:\n${problems.join('\n')}`);
}
console.log(`ok - macOS wording, shortcuts and window chrome (${files.length} files)`);

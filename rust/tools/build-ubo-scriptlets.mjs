// Converts uBlock Origin's scriptlets into adblock-rust resources.
//
// Shields downloads Brave's adblock-resources, which only holds Brave's own
// extras; uBlock Origin's scriptlets (set-constant, json-prune, trusted-*, …)
// are what its filter lists reference, e.g. for YouTube ads. This script turns
// uBO's ES-module scriptlets into the JSON format `Engine::use_resources` takes.
//
// Usage (from a uBlock Origin checkout or release tarball):
//   node rust/tools/build-ubo-scriptlets.mjs <uBlock>/src/js/resources rust/resources/ubo-scriptlets.json <uBO version>
//
// The output contains uBlock Origin code and is licensed GPL-3.0 (see
// rust/resources/README.md). It is downloaded by the app at runtime, not
// compiled into it.
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const [resourcesDir, outFile, version = 'unknown'] = process.argv.slice(2);
if (!resourcesDir || !outFile) {
  console.error('usage: node build-ubo-scriptlets.mjs <uBO src/js/resources dir> <out.json> [version]');
  process.exit(1);
}

const { builtinScriptlets } = await import(pathToFileURL(path.resolve(resourcesDir, 'scriptlets.js')).href);
const b64 = (s) => Buffer.from(s, 'utf8').toString('base64');

const resources = [];
for (const s of builtinScriptlets) {
  if (typeof s.fn !== 'function' || !s.name) continue;
  const source = s.fn.toString();
  // adblock-rust invokes function-style scriptlets by their declared name.
  if (!/^function\s+[^\s(]+\s*\(/.test(source)) {
    console.warn(`skipping ${s.name}: not a named function declaration`);
    continue;
  }
  const resource = {
    name: s.name,
    aliases: Array.isArray(s.aliases) ? s.aliases : [],
    kind: { mime: s.name.endsWith('.fn') ? 'fn/javascript' : 'application/javascript' },
    content: b64(source),
  };
  const deps = (s.dependencies || []).filter((d) => typeof d === 'string');
  if (deps.length) resource.dependencies = deps;
  // Trusted scriptlets only run from trusted lists (uBO's own), permission bit 0b1.
  if (s.requiresTrust) resource.permission = 1;
  resources.push(resource);
}

fs.mkdirSync(path.dirname(outFile), { recursive: true });
fs.writeFileSync(outFile, JSON.stringify(resources));
console.log(`wrote ${resources.length} scriptlet resources from uBlock Origin ${version} to ${outFile}`);

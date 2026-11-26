// Bundles each *.test.ts here with esbuild (Vite's own copy) and runs it in Node.
import { build } from 'esbuild';
import { readdirSync, mkdtempSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const out = mkdtempSync(join(tmpdir(), 'serica-tests-'));
let failed = 0;
try {
  for (const f of readdirSync(here).filter((n) => n.endsWith('.test.ts'))) {
    const file = join(out, f.replace(/\.ts$/, '.mjs'));
    await build({ entryPoints: [join(here, f)], bundle: true, platform: 'node', format: 'esm', outfile: file, logLevel: 'error' });
    console.log(`# ${f}`);
    try {
      await import(pathToFileURL(file).href);
    } catch (e) {
      failed++;
      console.error(`FAILED ${f}:`, e?.message ?? e);
    }
  }
} finally {
  rmSync(out, { recursive: true, force: true });
}
process.exit(failed ? 1 : 0);

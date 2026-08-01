// Copy the built assets into the Rust crate, after re-checking they can actually be served.
//
// The Vite config is written to emit nothing inline, but a build tool's configuration is a
// statement of intent and this is a security control: the server's Content-Security-Policy has no
// `unsafe-inline`, so a single injected `<style>` block would produce a page that renders
// unstyled, in a browser, at runtime, with the reason buried in a console nobody is watching.
//
// So the emitted bytes are inspected here rather than trusted:
//
//   * the document may carry no inline `<script>` and no `<style>` block
//   * no element may carry an inline event handler or a `style=` attribute
//   * every URL it references must be same-origin and must exist in the output
//   * nothing may reference a remote origin, in any file
//   * every file the Rust `include_bytes!` table names must be present
//
// A failure here fails `npm run build`. The fix is always the build, never the policy.

import { copyFileSync, mkdirSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const dist = join(root, 'dist');
const target = join(root, '..', '..', 'crates', 'nerve-server', 'assets');

/** Files the Rust asset table compiles in. Keep in step with `crates/nerve-server/src/assets.rs`. */
const REQUIRED = ['index.html', 'assets/nerve.js', 'assets/nerve.css', 'assets/favicon.svg'];

const problems = [];
const fail = (message) => problems.push(message);

function walk(directory, base = directory) {
  const found = [];
  for (const name of readdirSync(directory)) {
    const path = join(directory, name);
    if (statSync(path).isDirectory()) found.push(...walk(path, base));
    else found.push(relative(base, path).split('\\').join('/'));
  }
  return found;
}

let emitted;
try {
  emitted = walk(dist);
} catch {
  console.error('embed: no dist/ — run `vite build` first');
  process.exit(1);
}

// ---- the document must need no exception to the policy --------------------------------------

const html = readFileSync(join(dist, 'index.html'), 'utf8');

// `<script src=…>` is fine; `<script>` with a body is not. The distinction is the whole point.
for (const match of html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi)) {
  const [, attributes, body] = match;
  if (body.trim().length > 0) {
    fail(`index.html carries an inline <script> body, which script-src 'self' will not run`);
  }
  if (!/\bsrc\s*=/.test(attributes)) {
    fail(`index.html carries a <script> with no src`);
  }
}
if (/<style\b/i.test(html)) {
  fail(`index.html carries a <style> block, which style-src 'self' will not apply`);
}
if (/\son[a-z]+\s*=/i.test(html)) {
  fail(`index.html carries an inline event handler attribute`);
}
if (/\sstyle\s*=\s*["']/i.test(html)) {
  fail(`index.html carries a style= attribute, which style-src 'self' will not apply`);
}

// ---- nothing may reach off this origin ------------------------------------------------------

/**
 * Absolute URLs that are allowed to appear in the output, with the reason each one is inert.
 *
 * This list is deliberately short and deliberately explicit. A remote origin that reaches the
 * network would be blocked at runtime by `default-src 'none'; connect-src 'self'` anyway — the
 * reason to catch it here is that a blocked request is a silent, invisible failure, and the
 * places it does real damage are stylesheets and markup, where the page simply renders wrong.
 */
const INERT = [
  { prefix: 'http://www.w3.org/', why: 'the SVG/XML namespace, which is an identifier and is never requested' },
  { prefix: 'http://127.0.0.1', why: 'this server’s own loopback origin' },
  { prefix: 'https://127.0.0.1', why: 'this server’s own loopback origin' },
  { prefix: 'https://react.dev/errors/', why: 'React’s minified-error explainer, a string in a thrown message' },
];

for (const name of emitted) {
  const text = readFileSync(join(dist, name));
  // Binary files are skipped for the text checks; none are emitted today, but a future image
  // asset must not fail the build with a spurious match on random bytes.
  if (text.includes(0)) continue;
  const source = text.toString('utf8');

  for (const match of source.matchAll(/\bhttps?:\/\/[^\s'"`)]+/g)) {
    if (INERT.some((entry) => match[0].startsWith(entry.prefix))) continue;
    fail(`${name} references a remote origin: ${match[0]}`);
  }

  if (/\bdata:\s*(text\/javascript|application\/javascript)/i.test(source)) {
    fail(`${name} carries a data: script URL`);
  }

  // A stylesheet is the one place where an off-origin reference is both easy to introduce and
  // invisible when it fails, so every `url()` in the CSS is checked for being local.
  if (name.endsWith('.css')) {
    for (const match of source.matchAll(/url\(\s*['"]?([^'")]+)['"]?\s*\)/g)) {
      const url = match[1].trim();
      if (url.startsWith('data:') || url.startsWith('/') || url.startsWith('./') || url.startsWith('#')) {
        continue;
      }
      if (/^[a-z][a-z0-9+.-]*:/i.test(url)) {
        fail(`${name} loads ${url} from off this origin`);
      }
    }
    if (/@import/.test(source)) {
      fail(`${name} uses @import, which fetches a second stylesheet at runtime`);
    }
  }
}

// ---- every URL the document names must have been emitted ------------------------------------

for (const match of html.matchAll(/\b(?:src|href)\s*=\s*["']([^"']+)["']/g)) {
  const url = match[1];
  if (url.startsWith('data:') || url.startsWith('#')) continue;
  const wanted = url.replace(/^\//, '').split('?')[0];
  if (!emitted.includes(wanted)) {
    fail(`index.html references ${url}, which the build did not emit`);
  }
}

// ---- and the Rust table must find everything it names ---------------------------------------

for (const name of REQUIRED) {
  if (!emitted.includes(name)) {
    fail(`the Rust asset table names ${name}, which the build did not emit`);
  }
}

if (problems.length > 0) {
  console.error(`embed: refusing to embed — ${problems.length} problem${problems.length === 1 ? '' : 's'}\n`);
  for (const problem of problems) console.error(`  ${problem}`);
  console.error('\nFix the build. Do not weaken the Content-Security-Policy.');
  process.exit(1);
}

// ---- copy ------------------------------------------------------------------------------------

let bytes = 0;
for (const name of emitted) {
  const destination = join(target, name);
  mkdirSync(dirname(destination), { recursive: true });
  copyFileSync(join(dist, name), destination);
  bytes += statSync(join(dist, name)).size;
}

const listing = emitted
  .map((name) => `    ${name.padEnd(22)} ${String(statSync(join(dist, name)).size).padStart(8)} B`)
  .join('\n');
console.log(`embed: ${emitted.length} files, ${bytes} B → crates/nerve-server/assets\n${listing}`);

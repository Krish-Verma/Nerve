// The frontend gate.
//
// This is not a style linter and it is not trying to be one — `tsc --strict` already rejects the
// things a style linter would catch here, and a real linter is another distributed-tooling tree to
// licence-review. What this checks is the small set of rules that are *product* rules, where being
// wrong is a security or licensing failure rather than an untidy diff:
//
//   1. THREAT-MODEL T5. No `dangerouslySetInnerHTML`, no `innerHTML`, no `eval`, no
//      `document.write`, no `new Function`. Repository content is hostile and is rendered on every
//      screen of this app; the only safe way to put it on the page is as a React child, and the
//      only way to keep that true is to make the alternatives fail the build.
//   2. No remote origin. The CSP forbids one, so a `https://` in the source is a page that will
//      break at runtime in a way nobody sees until a screenshot is taken.
//   3. No storage of the session token. `nerve serve` promises the token is never written to
//      disk; `localStorage` and `sessionStorage` both persist for session restore.
//   4. Runtime dependencies are `react` and `react-dom`, exactly. This is a licence-surface rule
//      (see docs/plans/slice-04-visual-explorer.md P5), so it is enforced, not documented.
//
// Exit code is the whole interface: 0 clean, 1 with every violation printed.

import { readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const problems = [];

function walk(directory) {
  const found = [];
  for (const name of readdirSync(directory)) {
    const path = join(directory, name);
    if (statSync(path).isDirectory()) found.push(...walk(path));
    else found.push(path);
  }
  return found;
}

const sources = walk(join(root, 'src')).filter((path) => /\.(ts|tsx|mjs|css)$/.test(path));

// ---- 1, 2, 3: what may not appear in the source ---------------------------------------------

const FORBIDDEN = [
  {
    pattern: /dangerouslySetInnerHTML/,
    why: 'T5: repository content is hostile; render it as a React child, never as markup',
  },
  { pattern: /\.innerHTML\b/, why: 'T5: assigning innerHTML parses attacker-controlled text as markup' },
  { pattern: /\.outerHTML\s*=/, why: 'T5: assigning outerHTML parses attacker-controlled text as markup' },
  { pattern: /\binsertAdjacentHTML\b/, why: 'T5: insertAdjacentHTML parses its argument as markup' },
  { pattern: /\beval\s*\(/, why: 'T5: the CSP has no unsafe-eval, so this cannot run anyway' },
  { pattern: /new\s+Function\s*\(/, why: 'T5: Function() is eval under another name' },
  { pattern: /document\.write\b/, why: 'T5: document.write parses its argument as markup' },
  { pattern: /\bjavascript:/, why: 'T5: a javascript: URL is script in an attribute' },
  {
    pattern: /https?:\/\/(?!127\.0\.0\.1)/,
    why: 'offline-first: the CSP forbids every remote origin',
  },
  {
    pattern: /\b(localStorage|sessionStorage|indexedDB)\b/,
    why: 'the session token must not outlive the tab; nerve serve promises it never reaches disk',
  },
  { pattern: /@import\s+url\(/, why: 'offline-first: no stylesheet may be fetched at runtime' },
];

for (const path of sources) {
  const text = readFileSync(path, 'utf8');
  const lines = text.split('\n');
  lines.forEach((line, index) => {
    // A line that names a rule in order to forbid it is not a violation of it. Only this file
    // and the comments that explain the rule are allowed to mention the forbidden spellings.
    const isComment = /^\s*(\/\/|\*|\/\*)/.test(line);
    for (const rule of FORBIDDEN) {
      if (!rule.pattern.test(line)) continue;
      if (isComment) continue;
      problems.push(`${relative(root, path)}:${index + 1}  ${rule.why}\n    ${line.trim()}`);
    }
  });
}

// ---- 4: the distributed dependency surface --------------------------------------------------

const manifest = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
const runtime = Object.keys(manifest.dependencies ?? {}).sort();
const ALLOWED = ['react', 'react-dom'];
if (runtime.join(',') !== ALLOWED.join(',')) {
  problems.push(
    `package.json  runtime dependencies must be exactly ${ALLOWED.join(' + ')}, found ${
      runtime.length === 0 ? '(none)' : runtime.join(', ')
    }\n    every runtime dependency is compiled into the nerve binary and must be licence-reviewed`,
  );
}

// A dependency that is not imported anywhere is a licence surface carried for nothing.
const imported = new Set();
for (const path of sources) {
  for (const match of readFileSync(path, 'utf8').matchAll(/from\s+['"]([^'".][^'"]*)['"]/g)) {
    const specifier = match[1];
    imported.add(specifier.startsWith('@') ? specifier.split('/').slice(0, 2).join('/') : specifier.split('/')[0]);
  }
}
for (const name of runtime) {
  const used = [...imported].some((specifier) => specifier === name || specifier.startsWith(`${name}/`));
  if (!used) problems.push(`package.json  "${name}" is declared but never imported`);
}

// ---- report ---------------------------------------------------------------------------------

if (problems.length > 0) {
  console.error(`lint: ${problems.length} problem${problems.length === 1 ? '' : 's'}\n`);
  for (const problem of problems) console.error(`  ${problem}\n`);
  process.exit(1);
}

console.log(`lint: ${sources.length} files clean · runtime deps ${runtime.join(' + ')}`);

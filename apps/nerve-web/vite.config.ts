import { defineConfig } from 'vite';

// The build has one job beyond bundling: emit assets the server's Content-Security-Policy can
// serve without an exception. That policy has no `unsafe-inline`, so nothing may end up inline in
// the document — no `<style>` block, no inline `<script>`, no data: script URL, no injected
// preload shim. Every option below exists to hold that line, and `tools/embed.mjs` re-checks the
// emitted HTML rather than trusting this file.
//
// File names are fixed rather than content-hashed because the output is compiled into the Rust
// binary through an explicit `include_bytes!` table. A hashed name would mean editing Rust on
// every rebuild, and the server already sends `Cache-Control: no-store`, so the cache-busting a
// hash buys is worth nothing here.
export default defineConfig({
  // No plugin is used. `@vitejs/plugin-react` exists for dev-server Fast Refresh, which this
  // project never runs — the app is only ever served from the Rust binary — and it would pull
  // Babel into a build tree we would then have to licence-review.
  esbuild: {
    jsx: 'automatic',
    jsxImportSource: 'react',
  },
  build: {
    target: 'es2022',
    outDir: 'dist',
    emptyOutDir: true,
    sourcemap: false,
    cssCodeSplit: false,
    // Vite would otherwise inline small assets as `data:` URIs. Keeping every asset a real file
    // keeps the served bytes auditable one file at a time.
    assetsInlineLimit: 0,
    // The module-preload polyfill is injected as an inline script. It is not needed for a page
    // served to one modern browser on loopback, and it would violate the policy.
    modulePreload: false,
    rollupOptions: {
      output: {
        entryFileNames: 'assets/nerve.js',
        chunkFileNames: 'assets/nerve-[name].js',
        assetFileNames: (info) =>
          info.names?.some((name) => name.endsWith('.css'))
            ? 'assets/nerve.css'
            : 'assets/[name][extname]',
      },
    },
  },
});

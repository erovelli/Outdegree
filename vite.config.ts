import { readFileSync } from "node:fs";
import { fileURLToPath, URL } from "node:url";
import { defineConfig, type Plugin } from "vite";
import { crx } from "@crxjs/vite-plugin";
import manifest from "./extension/manifest.config";

// Embed the WASM binary as inlined bytes so the dashboard instantiates it via
// `WebAssembly.instantiate(bytes)` instead of `fetch()`. Under the manifest's
// CSP `connect-src 'none'` (§5/§12.1), fetch — even of the extension's own
// resources — is browser-blocked, so the default wasm-bindgen `--target web`
// fetch path would fail. Passing bytes keeps the local-only guarantee intact.
const WASM_FILE = fileURLToPath(
  new URL(
    "./extension/src/wasm/browsing_graph_core_bg.wasm",
    import.meta.url
  )
);
const VIRTUAL_WASM = "virtual:wasm-bytes";

function wasmInlineBytes(): Plugin {
  const resolved = "\0" + VIRTUAL_WASM;
  return {
    name: "wasm-inline-bytes",
    enforce: "pre",
    resolveId(id) {
      if (id === VIRTUAL_WASM) return resolved;
    },
    load(id) {
      if (id === resolved) {
        const b64 = readFileSync(WASM_FILE).toString("base64");
        return (
          `const b64 = "${b64}";\n` +
          `const bin = atob(b64);\n` +
          `const bytes = new Uint8Array(bin.length);\n` +
          `for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);\n` +
          `export default bytes;\n`
        );
      }
    },
    // Neutralize wasm-bindgen's dead fetch self-loader (we always pass bytes).
    // Removing the `new URL(..., import.meta.url)` also stops Vite from emitting
    // a duplicate .wasm asset, and leaves the bundle with zero network surface.
    transform(code, id) {
      if (id.endsWith("browsing_graph_core.js")) {
        const out = code
          .replace(
            /module_or_path\s*=\s*new URL\([^)]*import\.meta\.url\s*\);/g,
            "/* wasm is provided as inlined bytes */"
          )
          .replace(
            /module_or_path\s*=\s*fetch\(\s*module_or_path\s*\);/g,
            "throw new Error('Browsing Graph: wasm must be instantiated from bytes');"
          );
        return { code: out, map: null };
      }
    },
  };
}

// Build pipeline (§9). `--target web` WASM avoids top-level-await; if the wasm
// asset misresolves under a future Vite, add `vite-plugin-wasm`. Vite 8 renames
// `build.rollupOptions` → `build.rolldownOptions`.
export default defineConfig({
  root: fileURLToPath(new URL("./extension", import.meta.url)),
  build: {
    outDir: fileURLToPath(new URL("./dist", import.meta.url)),
    emptyOutDir: true,
    target: "esnext",
    // The extension is a single bundle, so there is nothing to preload. Disabling
    // the polyfill removes Vite's only `fetch()` call — leaving the built bundle
    // with zero network surface (§12.7).
    modulePreload: false,
    chunkSizeWarningLimit: 4096,
    rollupOptions: {
      // dashboard.html is opened at runtime (not referenced in the manifest),
      // so it must be declared as an explicit input to be built.
      input: {
        dashboard: fileURLToPath(
          new URL("./extension/dashboard.html", import.meta.url)
        ),
      },
    },
  },
  plugins: [wasmInlineBytes(), crx({ manifest })],
});

// Ambient declaration for the wasm-pack (`--target web`) output, so the TS layer
// typechecks before the WASM artifact is generated into src/wasm/. The real
// generated `browsing_graph_core.d.ts` takes precedence when present.
declare module "*/browsing_graph_core.js" {
  export default function init(input?: unknown): Promise<unknown>;
  export function mount(rootId: string): void;
}

// Inlined WASM bytes provided by the `wasm-inline-bytes` Vite plugin.
declare module "virtual:wasm-bytes" {
  const bytes: Uint8Array;
  export default bytes;
}

// Vite `?raw` imports: the file's contents as a string, inlined at build time.
// Used to bundle the onboarding sample fixture without a fetch (§F4).
declare module "*?raw" {
  const content: string;
  export default content;
}

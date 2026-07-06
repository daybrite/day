// ArkTS type declaration for the Rust native module (libentry.so). The module exposes a single
// `start(nodeContent, widthVp, heightVp, density)` entry (registered by the C++ shim's NAPI init).
export const start: (content: Object, widthVp: number, heightVp: number, density: number) => void;

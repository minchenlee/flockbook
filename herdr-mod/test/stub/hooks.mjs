// Resolves the app's built-in modules to the fakes.
const map = { "gpui-kit": "./ui.mjs", "gpui-base": "./ui.mjs", herdr: "./herdr.mjs" };
export async function resolve(spec, ctx, next) {
  if (map[spec]) return { url: new URL(map[spec], import.meta.url).href, shortCircuit: true };
  return next(spec, ctx);
}

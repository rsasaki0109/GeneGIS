// Run the browser build of GeneGIS in Node: load the bundled samples, ask a
// question, and require a verified answer (every step check passed).
//
// Usage: node scripts/smoke-wasm.mjs [pkg_dir]
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const pkg = resolve(process.argv[2] ?? "public/try/pkg");
const { initSync, GeneGis } = await import(pathToFileURL(`${pkg}/genegis_wasm.js`).href);
initSync({ module: await readFile(`${pkg}/genegis_wasm_bg.wasm`) });

const gis = new GeneGis();
const layers = JSON.parse(gis.loadSamples());
if (layers.length !== 5) throw new Error(`expected 5 sample layers, got ${layers.length}`);

const answer = JSON.parse(gis.ask("区の人口密度", ""));
const checks = answer.result.receipt.steps.flatMap((step) => step.checks);
if (!checks.length || !checks.every((check) => check.passed)) throw new Error("unverified answer");
if (answer.result.layer.feature_count !== 16) throw new Error("expected 16 wards");

let refused = false;
try {
  gis.ask("明日の天気を教えて", "");
} catch {
  refused = true;
}
if (!refused) throw new Error("an unanswerable question must be refused");

console.log(`wasm smoke ok: ${checks.length} checks passed, digest ${answer.result.receipt.result_digest}`);

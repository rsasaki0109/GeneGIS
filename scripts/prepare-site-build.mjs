import { copyFile, mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const source = resolve(root, ".openai", "hosting.json");
const destination = resolve(root, "dist", ".openai", "hosting.json");

await mkdir(dirname(destination), { recursive: true });
await copyFile(source, destination);
console.log("Prepared Sites metadata at dist/.openai/hosting.json");

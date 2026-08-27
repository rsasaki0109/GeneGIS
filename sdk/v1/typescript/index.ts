export const PLUGIN_API_VERSION = "1.0.0" as const;

export type PluginCapability =
  | "read_catalog"
  | "read_storage"
  | "analysis_step"
  | "render_hook"
  | "export_artifact"
  | "publish_stac";

export interface PluginManifestV1 {
  id: string;
  name?: string;
  version: string;
  api_version: `1.${number}.${number}`;
  description?: string;
  author?: string;
  capabilities: PluginCapability[];
  artifact_digest: `sha256:${string}`;
  wasm?: { entry: string };
}

const capabilities = new Set<PluginCapability>([
  "read_catalog", "read_storage", "analysis_step", "render_hook",
  "export_artifact", "publish_stac",
]);

export function assertPluginManifestV1(value: unknown): asserts value is PluginManifestV1 {
  if (!value || typeof value !== "object") throw new Error("manifest must be an object");
  const item = value as Record<string, unknown>;
  const keys = new Set(["id", "name", "version", "api_version", "description", "author", "capabilities", "artifact_digest", "wasm"]);
  if (Object.keys(item).some((key) => !keys.has(key))) throw new Error("unknown manifest field");
  if (typeof item.id !== "string" || !/^[a-z0-9-]+$/.test(item.id)) throw new Error("invalid plugin id");
  if (typeof item.version !== "string" || !/^\d+\.\d+\.\d+$/.test(item.version)) throw new Error("invalid version");
  if (typeof item.api_version !== "string" || !/^1\.\d+\.\d+$/.test(item.api_version)) throw new Error("unsupported API version");
  if (!Array.isArray(item.capabilities) || item.capabilities.length === 0 || new Set(item.capabilities).size !== item.capabilities.length || item.capabilities.some((capability) => !capabilities.has(capability as PluginCapability))) throw new Error("invalid capabilities");
  if (typeof item.artifact_digest !== "string" || !/^sha256:[0-9a-fA-F]{64}$/.test(item.artifact_digest)) throw new Error("invalid artifact digest");
}

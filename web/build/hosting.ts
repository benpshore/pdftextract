// `.openai/hosting.json` is provisioned by the managed Site: the logical D1/R2
// binding names and the enabled capabilities (docs/WEB_ALPHA.md). A fresh
// checkout has no such file, and a build must still produce a local-only
// preview rather than fail while loading its own Vite config. The fallback is
// exactly the documented local configuration; it never carries credentials,
// database identifiers or deployment identity.
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

export type HostingConfig = {
  d1?: string;
  r2?: string;
  capabilities?: string[];
};

export const LOCAL_HOSTING_CONFIG: HostingConfig = { d1: "DB", r2: "BUCKET" };

export function hostingConfigPath(root: string): string {
  return resolve(root, ".openai", "hosting.json");
}

/**
 * The hosting configuration in effect for `root`: the Site's file when it
 * exists (`source: "file"`), else the local-only defaults (`source: "fallback"`).
 */
export function readHostingConfig(root: string): {
  config: HostingConfig;
  source: "file" | "fallback";
} {
  const path = hostingConfigPath(root);
  if (!existsSync(path)) {
    return { config: LOCAL_HOSTING_CONFIG, source: "fallback" };
  }
  const parsed: unknown = JSON.parse(readFileSync(path, "utf8"));
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error(`${path} must contain a JSON object`);
  }
  return { config: parsed as HostingConfig, source: "file" };
}

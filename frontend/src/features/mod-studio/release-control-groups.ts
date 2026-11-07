export const PLUGIN_STATE = {
  loaded: "Loaded",
  unloaded: "Disabled",
  unload_pending: "Unloading",
  failed: "Load failed",
  discovered: "Discovered",
  faulted: "Plugin fault",
  blocked: "Operation blocked",
  release_pending: "Releasing image",
  validated: "Candidate validated",
  faulted_unloaded: "Unloaded after fault",
  rejected: "Candidate rejected",
} as const;

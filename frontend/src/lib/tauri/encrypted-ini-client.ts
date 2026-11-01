import { invokeCommand } from "@/lib/tauri/command-error";
import type { EncryptedIniSnapshot } from "@/lib/tauri/generated/EncryptedIniSnapshot";
import type { OpenEncryptedIniResult } from "@/lib/tauri/generated/OpenEncryptedIniResult";
import type { SaveEncryptedIniRequest } from "@/lib/tauri/generated/SaveEncryptedIniRequest";
import type { SaveEncryptedIniResult } from "@/lib/tauri/generated/SaveEncryptedIniResult";

export const encryptedIniClient = {
  getSnapshot: () =>
    invokeCommand<EncryptedIniSnapshot>("get_encrypted_ini_snapshot"),
  open: () => invokeCommand<OpenEncryptedIniResult>("open_encrypted_ini"),
  reload: () => invokeCommand<EncryptedIniSnapshot>("reload_encrypted_ini"),
  save: (request: SaveEncryptedIniRequest) =>
    invokeCommand<SaveEncryptedIniResult>("save_encrypted_ini", { request }),
  clear: () => invokeCommand<EncryptedIniSnapshot>("clear_encrypted_ini"),
};

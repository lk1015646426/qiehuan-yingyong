// Detection result for the local TRAE Work CN install.
// Mirrors `WorkCnInstallation` in src-tauri/src/models/work_cn.rs.
export interface WorkCnInstallation {
  installed: boolean;
  executablePath: string | null;
  userDataDir: string | null;
  storagePath: string | null;
  displayName: string | null;
  version: string | null;
  legacyPath: boolean;
}

import { invoke } from '@tauri-apps/api/tauri';
import { listen, UnlistenFn } from '@tauri-apps/api/event';

// Types for the new .7z.tlock format
export interface TlockMetadata {
  version: string;
  locked: boolean;
  created: string;
  unlocks: string;
  duration: string;
  originalFile: string;
  originalSize?: number;
  compressedSize?: number;
  fileCount?: number;
}

export interface LockedItem {
  id: string;
  name: string;
  type: 'file' | 'folder';
  keyPath: string;        // For legacy .key.md files
  tlockPath?: string;     // For new .7z.tlock files
  zipPath: string;
  created: Date;
  unlocks: Date;
  isReady: boolean;
  metadata?: TlockMetadata;
  isLegacyFormat?: boolean; // true if using old .key.md format
  unlockedPath?: string;  // Path to unlocked content if already extracted
}

export interface LockOptions {
  deleteOriginal?: boolean;
}

export interface LockResult {
  success: boolean;
  keyPath?: string;
  tlockPath?: string;     // New format path
  zipPath?: string;
  unlockTime?: string;
  error?: string;
  /** Whether the original file/folder was deleted after locking */
  originalDeleted?: boolean;
  /** Error message if deletion was requested but failed (archive still created successfully) */
  deletionError?: string;
}

export interface UnlockResult {
  success: boolean;
  outputPath?: string;
  error?: string;
}

export interface MigrationResult {
  success: boolean;
  tlockPath?: string;
  error?: string;
}

export interface AppSettings {
  vaults: string[];
}

// Progress event types
export interface LockProgressEvent {
  stage: 'compressing' | 'encrypting' | 'finalizing';
  progress: number;  // 0-100
  currentFile?: string;
  bytesProcessed?: number;
  totalBytes?: number;
}

export interface UnlockProgressEvent {
  stage: 'decrypting' | 'extracting' | 'finalizing';
  progress: number;  // 0-100
  currentFile?: string;
  bytesProcessed?: number;
  totalBytes?: number;
}

interface AppStateResponse {
  settings: AppSettings;
  locked_items: any[];
}

/**
 * Listen for lock progress events from the backend
 */
export async function onLockProgress(callback: (event: LockProgressEvent) => void): Promise<UnlistenFn> {
  return await listen<LockProgressEvent>('lock-progress', (event) => {
    callback(event.payload);
  });
}

/**
 * Listen for unlock progress events from the backend
 */
export async function onUnlockProgress(callback: (event: UnlockProgressEvent) => void): Promise<UnlistenFn> {
  return await listen<UnlockProgressEvent>('unlock-progress', (event) => {
    callback(event.payload);
  });
}

/**
 * Open a path in the system file explorer (cross-platform)
 * @param path - Path to file or directory to open
 */
export async function openInExplorer(path: string): Promise<void> {
  await invoke('open_in_explorer', { path });
}

/**
 * Lock a file or folder with a time-based key
 * @param path - Path to file or folder to lock
 * @param unlockTime - ISO timestamp when the file can be unlocked
 * @param vault - Optional vault directory to store the locked file
 * @param options - Additional options (deleteOriginal, etc.)
 */
export async function lockItem(
  path: string,
  unlockTime: string,
  vault?: string,
  options?: LockOptions
): Promise<LockResult> {
  try {
    const result = await invoke<any>('lock_item', {
      filePath: path,
      unlockTime,
      vault: vault || null,
      deleteOriginal: options?.deleteOriginal || false
    });
    return {
      success: true,
      keyPath: result.key_path,
      tlockPath: result.tlock_path,
      zipPath: result.archive_path,
      unlockTime: result.unlocks_at,
      originalDeleted: result.original_deleted || false,
      deletionError: result.deletion_error || undefined
    };
  } catch (error) {
    return {
      success: false,
      error: error instanceof Error ? error.message : String(error)
    };
  }
}

/**
 * Unlock a previously locked item using its key file (legacy format)
 * @param keyPath - Path to .key.md file
 */
export async function unlockItem(keyPath: string): Promise<UnlockResult> {
  try {
    const outputPath = await invoke<string>('unlock_item', {
      keyPath
    });
    return {
      success: true,
      outputPath
    };
  } catch (error) {
    return {
      success: false,
      error: error instanceof Error ? error.message : String(error)
    };
  }
}

/**
 * Unlock a .7z.tlock file (new unified format)
 * @param tlockPath - Path to .7z.tlock file
 * @param outputDir - Optional output directory (defaults to same directory as tlock file)
 */
export async function unlockTlockFile(tlockPath: string, outputDir?: string): Promise<UnlockResult> {
  try {
    const outputPath = await invoke<string>('unlock_tlock_file', {
      tlockPath,
      outputDir: outputDir || null
    });
    return {
      success: true,
      outputPath
    };
  } catch (error) {
    return {
      success: false,
      error: error instanceof Error ? error.message : String(error)
    };
  }
}

/**
 * Migrate a legacy .key.md + .7z file pair to the new .7z.tlock format
 * @param keyPath - Path to the legacy .key.md file
 * @param deleteOriginal - Whether to delete the original files after migration
 */
export async function migrateToTlock(keyPath: string, deleteOriginal?: boolean): Promise<MigrationResult> {
  try {
    const result = await invoke<any>('migrate_to_tlock', {
      keyPath,
      deleteOriginal: deleteOriginal || false
    });
    return {
      success: true,
      tlockPath: result.tlock_path
    };
  } catch (error) {
    return {
      success: false,
      error: error instanceof Error ? error.message : String(error)
    };
  }
}

/**
 * Check if a file is a legacy .key.md file
 */
export function isLegacyKeyFile(path: string): boolean {
  return path.toLowerCase().endsWith('.key.md');
}

/**
 * Check if a file is the new .7z.tlock format
 */
export function isTlockFile(path: string): boolean {
  return path.toLowerCase().endsWith('.7z.tlock') || path.toLowerCase().endsWith('.tlock');
}

/**
 * Get complete application state from backend
 * This is the single source of truth for all app state
 */
export async function getAppState(): Promise<{ settings: AppSettings; lockedItems: LockedItem[] }> {
  try {
    const state = await invoke<AppStateResponse>('get_app_state');
    return {
      settings: state.settings,
      lockedItems: state.locked_items.map(item => {
        // Use backend's is_legacy_format field, fallback to detection
        const keyPath = item.key_path || '';
        const tlockPath = item.tlock_path || '';
        const isLegacy = item.is_legacy_format ?? (keyPath.endsWith('.key.md') && !tlockPath);

        return {
          id: item.id,
          name: item.name,
          type: 'file' as const,
          keyPath: keyPath,
          tlockPath: tlockPath || undefined,
          zipPath: item.archive_path,
          created: new Date(item.created_at),
          unlocks: new Date(item.unlocks_at),
          isReady: item.is_unlockable || new Date(item.unlocks_at) <= new Date(),
          isLegacyFormat: isLegacy,
          unlockedPath: item.unlocked_path || undefined,
          metadata: item.metadata ? {
            version: item.metadata.version || '1.0',
            locked: item.metadata.locked ?? true,
            created: item.metadata.created || item.created_at,
            unlocks: item.metadata.unlocks || item.unlocks_at,
            duration: item.metadata.duration || '',
            originalFile: item.metadata.original_file || item.name,
            originalSize: item.metadata.original_size,
            compressedSize: item.metadata.compressed_size,
            fileCount: item.metadata.file_count
          } : undefined
        };
      })
    };
  } catch (error) {
    console.error('Failed to get app state:', error);
    return { settings: { vaults: [] }, lockedItems: [] };
  }
}

/**
 * Save application settings to backend
 */
export async function saveSettings(settings: AppSettings): Promise<boolean> {
  try {
    await invoke('save_settings', { settings });
    return true;
  } catch (error) {
    console.error('Failed to save settings:', error);
    return false;
  }
}

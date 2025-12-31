/**
 * Get an appropriate icon name for a file based on its extension
 * Returns icon names compatible with common icon libraries (e.g., lucide-svelte)
 * @param filename - Name of the file
 * @returns Icon name string
 */
export function getFileIcon(filename: string): string {
  const extension = filename.split('.').pop()?.toLowerCase() || '';

  // Document types
  if (['pdf'].includes(extension)) return 'file-text';
  if (['doc', 'docx', 'txt', 'rtf', 'odt'].includes(extension)) return 'file-text';

  // Spreadsheets
  if (['xls', 'xlsx', 'csv', 'ods'].includes(extension)) return 'table';

  // Presentations
  if (['ppt', 'pptx', 'odp'].includes(extension)) return 'presentation';

  // Images
  if (['jpg', 'jpeg', 'png', 'gif', 'bmp', 'svg', 'webp', 'ico'].includes(extension)) {
    return 'image';
  }

  // Videos
  if (['mp4', 'avi', 'mov', 'wmv', 'flv', 'mkv', 'webm'].includes(extension)) {
    return 'video';
  }

  // Audio
  if (['mp3', 'wav', 'flac', 'aac', 'ogg', 'm4a', 'wma'].includes(extension)) {
    return 'music';
  }

  // Archives
  if (['zip', 'rar', '7z', 'tar', 'gz', 'bz2', 'xz'].includes(extension)) {
    return 'archive';
  }

  // Code files
  if (['js', 'ts', 'jsx', 'tsx', 'py', 'java', 'c', 'cpp', 'cs', 'go', 'rs', 'php', 'rb', 'swift'].includes(extension)) {
    return 'code';
  }

  // Web files
  if (['html', 'css', 'scss', 'sass', 'less'].includes(extension)) {
    return 'globe';
  }

  // Config files
  if (['json', 'xml', 'yaml', 'yml', 'toml', 'ini', 'conf', 'config'].includes(extension)) {
    return 'settings';
  }

  // Default
  return 'file';
}

/**
 * Format file size in bytes to a human-readable string
 * @param bytes - File size in bytes
 * @returns Formatted string like "1.5 MB" or "234 KB"
 */
export function formatFileSize(bytes: number): string {
  if (bytes === 0) return '0 Bytes';
  if (bytes < 0) return 'Invalid size';

  const units = ['Bytes', 'KB', 'MB', 'GB', 'TB', 'PB'];
  const k = 1024;
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  const size = bytes / Math.pow(k, i);

  // Format with appropriate decimal places
  const formatted = i === 0 ? size.toString() : size.toFixed(2);

  return `${formatted} ${units[i]}`;
}

/**
 * Check if a path represents a folder
 * This is a simple heuristic based on the path string
 * @param path - File or folder path
 * @returns True if the path appears to be a folder
 */
export function isFolder(path: string): boolean {
  // Check if path ends with a directory separator
  if (path.endsWith('/') || path.endsWith('\\')) {
    return true;
  }

  // Check if path has no extension (simple heuristic)
  const lastPart = path.split(/[/\\]/).pop() || '';
  const hasExtension = lastPart.includes('.');

  // If no extension and doesn't look like a hidden file, assume it's a folder
  return !hasExtension && !lastPart.startsWith('.');
}

/**
 * Extract filename from a full path
 * @param path - Full file path
 * @returns Just the filename
 */
export function getFileName(path: string): string {
  return path.split(/[/\\]/).pop() || path;
}

/**
 * Extract directory path from a full file path
 * @param path - Full file path
 * @returns Directory path
 */
export function getDirectory(path: string): string {
  const parts = path.split(/[/\\]/);
  parts.pop();
  return parts.join('/') || '/';
}

/**
 * Get file extension
 * @param filename - Name of the file
 * @returns File extension without the dot, or empty string if no extension
 */
export function getExtension(filename: string): string {
  const parts = filename.split('.');
  return parts.length > 1 ? parts.pop()?.toLowerCase() || '' : '';
}

/**
 * Validate if a filename is safe (no special characters that could cause issues)
 * @param filename - Name to validate
 * @returns True if the filename is safe
 */
export function isValidFilename(filename: string): boolean {
  // Disallow empty names
  if (!filename || filename.trim().length === 0) return false;

  // Disallow reserved names on Windows
  const reserved = ['CON', 'PRN', 'AUX', 'NUL', 'COM1', 'COM2', 'COM3', 'COM4',
                    'COM5', 'COM6', 'COM7', 'COM8', 'COM9', 'LPT1', 'LPT2',
                    'LPT3', 'LPT4', 'LPT5', 'LPT6', 'LPT7', 'LPT8', 'LPT9'];
  const nameWithoutExt = filename.split('.')[0].toUpperCase();
  if (reserved.includes(nameWithoutExt)) return false;

  // Disallow invalid characters
  const invalidChars = /[<>:"|?*\x00-\x1f]/;
  if (invalidChars.test(filename)) return false;

  return true;
}

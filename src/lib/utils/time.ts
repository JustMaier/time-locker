/**
 * Format a duration in milliseconds to a human-readable string
 * @param ms - Duration in milliseconds
 * @returns Formatted string like "2d 5h 30m" or "45s"
 */
export function formatDuration(ms: number): string {
  if (ms < 0) return '0s';

  const seconds = Math.floor(ms / 1000);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);

  const parts: string[] = [];

  if (days > 0) {
    parts.push(`${days}d`);
  }
  if (hours % 24 > 0) {
    parts.push(`${hours % 24}h`);
  }
  if (minutes % 60 > 0) {
    parts.push(`${minutes % 60}m`);
  }
  if (seconds % 60 > 0 && parts.length === 0) {
    parts.push(`${seconds % 60}s`);
  }

  return parts.join(' ') || '0s';
}

/**
 * Format a countdown to an unlock date
 * @param unlockDate - The date when the item unlocks
 * @returns Live countdown string like "Unlocks in 2d 5h 30m" or "Ready to unlock!"
 */
export function formatCountdown(unlockDate: Date): string {
  const now = new Date();
  const timeUntilUnlock = unlockDate.getTime() - now.getTime();

  if (timeUntilUnlock <= 0) {
    return 'Ready to unlock!';
  }

  return `Unlocks in ${formatDuration(timeUntilUnlock)}`;
}

/**
 * Parse a duration string to milliseconds
 * Supports formats like: "5m", "2h", "1d", "30s", "1d 12h", "2h 30m 15s"
 * @param input - Duration string
 * @returns Duration in milliseconds
 */
export function parseDuration(input: string): number {
  const trimmed = input.trim().toLowerCase();
  let totalMs = 0;

  // Match patterns like "5m", "2h", "1d 12h 30m", etc.
  const patterns = [
    { regex: /(\d+)d/g, multiplier: 24 * 60 * 60 * 1000 }, // days
    { regex: /(\d+)h/g, multiplier: 60 * 60 * 1000 },      // hours
    { regex: /(\d+)m/g, multiplier: 60 * 1000 },           // minutes
    { regex: /(\d+)s/g, multiplier: 1000 }                 // seconds
  ];

  for (const { regex, multiplier } of patterns) {
    let match;
    while ((match = regex.exec(trimmed)) !== null) {
      totalMs += parseInt(match[1]) * multiplier;
    }
  }

  return totalMs;
}

/**
 * Check if an item is ready to be unlocked
 * @param unlockDate - The date when the item unlocks
 * @returns True if the current time is past the unlock time
 */
export function isUnlockReady(unlockDate: Date): boolean {
  return new Date() >= unlockDate;
}

/**
 * Calculate the unlock date from a duration string
 * @param duration - Duration string like "5m", "2h", etc.
 * @returns ISO 8601 timestamp string
 */
export function calculateUnlockTime(duration: string): string {
  const now = new Date();
  const durationMs = parseDuration(duration);
  const unlockDate = new Date(now.getTime() + durationMs);
  return unlockDate.toISOString();
}

/**
 * Format a date to a localized string
 * @param date - Date to format
 * @returns Formatted date string like "Dec 21, 2025, 3:45 PM"
 */
export function formatDate(date: Date): string {
  return new Intl.DateTimeFormat('en-US', {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
    hour12: true
  }).format(date);
}

/**
 * Get relative time string (e.g., "2 hours ago", "in 5 minutes")
 * @param date - Date to compare
 * @returns Relative time string
 */
export function getRelativeTime(date: Date): string {
  const now = new Date();
  const diffMs = date.getTime() - now.getTime();
  const absDiff = Math.abs(diffMs);

  const seconds = Math.floor(absDiff / 1000);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);

  const isPast = diffMs < 0;
  const suffix = isPast ? 'ago' : 'from now';

  if (days > 0) {
    return `${days} day${days > 1 ? 's' : ''} ${suffix}`;
  }
  if (hours > 0) {
    return `${hours} hour${hours > 1 ? 's' : ''} ${suffix}`;
  }
  if (minutes > 0) {
    return `${minutes} minute${minutes > 1 ? 's' : ''} ${suffix}`;
  }
  return `${seconds} second${seconds !== 1 ? 's' : ''} ${suffix}`;
}

<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import type { LockedItem } from '../stores/vault';

  export let item: LockedItem;
  export let onUnlock: (id: string) => void;

  let timeRemaining = '';
  let progress = 0;
  let isUnlockable = false;
  let interval: number;

  function updateCountdown() {
    const now = Date.now();
    const remaining = item.unlockAt - now;

    if (remaining <= 0) {
      isUnlockable = true;
      timeRemaining = 'Ready to unlock';
      progress = 100;
      return;
    }

    const total = item.unlockAt - item.lockedAt;
    progress = ((total - remaining) / total) * 100;

    const days = Math.floor(remaining / (1000 * 60 * 60 * 24));
    const hours = Math.floor((remaining % (1000 * 60 * 60 * 24)) / (1000 * 60 * 60));
    const minutes = Math.floor((remaining % (1000 * 60 * 60)) / (1000 * 60));
    const seconds = Math.floor((remaining % (1000 * 60)) / 1000);

    const parts = [];
    if (days > 0) parts.push(`${days}d`);
    if (hours > 0) parts.push(`${hours}h`);
    if (minutes > 0) parts.push(`${minutes}m`);
    if (seconds > 0 || parts.length === 0) parts.push(`${seconds}s`);

    timeRemaining = parts.join(' ');
  }

  onMount(() => {
    updateCountdown();
    interval = window.setInterval(updateCountdown, 1000);
  });

  onDestroy(() => {
    if (interval) clearInterval(interval);
  });

  function getFileIcon(type: string) {
    return type === 'folder' ? '📁' : '📄';
  }
</script>

<div class="item-card">
  <div class="item-header">
    <span class="item-icon">{getFileIcon(item.type)}</span>
    <div class="item-info">
      <h4 class="item-name">{item.name}</h4>
      <p class="item-path">{item.path}</p>
    </div>
  </div>

  <div class="progress-container">
    <div class="progress-bar" style="width: {progress}%"></div>
  </div>

  <div class="item-footer">
    <div class="timer" class:unlockable={isUnlockable}>
      {timeRemaining}
    </div>
    {#if isUnlockable}
      <button class="unlock-btn" on:click={() => onUnlock(item.id)}>
        🔓 Unlock
      </button>
    {/if}
  </div>
</div>

<style>
  .item-card {
    background: #1a1a1a;
    border: 1px solid #333;
    border-radius: 8px;
    padding: 1rem;
    transition: all 0.2s ease;
  }

  .item-card:hover {
    border-color: #444;
    background: #1f1f1f;
  }

  .item-header {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-bottom: 1rem;
  }

  .item-icon {
    font-size: 2rem;
  }

  .item-info {
    flex: 1;
    min-width: 0;
  }

  .item-name {
    font-size: 0.875rem;
    font-weight: 600;
    color: #fff;
    margin: 0 0 0.25rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .item-path {
    font-size: 0.75rem;
    color: #888;
    margin: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .progress-container {
    width: 100%;
    height: 4px;
    background: #2a2a2a;
    border-radius: 2px;
    overflow: hidden;
    margin-bottom: 1rem;
  }

  .progress-bar {
    height: 100%;
    background: linear-gradient(90deg, #6366f1, #8b5cf6);
    border-radius: 2px;
    transition: width 1s linear;
  }

  .item-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .timer {
    font-family: monospace;
    font-size: 0.875rem;
    color: #aaa;
    font-weight: 600;
  }

  .timer.unlockable {
    color: #22c55e;
  }

  .unlock-btn {
    padding: 0.5rem 1rem;
    background: #22c55e;
    color: #fff;
    border: none;
    border-radius: 6px;
    cursor: pointer;
    font-size: 0.875rem;
    font-weight: 600;
    transition: all 0.2s ease;
  }

  .unlock-btn:hover {
    background: #16a34a;
    transform: scale(1.05);
  }

  .unlock-btn:active {
    transform: scale(0.95);
  }
</style>
